//! Cross-platform text insertion into the focused application.
//!
//! Exposed as a library so it can be used both by the standalone binary
//! (used by the Electron app) and in-process by other crates (e.g. the
//! `ito-tray` app).

#[cfg(target_os = "linux")]
use enigo::{Enigo, Key, Keyboard, Settings};

#[cfg(target_os = "macos")]
pub mod macos_writer;
#[cfg(target_os = "macos")]
pub use macos_writer::type_text_macos;

#[cfg(target_os = "windows")]
pub mod windows_writer;
#[cfg(target_os = "windows")]
pub use windows_writer::type_text_windows;

/// Types `text` into the currently focused application.
/// `char_delay` is the per-character delay in milliseconds (ignored on
/// Windows, where text is inserted via clipboard paste).
pub fn type_text(text: &str, char_delay: u64) -> Result<(), String> {
    if text.is_empty() {
        return Err("Text cannot be empty".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        type_text_macos(text, char_delay)
    }

    #[cfg(target_os = "windows")]
    {
        type_text_windows(text, char_delay)
    }

    #[cfg(target_os = "linux")]
    {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Error initializing enigo: {}", e))?;

        if char_delay > 0 {
            for ch in text.chars() {
                enigo
                    .text(&ch.to_string())
                    .map_err(|e| format!("Error typing character '{}': {}", ch, e))?;
                std::thread::sleep(std::time::Duration::from_millis(char_delay));
            }
        } else {
            enigo
                .text(text)
                .map_err(|e| format!("Error typing text: {}", e))?;
        }

        // Patch fix: Send 'A' key release to clean up any phantom stuck KeyA
        // events. This addresses a bug where synthetic events from text typing
        // can cause the global key listener to receive keydown events without
        // corresponding keyup events
        if let Err(e) = enigo.key(Key::Unicode('a'), enigo::Direction::Release) {
            // Don't fail on this error since it's just a cleanup operation
            eprintln!("Warning: Failed to send cleanup 'a' key release: {}", e);
        }

        Ok(())
    }
}
