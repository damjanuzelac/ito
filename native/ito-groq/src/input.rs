//! Keyboard in and keyboard out.
//!
//! In: exact-chord matching on top of the shared global-key-listener, driving
//! the session. Rules ported from the Electron app (lib/media/keyboard.ts):
//! - a chord matches when the pressed set contains exactly its keys
//!   (order-independent, same size);
//! - a full match while idle starts a session (hold-to-talk);
//! - releasing any chord key completes the session;
//! - keys stuck for more than 5 s are purged (unless part of the active chord).
//!
//! Out: the transcript is injected as synthetic Unicode keystrokes. The
//! obvious alternative — copy to the clipboard and send Ctrl+V — is available
//! as a fallback, but it is not the default: it takes the user's clipboard
//! away and needs a delay before it can be handed back.

use crate::{Config, ControlMsg};
use crossbeam_channel::Sender;
use global_key_listener::{normalize_key_name, HotkeyCombo, KeyListenerState, ListenerEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const STUCK_KEY_TIMEOUT: Duration = Duration::from_secs(5);

/// Registers the configured chord on the shared listener state so it is
/// suppressed from reaching the focused application. Clears the registration
/// when listening is disabled.
pub fn register_hotkey(state: &KeyListenerState, config: &Config, enabled: bool) {
    if !enabled {
        state.register_hotkeys(Vec::new());
        return;
    }
    state.register_hotkeys(vec![HotkeyCombo {
        keys: config.hotkey.clone(),
    }]);
}

struct MatcherState {
    pressed: HashMap<String, Instant>,
    chord_active: bool,
}

fn chord_matches(chord: &[String], pressed: &HashMap<String, Instant>) -> bool {
    !chord.is_empty()
        && chord.len() == pressed.len()
        && chord
            .iter()
            .all(|key| pressed.contains_key(&normalize_key_name(key)))
}

/// The event handler installed into the grab loop. Kept separate from the
/// thread spawn so the chord logic is unit-testable.
pub fn make_key_event_handler(
    config: Arc<RwLock<Config>>,
    enabled: Arc<AtomicBool>,
    tx: Sender<ControlMsg>,
) -> impl Fn(ListenerEvent) + Send + Sync + 'static {
    let matcher = Mutex::new(MatcherState {
        pressed: HashMap::new(),
        chord_active: false,
    });

    move |event: ListenerEvent| {
        let mut state = matcher.lock().unwrap();
        let key = normalize_key_name(&event.key);
        let now = Instant::now();
        let chord = config.read().unwrap().hotkey.clone();

        match event.event_type {
            "keydown" => {
                // Stuck-key watchdog: purge keys held implausibly long, unless
                // they belong to the chord currently being held.
                let chord_active = state.chord_active;
                state.pressed.retain(|pressed_key, since| {
                    now.duration_since(*since) < STUCK_KEY_TIMEOUT
                        || (chord_active
                            && chord.iter().any(|k| normalize_key_name(k) == *pressed_key))
                });

                state.pressed.entry(key).or_insert(now);

                if !enabled.load(Ordering::SeqCst) || state.chord_active {
                    return;
                }
                if chord_matches(&chord, &state.pressed) {
                    state.chord_active = true;
                    let _ = tx.send(ControlMsg::Start);
                }
            }
            "keyup" => {
                state.pressed.remove(&key);

                if state.chord_active {
                    let still_held = chord
                        .iter()
                        .all(|k| state.pressed.contains_key(&normalize_key_name(k)));
                    if !still_held {
                        state.chord_active = false;
                        let _ = tx.send(ControlMsg::Complete);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Spawns the global grab loop on its own thread.
pub fn spawn_listener(
    listener_state: Arc<KeyListenerState>,
    config: Arc<RwLock<Config>>,
    enabled: Arc<AtomicBool>,
    tx: Sender<ControlMsg>,
) {
    let handler = make_key_event_handler(config, enabled, tx);
    std::thread::spawn(move || {
        if let Err(e) = global_key_listener::run_grab(listener_state, handler) {
            eprintln!("[ito] Keyboard grab failed: {e:?}");
        }
    });
}

// ---------------------------------------------------------------------------
// Text insertion
// ---------------------------------------------------------------------------

/// Types `text` into the focused application.
pub fn insert_text(text: &str, via_clipboard: bool) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        if via_clipboard {
            windows_input::paste(text)
        } else {
            windows_input::type_unicode(text)
        }
    }
    #[cfg(not(windows))]
    {
        // The headless dev mode has no focused application to type into.
        let _ = via_clipboard;
        println!("{text}");
        Ok(())
    }
}

#[cfg(windows)]
mod windows_input {
    use std::mem::size_of;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };

    /// Events per `SendInput` call. One call for the whole transcript would
    /// also work, but batching keeps a long dictation from arriving as a
    /// single oversized burst that some applications drop part of.
    const BATCH: usize = 256;

    fn keyboard_event(virtual_key: VIRTUAL_KEY, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// A synthetic press+release of one UTF-16 code unit. `wVk` must be zero
    /// for Unicode injection; the character travels in `wScan`.
    fn unicode_pair(unit: u16) -> [INPUT; 2] {
        [
            keyboard_event(0, unit, KEYEVENTF_UNICODE),
            keyboard_event(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ]
    }

    fn send(events: &[INPUT]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let sent = unsafe {
            SendInput(
                events.len() as u32,
                events.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent as usize == events.len() {
            Ok(())
        } else {
            Err(format!(
                "SendInput accepted {sent} of {} events",
                events.len()
            ))
        }
    }

    /// Types the text directly. No clipboard involved, no delays.
    ///
    /// `encode_utf16` handles characters outside the BMP by emitting a
    /// surrogate pair, and Windows reassembles the two events into one
    /// character, so this is not limited to Latin or Croatian text.
    pub fn type_unicode(text: &str) -> Result<(), String> {
        let mut batch: Vec<INPUT> = Vec::with_capacity(BATCH);
        for unit in text.encode_utf16() {
            batch.extend_from_slice(&unicode_pair(unit));
            if batch.len() >= BATCH {
                send(&batch)?;
                batch.clear();
            }
        }
        send(&batch)
    }

    /// Fallback for applications that ignore synthetic Unicode input: put the
    /// text on the clipboard and send Ctrl+V.
    ///
    /// The previous clipboard contents are restored from a background thread.
    /// The delay is unavoidable — paste it back too early and the application
    /// pastes the wrong thing — but it must not be paid on the dictation path.
    pub fn paste(text: &str) -> Result<(), String> {
        use clipboard_win::{formats, get_clipboard, set_clipboard};

        let previous: Option<String> = get_clipboard(formats::Unicode).ok();
        set_clipboard(formats::Unicode, text)
            .map_err(|e| format!("Failed to set clipboard: {e:?}"))?;

        send(&[
            keyboard_event(VK_CONTROL, 0, 0),
            keyboard_event(VK_V, 0, 0),
            keyboard_event(VK_V, 0, KEYEVENTF_KEYUP),
            keyboard_event(VK_CONTROL, 0, KEYEVENTF_KEYUP),
        ])?;

        if let Some(previous) = previous {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let _ = set_clipboard(formats::Unicode, &previous);
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    fn event(event_type: &'static str, key: &str) -> ListenerEvent {
        ListenerEvent {
            event_type,
            key: key.to_string(),
            raw_code: None,
        }
    }

    fn handler_with_defaults() -> (
        impl Fn(ListenerEvent),
        crossbeam_channel::Receiver<ControlMsg>,
    ) {
        let config = Arc::new(RwLock::new(Config::default()));
        let enabled = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded();
        (make_key_event_handler(config, enabled, tx), rx)
    }

    #[test]
    fn hold_and_release_the_chord() {
        let (handler, rx) = handler_with_defaults();

        handler(event("keydown", "ControlLeft"));
        assert!(rx.try_recv().is_err(), "partial chord must not start");
        handler(event("keydown", "MetaLeft"));
        assert!(matches!(rx.try_recv().unwrap(), ControlMsg::Start));

        handler(event("keyup", "MetaLeft"));
        assert!(matches!(rx.try_recv().unwrap(), ControlMsg::Complete));
        handler(event("keyup", "ControlLeft"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn extra_key_prevents_match() {
        let (handler, rx) = handler_with_defaults();

        handler(event("keydown", "ControlLeft"));
        handler(event("keydown", "KeyA"));
        handler(event("keydown", "MetaLeft"));
        assert!(rx.try_recv().is_err(), "chord + extra key must not match");
    }

    #[test]
    fn disabled_listener_never_starts() {
        let config = Arc::new(RwLock::new(Config::default()));
        let enabled = Arc::new(AtomicBool::new(false));
        let (tx, rx) = unbounded();
        let handler = make_key_event_handler(config, enabled, tx);

        handler(event("keydown", "ControlLeft"));
        handler(event("keydown", "MetaLeft"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_custom_chord_from_the_ini_is_honoured() {
        let config = Arc::new(RwLock::new(Config {
            hotkey: vec!["Alt".to_string(), "ShiftLeft".to_string()],
            ..Config::default()
        }));
        let enabled = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded();
        let handler = make_key_event_handler(config, enabled, tx);

        handler(event("keydown", "ControlLeft"));
        handler(event("keydown", "MetaLeft"));
        assert!(rx.try_recv().is_err(), "the old default must not fire");

        handler(event("keyup", "ControlLeft"));
        handler(event("keyup", "MetaLeft"));
        handler(event("keydown", "Alt"));
        handler(event("keydown", "ShiftLeft"));
        assert!(matches!(rx.try_recv().unwrap(), ControlMsg::Start));
    }
}
