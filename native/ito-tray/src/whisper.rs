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

    /// Transcribes 16 kHz mono i16 PCM. `language` of "auto" enables
    /// detection. `initial_prompt` carries the vocabulary hint.
    pub fn transcribe(
        &self,
        pcm: &[i16],
        language: &str,
        initial_prompt: &str,
        no_speech_threshold: f32,
    ) -> Result<TranscribeOutcome> {
        let mut audio = vec![0.0f32; pcm.len()];
        whisper_rs::convert_integer_to_float_audio(pcm, &mut audio)
            .context("PCM conversion failed")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
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
}
