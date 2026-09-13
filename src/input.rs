//! Terminal event decoding: key events to application commands.
//!
//! Pure mapping only — no terminal I/O. `app.rs` owns reading/draining
//! `crossterm` events and folding the resulting commands into `View` state.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PageDown,
    PageUp,
    NudgeDown,
    NudgeUp,
    PanLeft,
    PanRight,
    PanLeftFull,
    PanRightFull,
    Top,
    Bottom,
    ZoomIn,
    ZoomOut,
    FitWhole,
    Reset,
    Rerender,
    Quit,
}

/// Map one key event to a command, or `None` for unmapped/unknown input.
pub fn map_key(key: KeyEvent) -> Option<Command> {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        Char(' ') | Enter | Char('J') => Some(Command::PageDown),
        Char('f') if ctrl => Some(Command::PageDown),
        PageDown => Some(Command::PageDown),

        Char('b') | Char('K') => Some(Command::PageUp),
        PageUp => Some(Command::PageUp),

        Char('j') | Down => Some(Command::NudgeDown),
        Char('k') | Up => Some(Command::NudgeUp),

        Char('h') | Left => Some(Command::PanLeft),
        Char('l') | Right => Some(Command::PanRight),
        Char('H') => Some(Command::PanLeftFull),
        Char('L') => Some(Command::PanRightFull),

        Char('g') => Some(Command::Top),
        Char('G') => Some(Command::Bottom),

        Char('+') | Char('=') | Char('i') => Some(Command::ZoomIn),
        Char('-') | Char('_') | Char('o') => Some(Command::ZoomOut),
        Char('f') => Some(Command::FitWhole),
        Char('0') => Some(Command::Reset),

        Char('r') => Some(Command::Rerender),
        Char('q') | Esc => Some(Command::Quit),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn page_down_mappings() {
        for k in [
            key(KeyCode::Char(' ')),
            key(KeyCode::Enter),
            key(KeyCode::Char('J')),
            key_ctrl(KeyCode::Char('f')),
            key(KeyCode::PageDown),
        ] {
            assert_eq!(map_key(k), Some(Command::PageDown));
        }
    }

    #[test]
    fn page_up_mappings() {
        for k in [
            key(KeyCode::Char('b')),
            key(KeyCode::Char('K')),
            key_ctrl(KeyCode::Char('b')),
            key(KeyCode::PageUp),
        ] {
            assert_eq!(map_key(k), Some(Command::PageUp));
        }
    }

    #[test]
    fn nudge_mappings() {
        assert_eq!(map_key(key(KeyCode::Char('j'))), Some(Command::NudgeDown));
        assert_eq!(map_key(key(KeyCode::Down)), Some(Command::NudgeDown));
        assert_eq!(map_key(key(KeyCode::Char('k'))), Some(Command::NudgeUp));
        assert_eq!(map_key(key(KeyCode::Up)), Some(Command::NudgeUp));
    }

    #[test]
    fn pan_mappings() {
        assert_eq!(map_key(key(KeyCode::Char('h'))), Some(Command::PanLeft));
        assert_eq!(map_key(key(KeyCode::Left)), Some(Command::PanLeft));
        assert_eq!(map_key(key(KeyCode::Char('l'))), Some(Command::PanRight));
        assert_eq!(map_key(key(KeyCode::Right)), Some(Command::PanRight));
        assert_eq!(map_key(key(KeyCode::Char('H'))), Some(Command::PanLeftFull));
        assert_eq!(
            map_key(key(KeyCode::Char('L'))),
            Some(Command::PanRightFull)
        );
    }

    #[test]
    fn top_bottom_mappings() {
        assert_eq!(map_key(key(KeyCode::Char('g'))), Some(Command::Top));
        assert_eq!(map_key(key(KeyCode::Char('G'))), Some(Command::Bottom));
    }

    #[test]
    fn zoom_mappings() {
        assert_eq!(map_key(key(KeyCode::Char('+'))), Some(Command::ZoomIn));
        assert_eq!(map_key(key(KeyCode::Char('='))), Some(Command::ZoomIn));
        assert_eq!(map_key(key(KeyCode::Char('i'))), Some(Command::ZoomIn));
        assert_eq!(map_key(key(KeyCode::Char('-'))), Some(Command::ZoomOut));
        assert_eq!(map_key(key(KeyCode::Char('_'))), Some(Command::ZoomOut));
        assert_eq!(map_key(key(KeyCode::Char('o'))), Some(Command::ZoomOut));
        assert_eq!(map_key(key(KeyCode::Char('0'))), Some(Command::Reset));
        assert_eq!(map_key(key(KeyCode::Char('f'))), Some(Command::FitWhole));
    }

    #[test]
    fn rerender_mapping() {
        assert_eq!(map_key(key(KeyCode::Char('r'))), Some(Command::Rerender));
    }

    #[test]
    fn quit_mappings() {
        assert_eq!(map_key(key(KeyCode::Char('q'))), Some(Command::Quit));
        assert_eq!(map_key(key(KeyCode::Esc)), Some(Command::Quit));
    }

    #[test]
    fn bare_escape_with_release_kind_still_quits() {
        let mut k = key(KeyCode::Esc);
        k.kind = KeyEventKind::Press;
        assert_eq!(map_key(k), Some(Command::Quit));
    }

    #[test]
    fn unknown_keys_do_nothing() {
        for k in [
            key(KeyCode::Char('z')),
            key(KeyCode::Char('1')),
            key(KeyCode::Tab),
            key(KeyCode::F(5)),
            key_ctrl(KeyCode::Char('x')),
        ] {
            assert_eq!(map_key(k), None);
        }
    }
}
