//! Picker setup and the Kitty-protocol requirement.
//!
//! `Picker::from_query_stdio()` owns all terminal pixel/capability probing
//! (see `ratatui_image::picker`); this module only enforces the product
//! policy that the selected protocol must be Kitty, and must never run
//! before the tmux passthrough gate in `terminal.rs` has already passed.

use ratatui_image::picker::{Picker, ProtocolType};

use crate::error::FuError;
use crate::terminal::TMUX_PASSTHROUGH_ERROR;

/// Query the terminal for its graphics capabilities. Must only be called
/// after `terminal::check_tmux_passthrough` has already succeeded, so a
/// disabled passthrough never triggers the library's own tmux probing.
///
/// `is_tmux` is our own, authoritative `$TMUX`-based determination (the same
/// one `terminal::check_tmux_passthrough` used). The picker decides its own,
/// separate `is_tmux` flag — which controls whether Kitty output gets
/// wrapped in the DCS passthrough sequence tmux requires — purely from
/// `$TERM`/`$TERM_PROGRAM` string prefixes, so it can disagree with reality
/// whenever tmux reports a non-`tmux`-prefixed `TERM` (e.g. `screen-256color`,
/// a common default). When that happens the picker silently sends *unwrapped*
/// Kitty escapes, which tmux correctly drops, leaving a blank pane that still
/// reports success. If we already know we're in tmux, nudge its detection to
/// agree before it ever queries.
pub fn init_picker(is_tmux: bool) -> Result<Picker, FuError> {
    if is_tmux && !picker_would_detect_tmux() {
        std::env::set_var("TERM_PROGRAM", "tmux");
    }
    Ok(Picker::from_query_stdio()?)
}

fn picker_would_detect_tmux() -> bool {
    tmux_hinted_by_env(
        std::env::var("TERM").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
    )
}

/// Mirrors `ratatui_image::picker`'s own (private) tmux-detection heuristic.
fn tmux_hinted_by_env(term: Option<&str>, term_program: Option<&str>) -> bool {
    term.is_some_and(|t| t.starts_with("tmux")) || term_program == Some("tmux")
}

/// Enforce that the picker selected the Kitty protocol. Under tmux, any
/// failure surfaces the same passthrough-configuration message, since a
/// non-Kitty selection there almost always means passthrough isn't really
/// active end-to-end even if the option reads "on".
pub fn require_kitty(is_tmux: bool, protocol: ProtocolType) -> Result<(), FuError> {
    if protocol == ProtocolType::Kitty {
        return Ok(());
    }
    if is_tmux {
        Err(FuError::new(TMUX_PASSTHROUGH_ERROR))
    } else {
        Err(FuError::new(
            "terminal does not support the Kitty graphics protocol",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_protocol_passes() {
        assert!(require_kitty(false, ProtocolType::Kitty).is_ok());
        assert!(require_kitty(true, ProtocolType::Kitty).is_ok());
    }

    #[test]
    fn non_kitty_outside_tmux_reports_generic_error() {
        let err = require_kitty(false, ProtocolType::Halfblocks).unwrap_err();
        assert_eq!(
            err.to_string(),
            "fu: terminal does not support the Kitty graphics protocol"
        );
    }

    #[test]
    fn non_kitty_inside_tmux_reports_passthrough_error() {
        let err = require_kitty(true, ProtocolType::Sixel).unwrap_err();
        assert_eq!(
            err.to_string(),
            "fu: tmux pixel graphics require: set -g allow-passthrough on"
        );
    }

    #[test]
    fn tmux_hinted_by_term_prefix() {
        assert!(tmux_hinted_by_env(Some("tmux-256color"), None));
    }

    #[test]
    fn tmux_hinted_by_term_program() {
        assert!(tmux_hinted_by_env(Some("xterm-ghostty"), Some("tmux")));
    }

    #[test]
    fn tmux_not_hinted_by_screen_term() {
        // The common case that breaks the picker's own detection: tmux
        // reporting a `screen`-derived TERM with neither hint present.
        assert!(!tmux_hinted_by_env(Some("screen-256color"), None));
    }

    #[test]
    fn tmux_not_hinted_when_nothing_set() {
        assert!(!tmux_hinted_by_env(None, None));
    }
}
