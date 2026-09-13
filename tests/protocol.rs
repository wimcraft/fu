//! Integration-level tests for the tmux/Kitty policy gate.

use fu::terminal::CommandRunner;
use ratatui_image::picker::ProtocolType;

struct ScriptedTmux(&'static str);

impl CommandRunner for ScriptedTmux {
    fn run(&self, _program: &str, _args: &[&str]) -> Option<String> {
        Some(self.0.to_string())
    }
}

#[test]
fn tmux_off_fails_before_a_picker_would_ever_be_touched() {
    // The gate itself never constructs a Picker; a caller following the
    // documented order (gate first, then `protocol::init_picker`) never
    // triggers the library's own tmux side effects when this fails.
    let err = fu::terminal::check_tmux_passthrough(true, &ScriptedTmux("off")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "fu: tmux pixel graphics require: set -g allow-passthrough on"
    );
}

#[test]
fn non_kitty_picker_selection_is_rejected() {
    for protocol in [
        ProtocolType::Halfblocks,
        ProtocolType::Sixel,
        ProtocolType::Iterm2,
    ] {
        assert!(fu::protocol::require_kitty(false, protocol).is_err());
    }
    assert!(fu::protocol::require_kitty(false, ProtocolType::Kitty).is_ok());
}
