//! Optional global reveal shortcut, independent of Everything search.
use super::*;
use super::search::hotkey::{self, Shortcut};
pub(super) const ID: i32 = 0x4c51;
thread_local! { static STATUS: RefCell<String> = const { RefCell::new(String::new()) }; }

pub(super) fn default_shortcut() -> Shortcut { Shortcut { key: 0x44, modifiers: 3 } }
pub(super) fn enabled(store: &WorkspaceStore) -> bool {
    store.preference("show_panels_enabled").ok().flatten().as_deref() == Some("1")
}
pub(super) fn settings(store: &WorkspaceStore) -> Shortcut {
    store.preference("show_panels_hotkey").ok().flatten()
        .and_then(|raw| hotkey::decode(&raw)).unwrap_or_else(default_shortcut)
}
pub(super) fn save(store: &WorkspaceStore, value: Shortcut) -> Result<(), String> {
    if !hotkey::valid(value) {
        return Err(crate::i18n::text("ui-use-ctrl-or-alt-with-a-letter-digit-space-or-function-key-avoid-exis").into());
    }
    store.save_preference("show_panels_hotkey", &format!("{}:{}", value.key, value.modifiers))
        .map_err(|e| e.to_string())
}
pub(super) fn status() -> String { STATUS.with(|s| s.borrow().clone()) }
pub(super) fn update_status(value: String) { STATUS.with(|s| *s.borrow_mut() = value); }
pub(super) fn activate(state: &Rc<RefCell<PaneApp>>) {
    if enabled(&state.borrow().store) { quick_reveal::show_all(state); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    #[test]
    fn disabled_does_nothing_and_enabled_reveals_every_panel_type() {
        let mut app = super::super::tests::test_state();
        for n in 10..13 {
            let id = PanelId::new(n);
            let mut panel = Panel::new(id, format!("Panel {n}"), RectDip::default());
            if n == 11 { panel.set_folder(Some(std::env::temp_dir())); }
            if n == 12 { panel.set_search(true); }
            app.workspace.add_panel(panel).unwrap();
            let model = Rc::new(RefCell::new(create_model(&app, id).unwrap()));
            let window = windows_window::Window::new("Reveal shortcut test")
                .style(WS_POPUP).size(100, 100).create().unwrap();
            super::super::window::set_layer(window.hwnd().cast(), n == 11);
            unsafe { ShowWindow(window.hwnd().cast(), SW_HIDE); }
            app.views.push(View { id, target: Rc::new(std::cell::Cell::new(id)), window, model });
        }
        let state = Rc::new(RefCell::new(app));
        activate(&state);
        for view in &state.borrow().views { assert_eq!(unsafe { IsWindowVisible(view.window.hwnd().cast()) }, 0); }
        state.borrow().store.save_preference("show_panels_enabled", "1").unwrap();
        activate(&state);
        for (i, view) in state.borrow().views.iter().enumerate() {
            assert_ne!(unsafe { IsWindowVisible(view.window.hwnd().cast()) }, 0);
            assert_eq!(quick_reveal::permanent_topmost(view.window.hwnd().cast()), i == 1);
        }
    }
}
