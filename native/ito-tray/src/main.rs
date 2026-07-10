//! Ito Tray — standalone local voice dictation.
//!
//! Hold the configured hotkey, speak, release: the transcript is typed into
//! the focused application. Transcription runs in-process (whisper.cpp), no
//! server or cloud required. See config.rs for configuration.
//!
//! Modes:
//! - default (Windows): tray application
//! - `--transcribe-file <wav>`: headless one-shot pipeline run (dev/testing)
//! - `--listen` (non-Windows): headless hotkey loop for development

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio_pipeline;
mod config;
mod groq;
mod history;
mod hotkeys;
mod model_download;
mod prompt;
mod session;
#[cfg(windows)]
mod tray;
mod whisper;

use anyhow::{Context, Result};
use config::Config;
use global_key_listener::KeyListenerState;
use prompt::ItoMode;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Some(idx) = args.iter().position(|a| a == "--transcribe-file") {
        let Some(path) = args.get(idx + 1) else {
            eprintln!("Usage: ito-tray --transcribe-file <wav>");
            std::process::exit(2);
        };
        match transcribe_file(path) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }

    if let Err(e) = run_app(args.iter().any(|a| a == "--listen")) {
        eprintln!("Fatal: {e:#}");
        std::process::exit(1);
    }
}

/// Headless one-shot pipeline: WAV file → enhance → whisper → mode detection.
fn transcribe_file(path: &str) -> Result<()> {
    let config = Config::load_or_create()?;
    let bytes = std::fs::read(path).with_context(|| format!("Failed to read {path}"))?;
    let (samples, rate) =
        audio_pipeline::read_wav_pcm16(&bytes).map_err(|e| anyhow::anyhow!("Invalid WAV: {e}"))?;

    let samples = if rate == audio_pipeline::SAMPLE_RATE {
        samples
    } else {
        eprintln!("[ito-tray] Resampling {rate} Hz → 16000 Hz");
        resample_linear_i16(&samples, rate, audio_pipeline::SAMPLE_RATE)
    };

    let engine = session::prepare_engine(&config, &|status| eprintln!("[ito-tray] {status}"))?;

    let context = prompt::WindowContext::default();
    match session::process_audio(&engine, &config, &samples, ItoMode::Transcribe, &context)? {
        Some((text_to_insert, transcript, llm_output)) => {
            println!("transcript: {transcript}");
            if let Some(llm) = llm_output {
                println!("llm_output: {llm}");
            }
            println!("inserted_text: {text_to_insert}");
        }
        None => println!("(no speech detected)"),
    }
    Ok(())
}

fn resample_linear_i16(input: &[i16], in_rate: u32, out_rate: u32) -> Vec<i16> {
    if input.is_empty() || in_rate == out_rate {
        return input.to_vec();
    }
    let out_len = (input.len() as u64 * out_rate as u64 / in_rate as u64) as usize;
    let step = in_rate as f64 / out_rate as f64;
    let mut out = Vec::with_capacity(out_len);
    let mut pos = 0.0f64;
    for _ in 0..out_len {
        let idx = pos as usize;
        let sample = if idx + 1 >= input.len() {
            *input.last().unwrap()
        } else {
            let frac = (pos - idx as f64) as f32;
            let a = input[idx] as f32;
            let b = input[idx + 1] as f32;
            (a + (b - a) * frac) as i16
        };
        out.push(sample);
        pos += step;
    }
    out
}

/// The full application: config, hotkey listener, session thread, and (on
/// Windows) the tray UI.
fn run_app(_headless_listen: bool) -> Result<()> {
    let config = Arc::new(RwLock::new(Config::load_or_create()?));
    let listener_state = Arc::new(KeyListenerState::new());
    let enabled = Arc::new(AtomicBool::new(true));
    let (session_tx, session_rx) = crossbeam_channel::unbounded::<session::ControlMsg>();

    hotkeys::register_config_hotkeys(&listener_state, &config.read().unwrap(), true);
    hotkeys::spawn_listener(
        Arc::clone(&listener_state),
        Arc::clone(&config),
        Arc::clone(&enabled),
        session_tx.clone(),
    );

    #[cfg(windows)]
    {
        let session_config = Arc::clone(&config);
        tray::run(
            Arc::clone(&config),
            listener_state,
            enabled,
            session_tx,
            move |status| {
                std::thread::spawn(move || {
                    session::run_session_loop(session_rx, session_config, move |s| status(s));
                });
            },
        );
    }

    #[cfg(not(windows))]
    {
        if !_headless_listen {
            eprintln!(
                "ito-tray: the tray UI is Windows-only. Use --listen for a headless hotkey loop\n\
                 or --transcribe-file <wav> for a one-shot pipeline run."
            );
            std::process::exit(2);
        }
        eprintln!(
            "[ito-tray] Headless mode: hold the configured hotkey to dictate. Ctrl+C to quit."
        );
        session::run_session_loop(session_rx, config, |status| {
            eprintln!("[ito-tray] {status}");
        });
        Ok(())
    }
}
