//! Embedded Whisper transcription via whisper-rs (whisper.cpp bindings).

use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

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

        let resolved_language = if language == "auto" {
            self.detect_language(&audio, auto_languages, threads)
                .unwrap_or_else(|| "auto".to_string())
        } else {
            language.to_string()
        };

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(threads as std::os::raw::c_int);
        params.set_language(Some(&resolved_language));
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
            .full(params, &audio)
            .context("Whisper inference failed")?;

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

    /// Restricted language detection: runs whisper's language auto-detect but
    /// only considers `candidates`, returning the one with the highest
    /// probability. Returns `None` (→ fall back to full auto-detect) when the
    /// list is empty or detection fails.
    fn detect_language(
        &self,
        audio: &[f32],
        candidates: &[String],
        threads: usize,
    ) -> Option<String> {
        match candidates {
            [] => return None,
            [only] => return Some(only.clone()),
            _ => {}
        }

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
            .map(|(lang, prob)| {
                eprintln!("[ito-tray] Detected language: {lang} (p = {prob:.3})");
                lang
            })
    }
}
