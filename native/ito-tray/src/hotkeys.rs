//! Global hotkey handling: exact-chord matching on top of the shared
//! global-key-listener library, driving the session state machine.
//!
//! Rules ported from the Electron app (lib/media/keyboard.ts):
//! - a chord matches when the pressed set contains exactly its keys
//!   (order-independent, same size);
//! - full match while idle starts a session (hold-to-talk);
//! - releasing any chord key completes the session;
//! - keys stuck for more than 5 s are purged (unless part of the active chord).

use crate::config::Config;
use crate::prompt::ItoMode;
use crate::session::ControlMsg;
use crossbeam_channel::Sender;
use global_key_listener::{normalize_key_name, HotkeyCombo, KeyListenerState, ListenerEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const STUCK_KEY_TIMEOUT: Duration = Duration::from_secs(5);

/// Registers the configured chords on the shared listener state so they are
/// suppressed from reaching the focused application. Clears the registration
/// when listening is disabled.
pub fn register_config_hotkeys(state: &KeyListenerState, config: &Config, enabled: bool) {
    if !enabled {
        state.register_hotkeys(Vec::new());
        return;
    }
    state.register_hotkeys(vec![
        HotkeyCombo {
            keys: config.hotkey_transcribe.clone(),
        },
        HotkeyCombo {
            keys: config.hotkey_edit.clone(),
        },
    ]);
}

struct MatcherState {
    pressed: HashMap<String, Instant>,
    active_chord: Option<Vec<String>>,
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
        active_chord: None,
    });

    move |event: ListenerEvent| {
        let mut state = matcher.lock().unwrap();
        let key = normalize_key_name(&event.key);
        let now = Instant::now();

        match event.event_type {
            "keydown" => {
                // Stuck-key watchdog: purge keys held implausibly long,
                // unless they belong to the currently active chord.
                let active_chord = state.active_chord.clone();
                state.pressed.retain(|pressed_key, since| {
                    now.duration_since(*since) < STUCK_KEY_TIMEOUT
                        || active_chord.as_ref().is_some_and(|chord| {
                            chord.iter().any(|k| normalize_key_name(k) == *pressed_key)
                        })
                });

                state.pressed.entry(key).or_insert(now);

                if !enabled.load(Ordering::SeqCst) || state.active_chord.is_some() {
                    return;
                }

                let cfg = config.read().unwrap();
                if chord_matches(&cfg.hotkey_transcribe, &state.pressed) {
                    state.active_chord = Some(cfg.hotkey_transcribe.clone());
                    let _ = tx.send(ControlMsg::Start(ItoMode::Transcribe));
                } else if chord_matches(&cfg.hotkey_edit, &state.pressed) {
                    state.active_chord = Some(cfg.hotkey_edit.clone());
                    let _ = tx.send(ControlMsg::Start(ItoMode::Edit));
                }
            }
            "keyup" => {
                state.pressed.remove(&key);

                if let Some(chord) = &state.active_chord {
                    let still_held = chord
                        .iter()
                        .all(|k| state.pressed.contains_key(&normalize_key_name(k)));
                    if !still_held {
                        state.active_chord = None;
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
            eprintln!("[ito-tray] Keyboard grab failed: {e:?}");
        }
    });
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

    #[test]
    fn hold_and_release_transcribe_chord() {
        let config = Arc::new(RwLock::new(Config::default()));
        let enabled = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded();
        let handler = make_key_event_handler(config, enabled, tx);

        handler(event("keydown", "ControlLeft"));
        assert!(rx.try_recv().is_err(), "partial chord must not start");
        handler(event("keydown", "MetaLeft"));
        assert!(matches!(
            rx.try_recv().unwrap(),
            ControlMsg::Start(ItoMode::Transcribe)
        ));

        handler(event("keyup", "MetaLeft"));
        assert!(matches!(rx.try_recv().unwrap(), ControlMsg::Complete));
        handler(event("keyup", "ControlLeft"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn edit_chord_starts_edit_mode() {
        let config = Arc::new(RwLock::new(Config::default()));
        let enabled = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded();
        let handler = make_key_event_handler(config, enabled, tx);

        handler(event("keydown", "Alt"));
        handler(event("keydown", "ControlLeft"));
        assert!(matches!(
            rx.try_recv().unwrap(),
            ControlMsg::Start(ItoMode::Edit)
        ));
    }

    #[test]
    fn extra_key_prevents_match() {
        let config = Arc::new(RwLock::new(Config::default()));
        let enabled = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded();
        let handler = make_key_event_handler(config, enabled, tx);

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
}
