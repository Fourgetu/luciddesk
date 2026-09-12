//! Pane-local shortcuts. Text editors retain their own keyboard handling.
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Navigate(u16),
    Open,
    Peek,
    Rename,
    Refresh,
    Cancel,
    Menu,
    File(desktop_shell::FileCommand),
    SelectAll,
    ToggleSelection,
}

#[derive(Default)]
pub(super) struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub windows: bool,
}

impl Modifiers {
    pub fn current() -> Self {
        let down = |key| unsafe { GetKeyState(i32::from(key)) < 0 };
        Self {
            ctrl: down(VK_CONTROL),
            shift: down(VK_SHIFT),
            alt: down(VK_MENU),
            windows: down(VK_LWIN) || down(VK_RWIN),
        }
    }
}

pub(super) fn command(key: u16, modifiers: &Modifiers, repeat: bool) -> Option<Command> {
    if modifiers.alt || modifiers.windows {
        return None;
    }
    if matches!(
        key,
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
    ) {
        return Some(Command::Navigate(key));
    }
    if modifiers.ctrl {
        if modifiers.shift || repeat {
            return None;
        }
        return match key {
            0x41 => Some(Command::SelectAll),
            VK_SPACE => Some(Command::ToggleSelection),
            0x43 => Some(Command::File(desktop_shell::FileCommand::Copy)),
            0x58 => Some(Command::File(desktop_shell::FileCommand::Cut)),
            0x56 => Some(Command::File(desktop_shell::FileCommand::Paste)),
            _ => None,
        };
    }
    if modifiers.shift {
        return (key == VK_F10 && !repeat).then_some(Command::Menu);
    }
    match key {
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT => {
            Some(Command::Navigate(key))
        }
        VK_ESCAPE => Some(Command::Cancel),
        VK_RETURN if !repeat => Some(Command::Open),
        VK_F2 if !repeat => Some(Command::Rename),
        VK_F5 if !repeat => Some(Command::Refresh),
        VK_APPS if !repeat => Some(Command::Menu),
        VK_DELETE if !repeat => Some(Command::File(desktop_shell::FileCommand::Delete)),
        _ => None,
    }
}

pub(super) fn next_selection(
    key: u16,
    selected: Option<usize>,
    count: usize,
    columns: usize,
    visible_rows: usize,
) -> Option<usize> {
    let last = count.checked_sub(1)?;
    if key == VK_END {
        return Some(last);
    }
    if key == VK_HOME || selected.is_none() {
        return Some(0);
    }
    let at = selected.unwrap().min(last);
    let columns = columns.max(1);
    let page = columns.saturating_mul(visible_rows.max(1));
    Some(match key {
        VK_LEFT => at.saturating_sub(1),
        VK_RIGHT => at.saturating_add(1).min(last),
        VK_UP => at.saturating_sub(columns),
        VK_DOWN => at.saturating_add(columns).min(last),
        VK_PRIOR => at.saturating_sub(page),
        VK_NEXT => at.saturating_add(page).min(last),
        _ => at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_and_repeats_do_not_trigger_unintended_commands() {
        let plain = Modifiers::default();
        assert_eq!(command(VK_SPACE, &plain, false), None);
        assert_eq!(command(VK_SPACE, &plain, true), None);
        assert_eq!(
            command(
                VK_SPACE,
                &Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
        assert_eq!(
            command(
                VK_SPACE,
                &Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
        assert_eq!(
            command(
                VK_SPACE,
                &Modifiers {
                    windows: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
        for (key, expected) in [
            (VK_RETURN, Command::Open),
            (VK_F2, Command::Rename),
            (VK_F5, Command::Refresh),
            (VK_APPS, Command::Menu),
        ] {
            assert_eq!(command(key, &plain, false), Some(expected));
            assert_eq!(command(key, &plain, true), None);
            assert_eq!(
                command(
                    key,
                    &Modifiers {
                        ctrl: true,
                        ..Modifiers::default()
                    },
                    false
                ),
                None
            );
            assert_eq!(
                command(
                    key,
                    &Modifiers {
                        alt: true,
                        ..Modifiers::default()
                    },
                    false
                ),
                None
            );
        }
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(command(VK_F10, &shift, false), Some(Command::Menu));
        assert_eq!(command(VK_F10, &plain, false), None);
        assert_eq!(
            command(VK_RIGHT, &shift, false),
            Some(Command::Navigate(VK_RIGHT))
        );
        assert_eq!(
            command(VK_RIGHT, &plain, true),
            Some(Command::Navigate(VK_RIGHT))
        );
        assert_eq!(command(0x41, &plain, false), None);
    }

    #[test]
    fn navigation_handles_empty_first_selection_pages_and_boundaries() {
        for key in [
            VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_PRIOR, VK_NEXT,
        ] {
            assert_eq!(next_selection(key, None, 0, 3, 2), None);
            assert_eq!(next_selection(key, None, 11, 3, 2), Some(0));
        }
        for (key, at, expected) in [
            (VK_END, 0, 10),
            (VK_HOME, 10, 0),
            (VK_NEXT, 1, 7),
            (VK_NEXT, 7, 10),
            (VK_PRIOR, 7, 1),
            (VK_PRIOR, 1, 0),
            (VK_UP, 1, 0),
            (VK_DOWN, 9, 10),
            (VK_LEFT, 0, 0),
            (VK_RIGHT, 10, 10),
        ] {
            assert_eq!(next_selection(key, Some(at), 11, 3, 2), Some(expected));
        }
    }

    #[test]
    fn file_shortcuts_require_exact_modifiers_and_ignore_autorepeat() {
        use desktop_shell::FileCommand;
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        assert_eq!(command(0x41, &ctrl, false), Some(Command::SelectAll));
        assert_eq!(
            command(VK_SPACE, &ctrl, false),
            Some(Command::ToggleSelection)
        );
        let range = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            command(VK_END, &range, true),
            Some(Command::Navigate(VK_END))
        );
        assert_eq!(command(0x41, &range, false), None);
        for (key, operation) in [
            (0x43, FileCommand::Copy),
            (0x58, FileCommand::Cut),
            (0x56, FileCommand::Paste),
        ] {
            assert_eq!(command(key, &ctrl, false), Some(Command::File(operation)));
            assert_eq!(command(key, &ctrl, true), None);
            assert_eq!(command(key, &Modifiers::default(), false), None);
        }
        assert_eq!(
            command(VK_DELETE, &Modifiers::default(), false),
            Some(Command::File(FileCommand::Delete))
        );
        assert_eq!(
            command(
                VK_DELETE,
                &Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                false
            ),
            None
        );
    }
}
