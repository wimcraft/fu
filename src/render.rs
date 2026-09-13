//! One-time Kitty image upload plus cheap source-rectangle placements.
//!
//! The decoded source is encoded for transmission once on a background
//! thread. Pan, scroll, and zoom then replace one Kitty placement by sending
//! only its crop coordinates; source pixels are never cropped, resized, or
//! retransmitted during navigation.

use std::fmt::Write as _;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use base64::Engine as _;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder as _, RgbaImage};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;

use crate::view::{FitMode, Geometry, View};

const CHARS_PER_CHUNK: usize = 4096;
const BYTES_PER_CHUNK: usize = (CHARS_PER_CHUNK / 4) * 3;
const PLACEMENT_ID: u32 = 1;
static NEXT_IMAGE_ID: AtomicU32 = AtomicU32::new(1);

pub enum Poll<T> {
    Ready(T),
    Empty,
    Disconnected,
}

/// Prepares the one-time source upload without blocking terminal input.
pub struct Renderer {
    result_rx: Receiver<Result<KittyImage, String>>,
    _worker: JoinHandle<()>,
}

impl Renderer {
    pub fn spawn(source: Arc<RgbaImage>, is_tmux: bool) -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let image = KittyImage::new(&source, next_image_id(), is_tmux);
            let _ = result_tx.send(image);
        });
        Self {
            result_rx,
            _worker: worker,
        }
    }

    pub fn try_recv(&self) -> Poll<Result<KittyImage, String>> {
        match self.result_rx.try_recv() {
            Ok(image) => Poll::Ready(image),
            Err(TryRecvError::Empty) => Poll::Empty,
            Err(TryRecvError::Disconnected) => Poll::Disconnected,
        }
    }
}

/// A terminal-resident source image. Rendering after the first frame emits
/// only a small placement command that selects a source rectangle.
pub struct KittyImage {
    id: u32,
    width: u32,
    height: u32,
    is_tmux: bool,
    transmission: String,
    transmitted: AtomicBool,
}

impl KittyImage {
    fn new(source: &RgbaImage, id: u32, is_tmux: bool) -> Result<Self, String> {
        Ok(Self {
            id,
            width: source.width(),
            height: source.height(),
            is_tmux,
            transmission: transmission(source, id, is_tmux)?,
            transmitted: AtomicBool::new(false),
        })
    }

    /// Force the next draw to upload the source again before placing it.
    pub fn retransmit(&self) {
        self.transmitted.store(false, Ordering::Release);
    }

    /// Delete this image and all of its placements, freeing terminal storage.
    pub fn delete_sequence(&self) -> String {
        command(self.is_tmux, &format!("a=d,d=I,i={},q=2", self.id))
    }

    pub fn render(&self, view: &View, geometry: &Geometry, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let crop = view.crop(geometry, self.width, self.height);
        let mut output = String::new();
        if !self.transmitted.swap(true, Ordering::AcqRel) {
            output.push_str(&self.transmission);
        }

        let constraint = match view.fit {
            FitMode::Width => format!("c={}", geometry.draw_columns),
            FitMode::Whole
                if crop.width as u64 * geometry.draw_pixel_height() as u64
                    >= crop.height as u64 * geometry.draw_pixel_width() as u64 =>
            {
                format!("c={}", geometry.draw_columns)
            }
            FitMode::Whole => format!("r={}", geometry.draw_rows),
        };
        output.push_str(&command(
            self.is_tmux,
            &format!(
                "a=p,i={},p={PLACEMENT_ID},x={},y={},w={},h={},\
                 {constraint},C=1,q=2",
                self.id, crop.x, crop.y, crop.width, crop.height
            ),
        ));

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let Some(cell) = buf.cell_mut((x, y)) else {
                    continue;
                };
                if x == area.left() && y == area.top() {
                    cell.set_symbol(&output)
                        .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()));
                } else {
                    cell.set_diff_option(CellDiffOption::Skip);
                }
            }
        }
    }
}

fn next_image_id() -> u32 {
    let sequence = NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed);
    let id = std::process::id().wrapping_mul(0x9E37_79B1) ^ sequence;
    id.max(1)
}

fn wrappers(is_tmux: bool) -> (&'static str, &'static str, &'static str) {
    if is_tmux {
        ("\x1bPtmux;", "\x1b\x1b", "\x1b\\")
    } else {
        ("", "\x1b", "")
    }
}

fn command(is_tmux: bool, control: &str) -> String {
    let (start, escape, end) = wrappers(is_tmux);
    format!("{start}{escape}_G{control};{escape}\\{end}")
}

fn transmission(source: &RgbaImage, id: u32, is_tmux: bool) -> Result<String, String> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            source.as_raw(),
            source.width(),
            source.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("cannot prepare terminal image: {error}"))?;

    let chunk_count = png.len().div_ceil(BYTES_PER_CHUNK);
    let (start, escape, end) = wrappers(is_tmux);
    let wrapper_len = start.len() + escape.len() * 2 + end.len() + 64;
    let encoded_len = png.len().div_ceil(3) * 4;
    let mut output =
        String::with_capacity(encoded_len.saturating_add(chunk_count.saturating_mul(wrapper_len)));

    for (index, chunk) in png.chunks(BYTES_PER_CHUNK).enumerate() {
        output.push_str(start);
        write!(output, "{escape}_Gq=2,").unwrap();
        if index == 0 {
            write!(output, "a=t,i={id},f=100,t=d,").unwrap();
        }
        let more = u8::from(index + 1 < chunk_count);
        write!(output, "m={more};").unwrap();
        base64::engine::general_purpose::STANDARD.encode_string(chunk, &mut output);
        write!(output, "{escape}\\{end}").unwrap();
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;

    fn source() -> RgbaImage {
        RgbaImage::from_pixel(40, 100, image::Rgba([255, 0, 0, 255]))
    }

    fn geometry() -> Geometry {
        Geometry::new(10, 5, 100, 100)
    }

    #[test]
    fn source_pixels_are_uploaded_only_on_first_render() {
        let image = KittyImage::new(&source(), 42, false).unwrap();
        let area = Rect::new(0, 0, 9, 3);
        let mut first = Buffer::empty(area);
        image.render(&View::new(), &geometry(), area, &mut first);
        let first_output = first.cell((0, 0)).unwrap().symbol();
        assert!(first_output.contains("_Gq=2,a=t,i=42"));
        assert!(first_output.contains("_Ga=p,i=42,p=1"));

        let mut view = View::new();
        view.nudge_down(&geometry(), 40, 100);
        let mut second = Buffer::empty(area);
        image.render(&view, &geometry(), area, &mut second);
        let second_output = second.cell((0, 0)).unwrap().symbol();
        assert!(!second_output.contains("a=t"));
        assert!(second_output.contains("_Ga=p,i=42,p=1"));
        assert!(second_output.len() < 200);
    }

    #[test]
    fn fit_whole_uses_height_for_a_portrait_image() {
        let image = KittyImage::new(&source(), 42, false).unwrap();
        let area = Rect::new(0, 0, 9, 3);
        let mut view = View::new();
        view.fit_whole();
        let mut buffer = Buffer::empty(area);
        image.render(&view, &geometry(), area, &mut buffer);
        let output = buffer.cell((0, 0)).unwrap().symbol();
        assert!(output.contains("x=0,y=0,w=40,h=100,r=3"));
    }

    #[test]
    fn tmux_commands_are_passthrough_wrapped() {
        let sequence = command(true, "a=p,i=42,p=1");
        assert_eq!(
            sequence,
            "\x1bPtmux;\x1b\x1b_Ga=p,i=42,p=1;\x1b\x1b\\\x1b\\"
        );
    }

    #[test]
    fn renderer_prepares_one_terminal_image() {
        let renderer = Renderer::spawn(Arc::new(source()), false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match renderer.try_recv() {
                Poll::Ready(Ok(image)) => {
                    assert_eq!(image.width, 40);
                    assert_eq!(image.height, 100);
                    break;
                }
                Poll::Ready(Err(error)) => panic!("{error}"),
                Poll::Empty if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Poll::Empty | Poll::Disconnected => panic!("renderer did not produce an image"),
            }
        }
    }
}
