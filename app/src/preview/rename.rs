//! Native inline EDIT with its own redirected surface above the composition pane.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
use super::GroupModel;
use desktop_core::ShellIdentity;
use std::{cell::RefCell, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::*,
    UI::{
        Controls::*,
        HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi},
        Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const PROPERTY: windows_sys::core::PCWSTR = windows_sys::w!("LucidPane.RenameEdit");
const SUBCLASS: usize = 0x4c50_524e;
const FINISH: u32 = WM_APP + 71;

struct Name {
    text: String,
    suffix: String,
    selection: usize,
}
impl Name {
    fn new(identity: &ShellIdentity, label: &str) -> Self {
        let mut text = label.to_string();
        let mut suffix = String::new();
        let mut selection = text.encode_utf16().count();
        if let ShellIdentity::FileSystem { path, .. } = identity {
            if let Some(filename) = path.file_name() {
                text = filename.to_string_lossy().into_owned();
                if !path.is_dir()
                    && let Some(dot) = text.rfind('.').filter(|&i| i > 0)
                {
                    if text[..dot] == *label {
                        suffix = text[dot..].to_string();
                        text.truncate(dot);
                    }
                    selection = text[..dot.min(text.len())].encode_utf16().count();
                } else {
                    selection = text.encode_utf16().count();
                }
            }
        }
        Self {
            text,
            suffix,
            selection,
        }
    }
    fn committed(&self, value: &str) -> String {
        format!("{value}{}", self.suffix)
    }
}
struct Editor {
    owner: HWND,
    identity: ShellIdentity,
    model: Rc<RefCell<GroupModel>>,
    name: Name,
    font: HFONT,
    composing: bool,
    finishing: bool,
}
pub(super) fn active(owner: HWND) -> bool {
    unsafe { !GetPropW(owner, PROPERTY).is_null() }
}

pub(super) fn show(
    owner: HWND,
    identity: &ShellIdentity,
    label: &str,
    model: Rc<RefCell<GroupModel>>,
) -> Result<(), String> {
    unsafe {
        if active(owner) {
            SetFocus(GetPropW(owner, PROPERTY));
            return Ok(());
        }
        let dpi = GetDpiForWindow(owner).max(96);
        let mut logical_font = LOGFONTW::default();
        if SystemParametersInfoForDpi(
            SPI_GETICONTITLELOGFONT,
            size_of::<LOGFONTW>() as u32,
            (&raw mut logical_font).cast(),
            0,
            dpi,
        ) == 0
        {
            return Err("无法读取桌面字体".into());
        }
        let font = CreateFontIndirectW(&raw const logical_font);
        if font.is_null() {
            return Err("无法创建重命名字体".into());
        }
        let name = Name::new(identity, label);
        let text = wide(&name.text);
        let edit = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            windows_sys::w!("EDIT"),
            text.as_ptr(),
            WS_POPUP
                | WS_BORDER
                | WS_TABSTOP
                | ES_CENTER as u32
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32,
            0,
            0,
            1,
            1,
            owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if edit.is_null() {
            DeleteObject(font);
            return Err("无法创建图标名称编辑框".into());
        }
        let editor = Box::new(Editor {
            owner,
            identity: identity.clone(),
            model: model.clone(),
            name,
            font,
            composing: false,
            finishing: false,
        });
        let selection = editor.name.selection;
        let pointer = Box::into_raw(editor);
        if SetWindowSubclass(edit, Some(edit_proc), SUBCLASS, pointer as usize) == 0 {
            drop(Box::from_raw(pointer));
            DestroyWindow(edit);
            DeleteObject(font);
            return Err("无法连接名称编辑框".into());
        }
        if SetPropW(owner, PROPERTY, edit) == 0
            || SetWindowSubclass(owner, Some(owner_proc), SUBCLASS, pointer as usize) == 0
        {
            DestroyWindow(edit);
            return Err("无法连接分组编辑状态".into());
        }
        model.borrow_mut().renaming = Some(identity.clone());
        SendMessageW(edit, WM_SETFONT, font as usize, 0);
        SendMessageW(edit, EM_LIMITTEXT, 255, 0);
        resize(edit, pointer);
        ShowWindow(edit, SW_SHOW);
        InvalidateRect(owner, std::ptr::null(), 0);
        SetForegroundWindow(edit);
        SetFocus(edit);
        SendMessageW(
            edit,
            EM_SETSEL,
            0,
            isize::try_from(selection).unwrap_or(isize::MAX),
        );
        Ok(())
    }
}
fn text(edit: HWND) -> String {
    let mut value = [0u16; 256];
    let len = unsafe { GetWindowTextW(edit, value.as_mut_ptr(), 256) };
    String::from_utf16_lossy(&value[..len.max(0) as usize])
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

unsafe fn resize(edit: HWND, pointer: *mut Editor) {
    let editor = unsafe { &*pointer };
    let Ok(model) = editor.model.try_borrow() else {
        return;
    };
    let Some(index) = model
        .items
        .iter()
        .position(|item| item.identity == editor.identity)
    else {
        unsafe {
            PostMessageW(edit, FINISH, 0, 0);
        }
        return;
    };
    let mut client = RECT::default();
    unsafe {
        GetClientRect(editor.owner, &raw mut client);
    }
    let scale = unsafe { GetDpiForWindow(editor.owner) }.max(96) as f32 / 96.0;
    let grid = model.grid(client.right as f32 / scale, client.bottom as f32 / scale);
    let (x, y) = model.cell(grid, index);
    let center = (x + grid.cell_width / 2.0) * scale;
    let top =
        ((y + grid.icon_size + if model.managed { 4.0 } else { 9.0 }) * scale).round() as i32 - 2;
    let maximum = (grid.cell_width * scale).round() as i32;
    drop(model);
    unsafe {
        let dc = GetDC(edit);
        let old = SelectObject(dc, editor.font);
        let mut metrics = TEXTMETRICW::default();
        GetTextMetricsW(dc, &raw mut metrics);
        let mut value = wide(&text(edit));
        let mut bounds = RECT {
            right: (maximum - 8).max(12),
            bottom: 4096,
            ..Default::default()
        };
        DrawTextW(
            dc,
            value.as_mut_ptr(),
            -1,
            &raw mut bounds,
            DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        ReleaseDC(edit, dc);
        let width = (bounds.right - bounds.left + 8).clamp(24, maximum.max(24));
        let height = (bounds.bottom - bounds.top)
            .max(metrics.tmHeight)
            .min(metrics.tmHeight.max(1) * 6)
            + 4;
        let left =
            ((center - width as f32 / 2.0).round() as i32).clamp(0, (client.right - width).max(0));
        let mut before = RECT::default();
        GetWindowRect(edit, &raw mut before);
        // A child EDIT shares the owner's missing GDI redirection bitmap and
        // disappears underneath DirectComposition. An owned popup has its own
        // surface; its position must therefore be expressed in screen pixels.
        let mut origin = POINT { x: left, y: top };
        ClientToScreen(editor.owner, &raw mut origin);
        if (
            before.left,
            before.top,
            before.right - before.left,
            before.bottom - before.top,
        ) != (origin.x, origin.y, width, height)
        {
            SetWindowPos(
                edit,
                HWND_TOP,
                origin.x,
                origin.y,
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }
}
unsafe fn finish(edit: HWND, pointer: *mut Editor, commit: bool) {
    if unsafe { (*pointer).finishing } {
        return;
    }
    unsafe {
        (*pointer).finishing = true;
    }
    let owner = unsafe { (*pointer).owner };
    if !commit {
        unsafe {
            DestroyWindow(edit);
        }
        return;
    }
    let value = text(edit);
    if value.trim().is_empty() {
        unsafe {
            (*pointer).finishing = false;
            SetFocus(edit);
        }
        return;
    }
    let (identity, name, unchanged) = unsafe {
        (
            (*pointer).identity.clone(),
            (*pointer).name.committed(&value),
            value == (*pointer).name.text,
        )
    };
    let result = if unchanged {
        Ok(true)
    } else {
        desktop_shell::rename_shell_identity(
            windows::Win32::Foundation::HWND(owner),
            &identity,
            &name,
        )
    };
    // Shell can pump messages, including closing the owning pane.
    if unsafe { GetPropW(owner, PROPERTY) } != edit {
        return;
    }
    match result {
        Ok(true) => unsafe {
            DestroyWindow(edit);
        },
        outcome => unsafe {
            if let Err(error) = outcome {
                MessageBoxW(
                    owner,
                    wide(&format!("重命名失败：{error}")).as_ptr(),
                    windows_sys::w!("LucidPane"),
                    MB_OK | MB_ICONERROR,
                );
            }
            if GetPropW(owner, PROPERTY) == edit {
                (*pointer).finishing = false;
                SetFocus(edit);
            }
        },
    }
}
unsafe extern "system" fn edit_proc(
    edit: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    data: usize,
) -> isize {
    let pointer = data as *mut Editor;
    match msg {
        WM_IME_STARTCOMPOSITION => unsafe {
            (*pointer).composing = true;
        },
        WM_IME_ENDCOMPOSITION => unsafe {
            (*pointer).composing = false;
        },
        WM_KEYDOWN if [usize::from(VK_RETURN), usize::from(VK_ESCAPE)].contains(&wp) => {
            if !unsafe { (*pointer).composing } {
                unsafe {
                    PostMessageW(edit, FINISH, usize::from(wp == usize::from(VK_RETURN)), 0);
                }
                return 0;
            }
        }
        WM_CHAR if wp == 13 || wp == 27 => return 0,
        WM_GETDLGCODE => return (DLGC_WANTALLKEYS | DLGC_WANTCHARS) as isize,
        WM_KILLFOCUS => {
            if !unsafe { (*pointer).finishing } {
                unsafe {
                    PostMessageW(edit, FINISH, 1, 0);
                }
            }
        }
        FINISH => {
            unsafe {
                finish(edit, pointer, wp != 0);
            }
            return 0;
        }
        WM_NCDESTROY => {
            let editor = unsafe { Box::from_raw(pointer) };
            unsafe {
                RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS);
                RemoveWindowSubclass(editor.owner, Some(owner_proc), SUBCLASS);
                RemovePropW(editor.owner, PROPERTY);
                if let Ok(mut model) = editor.model.try_borrow_mut() {
                    model.renaming = None;
                }
                let result = DefSubclassProc(edit, msg, wp, lp);
                DeleteObject(editor.font);
                InvalidateRect(editor.owner, std::ptr::null(), 0);
                return result;
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(edit, msg, wp, lp) }
}
unsafe extern "system" fn owner_proc(
    owner: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    data: usize,
) -> isize {
    let edit = unsafe { GetPropW(owner, PROPERTY) };
    let result = unsafe { DefSubclassProc(owner, msg, wp, lp) };
    if edit.is_null() || unsafe { GetPropW(owner, PROPERTY) } != edit {
        return result;
    }
    if [WM_PAINT, WM_SIZE, WM_MOVE, WM_DPICHANGED, WM_MOUSEWHEEL].contains(&msg)
        || (msg == WM_COMMAND && lp == edit as isize && (wp >> 16) == EN_CHANGE as usize)
    {
        unsafe {
            resize(edit, data as *mut Editor);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(name: &str) -> ShellIdentity {
        ShellIdentity::FileSystem {
            path: std::env::temp_dir().join(name),
            volume_id: None,
            file_id: None,
        }
    }
    #[test]
    fn hidden_extensions_are_preserved_and_visible_extensions_are_not_preselected() {
        let name = Name::new(&identity("网易云音乐.lnk"), "网易云音乐");
        assert_eq!(name.text, "网易云音乐");
        assert_eq!(name.selection, 5);
        assert_eq!(name.committed("音乐"), "音乐.lnk");
        let name = Name::new(&identity("报告.txt"), "报告.txt");
        assert_eq!(name.text, "报告.txt");
        assert_eq!(name.selection, 2);
        assert_eq!(name.committed("新报告.txt"), "新报告.txt");
    }
    #[test]
    fn inline_editor_tracks_label_and_escape_cleans_up_without_a_dialog() {
        let identity = identity("网易云音乐.lnk");
        let model = Rc::new(RefCell::new(GroupModel {
            theme: desktop_core::PanelTheme::System,
            dark: true,
            desktop: false,
            managed: true,
            spacing: (100.0, 110.0),
            hovered_item: None,
            focused: true,
            auto_hide: false,
            reveal: 1.0,
            hovered_button: None,
            backdrop: desktop_core::Backdrop::Acrylic,
            native_material: false,
            title: "测试".into(),
            items: vec![super::super::Item {
                identity: identity.clone(),
                label: "网易云音乐".into(),
                image: None,
                position: desktop_core::PointDip::default(),
            }],
            icon_size: 48.0,
            selected: Some(0),
            renaming: None,
            scroll: 0,
            collapsed: false,
            loading: false,
        }));
        unsafe {
            let owner = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP,
                windows_sys::w!("STATIC"),
                windows_sys::w!("Inline rename fixture"),
                WS_POPUP,
                0,
                0,
                400,
                300,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!owner.is_null());
            show(owner, &identity, "网易云音乐", model.clone()).unwrap();
            let edit = GetPropW(owner, PROPERTY);
            assert_eq!(GetParent(edit), owner);
            assert_eq!(GetWindowLongW(edit, GWL_STYLE) as u32 & WS_CHILD, 0);
            assert_ne!(GetWindowLongW(edit, GWL_STYLE) as u32 & WS_POPUP, 0);
            assert_eq!(
                GetWindowLongW(edit, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP,
                0
            );
            assert_eq!(text(edit), "网易云音乐");
            assert_eq!(model.borrow().renaming.as_ref(), Some(&identity));
            let mut bounds = RECT::default();
            GetWindowRect(edit, &raw mut bounds);
            MapWindowPoints(std::ptr::null_mut(), owner, (&raw mut bounds).cast(), 2);
            let dpi = GetDpiForWindow(owner).max(96) as f32 / 96.0;
            assert_eq!(
                bounds.top,
                ((super::super::layout::HEADER + super::super::layout::PADDING + 48.0 + 4.0) * dpi)
                    .round() as i32
                    - 2
            );
            let mut start = 99u32;
            let mut end = 99u32;
            SendMessageW(
                edit,
                EM_GETSEL,
                (&raw mut start) as usize,
                (&raw mut end) as isize,
            );
            assert_eq!((start, end), (0, 5));
            let mut original = RECT::default();
            GetWindowRect(edit, &raw mut original);
            SetWindowPos(
                owner,
                std::ptr::null_mut(),
                137,
                81,
                400,
                300,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut moved = RECT::default();
            GetWindowRect(edit, &raw mut moved);
            assert_eq!(
                (moved.left - original.left, moved.top - original.top),
                (137, 81)
            );
            SetWindowTextW(edit, windows_sys::w!("取消后不能提交"));
            SendMessageW(edit, WM_KEYDOWN, VK_ESCAPE as usize, 0);
            let mut message = MSG::default();
            while PeekMessageW(&raw mut message, edit, FINISH, FINISH, PM_REMOVE) != 0 {
                DispatchMessageW(&raw const message);
            }
            assert!(!active(owner));
            assert!(model.borrow().renaming.is_none());
            assert_eq!(IsWindow(edit), 0);
            show(owner, &identity, "网易云音乐", model.clone()).unwrap();
            let edit = GetPropW(owner, PROPERTY);
            SendMessageW(edit, WM_IME_STARTCOMPOSITION, 0, 0);
            SendMessageW(edit, WM_KEYDOWN, VK_RETURN as usize, 0);
            assert_eq!(
                PeekMessageW(&raw mut message, edit, FINISH, FINISH, PM_REMOVE),
                0,
                "IME confirmation must not finish rename"
            );
            SendMessageW(edit, WM_IME_ENDCOMPOSITION, 0, 0);
            SendMessageW(edit, WM_KEYDOWN, VK_RETURN as usize, 0);
            while PeekMessageW(&raw mut message, edit, FINISH, FINISH, PM_REMOVE) != 0 {
                DispatchMessageW(&raw const message);
            }
            assert!(
                !active(owner),
                "Enter must finish unchanged names without a Shell operation"
            );
            show(owner, &identity, "网易云音乐", model.clone()).unwrap();
            DestroyWindow(owner);
            assert!(
                model.borrow().renaming.is_none(),
                "Destroying the pane must release the editor"
            );
        }
    }
}
