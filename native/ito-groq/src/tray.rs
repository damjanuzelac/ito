//! Windows tray UI: icon, menu and status tooltip on the main thread.

use crate::input::register_hotkey;
use crate::overlay::Overlay;
use crate::{save_config, Config, ControlMsg};
use crossbeam_channel::Sender;
use global_key_listener::KeyListenerState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

#[derive(Debug)]
pub enum UserEvent {
    Status(String),
    Menu(MenuEvent),
}

/// Overlay animation frame interval (~30 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

// Status colors for the generated tray icon.
const COLOR_IDLE: [u8; 3] = [0x43, 0x67, 0x9d]; // Ito blue — Ready/Disabled
const COLOR_RECORDING: [u8; 3] = [0xd0, 0x3b, 0x3b]; // red — actively recording
const COLOR_BUSY: [u8; 3] = [0xe0, 0x9b, 0x2a]; // amber — transcribing

/// A simple generated icon (filled circle) so no image assets are needed.
fn tray_icon(rgb: [u8; 3]) -> Icon {
    const SIZE: i32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = (SIZE - 1) as f32 / 2.0;
    let radius = SIZE as f32 * 0.42;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if (dx * dx + dy * dy).sqrt() <= radius {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xff]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).expect("Failed to build tray icon")
}

/// Picks the icon color for a status string emitted by the session loop.
fn color_for_status(status: &str) -> [u8; 3] {
    if status.starts_with("Recording") {
        COLOR_RECORDING
    } else if status.starts_with("Transcribing") {
        COLOR_BUSY
    } else {
        COLOR_IDLE
    }
}

fn open_in_explorer(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// Runs the tray event loop on the main thread; never returns.
///
/// `spawn_session` is called once with a status callback wired to the tray
/// tooltip — main uses it to start the session thread.
pub fn run(
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    listener_state: Arc<KeyListenerState>,
    enabled: Arc<AtomicBool>,
    session_tx: Sender<ControlMsg>,
    spawn_session: impl FnOnce(Box<dyn Fn(String) + Send + Sync>),
) -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Forward menu events into the event loop
    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    // Session thread reports status through the event loop proxy
    let status_proxy = event_loop.create_proxy();
    spawn_session(Box::new(move |status: String| {
        let _ = status_proxy.send_event(UserEvent::Status(status));
    }));

    let language = config.read().unwrap().language.clone();

    let menu = Menu::new();
    let toggle_item = CheckMenuItem::new("Listening enabled", true, true, None);
    let lang_hr = CheckMenuItem::new("Croatian", true, language == "hr", None);
    let lang_en = CheckMenuItem::new("English", true, language == "en", None);
    let lang_auto = CheckMenuItem::new("Auto-detect language", true, language == "auto", None);
    let reload_item = MenuItem::new("Reload ito.ini", true, None);
    let open_data_item = MenuItem::new("Open ito.ini folder", true, None);
    let quit_item = MenuItem::new("Quit Ito", true, None);
    menu.append_items(&[
        &toggle_item,
        &PredefinedMenuItem::separator(),
        &lang_hr,
        &lang_en,
        &lang_auto,
        &PredefinedMenuItem::separator(),
        &reload_item,
        &open_data_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])
    .expect("Failed to build tray menu");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Ito — starting...")
        .with_icon(tray_icon(COLOR_BUSY))
        .build()
        .expect("Failed to create tray icon");

    let toggle_id = toggle_item.id().clone();
    let lang_hr_id = lang_hr.id().clone();
    let lang_en_id = lang_en.id().clone();
    let lang_auto_id = lang_auto.id().clone();
    let reload_id = reload_item.id().clone();
    let open_data_id = open_data_item.id().clone();
    let quit_id = quit_item.id().clone();

    // On-screen recording indicator (best-effort; dictation works without it).
    let mut overlay = Overlay::new(&event_loop, &language)
        .map_err(|e| eprintln!("[ito] Overlay unavailable: {e:#}"))
        .ok();

    let mut last_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        // While the bar is on screen it animates; otherwise the loop idles.
        *control_flow = match overlay.as_ref() {
            Some(ov) if ov.is_visible() => ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL),
            _ => ControlFlow::Wait,
        };

        // Advance on a fixed cadence: the loop also wakes on unrelated events,
        // and animating on every one of those makes the motion uneven.
        if let Some(ov) = overlay.as_mut() {
            if ov.is_visible() && last_frame.elapsed() >= FRAME_INTERVAL {
                last_frame = Instant::now();
                ov.tick();
            }
        }

        if let tao::event::Event::UserEvent(user_event) = event {
            match user_event {
                UserEvent::Status(status) => {
                    let _ = tray.set_icon(Some(tray_icon(color_for_status(&status))));
                    let _ = tray.set_tooltip(Some(format!("Ito — {status}")));
                    if let Some(ov) = overlay.as_mut() {
                        if status.starts_with("Recording") {
                            ov.show();
                        } else {
                            ov.hide();
                        }
                    }
                }
                UserEvent::Menu(menu_event) => {
                    let id = menu_event.id();
                    if *id == lang_hr_id || *id == lang_en_id || *id == lang_auto_id {
                        let chosen = if *id == lang_hr_id {
                            "hr"
                        } else if *id == lang_en_id {
                            "en"
                        } else {
                            "auto"
                        };
                        // Radio behavior: the menu toggles the clicked item itself.
                        lang_hr.set_checked(chosen == "hr");
                        lang_en.set_checked(chosen == "en");
                        lang_auto.set_checked(chosen == "auto");

                        let mut cfg = config.write().unwrap();
                        cfg.language = chosen.to_string();
                        if let Err(e) = save_config(&cfg, &config_path) {
                            eprintln!("[ito] Failed to save language: {e:#}");
                        }
                        drop(cfg);

                        if let Some(ov) = overlay.as_mut() {
                            ov.set_language(chosen);
                        }
                    } else if *id == toggle_id {
                        let now_enabled = toggle_item.is_checked();
                        enabled.store(now_enabled, Ordering::SeqCst);
                        register_hotkey(&listener_state, &config.read().unwrap(), now_enabled);
                        let _ = tray.set_tooltip(Some(if now_enabled {
                            "Ito — Ready".to_string()
                        } else {
                            "Ito — Disabled".to_string()
                        }));
                    } else if *id == reload_id {
                        let _ = session_tx.send(ControlMsg::ReloadConfig);
                        // Re-register the hotkey once the new config is readable
                        register_hotkey(
                            &listener_state,
                            &config.read().unwrap(),
                            enabled.load(Ordering::SeqCst),
                        );
                    } else if *id == open_data_id {
                        if let Some(dir) = config_path.parent() {
                            open_in_explorer(dir);
                        }
                    } else if *id == quit_id {
                        let _ = session_tx.send(ControlMsg::Quit);
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
        }
    })
}
