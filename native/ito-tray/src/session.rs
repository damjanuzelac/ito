//! Dictation session state machine: idle → recording → transcribing → typing.
//!
//! Owns the audio capture, the Whisper engine and the history database on a
//! single thread (cpal streams are not Send).

use crate::audio_pipeline::{duration_ms, enhance_pcm16, SAMPLE_RATE};
use crate::config::{self, Config};
use crate::groq;
use crate::history::History;
use crate::model_download::ensure_model;
use crate::prompt::{
    create_transcription_prompt, detect_ito_mode, full_vocabulary, ItoMode, WindowContext,
};
use crate::whisper::{TranscribeOutcome, WhisperEngine};
use anyhow::Result;
use audio_recorder::{AudioConfigInfo, AudioSink, CaptureSession};
use crossbeam_channel::Receiver;
use std::sync::{Arc, Mutex, RwLock};

/// Sessions shorter than this are discarded (same gate as the Electron app).
pub const MIN_AUDIO_DURATION_MS: u64 = 100;

#[derive(Debug)]
#[allow(dead_code)] // Cancel/ReloadConfig/Quit are constructed by the Windows tray UI
pub enum ControlMsg {
    Start(ItoMode),
    Complete,
    Cancel,
    ReloadConfig,
    Quit,
}

/// Collects the 16 kHz mono i16 output of the audio pipeline in memory.
#[derive(Default)]
struct CollectSink {
    samples: Mutex<Vec<i16>>,
}

impl AudioSink for CollectSink {
    fn on_config(&self, _config: AudioConfigInfo) {}

    fn on_chunk(&self, pcm: &[i16]) {
        self.samples.lock().unwrap().extend_from_slice(pcm);
    }

    fn on_drain_complete(&self) {}
}

fn capture_window_context() -> WindowContext {
    match active_win_pos_rs::get_active_window() {
        Ok(window) => WindowContext {
            window_title: window.title,
            app_name: window.app_name,
            context_text: String::new(),
        },
        Err(_) => WindowContext::default(),
    }
}

struct ActiveRecording {
    capture: CaptureSession,
    sink: Arc<CollectSink>,
    mode: ItoMode,
    window_context: WindowContext,
}

/// Prepares the Whisper engine (downloading the model on first run) and
/// reports progress through `status`.
pub fn prepare_engine(config: &Config, status: &dyn Fn(String)) -> Result<WhisperEngine> {
    let models_dir = config::models_dir()?;
    let model_file = config.model_file_name();
    let model_path = ensure_model(&models_dir, &model_file, &|downloaded, total| {
        if total > 0 {
            status(format!(
                "Downloading model {model_file}: {}%",
                downloaded * 100 / total
            ));
        } else {
            status(format!(
                "Downloading model {model_file}: {} MB",
                downloaded / (1024 * 1024)
            ));
        }
    })?;
    status(format!("Loading model {model_file}..."));
    let engine = WhisperEngine::load(&model_path)?;
    Ok(engine)
}

/// Transcribes prepared audio and, for edit mode, runs the Groq adjustment.
/// Returns (text to insert, raw transcript, optional LLM output).
pub fn process_audio(
    engine: &WhisperEngine,
    config: &Config,
    samples: &[i16],
    requested_mode: ItoMode,
    window_context: &WindowContext,
) -> Result<Option<(String, String, Option<String>)>> {
    let enhanced = enhance_pcm16(samples, SAMPLE_RATE);
    let vocabulary = full_vocabulary(&config.dictionary);
    let prompt = create_transcription_prompt(&vocabulary);

    let outcome = engine.transcribe(
        &enhanced,
        &config.language,
        &prompt,
        config.no_speech_threshold,
    )?;

    let transcript = match outcome {
        TranscribeOutcome::NoSpeech(prob) => {
            eprintln!("[ito-tray] No speech detected (no_speech_prob={prob:.2})");
            return Ok(None);
        }
        TranscribeOutcome::Text(text) if text.is_empty() => return Ok(None),
        TranscribeOutcome::Text(text) => text,
    };

    // The chord decides the mode; "hey ito" in a plain dictation also
    // triggers edit mode (same fallback as the server).
    let mode = match requested_mode {
        ItoMode::Edit => ItoMode::Edit,
        ItoMode::Transcribe => detect_ito_mode(&transcript),
    };

    if mode == ItoMode::Edit {
        if config.groq_api_key.is_empty() {
            eprintln!(
                "[ito-tray] Edit mode requested but no groq_api_key configured; inserting raw transcript"
            );
        } else {
            let adjusted = groq::adjust_transcript(
                &config.groq_api_key,
                &config.groq_model,
                &transcript,
                window_context,
            );
            return Ok(Some((adjusted.clone(), transcript, Some(adjusted))));
        }
    }

    Ok(Some((transcript.clone(), transcript, None)))
}

/// Runs the session loop until `Quit`. `status` receives human-readable state
/// updates (shown in the tray tooltip on Windows).
pub fn run_session_loop(
    rx: Receiver<ControlMsg>,
    config: Arc<RwLock<Config>>,
    status: impl Fn(String),
) {
    let engine = {
        let cfg = config.read().unwrap().clone();
        match prepare_engine(&cfg, &|s| status(s)) {
            Ok(engine) => engine,
            Err(e) => {
                status(format!("Model error: {e}"));
                eprintln!("[ito-tray] FATAL: {e:#}");
                return;
            }
        }
    };

    let history = match config::history_db_path().and_then(|p| History::open(&p)) {
        Ok(history) => Some(history),
        Err(e) => {
            eprintln!("[ito-tray] History disabled: {e:#}");
            None
        }
    };

    let host = audio_recorder::create_host();
    let mut active: Option<ActiveRecording> = None;
    status("Ready".to_string());

    while let Ok(msg) = rx.recv() {
        match msg {
            ControlMsg::Start(mode) => {
                if active.is_some() {
                    continue;
                }
                let window_context = capture_window_context();
                let sink = Arc::new(CollectSink::default());
                let microphone = config.read().unwrap().microphone.clone();
                match audio_recorder::start_capture(
                    &host,
                    Some(microphone.as_str()),
                    Arc::clone(&sink) as Arc<dyn AudioSink>,
                ) {
                    Ok(capture) => {
                        active = Some(ActiveRecording {
                            capture,
                            sink,
                            mode,
                            window_context,
                        });
                        status("Recording...".to_string());
                    }
                    Err(e) => {
                        eprintln!("[ito-tray] Failed to start capture: {e:#}");
                        status(format!("Mic error: {e}"));
                    }
                }
            }
            ControlMsg::Complete => {
                let Some(recording) = active.take() else {
                    continue;
                };
                // Blocks until the pipeline has flushed the last chunk
                recording.capture.stop();
                let samples = std::mem::take(&mut *recording.sink.samples.lock().unwrap());

                let audio_ms = duration_ms(samples.len(), SAMPLE_RATE);
                if audio_ms < MIN_AUDIO_DURATION_MS {
                    eprintln!("[ito-tray] Audio too short ({audio_ms} ms), skipping");
                    status("Ready".to_string());
                    continue;
                }

                status("Transcribing...".to_string());
                let cfg = config.read().unwrap().clone();
                match process_audio(
                    &engine,
                    &cfg,
                    &samples,
                    recording.mode,
                    &recording.window_context,
                ) {
                    Ok(Some((text_to_insert, transcript, llm_output))) => {
                        if let Err(e) = text_writer::type_text(&text_to_insert, 0) {
                            eprintln!("[ito-tray] Failed to insert text: {e}");
                        }
                        if let Some(history) = &history {
                            if let Err(e) = history.insert_interaction(
                                &transcript,
                                llm_output.as_deref(),
                                audio_ms,
                                SAMPLE_RATE,
                            ) {
                                eprintln!("[ito-tray] Failed to record history: {e:#}");
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("[ito-tray] Transcription failed: {e:#}");
                        status(format!("Error: {e}"));
                        continue;
                    }
                }
                status("Ready".to_string());
            }
            ControlMsg::Cancel => {
                if let Some(recording) = active.take() {
                    recording.capture.stop();
                }
                status("Ready".to_string());
            }
            ControlMsg::ReloadConfig => {
                match Config::load_or_create() {
                    Ok(new_config) => {
                        *config.write().unwrap() = new_config;
                        status("Config reloaded".to_string());
                    }
                    Err(e) => status(format!("Config error: {e}")),
                }
                // Note: a model change requires an app restart; the engine is
                // loaded once at startup.
            }
            ControlMsg::Quit => break,
        }
    }
}
