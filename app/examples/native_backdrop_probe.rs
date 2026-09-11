//! Own-window experiment only: native ListView foreground over the system's Mica/Acrylic.
use desktop_hook::{
    geometry::GeometrySession,
    protocol::{Area, PaneAppearance},
};
use std::ptr::{null, null_mut};
use windows::Win32::Graphics::Dwm::*;
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};
#[path = "../src/pane/acrylic.rs"]
mod acrylic;
// This standalone native comparison retains its original DWM entry points.
mod native_graphics {
    pub use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
}

// Isolated comparison of the private Accent API with the public DWM backdrop API.
// Do not use this experiment to change Explorer's composition attributes.
unsafe fn private_attribute<T>(
    hwnd: HWND,
    kind: u32,
    value: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    #[repr(C)]
    struct Attribute {
        kind: u32,
        data: *const std::ffi::c_void,
        size: usize,
    }
    type SetAttribute = unsafe extern "system" fn(HWND, *const Attribute) -> i32;
    let module = unsafe { GetModuleHandleW(windows_sys::w!("user32.dll")) };
    if module.is_null() {
        return Err("user32.dll is unavailable".into());
    }
    let proc = unsafe { GetProcAddress(module, windows_sys::s!("SetWindowCompositionAttribute")) }
        .ok_or("SetWindowCompositionAttribute is unavailable")?;
    let set: SetAttribute = unsafe { std::mem::transmute(proc) };
    let attribute = Attribute {
        kind,
        data: std::ptr::from_ref(value).cast(),
        size: size_of::<T>(),
    };
    if unsafe { set(hwnd, &raw const attribute) } == 0 {
        return Err(format!("Private composition attribute {kind} request failed").into());
    }
    Ok(())
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_ACTIVATE {
        println!(
            "ACTIVATION {}",
            if wp & 0xffff == WA_INACTIVE as usize {
                "inactive"
            } else {
                "active"
            }
        );
    }
    if msg == WM_NOTIFY && lp != 0 {
        let hdr = unsafe { &*(lp as *const NMHDR) };
        if hdr.code == LVN_GETDISPINFOW {
            let item = unsafe { &mut *(lp as *mut NMLVDISPINFOW) };
            item.item.pszText = windows_sys::w!("Native icon").cast_mut();
            item.item.iImage = 0;
            return 0;
        }
    }
    if msg == WM_TIMER || msg == WM_CLOSE {
        unsafe {
            DestroyWindow(hwnd);
        }
        return 0;
    }
    if msg == WM_DESTROY {
        unsafe {
            PostQuitMessage(0);
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
unsafe extern "system" fn focus_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_ACTIVATE {
        println!(
            "FOCUS CONTROL {}",
            if wp & 0xffff == WA_INACTIVE as usize {
                "inactive"
            } else {
                "active"
            }
        );
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _apartment = desktop_shell::ShellApartment::initialize_sta()?;
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        });
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            lpszClassName: windows_sys::w!("LucidPaneNativeBackdropProbe"),
            ..Default::default()
        };
        RegisterClassW(&raw const class);
        let clip_children = std::env::args().any(|a| a == "--clip-children");
        let parent = CreateWindowExW(
            0,
            class.lpszClassName,
            windows_sys::w!("LucidPane Native Backdrop Probe"),
            WS_OVERLAPPEDWINDOW | if clip_children { WS_CLIPCHILDREN } else { 0 },
            150,
            150,
            950,
            640,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        let view = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD | WS_VISIBLE | LVS_ICON | LVS_OWNERDATA | LVS_AUTOARRANGE,
            0,
            0,
            930,
            600,
            parent,
            null_mut(),
            null_mut(),
            null(),
        );
        let images = ImageList_Create(48, 48, ILC_COLOR32 | ILC_MASK, 1, 1);
        ImageList_ReplaceIcon(images, -1, LoadIconW(null_mut(), IDI_APPLICATION));
        SendMessageW(
            view,
            LVM_SETIMAGELIST,
            LVSIL_NORMAL as usize,
            images as isize,
        );
        SendMessageW(view, LVM_SETTEXTBKCOLOR, 0, CLR_NONE as isize);
        SendMessageW(view, LVM_SETTEXTCOLOR, 0, 0x00ff_ffff);
        SendMessageW(view, LVM_SETITEMCOUNT, 6, 0);
        let hwnd = windows::Win32::Foundation::HWND(parent);
        let host = std::env::args().any(|a| a == "--host-acrylic");
        let accent = std::env::args().any(|a| a == "--accent-acrylic");
        let force_active = std::env::args().any(|a| a == "--force-active");
        let kind = if host || accent {
            DWMSBT_NONE
        } else if std::env::args().any(|a| a == "--acrylic") {
            DWMSBT_TRANSIENTWINDOW
        } else {
            DWMSBT_MAINWINDOW
        };
        let label = if host {
            "HostBackdrop composition (layering comparison)"
        } else if accent {
            "Accent Acrylic (private API experiment)"
        } else if kind == DWMSBT_TRANSIENTWINDOW {
            "System Acrylic"
        } else {
            "System Mica"
        };
        let title: Vec<u16> = format!(
            "LucidPane Native Backdrop Probe - {label}{}\0",
            if force_active {
                " (persistent appearance experiment)"
            } else {
                ""
            }
        )
        .encode_utf16()
        .collect();
        SetWindowTextW(parent, title.as_ptr());
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            std::ptr::from_ref(&kind).cast(),
            4,
        )?;
        let dark = 1i32;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            4,
        )?;
        let margins = windows::Win32::UI::Controls::MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        DwmExtendFrameIntoClientArea(hwnd, &raw const margins)?;
        let _acrylic = if host {
            Some(acrylic::Acrylic::new(hwnd)?)
        } else {
            None
        };
        if accent {
            private_attribute(parent, 19, &[4u32, 2, 0x9920_1b18, 0])?;
        }
        if force_active {
            private_attribute(parent, 15, &1i32)?;
        }
        let geometry = GeometrySession::attach(view as isize)?;
        geometry.begin_positions();
        geometry.set_position(2, POINT { x: 410, y: 220 })?;
        geometry.commit_scene(
            &[PaneAppearance {
                bounds: Area {
                    left: 160,
                    top: 100,
                    right: 780,
                    bottom: 510,
                },
                material: 2,
                ..Default::default()
            }],
            [(2, 0)].into_iter().collect(),
        )?;
        ShowWindow(parent, SW_SHOWNOACTIVATE);
        UpdateWindow(parent);
        SetTimer(parent, 1, 120_000, None);
        let focus_control = if std::env::args().any(|a| a == "--focus-check") {
            let focus_class = WNDCLASSW {
                lpfnWndProc: Some(focus_proc),
                lpszClassName: windows_sys::w!("LucidPaneBackdropFocusControl"),
                hbrBackground: GetStockObject(WHITE_BRUSH),
                ..Default::default()
            };
            RegisterClassW(&raw const focus_class);
            let control = CreateWindowExW(
                0,
                focus_class.lpszClassName,
                windows_sys::w!("LucidPane Focus Control"),
                WS_OVERLAPPEDWINDOW,
                1120,
                150,
                400,
                200,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            );
            ShowWindow(control, SW_SHOWNOACTIVATE);
            control
        } else {
            null_mut()
        };
        println!(
            "READY own-window native backdrop: {label}; force_active_appearance={force_active}"
        );
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
        drop(geometry);
        if !focus_control.is_null() {
            DestroyWindow(focus_control);
        }
        ImageList_Destroy(images);
    }
    Ok(())
}
