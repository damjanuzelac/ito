//! On-screen recording indicator: a thin, always-on-top, click-through bar
//! just above the taskbar that grows outward from the centre with the
//! microphone level, so dictation is obvious without hunting for the tray icon.
//!
//! Painted as a premultiplied-BGRA DIB pushed through `UpdateLayeredWindow`.
//! That is the only way to get real per-pixel alpha (translucency and
//! anti-aliased round caps) on Windows 10 — a plain opaque blit can only ever
//! produce a hard-edged rectangle.

use anyhow::{Context, Result};
use std::ptr;
use std::time::Instant;
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, UpdateLayeredWindow, SPI_GETWORKAREA, ULW_ALPHA,
};

/// Bar length at full level, as a fraction of the screen width.
const MAX_WIDTH_FRACTION: f64 = 0.20;
/// Bar thickness in logical pixels.
const BAR_THICKNESS: f64 = 5.0;
/// Padding around the bar so anti-aliased caps are never clipped.
const PADDING: f64 = 4.0;
/// Gap between the bar and the top of the taskbar.
const BOTTOM_GAP: f64 = 10.0;
/// Shortest the bar gets, as a fraction of the track.
const MIN_FILL: f32 = 0.30;
/// Seconds per breath. A cosine ease makes both ends of the swing settle
/// gently, so the motion reads as calm rather than mechanical.
const BREATH_PERIOD: f32 = 2.6;

/// Bar colour, chosen by dictation language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarColor {
    /// Croatian
    Red,
    /// English
    Blue,
    /// Language not pinned
    Neutral,
}

impl BarColor {
    /// Straight (non-premultiplied) RGB.
    const fn rgb(self) -> (f32, f32, f32) {
        match self {
            Self::Red => (0.91, 0.24, 0.20),
            Self::Blue => (0.25, 0.55, 0.96),
            Self::Neutral => (0.78, 0.78, 0.80),
        }
    }

    pub fn for_language(language: &str) -> Self {
        match language {
            "hr" => Self::Red,
            "en" => Self::Blue,
            _ => Self::Neutral,
        }
    }
}

/// Breathing curve in 0..1 for a given time since the bar appeared.
fn breathing_level(elapsed_secs: f32) -> f32 {
    let turns = elapsed_secs / BREATH_PERIOD * std::f32::consts::TAU;
    // cos eases into both extremes, so there is no visible turnaround snap.
    (0.5 - 0.5 * turns.cos()).clamp(0.0, 1.0)
}

/// Half-length of the drawn bar, in pixels, for a smoothed level.
fn half_length(level: f32, track_width: f64) -> f64 {
    let fill = MIN_FILL + (1.0 - MIN_FILL) * level.clamp(0.0, 1.0);
    track_width * f64::from(fill) / 2.0
}

/// Owns the GDI device context and DIB the bar is painted into.
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u32,
    width: i32,
    height: i32,
}

impl Canvas {
    /// Creates a top-down 32-bit DIB. A negative height makes row 0 the top
    /// row, so buffer order matches screen order.
    fn new(width: i32, height: i32) -> Result<Self> {
        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        let mut pixels: *mut core::ffi::c_void = ptr::null_mut();
        unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                anyhow::bail!("CreateCompatibleDC failed");
            }
            let bitmap =
                CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, ptr::null_mut(), 0);
            if bitmap.is_null() || pixels.is_null() {
                DeleteDC(dc);
                anyhow::bail!("CreateDIBSection failed");
            }
            let old_bitmap = SelectObject(dc, bitmap as HGDIOBJ);
            Ok(Self {
                dc,
                bitmap,
                old_bitmap,
                pixels: pixels.cast::<u32>(),
                width,
                height,
            })
        }
    }

    fn as_mut_slice(&mut self) -> &mut [u32] {
        let len = (self.width * self.height) as usize;
        unsafe { std::slice::from_raw_parts_mut(self.pixels, len) }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.dc);
        }
    }
}

pub struct Overlay {
    window: Window,
    canvas: Canvas,
    color: BarColor,
    shown_at: Instant,
    level: f32,
    visible: bool,
}

impl Overlay {
    pub fn new<T>(target: &EventLoopWindowTarget<T>, language: &str) -> Result<Self> {
        let scale = target
            .primary_monitor()
            .map_or(1.0, |monitor| monitor.scale_factor());
        let screen_width = target
            .primary_monitor()
            .map_or(1920.0, |monitor| f64::from(monitor.size().width));

        let width = (screen_width * MAX_WIDTH_FRACTION + PADDING * 2.0 * scale).round() as i32;
        let height = ((BAR_THICKNESS + PADDING * 2.0) * scale).round() as i32;

        let window = WindowBuilder::new()
            .with_title("Ito Recording")
            .with_inner_size(PhysicalSize::new(width, height))
            .with_decorations(false)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            .with_focused(false)
            .with_skip_taskbar(true)
            .build(target)
            .context("Failed to create overlay window")?;
        // Sets WS_EX_TRANSPARENT (clicks pass through to the app being
        // dictated into) *and* WS_EX_LAYERED, which UpdateLayeredWindow needs.
        let _ = window.set_ignore_cursor_events(true);

        let canvas = Canvas::new(width, height)?;
        let overlay = Self {
            window,
            canvas,
            color: BarColor::for_language(language),
            shown_at: Instant::now(),
            level: 0.0,
            visible: false,
        };
        overlay.position_above_taskbar();
        Ok(overlay)
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_language(&mut self, language: &str) {
        self.color = BarColor::for_language(language);
        if self.visible {
            self.render();
        }
    }

    pub fn show(&mut self) {
        if !self.visible {
            self.position_above_taskbar();
            self.shown_at = Instant::now();
            self.level = 0.0;
            self.visible = true;
            self.render();
            self.window.set_visible(true);
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            self.window.set_visible(false);
            self.visible = false;
            self.level = 0.0;
        }
    }

    /// Advances the breathing animation one frame and repaints.
    pub fn tick(&mut self) {
        self.level = breathing_level(self.shown_at.elapsed().as_secs_f32());
        self.render();
    }

    /// Places the bar centred above the taskbar. tao's MonitorHandle only
    /// reports full screen bounds, so the work area comes from the Win32 API.
    fn position_above_taskbar(&self) {
        let size = self.window.inner_size();
        let mut work_area = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                ptr::addr_of_mut!(work_area).cast::<core::ffi::c_void>(),
                0,
            )
        };
        let (area_width, area_bottom) = if ok != 0 {
            (work_area.right - work_area.left, work_area.bottom)
        } else if let Some(monitor) = self.window.primary_monitor() {
            let screen = monitor.size();
            (screen.width as i32, screen.height as i32)
        } else {
            return;
        };

        let scale = self.window.scale_factor();
        let x = work_area.left + (area_width - size.width as i32) / 2;
        let y = area_bottom - size.height as i32 - (BOTTOM_GAP * scale).round() as i32;
        self.window.set_outer_position(PhysicalPosition::new(x, y));
    }

    /// Paints the capsule into the DIB and pushes it to the layered window.
    pub fn render(&mut self) {
        let (width, height) = (self.canvas.width, self.canvas.height);
        let scale = self.window.scale_factor();
        let track_width = f64::from(width) - PADDING * 2.0 * scale;
        let half = half_length(self.level, track_width);
        let radius = (BAR_THICKNESS * scale / 2.0).max(1.0);
        let center_x = f64::from(width) / 2.0;
        let center_y = f64::from(height) / 2.0;
        let (r, g, b) = self.color.rgb();

        let buffer = self.canvas.as_mut_slice();
        buffer.fill(0);

        // Capsule = every pixel within `radius` of the horizontal centre
        // segment. The distance field gives anti-aliased round caps for free.
        let segment_half = (half - radius).max(0.0);
        let y_start = ((center_y - radius - 1.0).floor().max(0.0)) as i32;
        let y_end = ((center_y + radius + 1.0).ceil().min(f64::from(height))) as i32;
        let x_start = ((center_x - half - 1.0).floor().max(0.0)) as i32;
        let x_end = ((center_x + half + 1.0).ceil().min(f64::from(width))) as i32;

        for y in y_start..y_end {
            let dy = f64::from(y) + 0.5 - center_y;
            for x in x_start..x_end {
                let dx = f64::from(x) + 0.5 - center_x;
                // Distance to the segment: clamp x onto it, then measure.
                let clamped = dx.clamp(-segment_half, segment_half);
                let distance = ((dx - clamped).powi(2) + dy * dy).sqrt();
                // 1 px feather from fully inside to fully outside.
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let alpha = (coverage * 255.0).round() as u32;
                // UpdateLayeredWindow expects premultiplied BGRA.
                let pr = (f64::from(r) * coverage * 255.0).round() as u32;
                let pg = (f64::from(g) * coverage * 255.0).round() as u32;
                let pb = (f64::from(b) * coverage * 255.0).round() as u32;
                buffer[(y * width + x) as usize] = (alpha << 24) | (pr << 16) | (pg << 8) | pb;
            }
        }

        let position = self.window.outer_position().unwrap_or_default();
        let top_left = POINT {
            x: position.x,
            y: position.y,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        unsafe {
            UpdateLayeredWindow(
                self.window.hwnd() as HWND,
                ptr::null_mut(),
                &top_left,
                &size,
                self.canvas.dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breathing_starts_small_and_swings_full() {
        assert!(breathing_level(0.0) < 0.01);
        assert!(breathing_level(BREATH_PERIOD / 2.0) > 0.99);
        assert!(breathing_level(BREATH_PERIOD) < 0.01);
    }

    #[test]
    fn breathing_stays_in_range_and_never_jumps() {
        let step = 1.0 / 30.0; // one animation frame
        let mut previous = breathing_level(0.0);
        let mut time = step;
        while time < BREATH_PERIOD * 3.0 {
            let level = breathing_level(time);
            assert!((0.0..=1.0).contains(&level));
            assert!(
                (level - previous).abs() < 0.1,
                "frame-to-frame jump of {} at t={time}",
                (level - previous).abs()
            );
            previous = level;
            time += step;
        }
    }

    #[test]
    fn shortest_bar_is_still_clearly_visible() {
        let half = half_length(0.0, 400.0);
        assert!(half > 400.0 * 0.1, "the bar must not shrink to a dot");
        assert!(half < 400.0 / 2.0, "and must leave room to grow");
    }

    #[test]
    fn full_level_fills_the_track() {
        let half = half_length(1.0, 400.0);
        assert!((half - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn length_grows_with_level_and_is_clamped() {
        assert!(half_length(0.5, 400.0) > half_length(0.2, 400.0));
        assert!((half_length(5.0, 400.0) - half_length(1.0, 400.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn language_picks_the_colour() {
        assert_eq!(BarColor::for_language("hr"), BarColor::Red);
        assert_eq!(BarColor::for_language("en"), BarColor::Blue);
        assert_eq!(BarColor::for_language("auto"), BarColor::Neutral);
    }
}
