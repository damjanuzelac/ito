//! Ito — voice dictation through the Groq speech-to-text API.
//!
//! Hold the hotkey, speak, release: the transcript is typed into the focused
//! application. The whole app is one exe plus an `ito.ini` beside it; there is
//! no local model, no database and no server. For offline dictation use a
//! dedicated local tool instead (see README).
//!
//! Modes:
//! - default (Windows): tray application
//! - `--transcribe-file <wav>`: one-shot run against a WAV file, no microphone
//! - `--listen` (non-Windows): headless hotkey loop for development

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod groq;
mod input;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod tray;

use anyhow::{Context, Result};
use audio_recorder::{AudioConfigInfo, AudioSink, CaptureSession};
use crossbeam_channel::Receiver;
use global_key_listener::KeyListenerState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

const CONFIG_FILE: &str = "ito.ini";
const LOG_FILE: &str = "ito.log";
/// The log is trimmed once it passes this, so it never needs maintenance.
const MAX_LOG_BYTES: u64 = 256 * 1024;

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

/// Writes progress to stderr *and* to a file beside `ito.ini`.
///
/// The release build is a GUI-subsystem binary, so a copy that survives being
/// launched from Explorer is the only way the timings are ever readable.
pub struct Logger {
    path: Option<PathBuf>,
}

impl Logger {
    fn beside(config_path: &std::path::Path) -> Self {
        Self {
            path: config_path.parent().map(|dir| dir.join(LOG_FILE)),
        }
    }

    pub fn line(&self, message: &str) {
        eprintln!("[ito] {message}");
        let Some(path) = &self.path else {
            return;
        };
        if std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
            let _ = std::fs::remove_file(path);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{message}");
        }
    }
}

/// Reattaches stdout/stderr to the terminal the app was started from, when
/// there is one. Without this the GUI-subsystem release build is silent even
/// under `ito.exe --transcribe-file clip.wav`.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

fn default_language() -> String {
    "auto".to_string()
}

fn default_model() -> String {
    "whisper-large-v3-turbo".to_string()
}

/// Matches the Windows default of the original Ito app: Ctrl+Win.
fn default_hotkey() -> Vec<String> {
    vec!["ControlLeft".to_string(), "MetaLeft".to_string()]
}

fn default_microphone() -> String {
    "default".to_string()
}

fn default_min_speech_ms() -> u64 {
    300
}

fn default_min_rms() -> f32 {
    200.0
}

/// Whisper reliably invents these on silence or noise; typing them is worse
/// than typing nothing.
fn default_ignore_phrases() -> Vec<String> {
    vec![
        "Titlovi HRT".to_string(),
        "Thank you.".to_string(),
        "Thanks for watching!".to_string(),
        "you".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Groq API key. Without it the app records but cannot transcribe.
    pub groq_api_key: String,
    /// Spoken language: "auto" (Groq detects), "hr" or "en".
    pub language: String,
    /// Groq speech model. `whisper-large-v3-turbo` is the fast one.
    pub model: String,
    /// Hold this chord to dictate (raw rdev key names, e.g. "ControlLeft").
    pub hotkey: Vec<String>,
    /// Input device name, or "default".
    pub microphone: String,
    /// Custom vocabulary passed to the API as a prompt hint.
    pub dictionary: Vec<String>,
    /// Clips shorter than this are discarded without an API call.
    pub min_speech_ms: u64,
    /// Clips quieter than this RMS are discarded without an API call, which
    /// is what stops silence from being transcribed into a hallucination.
    pub min_rms: f32,
    /// Transcripts equal to one of these (case-insensitive) are dropped.
    pub ignore_phrases: Vec<String>,
    /// Insert text by pasting instead of synthesizing keystrokes. Slower and
    /// it borrows the clipboard, but some applications ignore synthetic
    /// Unicode input.
    pub type_via_clipboard: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            groq_api_key: String::new(),
            language: default_language(),
            model: default_model(),
            hotkey: default_hotkey(),
            microphone: default_microphone(),
            dictionary: Vec::new(),
            min_speech_ms: default_min_speech_ms(),
            min_rms: default_min_rms(),
            ignore_phrases: default_ignore_phrases(),
            type_via_clipboard: false,
        }
    }
}

impl Config {
    /// True when the language should be sent to the API rather than detected.
    pub fn explicit_language(&self) -> Option<&str> {
        match self.language.as_str() {
            "" | "auto" => None,
            language => Some(language),
        }
    }

    /// True when the transcript is a known hallucination rather than speech.
    pub fn is_ignored(&self, transcript: &str) -> bool {
        let normalized = transcript.trim().trim_end_matches(['.', '!', '?']);
        self.ignore_phrases.iter().any(|phrase| {
            phrase
                .trim()
                .trim_end_matches(['.', '!', '?'])
                .eq_ignore_ascii_case(normalized)
        })
    }
}

/// Where `ito.ini` is looked for, most specific first: beside the exe (so the
/// app stays portable — copy the exe and the ini anywhere) and then in the
/// per-user config directory.
fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        paths.push(dir.join(CONFIG_FILE));
    }
    if let Some(dir) = dirs::config_dir() {
        paths.push(dir.join("Ito").join(CONFIG_FILE));
    }
    paths
}

/// Loads the config, creating it with defaults on first run. Returns the file
/// it was read from so the tray can write back to the same place.
pub fn load_or_create_config() -> Result<(Config, PathBuf)> {
    let candidates = candidate_paths();

    for path in &candidates {
        if path.exists() {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let config: Config = toml::from_str(&raw)
                .with_context(|| format!("Invalid config at {}", path.display()))?;
            return Ok((config, path.clone()));
        }
    }

    // Nothing yet: write defaults to the first location that accepts them.
    // The exe may live somewhere unwritable (Program Files), hence the loop.
    let config = Config::default();
    let mut last_error = None;
    for path in &candidates {
        match save_config(&config, path) {
            Ok(()) => {
                eprintln!("[ito] Created default config at {}", path.display());
                return Ok((config, path.clone()));
            }
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No writable config location")))
}

pub fn save_config(config: &Config, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(config)?;
    std::fs::write(path, raw).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)] // Cancel/ReloadConfig/Quit are sent by the Windows tray UI
pub enum ControlMsg {
    Start,
    Complete,
    Cancel,
    ReloadConfig,
    Quit,
}

/// Collects the 16 kHz mono i16 output of the capture pipeline in memory.
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

struct ActiveRecording {
    capture: CaptureSession,
    sink: Arc<CollectSink>,
}

/// Runs the session loop until `Quit`. `status` receives human-readable state
/// updates (the tray tooltip and the recording bar are driven from them).
pub fn run_session_loop(
    rx: &Receiver<ControlMsg>,
    config_path: PathBuf,
    config: &Arc<RwLock<Config>>,
    status: impl Fn(String),
) {
    let logger = Logger::beside(&config_path);
    let agent = groq::build_agent();
    let host = audio_recorder::create_host();
    let mut active: Option<ActiveRecording> = None;

    if config.read().unwrap().groq_api_key.is_empty() {
        status("No groq_api_key in ito.ini".to_string());
        logger.line(&format!(
            "No groq_api_key set. Put it in {} and pick \"Reload ito.ini\".",
            config_path.display()
        ));
    } else {
        status("Ready".to_string());
    }

    while let Ok(msg) = rx.recv() {
        match msg {
            ControlMsg::Start => {
                if active.is_some() {
                    continue;
                }
                let cfg = config.read().unwrap().clone();
                // Open the TLS connection now, while the user is still
                // speaking, so the upload does not pay for a handshake.
                groq::prewarm(&agent, &cfg.groq_api_key);

                let sink = Arc::new(CollectSink::default());
                match audio_recorder::start_capture(
                    &host,
                    Some(cfg.microphone.as_str()),
                    Arc::clone(&sink) as Arc<dyn AudioSink>,
                ) {
                    Ok(capture) => {
                        active = Some(ActiveRecording { capture, sink });
                        status("Recording...".to_string());
                    }
                    Err(e) => {
                        logger.line(&format!("Failed to start capture: {e:#}"));
                        status(format!("Mic error: {e}"));
                    }
                }
            }
            ControlMsg::Complete => {
                let Some(recording) = active.take() else {
                    continue;
                };
                let released_at = Instant::now();
                // Blocks until the pipeline has flushed the last chunk.
                recording.capture.stop();
                let samples = std::mem::take(&mut *recording.sink.samples.lock().unwrap());

                let cfg = config.read().unwrap().clone();
                status("Transcribing...".to_string());
                match dictate(&agent, &cfg, &samples, released_at, &logger) {
                    Ok(()) => status("Ready".to_string()),
                    Err(e) => {
                        logger.line(&format!("{e:#}"));
                        status(format!("Error: {e}"));
                    }
                }
            }
            ControlMsg::Cancel => {
                if let Some(recording) = active.take() {
                    recording.capture.stop();
                }
                status("Ready".to_string());
            }
            ControlMsg::ReloadConfig => match load_or_create_config() {
                Ok((new_config, _)) => {
                    *config.write().unwrap() = new_config;
                    status("Config reloaded".to_string());
                }
                Err(e) => status(format!("Config error: {e}")),
            },
            ControlMsg::Quit => break,
        }
    }
}

/// One clip: gate, prepare, transcribe, insert. Logs where the time went.
///
/// `released_at` is when the hotkey came up, so the reported total is the
/// delay the user actually perceives.
fn dictate(
    agent: &ureq::Agent,
    config: &Config,
    samples: &[i16],
    released_at: Instant,
    logger: &Logger,
) -> Result<()> {
    let audio_ms = audio::duration_ms(samples.len(), audio::SAMPLE_RATE);
    if audio_ms < config.min_speech_ms {
        logger.line(&format!("Clip too short ({audio_ms} ms), skipping"));
        return Ok(());
    }
    let level = audio::rms(samples);
    if level < config.min_rms {
        logger.line(&format!(
            "Clip too quiet (rms {level:.0} < {:.0}), skipping",
            config.min_rms
        ));
        return Ok(());
    }

    let prep_start = Instant::now();
    let wav = audio::to_wav(&audio::enhance_pcm16(samples, audio::SAMPLE_RATE));
    let prep_ms = prep_start.elapsed().as_millis();

    let api_start = Instant::now();
    let transcript = groq::transcribe_wav(agent, config, &wav)?;
    let api_ms = api_start.elapsed().as_millis();

    if transcript.is_empty() {
        logger.line("Empty transcript, nothing to insert");
        return Ok(());
    }
    if config.is_ignored(&transcript) {
        logger.line(&format!("Ignored phrase {transcript:?}, nothing to insert"));
        return Ok(());
    }

    let type_start = Instant::now();
    input::insert_text(&transcript, config.type_via_clipboard)
        .map_err(|e| anyhow::anyhow!("Failed to insert text: {e}"))?;
    let type_ms = type_start.elapsed().as_millis();

    logger.line(&format!(
        "rec {:.1}s | prep {prep_ms}ms | groq {api_ms}ms | type {type_ms}ms | \
         total {}ms | {} chars",
        audio_ms as f64 / 1000.0,
        released_at.elapsed().as_millis(),
        transcript.chars().count()
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn main() {
    #[cfg(windows)]
    attach_parent_console();

    let args: Vec<String> = std::env::args().collect();

    if let Some(idx) = args.iter().position(|a| a == "--transcribe-file") {
        let Some(path) = args.get(idx + 1) else {
            eprintln!("Usage: ito --transcribe-file <wav>");
            std::process::exit(2);
        };
        if let Err(e) = transcribe_file(path) {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = run_app(args.iter().any(|a| a == "--listen")) {
        eprintln!("Fatal: {e:#}");
        std::process::exit(1);
    }
}

/// One-shot run against a WAV file. Exercises the whole path except the
/// microphone and the hotkey, which is what makes it useful for testing.
fn transcribe_file(path: &str) -> Result<()> {
    let (config, _) = load_or_create_config()?;
    let bytes = std::fs::read(path).with_context(|| format!("Failed to read {path}"))?;
    let (samples, rate) =
        audio::read_wav_pcm16(&bytes).map_err(|e| anyhow::anyhow!("Invalid WAV: {e}"))?;
    anyhow::ensure!(
        rate == audio::SAMPLE_RATE,
        "Expected {} Hz mono WAV, got {rate} Hz",
        audio::SAMPLE_RATE
    );

    let agent = groq::build_agent();
    let wav = audio::to_wav(&audio::enhance_pcm16(&samples, audio::SAMPLE_RATE));
    let started = Instant::now();
    let transcript = groq::transcribe_wav(&agent, &config, &wav)?;
    println!("{transcript}");
    eprintln!("[ito] groq {}ms", started.elapsed().as_millis());
    Ok(())
}

/// The full application: config, hotkey listener, session thread, and (on
/// Windows) the tray UI.
fn run_app(_headless_listen: bool) -> Result<()> {
    let (initial_config, config_path) = load_or_create_config()?;
    let config = Arc::new(RwLock::new(initial_config));
    let listener_state = Arc::new(KeyListenerState::new());
    let enabled = Arc::new(AtomicBool::new(true));
    let (session_tx, session_rx) = crossbeam_channel::unbounded::<ControlMsg>();

    input::register_hotkey(&listener_state, &config.read().unwrap(), true);
    input::spawn_listener(
        Arc::clone(&listener_state),
        Arc::clone(&config),
        Arc::clone(&enabled),
        session_tx.clone(),
    );

    #[cfg(windows)]
    {
        let session_config = Arc::clone(&config);
        let session_path = config_path.clone();
        tray::run(
            config,
            config_path,
            listener_state,
            enabled,
            session_tx,
            move |status| {
                std::thread::spawn(move || {
                    run_session_loop(&session_rx, session_path, &session_config, status);
                });
            },
        );
    }

    #[cfg(not(windows))]
    {
        if !_headless_listen {
            eprintln!(
                "ito: the tray UI is Windows-only. Use --listen for a headless hotkey loop\n\
                 or --transcribe-file <wav> for a one-shot run."
            );
            std::process::exit(2);
        }
        eprintln!("[ito] Headless mode: hold the configured hotkey to dictate. Ctrl+C to quit.");
        run_session_loop(&session_rx, config_path, &config, |status| {
            eprintln!("[ito] {status}");
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips_through_the_ini() {
        let config = Config::default();
        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.hotkey, vec!["ControlLeft", "MetaLeft"]);
        assert_eq!(parsed.model, "whisper-large-v3-turbo");
        assert_eq!(parsed.language, "auto");
        assert!(parsed.groq_api_key.is_empty());
        assert!(!parsed.type_via_clipboard);
    }

    #[test]
    fn partial_ini_fills_defaults() {
        let parsed: Config = toml::from_str("language = \"hr\"\n").unwrap();
        assert_eq!(parsed.language, "hr");
        assert_eq!(parsed.model, "whisper-large-v3-turbo");
        assert_eq!(parsed.hotkey, vec!["ControlLeft", "MetaLeft"]);
    }

    #[test]
    fn auto_language_is_not_sent_to_the_api() {
        let mut config = Config::default();
        assert_eq!(config.explicit_language(), None);
        config.language = "hr".to_string();
        assert_eq!(config.explicit_language(), Some("hr"));
        config.language = String::new();
        assert_eq!(config.explicit_language(), None);
    }

    #[test]
    fn hallucinations_are_ignored_regardless_of_punctuation_and_case() {
        let config = Config::default();
        assert!(config.is_ignored("Thank you."));
        assert!(config.is_ignored("  thank you  "));
        assert!(config.is_ignored("Titlovi HRT"));
        assert!(!config.is_ignored("Thank you for the coffee"));
        assert!(!config.is_ignored("Hello there"));
    }
}
