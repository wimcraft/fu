//! Ratatui event loop: owns terminal input, viewport state, and cheap Kitty
//! placement updates. Source encoding happens once on a background thread.

use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use image::RgbaImage;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use ratatui_image::picker::Picker;

use crate::error::FuError;
use crate::input::{self, Command};
use crate::render::{self, KittyImage, Renderer};
use crate::terminal::{self as term, TerminalGuard};
use crate::view::{self, Geometry, View};

pub struct AppConfig {
    pub zoom: f32,
    pub overlap: f32,
    pub is_tmux: bool,
}

pub fn run(
    source: Arc<RgbaImage>,
    source_w: u32,
    source_h: u32,
    picker: Picker,
    config: AppConfig,
) -> Result<(), FuError> {
    let font_size = picker.font_size();
    let mut geometry = term::geometry(font_size)?;

    let mut guard = TerminalGuard::enter()?;
    term::signal::install();

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut view = View::new();
    view.zoom = config.zoom.max(1.0);
    view.clamp(&geometry, source_w, source_h);

    let renderer = Renderer::spawn(source, config.is_tmux);
    let mut image: Option<KittyImage> = None;
    let mut rendering = true;
    let mut needs_redraw = true;

    loop {
        if term::signal::requested() {
            return Ok(());
        }

        if image.is_none() {
            match renderer.try_recv() {
                render::Poll::Ready(Ok(loaded)) => {
                    guard.set_graphics_cleanup(loaded.delete_sequence());
                    image = Some(loaded);
                    rendering = false;
                    needs_redraw = true;
                }
                render::Poll::Ready(Err(error)) => return Err(FuError::new(error)),
                render::Poll::Empty => {}
                render::Poll::Disconnected => {
                    return Err(FuError::new("renderer stopped unexpectedly"));
                }
            }
        }

        if event::poll(Duration::from_millis(16))? {
            let mut dirty = false;
            loop {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if let Some(cmd) = input::map_key(key) {
                            if cmd == Command::Quit {
                                return Ok(());
                            }
                            if cmd == Command::Rerender {
                                if let Some(image) = image.as_ref() {
                                    image.retransmit();
                                }
                            }
                            apply(
                                cmd,
                                &mut view,
                                &geometry,
                                source_w,
                                source_h,
                                config.overlap,
                            );
                            dirty = true;
                        }
                    }
                    Event::Resize(_, _) => {
                        let new_geometry = term::geometry(font_size)?;
                        view.on_resize(&geometry, &new_geometry, source_w, source_h);
                        geometry = new_geometry;
                        dirty = true;
                    }
                    _ => {}
                }
                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
            if dirty {
                needs_redraw = true;
            }
        }

        if !needs_redraw {
            continue;
        }
        needs_redraw = false;

        terminal.draw(|frame| {
            let area = frame.area();
            // L-shaped reservation: a vertical position bar on the right
            // (like a scrollbar), a horizontal one under the image, then a
            // text status line — leaving the bottom-right corner cell blank,
            // same as a conventional scrollbar corner.
            let image_cols = area
                .width
                .saturating_sub(view::STATUS_COLS)
                .max(1)
                .min(area.width);
            let image_rows = area
                .height
                .saturating_sub(view::STATUS_ROWS)
                .max(1)
                .min(area.height);
            let remaining_rows = area.height - image_rows;
            let horizontal_bar_rows = remaining_rows.min(1);

            let image_area = Rect {
                x: area.x,
                y: area.y,
                width: image_cols,
                height: image_rows,
            };
            let vertical_bar_area = Rect {
                x: area.x + image_cols,
                y: area.y,
                width: area.width - image_cols,
                height: image_rows,
            };
            let horizontal_bar_area = Rect {
                x: area.x,
                y: area.y + image_rows,
                width: image_cols,
                height: horizontal_bar_rows,
            };
            let text_area = Rect {
                x: area.x,
                y: area.y + image_rows + horizontal_bar_rows,
                width: area.width,
                height: remaining_rows - horizontal_bar_rows,
            };

            if let Some(image) = image.as_ref() {
                image.render(&view, &geometry, image_area, frame.buffer_mut());
            }

            let vertical_bar = term::vertical_bar(&view, &geometry, source_w, source_h);
            let vertical_bar_lines: Vec<Line> = vertical_bar
                .into_iter()
                .map(|c| Line::raw(c.to_string()))
                .collect();
            frame.render_widget(Paragraph::new(vertical_bar_lines), vertical_bar_area);

            let horizontal_bar = term::horizontal_bar(&view, &geometry, source_w, source_h);
            frame.render_widget(Paragraph::new(horizontal_bar), horizontal_bar_area);

            frame.render_widget(
                Paragraph::new(term::status_line(&view, rendering)),
                text_area,
            );
        })?;
    }
}

fn apply(
    cmd: Command,
    view: &mut View,
    geometry: &Geometry,
    source_w: u32,
    source_h: u32,
    overlap: f32,
) {
    match cmd {
        Command::PageDown => view.page_down(geometry, source_w, source_h, overlap),
        Command::PageUp => view.page_up(geometry, source_w, source_h, overlap),
        Command::NudgeDown => view.nudge_down(geometry, source_w, source_h),
        Command::NudgeUp => view.nudge_up(geometry, source_w, source_h),
        Command::PanLeft => view.pan_left(geometry, source_w, source_h),
        Command::PanRight => view.pan_right(geometry, source_w, source_h),
        Command::PanLeftFull => view.pan_left_full(geometry, source_w, source_h),
        Command::PanRightFull => view.pan_right_full(geometry, source_w, source_h),
        Command::Top => view.top(),
        Command::Bottom => view.bottom(geometry, source_w, source_h),
        Command::ZoomIn => view.zoom_in(geometry, source_w, source_h),
        Command::ZoomOut => view.zoom_out(geometry, source_w, source_h),
        Command::FitWhole => view.fit_whole(),
        Command::Reset => view.reset(),
        Command::Rerender => {}
        Command::Quit => unreachable!("Quit is handled before apply() is called"),
    }
}
