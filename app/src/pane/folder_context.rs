//! Folder content invokes Explorer's native directory background menu.
use super::{Event, GroupModel};
use desktop_shell::FolderMenuResult;
use std::{cell::RefCell, rc::Rc};
use windows_sys::Win32::Foundation::{HWND, POINT};

pub(super) fn is_content_background(model: &GroupModel, y: Option<f32>, keyboard: bool) -> bool {
    model.folder.is_some()
        && !model.collapsed
        && if let Some(y) = y {
            y >= model.content_header()
                + if model.is_list() {
                    super::layout::LIST_HEADER
                } else {
                    0.0
                }
        } else {
            keyboard
        }
}

pub(super) fn show(
    owner: HWND,
    point: POINT,
    model: &Rc<RefCell<GroupModel>>,
    event: impl Fn(Event),
) -> Result<(), String> {
    let path = model.borrow().folder.clone();
    let Some(path) = path else {
        return Ok(());
    };
    // Release the model borrow before Shell calls, which can pump window messages.
    match desktop_shell::show_folder_menu(
        windows::Win32::Foundation::HWND(owner),
        &path,
        windows::Win32::Foundation::POINT {
            x: point.x,
            y: point.y,
        },
    )
    .map_err(|error| error.to_string())?
    {
        FolderMenuResult::Invoked {
            created: Some(path),
        } => event(Event::FolderItemCreated(path)),
        FolderMenuResult::Cancelled | FolderMenuResult::Invoked { created: None } => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn right_click_routes_title_columns_and_empty_content_separately() {
        let state = super::super::tests::test_state();
        let mut model = super::super::create_model(&state, desktop_core::PanelId::new(1)).unwrap();
        assert!(!is_content_background(&model, Some(200.0), false));
        model.folder = Some(std::path::PathBuf::from(r"C:\Data"));
        model.list_view = true;
        assert!(!is_content_background(
            &model,
            Some(model.content_header() - 1.0),
            false
        ));
        assert!(!is_content_background(
            &model,
            Some(model.content_header() + 1.0),
            false
        ));
        assert!(is_content_background(
            &model,
            Some(model.content_header() + super::super::layout::LIST_HEADER + 1.0),
            false
        ));
        assert!(is_content_background(&model, None, true));
        assert!(!is_content_background(&model, None, false));
        model.collapsed = true;
        assert!(!is_content_background(&model, None, true));
    }
}
