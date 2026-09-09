//! Cross-process integration test. Only targets a disposable `ListView` created by this binary.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
use desktop_hook::{
    HookSession,
    protocol::{
        Area, QUERY_AREA_COUNT, QUERY_AUTOARRANGE, QUERY_ITEM_AREA, QUERY_ITEM_COUNT, Request,
    },
};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::{
    ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, LVIF_TEXT, LVITEMW,
    LVM_ARRANGE, LVM_GETNUMBEROFWORKAREAS, LVM_INSERTITEMW, LVM_SETITEMCOUNT, LVM_SETWORKAREAS,
    LVS_AUTOARRANGE, LVS_ICON, LVS_OWNERDATA,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW, GWL_STYLE,
    GetMessageW, GetWindowLongW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW,
    TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW,
};

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == windows_sys::Win32::UI::WindowsAndMessaging::WM_NOTIFY && lp != 0 {
        let header = unsafe { &*(lp as *const windows_sys::Win32::UI::Controls::NMHDR) };
        if header.code == windows_sys::Win32::UI::Controls::LVN_GETDISPINFOW {
            let info =
                unsafe { &mut *(lp as *mut windows_sys::Win32::UI::Controls::NMLVDISPINFOW) };
            info.item.pszText = windows_sys::w!("Test 2").cast_mut();
            return 0;
        }
    }
    if msg == WM_APP + 1 {
        let lv =
            unsafe { FindWindowExW(hwnd, null_mut(), windows_sys::w!("SysListView32"), null()) };
        let mut count: u32 = 0;
        let owner_data = unsafe { GetWindowLongW(lv, GWL_STYLE) } & LVS_OWNERDATA as i32 != 0;
        if owner_data {
            let mut point = windows_sys::Win32::Foundation::POINT::default();
            unsafe {
                SendMessageW(
                    lv,
                    windows_sys::Win32::UI::Controls::LVM_GETITEMPOSITION,
                    2,
                    (&raw mut point) as isize,
                );
            }
            assert!(point.x < 400, "Mapped coordinates were not restored");
        } else {
            unsafe {
                SendMessageW(lv, LVM_GETNUMBEROFWORKAREAS, 0, (&raw mut count) as isize);
            }
        }
        let auto = unsafe { GetWindowLongW(lv, GWL_STYLE) } & LVS_AUTOARRANGE as i32 != 0;
        println!("RESTORED areas={count} auto={auto}");
        std::io::stdout().flush().unwrap();
        return 0;
    }
    if msg == WM_APP + 2 {
        let lv =
            unsafe { FindWindowExW(hwnd, null_mut(), windows_sys::w!("SysListView32"), null()) };
        unsafe {
            SendMessageW(lv, LVM_SETWORKAREAS, 0, 0);
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

// Keep the complete cross-process lifecycle together so cleanup remains visible.
#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    let geometry = std::env::args().any(|a| a == "--geometry");
    let watchdog = std::env::args().any(|a| a == "--watchdog");
    let owner_data = geometry || std::env::args().any(|a| a == "--owner-data");
    if std::env::args().any(|a| a == "--host") {
        return host(owner_data);
    }
    let dll = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("desktop_hook.dll");
    let exe = std::env::current_exe().unwrap();
    let mut command = Command::new(exe);
    command.arg("--host");
    if owner_data {
        command.arg("--owner-data");
    }
    let mut child = command
        .creation_flags(0x0800_0000)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).map_err(|e| e.to_string())?;
    let handles: Vec<isize> = line.trim().split(' ').map(|s| s.parse().unwrap()).collect();
    let (parent, view) = (handles[0], handles[1]);
    let owner = unsafe {
        CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("LucidPane Hook Test Controller"),
            0,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        )
    };
    let result = (|| {
        if owner_data && !geometry {
            return verify_owner_data_rejected(view, owner as isize, &dll);
        }
        let session = if geometry {
            HookSession::connect_geometry(view, owner as isize, &dll)?
        } else {
            HookSession::connect(view, owner as isize, &dll)?
        };
        assert_eq!(session.request(&Request::new(QUERY_AUTOARRANGE))?, 1);
        let areas = [
            Area {
                left: 0,
                top: 0,
                right: 300,
                bottom: 500,
            },
            Area {
                left: 400,
                top: 80,
                right: 750,
                bottom: 500,
            },
        ];
        session.set_areas(&areas)?;
        session.move_item_named(2, 450, 150, "Test 2")?;
        if geometry {
            use desktop_hook::protocol::{QUERY_HIT, QUERY_SHELL_GENERATION};
            let generation = session.request(&Request::new(QUERY_SHELL_GENERATION))?;
            let started = std::time::Instant::now();
            for frame in 0..120 {
                let x = 450 + frame;
                session.apply_layout(&areas, &[(2, x, 150, "Test 2".into())])?;
                let mut hit = Request::new(QUERY_HIT);
                hit.x = x + 30;
                hit.y = 160;
                assert_eq!(
                    session.request(&hit)?,
                    3,
                    "Hit testing lagged behind frame {frame}"
                );
                assert_eq!(
                    session.request(&Request::new(QUERY_SHELL_GENERATION))?,
                    generation,
                    "Our geometry commit incorrectly invalidated the Shell cache"
                );
            }
            println!(
                "PASS: 120 single-IPC geometry frames with matching hit tests; {:.2} ms/frame including verification (hidden fixture, not desktop FPS)",
                started.elapsed().as_secs_f64() * 1000.0 / 120.0
            );
            assert!(
                session
                    .apply_layout(
                        &areas,
                        &[
                            (2, 700, 150, "Test 2".into()),
                            (3, 750, 150, "stale identity".into())
                        ]
                    )
                    .is_err()
            );
            let mut hit = Request::new(QUERY_HIT);
            hit.x = 599;
            hit.y = 160;
            assert_eq!(
                session.request(&hit)?,
                3,
                "Rejected batch partially published an earlier item"
            );
            println!("PASS: invalid batch leaves the previous complete layout visible");
        }
        assert!(
            session
                .move_item_named(2, 40, 40, "wrong stale item")
                .is_err()
        );
        // External arrange and a Shell-like attempt to reset work areas must preserve grouping.
        unsafe {
            SendMessageW(view as HWND, LVM_ARRANGE, 0, 0);
        }
        if geometry {
            assert!(
                session.request(&Request::new(
                    desktop_hook::protocol::QUERY_SHELL_GENERATION
                ))? > 0,
                "Native arrangement must invalidate the baseline cache"
            );
        }
        let mut location = Request::new(QUERY_ITEM_AREA);
        location.item = 2;
        assert_eq!(session.request(&location)?, 1);
        if !geometry {
            unsafe {
                SendMessageW(parent as HWND, WM_APP + 2, 0, 0);
            }
        }
        assert_eq!(session.request(&location)?, 1);
        assert_eq!(session.request(&Request::new(QUERY_AUTOARRANGE))?, 1);
        assert_eq!(session.request(&Request::new(QUERY_AREA_COUNT))?, 2);
        assert_eq!(session.request(&Request::new(QUERY_ITEM_COUNT))?, 5);
        if watchdog {
            unsafe {
                DestroyWindow(owner);
            }
            std::thread::sleep(std::time::Duration::from_millis(1600));
            assert_ne!(
                session.request(&Request::new(desktop_hook::protocol::QUERY)),
                Ok(desktop_hook::protocol::OK)
            );
            println!("PASS: watchdog removed mapping after controller window was destroyed");
        }
        drop(session);
        unsafe {
            SendMessageW(parent as HWND, WM_APP + 1, 0, 0);
        }
        line.clear();
        output.read_line(&mut line).map_err(|e| e.to_string())?;
        assert!(line.contains("areas=0 auto=true"), "{line}");
        println!(
            "PASS: cross-process DLL hook, native per-area auto-arrange, item move, detach restoration; {line}"
        );
        Ok(())
    })();
    unsafe {
        DestroyWindow(owner);
        PostMessageW(parent as HWND, WM_CLOSE, 0, 0);
    }
    let _ = child.wait();
    result
}

fn verify_owner_data_rejected(
    view: isize,
    owner: isize,
    dll: &std::path::Path,
) -> Result<(), String> {
    // A missing DLL proves validation precedes loading or sending work-area messages.
    let missing = dll.with_file_name("does-not-exist.dll");
    let Err(error) = HookSession::connect(view, owner, &missing) else {
        return Err("Owner-data work-area backend was unexpectedly accepted".into());
    };
    assert!(error.contains("虚拟图标视图"), "{error}");
    let style = unsafe { GetWindowLongW(view as HWND, GWL_STYLE) };
    assert_ne!(style & LVS_OWNERDATA as i32, 0);
    assert_ne!(style & LVS_AUTOARRANGE as i32, 0);
    println!("PASS: owner-data view rejected before DLL loading; native styles unchanged");
    Ok(())
}

fn host(owner_data: bool) -> Result<(), String> {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        });
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            lpszClassName: windows_sys::w!("LucidPaneHookTestHost"),
            ..std::mem::zeroed()
        };
        RegisterClassW(&raw const class);
        let hwnd = CreateWindowExW(
            0,
            class.lpszClassName,
            windows_sys::w!("LucidPane isolated hook test"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            1000,
            700,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        let lv = CreateWindowExW(
            0,
            windows_sys::w!("SysListView32"),
            null(),
            WS_CHILD | LVS_ICON | LVS_AUTOARRANGE | if owner_data { LVS_OWNERDATA } else { 0 },
            0,
            0,
            900,
            600,
            hwnd,
            null_mut(),
            null_mut(),
            null(),
        );
        if lv.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if owner_data {
            SendMessageW(lv, LVM_SETITEMCOUNT, 5, 0);
        }
        for i in 0..if owner_data { 0 } else { 5 } {
            let mut text: Vec<u16> = format!("Test {i}").encode_utf16().chain(Some(0)).collect();
            let item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i,
                pszText: text.as_mut_ptr(),
                ..std::mem::zeroed()
            };
            SendMessageW(lv, LVM_INSERTITEMW, 0, (&raw const item) as isize);
        }
        println!("{} {}", hwnd as isize, lv as isize);
        std::io::stdout().flush().unwrap();
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
        Ok(())
    }
}
