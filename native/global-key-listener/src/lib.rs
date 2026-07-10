//! Global keyboard listening with hotkey suppression.
//!
//! The library exposes the chord-matching + event-suppression logic so it can
//! be used both by the standalone binary (stdin/stdout protocol, used by the
//! Electron app) and in-process by other crates (e.g. the `ito-tray` app).

#[cfg(target_os = "windows")]
use rdev::{simulate, EventType as SimEventType};
use rdev::{Event, EventType, Key};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub mod key_codes;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotkeyCombo {
    pub keys: Vec<String>,
}

/// A key event emitted to the embedding application.
#[derive(Debug, Clone)]
pub struct ListenerEvent {
    /// "keydown" or "keyup"
    pub event_type: &'static str,
    /// The raw rdev key name (e.g. "ControlLeft", "KeyC", "Unknown(179)")
    pub key: String,
    /// JS-compatible key code, when known
    pub raw_code: Option<u32>,
}

/// Normalizes raw key names for hotkey matching.
/// `Unknown(179)` is the "fast fn" key and is treated as `Function`.
pub fn normalize_key_name(key_name: &str) -> String {
    if key_name == "Unknown(179)" {
        "Function".to_string()
    } else {
        key_name.to_string()
    }
}

/// Shared listener state: registered hotkeys and currently pressed keys.
///
/// The grab callback runs on rdev's thread; commands arrive from other
/// threads, hence the internal synchronization.
#[derive(Default)]
pub struct KeyListenerState {
    registered_hotkeys: Mutex<Vec<HotkeyCombo>>,
    currently_pressed: Mutex<Vec<String>>,
    cmd_pressed: AtomicBool,
    ctrl_pressed: AtomicBool,
    copy_in_progress: AtomicBool,
}

impl KeyListenerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_hotkeys(&self, hotkeys: Vec<HotkeyCombo>) {
        let mut registered = self.registered_hotkeys.lock().unwrap();
        *registered = hotkeys;
    }

    pub fn registered_hotkey_count(&self) -> usize {
        self.registered_hotkeys.lock().unwrap().len()
    }

    /// The currently pressed keys (normalized names), for chord matching by
    /// the embedding application.
    pub fn pressed_keys(&self) -> Vec<String> {
        self.currently_pressed.lock().unwrap().clone()
    }

    /// Drops a pressed key from the tracked set (stuck-key watchdog support).
    pub fn release_key(&self, key: &str) {
        self.currently_pressed.lock().unwrap().retain(|k| k != key);
    }

    // Check if current pressed keys exactly match any registered hotkey
    // (all keys present, order-independent, same length).
    fn should_block(&self) -> bool {
        let registered = self.registered_hotkeys.lock().unwrap();
        let pressed = self.currently_pressed.lock().unwrap();

        for hotkey in registered.iter() {
            let all_pressed = hotkey.keys.iter().all(|key| pressed.contains(key));
            let same_length = hotkey.keys.len() == pressed.len();

            if all_pressed && !hotkey.keys.is_empty() && same_length {
                return true;
            }
        }
        false
    }

    fn any_hotkey_uses_function(&self) -> bool {
        self.registered_hotkeys
            .lock()
            .unwrap()
            .iter()
            .any(|hotkey| hotkey.keys.contains(&"Function".to_string()))
    }

    /// Processes one rdev event. Emits keydown/keyup events through `emit`
    /// (except Cmd/Ctrl+C, which is swallowed to avoid feedback loops with
    /// clipboard-based text reading) and returns `None` to suppress the event
    /// from the OS when the pressed set exactly matches a registered hotkey.
    pub fn process(&self, event: Event, emit: &dyn Fn(ListenerEvent)) -> Option<Event> {
        match event.event_type {
            EventType::KeyPress(key) => {
                let key_name = format!("{:?}", key);

                // Ignore Cmd+C (macOS) and Ctrl+C (Windows/Linux) combinations to
                // prevent feedback loops with selected-text-reader
                if matches!(key, Key::KeyC)
                    && (self.cmd_pressed.load(Ordering::SeqCst)
                        || self.ctrl_pressed.load(Ordering::SeqCst))
                {
                    self.copy_in_progress.store(true, Ordering::SeqCst);
                    // Still pass through the event to the system but don't emit it
                    return Some(event);
                }

                // Update pressed keys BEFORE checking if we should block
                let normalized_key = normalize_key_name(&key_name);
                {
                    let mut pressed = self.currently_pressed.lock().unwrap();
                    if !pressed.contains(&normalized_key) {
                        pressed.push(normalized_key);
                    }
                }

                if matches!(key, Key::MetaLeft | Key::MetaRight) {
                    self.cmd_pressed.store(true, Ordering::SeqCst);
                }
                if matches!(key, Key::ControlLeft | Key::ControlRight) {
                    self.ctrl_pressed.store(true, Ordering::SeqCst);
                }

                emit(ListenerEvent {
                    event_type: "keydown",
                    key: key_name.clone(),
                    raw_code: key_codes::key_to_code(&key),
                });

                if self.should_block() {
                    // Windows-specific: Prevent Start menu from opening when the
                    // Windows key is part of a hotkey. Windows shows the Start menu
                    // if it sees "Win down → Win up" with no other keys in between.
                    // Injecting a harmless key (VK 0xFF, documented as "no mapping")
                    // "poisons" the sequence so Windows treats it as a combo.
                    #[cfg(target_os = "windows")]
                    {
                        let meta_involved = self.cmd_pressed.load(Ordering::SeqCst)
                            || self
                                .currently_pressed
                                .lock()
                                .unwrap()
                                .iter()
                                .any(|k| k == "MetaLeft" || k == "MetaRight");
                        if meta_involved {
                            let _ = simulate(&SimEventType::KeyPress(Key::Unknown(0xFF)));
                            let _ = simulate(&SimEventType::KeyRelease(Key::Unknown(0xFF)));
                        }
                    }
                    None // Block the event from reaching the OS
                } else if key_name == "Unknown(179)" && self.any_hotkey_uses_function() {
                    None // Block Unknown(179) if any hotkey uses Function
                } else {
                    Some(event) // Let it through
                }
            }
            EventType::KeyRelease(key) => {
                let key_name = format!("{:?}", key);
                let normalized_key = normalize_key_name(&key_name);

                self.currently_pressed
                    .lock()
                    .unwrap()
                    .retain(|k| k != &normalized_key);

                // Swallow the C release while a copy chord is in progress
                if matches!(key, Key::KeyC)
                    && (self.copy_in_progress.load(Ordering::SeqCst)
                        || self.cmd_pressed.load(Ordering::SeqCst)
                        || self.ctrl_pressed.load(Ordering::SeqCst))
                {
                    self.copy_in_progress.store(false, Ordering::SeqCst);
                    return Some(event);
                }

                if matches!(key, Key::MetaLeft | Key::MetaRight) {
                    self.cmd_pressed.store(false, Ordering::SeqCst);
                }
                if matches!(key, Key::ControlLeft | Key::ControlRight) {
                    self.ctrl_pressed.store(false, Ordering::SeqCst);
                }

                emit(ListenerEvent {
                    event_type: "keyup",
                    key: key_name,
                    raw_code: key_codes::key_to_code(&key),
                });

                // Always allow key release events through
                Some(event)
            }
            _ => Some(event), // Allow all other events
        }
    }
}

// rdev's grab takes a plain fn pointer, so the state and emitter for the
// (single) grab loop live in a process-wide slot.
type EmitFn = Box<dyn Fn(ListenerEvent) + Send + Sync>;
static GRAB_CONTEXT: std::sync::OnceLock<(Arc<KeyListenerState>, EmitFn)> =
    std::sync::OnceLock::new();

fn grab_callback(event: Event) -> Option<Event> {
    match GRAB_CONTEXT.get() {
        Some((state, emit)) => state.process(event, emit),
        None => Some(event),
    }
}

/// Runs the global grab loop on the current thread. Blocks forever (or until
/// rdev errors). Suppression decisions and event emission are handled by
/// `state.process`. Only one grab loop can exist per process; subsequent
/// calls keep the originally installed state/emitter.
pub fn run_grab<F>(state: Arc<KeyListenerState>, emit: F) -> Result<(), rdev::GrabError>
where
    F: Fn(ListenerEvent) + Send + Sync + 'static,
{
    let _ = GRAB_CONTEXT.set((state, Box::new(emit)));
    rdev::grab(grab_callback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(state: &KeyListenerState, key: Key) -> bool {
        let event = Event {
            event_type: EventType::KeyPress(key),
            time: std::time::SystemTime::now(),
            name: None,
        };
        state.process(event, &|_| {}).is_none()
    }

    fn release(state: &KeyListenerState, key: Key) {
        let event = Event {
            event_type: EventType::KeyRelease(key),
            time: std::time::SystemTime::now(),
            name: None,
        };
        state.process(event, &|_| {});
    }

    #[test]
    fn blocks_exact_chord_only() {
        let state = KeyListenerState::new();
        state.register_hotkeys(vec![HotkeyCombo {
            keys: vec!["ControlLeft".into(), "MetaLeft".into()],
        }]);

        assert!(!press(&state, Key::ControlLeft), "partial chord passes");
        assert!(press(&state, Key::MetaLeft), "full chord is suppressed");

        // Extra key breaks the exact match
        assert!(!press(&state, Key::KeyA));

        release(&state, Key::KeyA);
        release(&state, Key::MetaLeft);
        release(&state, Key::ControlLeft);
        assert!(state.pressed_keys().is_empty());
    }

    #[test]
    fn ctrl_c_is_not_emitted() {
        let state = KeyListenerState::new();
        let emitted = std::sync::Mutex::new(Vec::new());
        let event = Event {
            event_type: EventType::KeyPress(Key::ControlLeft),
            time: std::time::SystemTime::now(),
            name: None,
        };
        state.process(event, &|e| emitted.lock().unwrap().push(e.key));
        let event = Event {
            event_type: EventType::KeyPress(Key::KeyC),
            time: std::time::SystemTime::now(),
            name: None,
        };
        state.process(event, &|e| emitted.lock().unwrap().push(e.key));

        let emitted = emitted.lock().unwrap();
        assert_eq!(emitted.as_slice(), &["ControlLeft".to_string()]);
    }

    #[test]
    fn normalizes_fast_fn_key() {
        assert_eq!(normalize_key_name("Unknown(179)"), "Function");
        assert_eq!(normalize_key_name("KeyA"), "KeyA");
    }
}
