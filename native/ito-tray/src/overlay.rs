//! On-screen recording indicator: a small always-on-top, click-through
//! "● REC" pill shown near the bottom of the primary screen while recording,
//! so dictation is obvious without hunting for the tray icon.

use anyhow::{Context, Result};
use std::num::NonZeroU32;
use std::rc::Rc;
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder, WindowId};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA};

const BG: u32 = 0x001C_1C1E; // dark charcoal (softbuffer format: 0x00RRGGBB)
const RED: u32 = 0x00E0_392E; // record dot
const WHITE: u32 = 0x00F5_F5F5; // "REC" text

// 5x7 bitmaps for the only three glyphs we render, top row first.
const GLYPH_R: [u8; 7] = [
    0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
];
const GLYPH_E: [u8; 7] = [
    0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
];
const GLYPH_C: [u8; 7] = [
    0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
];

pub struct Overlay {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _context: softbuffer::Context<Rc<Window>>,
    visible: bool,
}

impl Overlay {
    pub fn new<T>(target: &EventLoopWindowTarget<T>) -> Result<Self> {
        let window = WindowBuilder::new()
            .with_title("Ito Recording")
            .with_inner_size(LogicalSize::new(148.0, 48.0))
            .with_decorations(false)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            .with_focused(false)
            .with_skip_taskbar(true)
            .build(target)
            .context("Failed to create overlay window")?;
        // Never steal clicks from the app being dictated into.
        let _ = window.set_ignore_cursor_events(true);

        let window = Rc::new(window);
        let context = softbuffer::Context::new(window.clone())
            .map_err(|e| anyhow::anyhow!("softbuffer context: {e}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|e| anyhow::anyhow!("softbuffer surface: {e}"))?;

        let overlay = Self {
            window,
            surface,
            _context: context,
            visible: false,
        };
        overlay.position_bottom_center();
        Ok(overlay)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn show(&mut self) {
        if !self.visible {
            self.position_bottom_center();
            self.window.set_visible(true);
            self.visible = true;
        }
        // Click-through makes the window WS_EX_LAYERED, and a layered window
        // stays blank unless its alpha is set — without this only the drop
        // shadow is visible. tao applies the style asynchronously, so this is
        // re-applied on every show rather than once at construction.
        unsafe {
            SetLayeredWindowAttributes(self.window.hwnd() as HWND, 0, 255, LWA_ALPHA);
        }
        self.window.request_redraw();
    }

    pub fn hide(&mut self) {
        if self.visible {
            self.window.set_visible(false);
            self.visible = false;
        }
    }

    fn position_bottom_center(&self) {
        if let Some(monitor) = self.window.primary_monitor() {
            let screen = monitor.size();
            let win = self.window.inner_size();
            let x = (screen.width as i32 - win.width as i32) / 2;
            let y = screen.height as i32 - win.height as i32 - (win.height as i32 * 3 / 2);
            self.window.set_outer_position(PhysicalPosition::new(x, y));
        }
    }

    /// Paints the "● REC" indicator. Called on the overlay's redraw request.
    pub fn render(&mut self) {
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        if self.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return;
        };

        let (width, height) = (size.width as usize, size.height as usize);
        buffer.fill(BG);

        // Record dot on the left.
        let radius = (height as i32 * 7 / 24).max(6);
        let cx = radius + height as i32 / 3;
        let cy = height as i32 / 2;
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= radius * radius {
                    buffer[(y as usize) * width + x as usize] = RED;
                }
            }
        }

        // "REC" text to the right of the dot.
        let scale = (height / 16).max(2) as i32;
        let text_x = cx + radius + height as i32 / 4;
        let text_y = cy - (7 * scale) / 2;
        let mut pen_x = text_x;
        for glyph in [&GLYPH_R, &GLYPH_E, &GLYPH_C] {
            draw_glyph(&mut buffer, (width, height), glyph, (pen_x, text_y), scale);
            pen_x += 6 * scale;
        }

        let _ = buffer.present();
    }
}

/// Blits one 5x7 glyph in white, scaled by `scale` and clipped to the buffer.
fn draw_glyph(
    buffer: &mut [u32],
    (width, height): (usize, usize),
    glyph: &[u8; 7],
    (ox, oy): (i32, i32),
    scale: i32,
) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) == 0 {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    let px = ox + col * scale + sx;
                    let py = oy + row as i32 * scale + sy;
                    if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                        buffer[(py as usize) * width + px as usize] = WHITE;
                    }
                }
            }
        }
    }
}
