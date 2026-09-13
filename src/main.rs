//! CLI, lifecycle, and error reporting. All terminal state is owned by
//! `fu::app::run`; this file only validates arguments, decodes the source
//! image, and runs the tmux/Kitty gates before handing off.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use image::RgbaImage;

use fu::app::{self, AppConfig};
use fu::error::FuError;
use fu::protocol;
use fu::terminal::{self, SystemCommandRunner};

/// view a zoomed, pannable region of an image in a Ghostty/tmux pane
#[derive(Parser, Debug)]
#[command(name = "fu", version, disable_version_flag = true)]
struct Cli {
    /// initial zoom multiplier; finite number >= 1
    #[arg(short = 'z', default_value_t = 1.0, value_parser = parse_zoom)]
    zoom: f32,

    /// repeated fraction for paging; 0 <= value < 1
    #[arg(short = 'o', default_value_t = 0.12, value_parser = parse_overlap)]
    overlap: f32,

    /// image to display
    image: PathBuf,
}

fn parse_zoom(raw: &str) -> Result<f32, String> {
    let value: f32 = raw.parse().map_err(|_| "must be a number".to_string())?;
    if !value.is_finite() || value < 1.0 {
        return Err("must be a finite number >= 1".to_string());
    }
    Ok(value)
}

fn parse_overlap(raw: &str) -> Result<f32, String> {
    let value: f32 = raw.parse().map_err(|_| "must be a number".to_string())?;
    if !value.is_finite() || !(0.0..1.0).contains(&value) {
        return Err("must be a fraction in [0, 1)".to_string());
    }
    Ok(value)
}

fn decode_image(path: &Path) -> Result<(RgbaImage, u32, u32), FuError> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| FuError::new(format!("cannot open {}: {e}", path.display())))?
        .with_guessed_format()
        .map_err(|e| FuError::new(format!("cannot read {}: {e}", path.display())))?;
    let decoded = reader
        .decode()
        .map_err(|e| FuError::new(format!("cannot decode {}: {e}", path.display())))?;

    let width = decoded.width();
    let height = decoded.height();
    if width == 0 || height == 0 {
        return Err(FuError::new(format!(
            "{} has zero dimensions",
            path.display()
        )));
    }
    Ok((decoded.to_rgba8(), width, height))
}

fn run(cli: Cli) -> Result<(), FuError> {
    let (rgba, width, height) = decode_image(&cli.image)?;

    let is_tmux = std::env::var_os("TMUX").is_some();
    terminal::check_tmux_passthrough(is_tmux, &SystemCommandRunner)?;

    let picker = protocol::init_picker(is_tmux)?;
    protocol::require_kitty(is_tmux, picker.protocol_type())?;

    let config = AppConfig {
        zoom: cli.zoom,
        overlap: cli.overlap,
        is_tmux,
    };
    app::run(Arc::new(rgba), width, height, picker, config)
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
