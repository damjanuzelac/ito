//! TOML configuration in the per-user data directory
//! (Windows: %APPDATA%\Ito-tray\config.toml).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default hotkeys match the Windows defaults of the Electron app
/// (lib/constants/keyboard-defaults.ts): Ctrl+Win for transcribe,
/// Ctrl+Alt for edit. Keys are raw rdev key names.
fn default_hotkey_transcribe() -> Vec<String> {
    vec!["ControlLeft".to_string(), "MetaLeft".to_string()]
}

fn default_hotkey_edit() -> Vec<String> {
    vec!["Alt".to_string(), "ControlLeft".to_string()]
}

fn default_microphone() -> String {
    "default".to_string()
}

fn default_model() -> String {
    "small".to_string()
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_no_speech_threshold() -> f32 {
    0.6
}

fn default_groq_model() -> String {
    "openai/gpt-oss-120b".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Hold this chord to dictate (raw rdev key names, e.g. "ControlLeft").
    pub hotkey_transcribe: Vec<String>,
    /// Hold this chord for edit mode ("hey ito" commands need no prefix).
    pub hotkey_edit: Vec<String>,
    /// Input device name, or "default".
    pub microphone: String,
    /// Whisper model: tiny | base | small | medium | large-v3 (ggml name suffix).
    pub model: String,
    /// Spoken language ("auto" for detection, or e.g. "en", "hr").
    pub language: String,
    /// Custom vocabulary fed to the transcription prompt.
    pub dictionary: Vec<String>,
    /// Segments with no_speech_prob above this are treated as silence.
    pub no_speech_threshold: f32,
    /// Optional Groq API key; enables edit mode ("hey ito" commands).
    pub groq_api_key: String,
    /// LLM used for edit mode.
    pub groq_model: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey_transcribe: default_hotkey_transcribe(),
            hotkey_edit: default_hotkey_edit(),
            microphone: default_microphone(),
            model: default_model(),
            language: default_language(),
            dictionary: Vec::new(),
            no_speech_threshold: default_no_speech_threshold(),
            groq_api_key: String::new(),
            groq_model: default_groq_model(),
        }
    }
}

/// Base data directory: %APPDATA%\Ito-tray on Windows,
/// ~/.config/Ito-tray elsewhere.
pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("Could not determine the user config directory")?;
    Ok(base.join("Ito-tray"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("config.toml"))
}

pub fn models_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("models"))
}

pub fn history_db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("ito-tray.db"))
}

impl Config {
    /// Loads the config, creating it with defaults on first run.
    pub fn load_or_create() -> Result<Self> {
        let path = config_path()?;
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let config: Config = toml::from_str(&raw)
                .with_context(|| format!("Invalid config at {}", path.display()))?;
            Ok(config)
        } else {
            let config = Config::default();
            config.save()?;
            eprintln!("[ito-tray] Created default config at {}", path.display());
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(&path, raw)?;
        Ok(())
    }

    /// The ggml model file name for the configured model size.
    pub fn model_file_name(&self) -> String {
        format!("ggml-{}.bin", self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let config = Config::default();
        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.hotkey_transcribe, vec!["ControlLeft", "MetaLeft"]);
        assert_eq!(parsed.hotkey_edit, vec!["Alt", "ControlLeft"]);
        assert_eq!(parsed.model, "small");
        assert_eq!(parsed.model_file_name(), "ggml-small.bin");
        assert!((parsed.no_speech_threshold - 0.6).abs() < f32::EPSILON);
        assert!(parsed.groq_api_key.is_empty());
    }

    #[test]
    fn partial_config_fills_defaults() {
        let parsed: Config = toml::from_str("model = \"tiny\"\n").unwrap();
        assert_eq!(parsed.model, "tiny");
        assert_eq!(parsed.language, "auto");
        assert_eq!(parsed.hotkey_transcribe, vec!["ControlLeft", "MetaLeft"]);
    }
}
