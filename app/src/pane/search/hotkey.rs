//! A single global activation binding, owned by the runtime window.
use super::everything_settings;
use crate::pane::*;
use windows_sys::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};

pub(in crate::pane) const ID: i32 = 0x4c50;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::pane) struct Shortcut {
    pub key: u16,
    pub modifiers: u8,
}
impl Default for Shortcut {
    fn default() -> Self {
        Self {
            key: VK_SPACE,
            modifiers: 3,
        }
    }
}
thread_local! {
    static CONFIG: RefCell<Shortcut> = RefCell::new(Shortcut::default());
    static STATUS: RefCell<String> = const { RefCell::new(String::new()) };
}
pub(in crate::pane) fn settings() -> Shortcut {
    CONFIG.with(|s| *s.borrow())
}
pub(in crate::pane) fn status() -> String {
    STATUS.with(|s| s.borrow().clone())
}
pub(in crate::pane) fn valid(value: Shortcut) -> bool {
    // Require Ctrl or Alt; reserve system/menu combinations and the debugger's F12.
    value.modifiers <= 7
        && value.modifiers & 5 != 0
        && value.key != VK_F12
        && peek::valid_shortcut(value.key, value.modifiers)
}
fn decode(raw: &str) -> Option<Shortcut> {
    let (key, modifiers) = raw.split_once(':')?;
    let value = Shortcut {
        key: key.parse().ok()?,
        modifiers: modifiers.parse().ok()?,
    };
    valid(value).then_some(value)
}
pub(in crate::pane) fn load(store: &WorkspaceStore) -> Result<(), String> {
    let value = store
        .preference("search_hotkey")
        .map_err(|e| e.to_string())?
        .and_then(|raw| decode(&raw))
        .unwrap_or_default();
    CONFIG.with(|s| *s.borrow_mut() = value);
    Ok(())
}
pub(in crate::pane) fn save(store: &WorkspaceStore, value: Shortcut) -> Result<(), String> {
    if !valid(value) {
        return Err(
            crate::i18n::text("ui-use-ctrl-or-alt-with-a-letter-digit-space-or-function-key-avoid-exis")
                .into(),
        );
    }
    store
        .save_preference(
            "search_hotkey",
            &format!("{}:{}", value.key, value.modifiers),
        )
        .map_err(|e| e.to_string())?;
    CONFIG.with(|s| *s.borrow_mut() = value);
    Ok(())
}
pub(in crate::pane) fn label(value: Shortcut) -> String {
    peek::shortcut_label(&peek::Settings {
        key: value.key,
        modifiers: value.modifiers,
        ..Default::default()
    })
}
fn flags(value: Shortcut) -> u32 {
    MOD_NOREPEAT
        | if value.modifiers & 1 != 0 {
            MOD_CONTROL
        } else {
            0
        }
        | if value.modifiers & 2 != 0 {
            MOD_SHIFT
        } else {
            0
        }
        | if value.modifiers & 4 != 0 { MOD_ALT } else { 0 }
}
#[derive(Default)]
pub(in crate::pane) struct Registration {
    hwnd: isize,
    desired: Option<Shortcut>,
    registered: bool,
    attempted: Option<std::time::Instant>,
}
impl Registration {
    pub fn retry_deadline(&self) -> Option<std::time::Instant> {
        self.attempted.filter(|_| self.desired.is_some() && !self.registered)
            .map(|time| time + std::time::Duration::from_secs(10))
    }
    pub fn update(&mut self, hwnd: isize, desired: Option<Shortcut>) {
        if self.desired == desired
            && (self.registered
                || desired.is_none()
                || self.attempted.is_some_and(|t| t.elapsed().as_secs() < 10))
        {
            return;
        }
        self.clear();
        self.hwnd = hwnd;
        self.desired = desired;
        self.attempted = Some(std::time::Instant::now());
        self.registered = desired.is_some_and(|value| unsafe { RegisterHotKey(hwnd as _, ID, flags(value), u32::from(value.key)) } != 0);
        let status = if desired.is_none() {
            crate::i18n::text("ui-search-is-disabled-global-shortcut-is-not-registered").into()
        } else if self.registered {
            crate::i18n::text("ui-works-globally-esc-cancels-recording").into()
        } else {
            crate::i18n::format("ui-is-unavailable-choose-another-shortcut", &[("arg0", format!("{}", label(desired.unwrap())))])
        };
        STATUS.with(|s| *s.borrow_mut() = status);
    }
    fn clear(&mut self) {
        if self.registered {
            unsafe {
                UnregisterHotKey(self.hwnd as _, ID);
            }
        }
        self.registered = false;
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(in crate::pane) fn activate(state: &Rc<RefCell<PaneApp>>) {
    let target = {
        let s = state.borrow();
        if !everything_settings::enabled(&s.store).unwrap_or(false) {
            return;
        }
        s.views
            .iter()
            .find(|v| s.workspace.panel(v.id).is_some_and(Panel::is_search))
            .map(|v| v.window.hwnd() as isize)
    };
    if let Some(hwnd) = target {
        unsafe {
            ShowWindow(hwnd as _, SW_SHOWNORMAL);
            SetForegroundWindow(hwnd as _);
            PostMessageW(hwnd as _, search::FOCUS_INPUT, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_registration_reports_conflicts_and_releases_disabled_binding() {
        let window = windows_window::Window::new("Hotkey test")
            .style(WS_POPUP)
            .on_message(|_, msg, _, _| (msg == WM_DESTROY).then_some(0))
            .create()
            .unwrap();
        let hwnd = window.hwnd() as isize;
        let shortcut = (VK_F13..=VK_F24).filter(|key| *key != VK_F12).map(|key| Shortcut { key, modifiers: 7 }).find(|value| unsafe { RegisterHotKey(hwnd as _, ID + 1, flags(*value), u32::from(value.key)) } != 0).expect("available test binding");
        let mut registration = Registration::default();
        registration.update(hwnd, Some(shortcut));
        assert!(
            !registration.registered,
            "must not steal an existing binding"
        );
        unsafe {
            UnregisterHotKey(hwnd as _, ID + 1);
        }
        registration.update(hwnd, None);
        registration.update(hwnd, Some(shortcut));
        assert!(registration.registered);
        registration.update(hwnd, None);
        assert_ne!(
            unsafe { RegisterHotKey(hwnd as _, ID + 1, flags(shortcut), u32::from(shortcut.key)) },
            0
        );
        unsafe {
            UnregisterHotKey(hwnd as _, ID + 1);
        }
    }
    #[test]
    fn shortcut_persists_and_reserves_system_and_file_commands() {
        let store = WorkspaceStore::open_in_memory().unwrap();
        save(&store, Shortcut::default()).unwrap();
        load(&store).unwrap();
        assert_eq!(settings(), Shortcut::default());
        assert_eq!(label(settings()), "Ctrl + Shift + Space");
        for value in [
            Shortcut {
                key: VK_F12,
                modifiers: 3,
            },
            Shortcut {
                key: VK_SPACE,
                modifiers: 4,
            },
            Shortcut {
                key: 0x43,
                modifiers: 1,
            },
            Shortcut {
                key: 0x41,
                modifiers: 0,
            },
        ] {
            assert!(save(&store, value).is_err());
        }
        assert_eq!(decode("broken"), None);
        assert_eq!(
            flags(Shortcut::default()),
            MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT
        );
    }
}
