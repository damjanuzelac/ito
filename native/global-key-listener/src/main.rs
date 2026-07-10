use chrono::Utc;
use global_key_listener::{run_grab, HotkeyCombo, KeyListenerState};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "macos")]
use cocoa::base::{id, nil};
#[cfg(target_os = "macos")]
use cocoa::foundation::{NSProcessInfo, NSString};
#[cfg(target_os = "macos")]
use objc::{msg_send, sel, sel_impl};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command")]
enum Command {
    #[serde(rename = "register_hotkeys")]
    RegisterHotkeys { hotkeys: Vec<HotkeyCombo> },
}

/// Prevents macOS App Nap from suspending this process.
/// Returns an activity token that must be retained for the entire process
/// lifetime. On non-macOS platforms, returns a dummy value.
#[cfg(target_os = "macos")]
fn prevent_app_nap() -> id {
    unsafe {
        let process_info = NSProcessInfo::processInfo(nil);
        let reason = NSString::alloc(nil)
            .init_str("Keyboard event monitoring requires continuous operation");

        // NSActivityOptions flags:
        // NSActivityUserInitiated = 0x00FFFFFF (includes all protective flags)
        // This prevents App Nap and idle system sleep
        let options: u64 = 0x00FFFFFF;

        let activity: id = msg_send![process_info, beginActivityWithOptions:options reason:reason];

        eprintln!("macOS App Nap prevention enabled for keyboard listener process");
        activity
    }
}

#[cfg(not(target_os = "macos"))]
fn prevent_app_nap() {
    // No-op on non-macOS platforms
}

fn main() {
    // Prevent macOS App Nap from suspending this process
    // Must retain this for the entire process lifetime
    #[allow(clippy::let_unit_value)]
    let _activity = prevent_app_nap();

    let state = Arc::new(KeyListenerState::new());

    // Spawn a thread to read commands from stdin
    {
        let state = Arc::clone(&state);
        thread::spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                match serde_json::from_str::<Command>(&line) {
                    Ok(Command::RegisterHotkeys { hotkeys }) => {
                        state.register_hotkeys(hotkeys);
                        eprintln!("Registered {} hotkeys", state.registered_hotkey_count());
                        io::stdout().flush().unwrap();
                    }
                    Err(e) => eprintln!("Error parsing command: {}", e),
                }
            }
        });
    }

    // Spawn heartbeat thread
    thread::spawn(|| {
        let mut heartbeat_id = 0u64;
        loop {
            thread::sleep(Duration::from_secs(10)); // Send heartbeat every 10 seconds

            heartbeat_id += 1;
            let heartbeat_json = json!({
                "type": "heartbeat_ping",
                "id": heartbeat_id.to_string(),
                "timestamp": Utc::now().to_rfc3339()
            });

            println!("{}", heartbeat_json);
            io::stdout().flush().unwrap();
        }
    });

    // Start grabbing events
    if let Err(error) = run_grab(state, |event| {
        let event_json = json!({
            "type": event.event_type,
            "key": event.key,
            "timestamp": Utc::now().to_rfc3339(),
            "raw_code": event.raw_code
        });
        println!("{}", event_json);
        io::stdout().flush().unwrap();
    }) {
        eprintln!("Error: {:?}", error);
    }
}
