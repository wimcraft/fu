//! Terminal lifecycle: pixel-geometry measurement, tmux preflight, raw-mode
//! guard, signal-driven shutdown, and status-line formatting.
//!
//! Pixel-size querying itself is owned by `ratatui-image`'s `Picker` (see
//! `protocol.rs`); this module turns the picker's font size plus
//! `crossterm`'s cell dimensions into a `view::Geometry`.

use std::io::{stdout, Write as _};
use std::process::{Command, Stdio};

use crossterm::cursor::{Hide, Show};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui_image::FontSize;

use crate::error::FuError;
use crate::view::Geometry;

pub const TMUX_PASSTHROUGH_ERROR: &str = "tmux pixel graphics require: set -g allow-passthrough on";

/// Compute draw-area geometry from the current terminal cell grid and the
/// picker's detected per-cell pixel size.
pub fn geometry(font_size: FontSize) -> Result<Geometry, FuError> {
    let (columns, rows) = crossterm::terminal::size()?;
    if columns == 0 || rows == 0 || font_size.width == 0 || font_size.height == 0 {
        return Err(FuError::new("terminal did not report pixel dimensions"));
    }
    let pixel_width = columns as u32 * font_size.width as u32;
    let pixel_height = rows as u32 * font_size.height as u32;
    Ok(Geometry::new(columns, rows, pixel_width, pixel_height))
}

/// Abstraction over running an external command, so the tmux preflight is
/// testable without a real `tmux` binary.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Validate tmux pixel-graphics prerequisites *before* any picker or raw-mode
/// setup happens, so a failing gate never touches the terminal.
pub fn check_tmux_passthrough(is_tmux: bool, runner: &impl CommandRunner) -> Result<(), FuError> {
    if !is_tmux {
        return Ok(());
    }
    match runner.run("tmux", &["show-option", "-gqv", "allow-passthrough"]) {
        Some(ref value) if value == "on" => Ok(()),
        _ => Err(FuError::new(TMUX_PASSTHROUGH_ERROR)),
    }
}

/// Format the plain-text status row: position, zoom, and a rendering indicator.
pub fn status_line(view: &crate::view::View, rendering: bool) -> String {
    let indicator = if rendering { "rendering…" } else { "ready" };
    let zoom = match view.fit {
        crate::view::FitMode::Width => format!("{:.1}", view.zoom),
        crate::view::FitMode::Whole => "fit".to_string(),
    };
    format!(" x={} y={} zoom={zoom} {indicator}", view.x, view.y)
}

/// Horizontal position bar for the bottom edge, `draw_columns` cells wide
/// (aligned under the image, not the vertical bar's corner column), marking
/// where the crop sits within the full source width — a horizontal
/// scrollbar in miniature.
pub fn horizontal_bar(
    view: &crate::view::View,
    geometry: &Geometry,
    source_w: u32,
    source_h: u32,
) -> String {
    let crop = view.crop(geometry, source_w, source_h);
    position_bar(crop.x, crop.width, source_w, geometry.draw_columns as usize)
}

/// Vertical position bar for the right edge, one character per row over
/// `draw_rows` rows, marking where the crop sits within the full source
/// height — a vertical scrollbar in miniature, alongside the image.
pub fn vertical_bar(
    view: &crate::view::View,
    geometry: &Geometry,
    source_w: u32,
    source_h: u32,
) -> Vec<char> {
    let crop = view.crop(geometry, source_w, source_h);
    position_bar(crop.y, crop.height, source_h, geometry.draw_rows as usize)
        .chars()
        .collect()
}

/// A `width`-cell bar with `▓` marking the `[position, position + extent)`
/// slice of `[0, total)`, and `░` everywhere else.
fn position_bar(position: u32, extent: u32, total: u32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let total = (total.max(1)) as f64;
    let width_f = width as f64;

    let start = ((position as f64 / total) * width_f).floor() as usize;
    let end_fraction = (((position + extent) as f64) / total).min(1.0);
    let end = (end_fraction * width_f).ceil() as usize;

    let start = start.min(width.saturating_sub(1));
    let end = end.max(start + 1).min(width);

    (0..width)
        .map(|i| if i >= start && i < end { '▓' } else { '░' })
        .collect()
}

/// Owns raw mode + alternate screen for the lifetime of the interactive
/// session. `Drop` always restores the terminal, including on early return
/// from an error or panic unwind.
pub struct TerminalGuard {
    graphics_cleanup: Option<String>,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self, FuError> {
        enable_raw_mode()?;
        crossterm::execute!(stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self {
            graphics_cleanup: None,
        })
    }

    pub fn set_graphics_cleanup(&mut self, sequence: String) {
        self.graphics_cleanup = Some(sequence);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut output = stdout();
        if let Some(sequence) = self.graphics_cleanup.as_deref() {
            let _ = output.write_all(sequence.as_bytes());
            let _ = output.flush();
        }
        let _ = crossterm::execute!(output, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Minimal SIGINT/SIGTERM handling via direct libc FFI (no extra dependency):
/// a signal delivered by `kill` bypasses raw-mode's ISIG suppression, which
/// only covers terminal-generated signals like a tty Ctrl-C.
pub mod signal {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;

    type Handler = extern "C" fn(i32);

    extern "C" {
        fn signal(signum: i32, handler: Handler) -> Handler;
    }

    extern "C" fn on_signal(_: i32) {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        unsafe {
            signal(SIGINT, on_signal);
            signal(SIGTERM, on_signal);
        }
    }

    pub fn requested() -> bool {
        SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn reset_for_test() {
        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRunner(Option<&'static str>);

    impl CommandRunner for FakeRunner {
        fn run(&self, _program: &str, _args: &[&str]) -> Option<String> {
            self.0.map(|s| s.to_string())
        }
    }

    #[test]
    fn non_tmux_skips_the_check_entirely() {
        let runner = FakeRunner(None);
        assert!(check_tmux_passthrough(false, &runner).is_ok());
    }

    #[test]
    fn tmux_on_passes() {
        let runner = FakeRunner(Some("on"));
        assert!(check_tmux_passthrough(true, &runner).is_ok());
    }

    #[test]
    fn tmux_off_fails_with_exact_message() {
        let runner = FakeRunner(Some("off"));
        let err = check_tmux_passthrough(true, &runner).unwrap_err();
        assert_eq!(
            err.to_string(),
            "fu: tmux pixel graphics require: set -g allow-passthrough on"
        );
    }

    #[test]
    fn tmux_command_failure_fails_closed() {
        let runner = FakeRunner(None);
        assert!(check_tmux_passthrough(true, &runner).is_err());
    }

    #[test]
    fn geometry_reports_error_on_zero_font_size() {
        let err = geometry(FontSize::new(0, 0));
        assert!(err.is_err());
    }

    #[test]
    fn signal_flag_starts_unset() {
        signal::reset_for_test();
        assert!(!signal::requested());
    }

    #[test]
    fn position_bar_marks_the_full_extent_at_zoom_one() {
        let bar = position_bar(0, 1000, 1000, 20);
        assert_eq!(bar, "▓".repeat(20));
    }

    #[test]
    fn position_bar_marks_only_the_start_when_pinned_top_left() {
        let bar = position_bar(0, 100, 1000, 20);
        assert!(bar.starts_with('▓'));
        assert!(bar.ends_with('░'));
    }

    #[test]
    fn position_bar_marks_the_end_when_pinned_to_the_far_edge() {
        let bar = position_bar(900, 100, 1000, 20);
        assert!(bar.ends_with('▓'));
        assert!(bar.starts_with('░'));
    }

    #[test]
    fn position_bar_always_marks_at_least_one_cell() {
        let bar = position_bar(0, 1, 1_000_000, 20);
        assert_eq!(bar.chars().filter(|&c| c == '▓').count(), 1);
    }

    #[test]
    fn horizontal_bar_is_draw_columns_wide() {
        let view = crate::view::View::new();
        let geometry = Geometry::new(40, 20, 400, 200);
        let bar = horizontal_bar(&view, &geometry, 2000, 1000);
        assert_eq!(bar.chars().count(), geometry.draw_columns as usize);
    }

    #[test]
    fn vertical_bar_has_one_character_per_draw_row() {
        let view = crate::view::View::new();
        let geometry = Geometry::new(40, 20, 400, 200);
        let bar = vertical_bar(&view, &geometry, 2000, 1000);
        assert_eq!(bar.len(), geometry.draw_rows as usize);
    }

    #[test]
    fn vertical_bar_marks_the_bottom_when_panned_to_the_end() {
        let geometry = Geometry::new(40, 20, 400, 200);
        let mut view = crate::view::View::new();
        view.bottom(&geometry, 2000, 6000);
        let bar = vertical_bar(&view, &geometry, 2000, 6000);
        assert_eq!(*bar.last().unwrap(), '▓');
    }
}
