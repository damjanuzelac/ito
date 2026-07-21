//! Embedded Whisper transcription via whisper-rs (whisper.cpp bindings).

use anyhow::{Context, Result};
use std::os::raw::c_int;
use std::path::Path;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

#[derive(Debug)]
pub enum TranscribeOutcome {
    Text(String),
    /// The first segment's no_speech_prob exceeded the threshold.
    NoSpeech(f32),
}

pub struct WhisperEngine {
    ctx: WhisperContext,
}

impl WhisperEngine {
    pub fn load(model_path: &Path) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .context("Model path is not valid UTF-8")?,
            WhisperContextParameters::default(),
        )
        .with_context(|| format!("Failed to load Whisper model {}", model_path.display()))?;
        Ok(Self { ctx })
    }

    /// Transcribes 16 kHz mono i16 PCM. `language` of "auto" restricts
    /// detection to `auto_languages` (falling back to full auto-detection when
    /// that list is empty). `initial_prompt` carries the vocabulary hint.
    pub fn transcribe(
        &self,
        pcm: &[i16],
        language: &str,
        auto_languages: &[String],
        initial_prompt: &str,
        no_speech_threshold: f32,
    ) -> Result<TranscribeOutcome> {
        let mut audio = vec![0.0f32; pcm.len()];
        whisper_rs::convert_integer_to_float_audio(pcm, &mut audio)
            .context("PCM conversion failed")?;

        // whisper.cpp defaults to 4 threads; use the physical cores available
        // (capped) so short clips transcribe in a couple of seconds rather than
        // grinding a single-digit thread count.
        let threads = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(4)
            .min(8);

        // Single pass: let whisper auto-detect (or use the forced language).
        let first_language = if language == "auto" { "auto" } else { language };
        let mut state = self.run_full(&audio, first_language, initial_prompt, threads)?;

        // When auto-detecting against a candidate list, only re-run if whisper
        // picked a language outside it (e.g. a short clip mis-detected as some
        // unrelated third language). The common case stays a single pass.
        if language == "auto" && auto_languages.len() >= 2 {
            let detected_id = state.full_lang_id_from_state();
            let is_candidate = auto_languages
                .iter()
                .any(|lang| whisper_rs::get_lang_id(lang) == Some(detected_id));
            if !is_candidate {
                if let Some(best) = self.best_candidate(&audio, auto_languages, threads) {
                    eprintln!(
                        "[ito-tray] Auto-detect picked a non-candidate; re-running as {best}"
                    );
                    state = self.run_full(&audio, &best, initial_prompt, threads)?;
                }
            }
        }

        // Gate on the first segment's no-speech probability, matching the
        // behavior of the server's ASR clients.
        if let Some(first) = state.get_segment(0) {
            let no_speech_prob = first.no_speech_probability();
            if no_speech_prob > no_speech_threshold {
                return Ok(TranscribeOutcome::NoSpeech(no_speech_prob));
            }
        }

        let mut text = String::new();
        for i in 0..state.full_n_segments() {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(segment_text) = segment.to_str_lossy() {
                    text.push_str(&segment_text);
                }
            }
        }

        Ok(TranscribeOutcome::Text(text.trim().to_string()))
    }

    /// Detects which of `candidates` is spoken in the clip. Used to tell a
    /// remote provider the language explicitly instead of letting it guess.
    pub fn detect_language(&self, pcm: &[i16], candidates: &[String]) -> Option<String> {
        let mut audio = vec![0.0f32; pcm.len()];
        whisper_rs::convert_integer_to_float_audio(pcm, &mut audio).ok()?;
        let threads = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(4)
            .min(8);
        self.best_candidate(&audio, candidates, threads)
    }

    /// Runs one whisper inference pass with the given (possibly "auto")
    /// language, returning the resulting state for segment extraction.
    fn run_full(
        &self,
        audio: &[f32],
        language: &str,
        initial_prompt: &str,
        threads: usize,
    ) -> Result<WhisperState> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(threads as c_int);
        params.set_language(Some(language));
        if !initial_prompt.is_empty() {
            params.set_initial_prompt(initial_prompt);
        }
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);

        let mut state = self.ctx.create_state().context("Failed to create state")?;
        state
            .full(params, audio)
            .context("Whisper inference failed")?;
        Ok(state)
    }

    /// Picks the highest-probability language among `candidates` using
    /// whisper's language detector. Only used on the rare fallback path.
    fn best_candidate(
        &self,
        audio: &[f32],
        candidates: &[String],
        threads: usize,
    ) -> Option<String> {
        let mut state = self.ctx.create_state().ok()?;
        state.pcm_to_mel(audio, threads).ok()?;
        let (_, probs) = state.lang_detect(0, threads).ok()?;

        candidates
            .iter()
            .filter_map(|lang| {
                let id = whisper_rs::get_lang_id(lang)? as usize;
                probs.get(id).map(|&p| (lang.clone(), p))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(lang, _)| lang)
    }
}
