//! Typed, single-line, user-facing errors. `Display` always renders the
//! `fu: ...` diagnostic exactly as it should be printed to stderr.

#[derive(Debug, thiserror::Error)]
#[error("fu: {message}")]
pub struct FuError {
    message: String,
}

impl FuError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for FuError {
    fn from(err: std::io::Error) -> Self {
        Self::new(err.to_string())
    }
}

impl From<image::ImageError> for FuError {
    fn from(err: image::ImageError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<ratatui_image::errors::Errors> for FuError {
    fn from(err: ratatui_image::errors::Errors) -> Self {
        Self::new(err.to_string())
    }
}
