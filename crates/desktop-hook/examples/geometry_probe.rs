//! Tests actual native virtual-list geometry. Never targets Explorer.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
use desktop_hook::geometry::GeometrySession;
use std::sync::atomic::{AtomicUsize, Ordering};
static NAME_MODE: AtomicUsize = AtomicUsize::new(0);
#[path = "support/drop_capture.rs"]
mod drop_capture;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, CreateRectRgn,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetUpdateRgn, PatBlt, PtInRegion,
    SelectClipRgn, SelectObject, ValidateRect, WHITENESS,
};
use windows_sys::Win32::UI::Controls::{
    ICC_LISTVIEW_CLASSES, ILC_COLOR32, ILC_MASK, INITCOMMONCONTROLSEX, ImageList_Create,
    ImageList_Destroy, ImageList_ReplaceIcon, InitCommonControlsEx, LVHITTESTINFO, LVIR_BOUNDS,
    LVIR_ICON, LVIR_LABEL, LVIS_SELECTED, LVITEMW, LVM_ARRANGE, LVM_GETITEMPOSITION,
    LVM_GETITEMRECT, LVM_GETITEMSTATE, LVM_GETSELECTEDCOUNT, LVM_HITTEST, LVM_SETIMAGELIST,
    LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVN_GETDISPINFOW, LVS_AUTOARRANGE, LVS_ICON, LVS_OWNERDATA,
    LVSIL_NORMAL, NMHDR, NMLVDISPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, GetWindowLongW, IDI_APPLICATION,
    LoadIconW, PRF_CLIENT, PRF_ERASEBKGND, RegisterClassW, SendMessageW, WM_NOTIFY, WM_PRINTCLIENT,
    WNDCLASSW, WS_CHILD,
};

unsafe extern "system" fn host(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NOTIFY && lp != 0 {
        let header = unsafe { &*(lp as *const NMHDR) };
        if header.code == LVN_GETDISPINFOW {
            let info = unsafe { &mut *(lp as *mut NMLVDISPINFOW) };
            info.item.pszText = windows_sys::w!("Native icon").cast_mut();
            if NAME_MODE.load(Ordering::Relaxed) != 0 {
                let mut index = info.item.iItem;
                if NAME_MODE.load(Ordering::Relaxed) == 2 {
                    index = match index {
                        2 => 3,
                        3 => 2,
                        i => i,
                    };
                }
                let names = [
                    windows_sys::w!("Identity 0"),
                    windows_sys::w!("Identity 1"),
                    windows_sys::w!("Identity 2"),
                    windows_sys::w!("Identity 3"),
                    windows_sys::w!("Identity 4"),
                ];
                info.item.pszText = names[usize::try_from(index).unwrap()].cast_mut();
            }
            info.item.iImage = 0;
            return 0;
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn rect(view: HWND, item: usize, kind: u32) -> RECT {
    let mut rect = RECT {
        left: kind as i32,
        ..RECT::default()
    };
    assert_ne!(
        unsafe { SendMessageW(view, LVM_GETITEMRECT, item, (&raw mut rect) as isize) },
        0
    );
    rect
}

fn point(view: HWND, item: usize) -> POINT {
    let mut point = POINT::default();
    assert_ne!(
        unsafe { SendMessageW(view, LVM_GETITEMPOSITION, item, (&raw mut point) as isize) },
        0
    );
    point
}

fn hit(view: HWND, point: POINT) -> (isize, u32) {
    let mut info = LVHITTESTINFO {
        pt: point,
        ..LVHITTESTINFO::default()
    };
    let index = unsafe { SendMessageW(view, LVM_HITTEST, 0, (&raw mut info) as isize) };
    (index, info.flags)
}

fn coordinates(rect: RECT) -> [i32; 4] {
    [rect.left, rect.top, rect.right, rect.bottom]
}

fn render(view: HWND, name: &str) -> Vec<u8> {
    render_region(view, name, None)
}

fn render_region(view: HWND, name: &str, previous: Option<&[u8]>) -> Vec<u8> {
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 900,
                biHeight: -600,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut bits,
            null_mut(),
            0,
        );
        assert!(!bitmap.is_null() && !bits.is_null());
        let previous_bitmap = SelectObject(dc, bitmap);
        if let Some(previous) = previous {
            std::ptr::copy_nonoverlapping(previous.as_ptr(), bits.cast::<u8>(), previous.len());
            let damage = CreateRectRgn(0, 0, 0, 0);
            assert!(GetUpdateRgn(view, damage, 0) > 1);
            assert_eq!(
                PtInRegion(damage, 800, 500),
                0,
                "Unrelated desktop pixels were invalidated"
            );
            SelectClipRgn(dc, damage);
            DeleteObject(damage);
        }
        PatBlt(dc, 0, 0, 900, 600, WHITENESS);
        SendMessageW(
            view,
            WM_PRINTCLIENT,
            dc as usize,
            (PRF_CLIENT | PRF_ERASEBKGND) as isize,
        );
        GdiFlush();
        let bytes = std::slice::from_raw_parts(bits as *const u8, 900 * 600 * 4).to_vec();
        let mut file = Vec::new();
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(54_u32 + bytes.len() as u32).to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&54_u32.to_le_bytes());
        file.extend_from_slice(std::slice::from_raw_parts(
            (&raw const info.bmiHeader).cast::<u8>(),
            40,
        ));
        file.extend_from_slice(&bytes);
        std::fs::write(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join(name),
            file,
        )
        .unwrap();
        SelectObject(dc, previous_bitmap);
        DeleteObject(bitmap);
        DeleteDC(dc);
        bytes
    }
}

fn crop(bytes: &[u8], bounds: RECT) -> Vec<u8> {
    let mut output = Vec::new();
    for y in bounds.top.max(0)..bounds.bottom.min(600) {
        for x in bounds.left.max(0)..bounds.right.min(900) {
            let offset = usize::try_from((y * 900 + x) * 4).unwrap();
            output.extend_from_slice(&bytes[offset..offset + 3]);
        }
    }
    output
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        });
        let class = WNDCLASSW {
            lpfnWndProc: Some(host),
            lpszClassName: windows_sys::w!("LucidPaneGeometryFixture"),
            ..WNDCLASSW::default()
        };
        RegisterClassW(&raw const class);
        let parent = CreateWindowExW(
            windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE
                | windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW,
            class.lpszClassName,
            null(),
            windows_sys::Win32::UI::WindowsAndMessaging::WS_POPUP,
            0,
            0,
            1100,
            700,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        let view = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD
                | windows_sys::Win32::UI::WindowsAndMessaging::WS_VISIBLE
                | LVS_ICON
                | LVS_OWNERDATA
                | LVS_AUTOARRANGE
                | windows_sys::Win32::UI::Controls::LVS_SHOWSELALWAYS,
            0,
            0,
            900,
            600,
            parent,
            null_mut(),
            null_mut(),
            null(),
        );
        let other = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD | LVS_ICON | LVS_OWNERDATA | LVS_AUTOARRANGE,
            0,
            0,
            900,
            600,
            parent,
            null_mut(),
            null_mut(),
            null(),
        );
        let images = ImageList_Create(32, 32, ILC_COLOR32 | ILC_MASK, 1, 1);
        let icon = LoadIconW(null_mut(), IDI_APPLICATION);
        ImageList_ReplaceIcon(images, -1, icon);
        SendMessageW(
            view,
            LVM_SETIMAGELIST,
            LVSIL_NORMAL as usize,
            images as isize,
        );
        SendMessageW(
            other,
            LVM_SETIMAGELIST,
            LVSIL_NORMAL as usize,
            images as isize,
        );
        SendMessageW(view, LVM_SETITEMCOUNT, 5, 0);
        SendMessageW(other, LVM_SETITEMCOUNT, 5, 0);
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            parent,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
        );
        windows_sys::Win32::Graphics::Gdi::UpdateWindow(view);
        let selected = LVITEMW {
            stateMask: LVIS_SELECTED,
            state: LVIS_SELECTED,
            ..LVITEMW::default()
        };
        SendMessageW(view, LVM_SETITEMSTATE, 2, (&raw const selected) as isize);
        let baseline = rect(view, 2, LVIR_ICON);
        let baseline_bounds = rect(view, 2, LVIR_BOUNDS);
        let label = rect(view, 2, LVIR_LABEL);
        let original = point(view, 2);
        let untouched = rect(other, 2, LVIR_ICON);
        println!(
            "Baseline icon={:?} label={:?} point={},{}",
            coordinates(baseline),
            coordinates(label),
            original.x,
            original.y
        );
        let baseline_image = render(view, "geometry-baseline.bmp");
        let result = (|| {
            let session = GeometrySession::attach(view as isize)?;
            let destination = POINT { x: 450, y: 180 };
            session.set_positions(&[(2, destination)])?;
            let moved = rect(view, 2, LVIR_ICON);
            let moved_label = rect(view, 2, LVIR_LABEL);
            let current = point(view, 2);
            println!(
                "Mapped icon={:?} label={:?} point={},{}",
                coordinates(moved),
                coordinates(moved_label),
                current.x,
                current.y
            );
            assert_eq!((current.x, current.y), (destination.x, destination.y));
            assert_eq!(moved.left - baseline.left, destination.x - original.x);
            assert_eq!(moved.top - baseline.top, destination.y - original.y);
            assert_eq!(moved_label.left - label.left, destination.x - original.x);
            let mapped_image = render(view, "geometry-mapped.bmp");
            let native_pixels = crop(&baseline_image, baseline_bounds);
            assert!(
                native_pixels.iter().any(|&b| b != 255),
                "Fixture produced no visible native content"
            );
            assert!(
                native_pixels == crop(&mapped_image, rect(view, 2, LVIR_BOUNDS)),
                "Native icon/label/selection pixels changed during translation"
            );
            let test = POINT {
                x: i32::midpoint(moved.left, moved.right),
                y: i32::midpoint(moved.top, moved.bottom),
            };
            let selected = hit(view, test);
            println!("Hit moved icon={selected:?}");
            assert_eq!(selected.0, 2);
            assert_ne!(
                hit(
                    view,
                    POINT {
                        x: i32::midpoint(baseline.left, baseline.right),
                        y: i32::midpoint(baseline.top, baseline.bottom)
                    }
                )
                .0,
                2
            );
            assert_eq!(
                coordinates(rect(other, 2, LVIR_ICON)),
                coordinates(untouched)
            );
            SendMessageW(view, LVM_ARRANGE, 0, 0);
            assert_eq!(coordinates(rect(view, 2, LVIR_ICON)), coordinates(moved));
            assert_ne!(GetWindowLongW(view, GWL_STYLE) & LVS_AUTOARRANGE as i32, 0);
            ValidateRect(view, null());
            session.begin_positions();
            session.set_position(2, POINT { x: 600, y: 180 })?;
            assert_eq!(
                coordinates(rect(view, 2, LVIR_ICON)),
                coordinates(moved),
                "Staged layout leaked before commit"
            );
            session.commit_positions()?;
            assert_eq!(point(view, 2).x, 600);
            let incremental = render_region(view, "geometry-incremental.bmp", Some(&mapped_image));
            let reference = render(view, "geometry-reference.bmp");
            assert!(
                incremental
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(reference.as_chunks::<4>().0.iter())
                    .all(|(a, b)| a[..3] == b[..3]),
                "Partial native repaint left stale pixels or lost icon/label content"
            );
            ValidateRect(view, null());
            session.begin_positions();
            session.set_position(2, POINT { x: 600, y: 180 })?;
            session.commit_positions()?;
            let damage = CreateRectRgn(0, 0, 0, 0);
            assert_eq!(
                GetUpdateRgn(view, damage, 0),
                1,
                "An unchanged layout requested a repaint"
            );
            DeleteObject(damage);
            println!(
                "PASS: partial native repaint matches full reference; unchanged frame has no damage"
            );
            assert_eq!(session.original_position(2)?.x, original.x);
            session.set_position(2, destination)?;
            println!(
                "Native translated rectangle calls={}",
                session.translated_rectangles()
            );
            let free = point(view, 4);
            let mut pane = desktop_hook::protocol::PaneAppearance {
                bounds: desktop_hook::protocol::Area {
                    left: 100,
                    top: 0,
                    right: 780,
                    bottom: 400,
                },
                ..Default::default()
            };
            for (to, from) in pane.title.iter_mut().zip("Native group".encode_utf16()) {
                *to = from;
            }
            session.set_texture(
                desktop_hook::protocol::TextureHeader {
                    version: desktop_hook::protocol::VERSION,
                    width: 8,
                    height: 8,
                    bounds: desktop_hook::protocol::Area {
                        left: 0,
                        top: 0,
                        right: 900,
                        bottom: 600,
                    },
                },
                &[190, 135, 75, 255].repeat(64),
            )?;
            session.begin_positions();
            session.set_position(2, destination)?;
            session.commit_scene(&[pane], [(2, 0)].into_iter().collect())?;
            assert_eq!(
                (point(view, 4).x, point(view, 4).y),
                (free.x, free.y),
                "Covered desktop item was displaced"
            );
            assert_eq!(
                hit(view, POINT { x: 337, y: 18 }).0,
                -1,
                "Clicked through a pane into a covered icon"
            );
            assert_eq!(
                hit(view, test).0,
                2,
                "Native grouped item stopped receiving input"
            );
            let pane_image = render(view, "geometry-pane.bmp");
            let pixel = |x: usize, y: usize| &pane_image[(y * 900 + x) * 4..(y * 900 + x) * 4 + 3];
            assert_eq!(
                pixel(337, 18),
                pixel(410, 18),
                "Covered native icon was still drawn over the pane"
            );
            assert_ne!(
                pixel(410, 250),
                &[255, 255, 255],
                "Pane background did not survive native painting"
            );
            session.begin_positions();
            session.set_position(2, destination)?;
            session.commit_scene(&[], std::collections::BTreeMap::new())?;
            assert_eq!(
                hit(view, POINT { x: 337, y: 18 }).0,
                4,
                "Moving the pane away did not restore the icon hit target"
            );
            let restored_scene = render(view, "geometry-pane-removed.bmp");
            assert!(
                restored_scene
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(mapped_image.as_chunks::<4>().0.iter())
                    .all(|(a, b)| a[..3] == b[..3]),
                "Removing pane decoration did not restore the original native desktop pixels"
            );
            println!(
                "PASS: pane background, covered-item drawing/input clipping, unchanged desktop positions and scene removal"
            );
            let slots: Vec<_> = (0..5)
                .map(|i| session.original_position(i).unwrap())
                .collect();
            session.begin_positions();
            for i in 0..5 {
                session.set_position(
                    i,
                    slots[usize::try_from(if i > 2 { i - 1 } else { i }).unwrap()],
                )?;
            }
            session.commit_scene(
                &[],
                [(2, desktop_hook::protocol::HIDDEN_ITEM as usize - 1)]
                    .into_iter()
                    .collect(),
            )?;
            let moved = rect(view, 3, LVIR_ICON);
            assert_eq!(
                hit(
                    view,
                    POINT {
                        x: i32::midpoint(moved.left, moved.right),
                        y: i32::midpoint(moved.top, moved.bottom)
                    }
                )
                .0,
                3,
                "Compacted icon did not own its new hit target"
            );
            let select = LVITEMW {
                stateMask: LVIS_SELECTED,
                state: LVIS_SELECTED,
                ..Default::default()
            };
            SendMessageW(
                view,
                LVM_SETITEMSTATE,
                usize::MAX,
                (&raw const select) as isize,
            );
            assert_eq!(
                SendMessageW(view, LVM_GETITEMSTATE, 2, LVIS_SELECTED as isize),
                0,
                "Hidden item included in select-all"
            );
            assert_eq!(SendMessageW(view, LVM_GETSELECTEDCOUNT, 0, 0), 4);
            SendMessageW(view, LVM_SETITEMSTATE, 2, (&raw const select) as isize);
            assert_eq!(
                SendMessageW(view, LVM_GETITEMSTATE, 2, LVIS_SELECTED as isize),
                0,
                "Hidden item accepted direct selection"
            );
            session.menu_selection(true);
            SendMessageW(view, LVM_SETITEMSTATE, 2, (&raw const select) as isize);
            assert_ne!(
                SendMessageW(view, LVM_GETITEMSTATE, 2, LVIS_SELECTED as isize),
                0,
                "Shell menu could not select its hidden target"
            );
            session.menu_selection(false);
            assert_eq!(
                SendMessageW(view, LVM_GETITEMSTATE, 2, LVIS_SELECTED as isize),
                0,
                "Menu selection leaked back to the desktop"
            );
            let hidden_image = render(view, "geometry-hybrid.bmp");
            let last = slots[4];
            assert_eq!(
                &hidden_image[(usize::try_from(last.y + 18).unwrap() * 900
                    + usize::try_from(last.x + 32).unwrap())
                    * 4..][..3],
                &[255, 255, 255],
                "Trailing slot was not cleared after compaction"
            );
            println!(
                "PASS: hybrid compact hit targets, hidden selection isolation, menu exception and cleared trailing slot"
            );
            {
                use windows_sys::Win32::UI::Controls::{LVINSERTMARK, LVIM_AFTER, LVM_SETINSERTMARK, LVM_GETINSERTMARK, LVM_GETINSERTMARKRECT};
                let mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, dwFlags: LVIM_AFTER, iItem: 4, ..Default::default() };
                assert_ne!(SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const mark) as isize), 0);
                let mut cached = LVINSERTMARK { cbSize: mark.cbSize, ..Default::default() };
                SendMessageW(view, LVM_GETINSERTMARK, 0, (&raw mut cached) as isize);
                assert_eq!((cached.iItem, cached.dwFlags & LVIM_AFTER), (4, LVIM_AFTER));
                let mut marker_rect = RECT::default();
                SendMessageW(view, LVM_GETINSERTMARKRECT, 0, (&raw mut marker_rect) as isize);
                let bounds = rect(view, 4, LVIR_BOUNDS);
                println!("Native insert mark {:?}, compact last bounds {:?}", coordinates(marker_rect), coordinates(bounds));
                assert!(marker_rect.left >= bounds.left - 8 && marker_rect.right <= bounds.right + 8, "Insertion marker did not follow compact native geometry");
                let clear = LVINSERTMARK { iItem: -1, ..mark };
                SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const clear) as isize);
            }
            NAME_MODE.store(1, Ordering::Relaxed);
            let names: Vec<_> = (0..5)
                .map(|item| desktop_hook::protocol::ItemPosition {
                    item,
                    name_hash: desktop_hook::protocol::name_hash(
                        format!("Identity {item}").encode_utf16(),
                    ),
                    reserved: if item == 2 {
                        desktop_hook::protocol::HIDDEN_ITEM
                    } else {
                        0
                    },
                    ..Default::default()
                })
                .collect();
            session.identities(&names, 0);
            NAME_MODE.store(2, Ordering::Relaxed);
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(view, null(), 0);
            windows_sys::Win32::Graphics::Gdi::UpdateWindow(view);
            let newly_hidden = rect(view, 3, LVIR_ICON);
            assert_eq!(
                hit(
                    view,
                    POINT {
                        x: i32::midpoint(newly_hidden.left, newly_hidden.right),
                        y: i32::midpoint(newly_hidden.top, newly_hidden.bottom)
                    }
                )
                .0,
                4,
                "Same-count reorder did not fill the hidden slot before controller polling"
            );
            assert_eq!(
                (point(view, 4).x, point(view, 4).y),
                (slots[3].x, slots[3].y),
                "Reorder left a hole in the native grid"
            );
            SendMessageW(
                view,
                LVM_SETITEMSTATE,
                usize::MAX,
                (&raw const select) as isize,
            );
            assert_eq!(
                SendMessageW(view, LVM_GETITEMSTATE, 3, LVIS_SELECTED as isize),
                0
            );
            assert_ne!(
                SendMessageW(view, LVM_GETITEMSTATE, 2, LVIS_SELECTED as isize),
                0,
                "Old hidden index remained blocked after a reorder"
            );
            println!(
                "PASS: native paint identifies a same-count reorder immediately, before any controller polling"
            );
            NAME_MODE.store(0, Ordering::Relaxed);
            drop(session);
            assert_eq!(coordinates(rect(view, 2, LVIR_ICON)), coordinates(baseline));
            assert_eq!(
                hit(
                    view,
                    POINT {
                        x: i32::midpoint(baseline.left, baseline.right),
                        y: i32::midpoint(baseline.top, baseline.bottom)
                    }
                )
                .0,
                2
            );
            // Reattach validates disable/re-enable lifetime without overwriting original trampolines.
            drop(GeometrySession::attach(view as isize)?);
            println!(
                "PASS: native virtual rectangles, label, hit-test, auto-arrange, view isolation, detach and reattach"
            );
            {
                use windows_sys::Win32::UI::{Controls::{LVS_ALIGNLEFT, LVM_SETICONSPACING, LVINSERTMARK, LVIM_AFTER, LVM_SETINSERTMARK, LVM_INSERTMARKHITTEST, LVM_GETINSERTMARKRECT}, WindowsAndMessaging::{SetWindowLongW, SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE}};
                SetWindowLongW(view, GWL_STYLE, (GetWindowLongW(view, GWL_STYLE).cast_unsigned() | LVS_ALIGNLEFT).cast_signed());
                SetWindowPos(view, null_mut(), 0, 0, 900, 2058, SWP_NOZORDER | SWP_NOACTIVATE);
                SendMessageW(view, LVM_SETICONSPACING, 0, (0x0070 | (0x0093 << 16)) as isize);
                SendMessageW(view, LVM_SETITEMCOUNT, 81, 0);
                SendMessageW(view, LVM_ARRANGE, 0, 0);
                let native: Vec<_> = (0..81).map(|i| point(view, i)).collect();
                let initial_mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: 80, dwFlags: LVIM_AFTER, ..Default::default() };
                SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const initial_mark) as isize);
                let session = GeometrySession::attach(view as isize)?;
                session.begin_positions();
                let mut rank = 0;
                for i in 0..81 {
                    let hidden = i == 59 || i == 60;
                    session.set_position(i, native[if hidden { usize::try_from(i).unwrap() } else { let r = rank; rank += 1; r }])?;
                }
                session.commit_scene(&[], [59,60].into_iter().map(|i| (i, desktop_hook::protocol::HIDDEN_ITEM as usize - 1)).collect())?;
                let last = point(view, 80);
                let bounds = rect(view, 80, LVIR_BOUNDS);
                let hit_start = std::time::Instant::now();
                for item in (0..81).filter(|i| *i != 59 && *i != 60) {
                    for kind in [LVIR_ICON, LVIR_LABEL] {
                        let r = rect(view, item, kind);
                        let center = POINT { x: i32::midpoint(r.left, r.right), y: i32::midpoint(r.top, r.bottom) };
                        assert_eq!(hit(view, center).0, item as isize, "Visible item {item} rejected in native region {kind}");
                    }
                }
                println!("PASS: all 79 visible icons and labels hit correctly, including collected-item old slots; {:?} total", hit_start.elapsed());
                println!("81 items: native last={},{} visible last={},{} bounds={:?}", native[80].x,native[80].y,last.x,last.y,coordinates(bounds));
                for rank in [58usize, 62, 75] {
                    let cursor = POINT { x: native[rank].x + 60, y: native[rank].y + 20 };
                    let mut mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: -1, ..Default::default() };
                    let ok = SendMessageW(view, LVM_INSERTMARKHITTEST, (&raw const cursor) as usize, (&raw mut mark) as isize);
                    println!("Interior slot={rank} hit={}:{} ok={ok}", mark.iItem, mark.dwFlags);
                    println!("Internal Shell hit {:?}", session.insertion_target(cursor));
                }
                for dy in [108, 220, 367] {
                    let cursor = POINT { x: last.x + 60, y: last.y + dy };
                    let mut mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: -1, ..Default::default() };
                    SendMessageW(view, LVM_INSERTMARKHITTEST, (&raw const cursor) as usize, (&raw mut mark) as isize);
                    println!("cursor {},{} native hit={}:{}",cursor.x,cursor.y,mark.iItem,mark.dwFlags);
                    mark.iItem = 80;
                    mark.dwFlags = LVIM_AFTER;
                    SendMessageW(view,LVM_SETINSERTMARK,0,(&raw const mark) as isize);
                    let mut marker = RECT::default();
                    SendMessageW(view,LVM_GETINSERTMARKRECT,0,(&raw mut marker) as isize);
                    println!("81 item marker {:?}",coordinates(marker));
                    assert!(marker.top >= bounds.bottom - 8 && marker.bottom <= bounds.bottom + 8, "Column insertion mark retained hidden gaps");
                }
                drop(session);
                // Reproduce Shell's canonical "before hidden 80" for the gap
                // after visible 79. The native marker/index must stay intact;
                // only its rendered position follows the compact boundary.
                for hidden in [vec![57, 58, 80], vec![13, 14, 80], vec![0, 79, 80]] {
                    let capture = drop_capture::Capture::attach(view);
                    let mut expected = Vec::new();
                    for &item in &hidden {
                        let rank = (0..item).filter(|i| !hidden.contains(i)).count();
                        let mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: rank as i32, dwFlags: 0, ..Default::default() };
                        SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const mark) as isize);
                        let mut r = RECT::default();
                        SendMessageW(view, LVM_GETINSERTMARKRECT, 0, (&raw mut r) as isize);
                        let next = (item..81).find(|i| !hidden.contains(i));
                        let canonical = next.map_or((80, true), |i| (i, false));
                        let compact_mark = LVINSERTMARK {
                            iItem: if next.is_some() { rank as i32 } else { (81 - hidden.len() - 1) as i32 },
                            dwFlags: if canonical.1 { LVIM_AFTER } else { 0 }, ..mark
                        };
                        SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const compact_mark) as isize);
                        let mut committed_rect = RECT::default();
                        SendMessageW(view, LVM_GETINSERTMARKRECT, 0, (&raw mut committed_rect) as isize);
                        expected.push((item, coordinates(r), canonical, coordinates(committed_rect)));
                    }
                    let session = GeometrySession::attach(view as isize)?;
                    session.begin_positions();
                    let mut rank = 0;
                    for item in 0..81 {
                        let slot = if hidden.contains(&item) { usize::try_from(item).unwrap() } else { let slot = rank; rank += 1; slot };
                        session.set_position(item, native[slot])?;
                    }
                    session.commit_scene(&[], hidden.iter().map(|i| (*i, desktop_hook::protocol::HIDDEN_ITEM as usize - 1)).collect())?;
                    let identities: Vec<_> = (0..81).map(|item| desktop_hook::protocol::ItemPosition {
                        item, name_hash: desktop_hook::protocol::name_hash("Native icon".encode_utf16()),
                        reserved: if hidden.contains(&item) { desktop_hook::protocol::HIDDEN_ITEM } else { 0 },
                        ..Default::default()
                    }).collect();
                    session.identities(&identities, 0);
                    for (item, expected, canonical, committed_rect) in expected {
                        let mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: item, dwFlags: 0, ..Default::default() };
                        SendMessageW(view, LVM_SETINSERTMARK, 0, (&raw const mark) as isize);
                        let mut actual = RECT::default();
                        SendMessageW(view, LVM_GETINSERTMARKRECT, 0, (&raw mut actual) as isize);
                        assert_eq!(coordinates(actual), expected, "Hidden marker anchor {item} retained a native gap with hidden={hidden:?}");
                        let mut retained = mark;
                        SendMessageW(view, windows_sys::Win32::UI::Controls::LVM_GETINSERTMARK, 0, (&raw mut retained) as isize);
                        assert_eq!((retained.iItem, retained.dwFlags & LVIM_AFTER), (item, 0), "Marker rendering changed Shell's drop target");
                        let result = capture.drop_at(637, 1336);
                        assert_eq!((result.0, result.1), canonical, "Drop still targeted hidden anchor {item}");
                        assert_eq!((result.2, result.3), (637, 1336), "Drop changed pointer coordinates");
                        SendMessageW(view, LVM_GETINSERTMARKRECT, 0, (&raw mut actual) as isize);
                        assert_eq!(coordinates(actual), committed_rect, "Canonical Drop marker left a hidden gap");
                    }
                    drop(session);
                    drop(capture);
                }
                println!("PASS: hidden insertion anchors and actual OLE proxy Drop agree at leading, middle, column-wrap and trailing boundaries; pointer unchanged");
            }
            Ok(())
        })();
        // Detach image lists before destroying both controls sharing the same image list.
        SendMessageW(view, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, 0);
        SendMessageW(other, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, 0);
        DestroyWindow(parent);
        ImageList_Destroy(images);
        result
    }
}
