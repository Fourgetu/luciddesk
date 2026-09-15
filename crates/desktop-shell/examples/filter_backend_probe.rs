//! Live production-backend regression: remove, refresh, pause, restore, owner death.
//! Run with LucidPane closed. Never deletes, moves, or opens desktop file content.
use desktop_hook::filter::FilterSession;
use desktop_shell::{NativeDesktopSnapshot, native_desktop_snapshot};
use std::{
    os::windows::process::CommandExt,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, IServiceProvider},
            Variant::VARIANT,
        },
        UI::Shell::*,
    },
    core::Interface,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sta = desktop_shell::ShellApartment::initialize_sta()?;
    if std::env::args().any(|arg| arg == "--restore-saved-baseline") {
        return restore_saved_baseline();
    }
    let before = native_desktop_snapshot()?;
    let view = desktop_hook::desktop_view()?;
    let owner = windows_window::Window::new("LucidPane Filter Regression")
        .size(1, 1)
        .style(WS_POPUP)
        .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        .on_message(|_, msg, _, _| (msg == WM_DESTROY).then_some(0))
        .create()
        .map_err(|error| error.to_string())?;
    unsafe {
        ShowWindow(owner.hwnd().cast(), SW_HIDE);
    }
    let exe = std::env::current_exe()?;
    let source = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("desktop_hook.dll");
    let bytes = std::fs::read(&source)?;
    let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let dll = exe
        .parent()
        .unwrap()
        .join(format!("filter-regression-{hash:x}.dll"));
    if !dll.exists() {
        std::fs::write(&dll, bytes)?;
    }
    let hook = FilterSession::connect(view, owner.hwnd() as isize, &dll)?;
    let targets: Vec<_> = before
        .items
        .iter()
        .skip(before.items.len() / 2)
        .filter(|(item, _, _)| {
            item.identity
                .file_system_path()
                .is_some_and(|path| path.exists())
        })
        .take(2)
        .map(|(item, _, _)| {
            item.identity
                .activation_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if targets.len() != 2 {
        return Err("At least two filesystem desktop items required".into());
    }
    hook.set_hidden(&targets)?;
    check(&before, &targets)?;
    println!(
        "removed={} -> {}",
        before.items.len(),
        before.items.len() - targets.len()
    );
    if std::env::args().any(|arg| arg == "--crash-child") {
        println!("owner exiting without Rust destructors");
        std::process::exit(0);
    }
    let source = desktop_shell::enumerate_desktop_source()?;
    assert!(targets.iter().all(|name| source.iter().any(|item| {
        item.identity
            .activation_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
    })));
    println!("independent_source_keeps_filtered_items=true");
    let mut reused = None;
    for _ in 0..5 {
        let prior = native_desktop_snapshot()?;
        let host = hook.prepare_menu(owner.hwnd() as isize, &targets, 600, 450)?;
        assert_ne!(host, view, "Menu must have its own Shell view");
        if let Some(previous) = reused { assert_eq!(host, previous, "Same selection must reuse its Shell view"); }
        reused = Some(host);
        check(&before, &targets)?;
        assert!(!hook.finish_menu()?);
        let after = native_desktop_snapshot()?;
        assert!(prior.items.iter().all(|(old,x,y)| after.items.iter().any(|(new,nx,ny)| old.identity.equivalent_to(&new.identity) && x==nx && y==ny)));
    }
    assert!(hook.prepare_menu(owner.hwnd() as isize, &["C:\\LucidPane-missing-menu-test-item.invalid".into()], 600, 450).is_err());
    check(&before, &targets)?;
    assert!(hook.is_alive(), "Menu failures must not disable desktop filtering");
    let recovered = hook.prepare_menu(owner.hwnd() as isize, &targets, 620, 470)?;
    assert_eq!(Some(recovered), reused, "A bad target must not replace the valid cached host");
    assert!(!hook.finish_menu()?);
    let different = hook.prepare_menu(owner.hwnd() as isize, &targets[..1], 620, 470)?;
    assert_ne!(different, 0, "A new selection must receive a validated view; Windows may recycle HWNDs");
    assert!(!hook.finish_menu()?);
    check(&before, &targets)?;
    println!("isolated_menu_prepare_release_and_error_preserve_desktop=true");
    hook.begin_update()?;
    check(&before, &targets)?;
    assert!(hook.begin_update().is_err(), "Nested identity transactions must be rejected");
    assert!(hook.is_alive(), "A transaction error must not disable filtering");
    hook.set_hidden(&targets)?;
    hook.finish_update()?;
    check(&before, &targets)?;
    println!("identity_update_does_not_restore_membership=true");
    // Simulate losing the ordinary UPDATE_END transport entirely. The owner
    // remains alive; only the independent release marker reaches Explorer.
    hook.begin_update()?;
    let generation = unsafe { GetPropW(view as _, windows_sys::w!("LucidPane.Filter.Ack.v1")) };
    unsafe { SetPropW(view as _, windows_sys::w!("LucidPane.Filter.UpdateRelease.v1"), generation); }
    let deadline = Instant::now() + Duration::from_secs(3);
    while unsafe { GetPropW(view as _, windows_sys::w!("LucidPane.Filter.UpdateReleased.v1")) } != generation {
        assert!(Instant::now() < deadline, "timer must release an abandoned update with a live owner");
        std::thread::sleep(Duration::from_millis(20));
    }
    check(&before, &targets)?;
    hook.finish_update()?;
    hook.begin_update()?;
    let next = unsafe { GetPropW(view as _, windows_sys::w!("LucidPane.Filter.Ack.v1")) };
    assert_ne!(generation, next);
    // The preceding marker is still present and must not release this update.
    std::thread::sleep(Duration::from_millis(1100));
    assert_ne!(unsafe { GetPropW(view as _, windows_sys::w!("LucidPane.Filter.UpdateReleased.v1")) }, next);
    hook.finish_update()?;
    let _ = hook.prepare_menu(owner.hwnd() as isize, &targets, 620, 470)?;
    hook.cancel_menu()?;
    let _ = hook.prepare_menu(owner.hwnd() as isize, &targets, 620, 470)?;
    hook.finish_menu()?;
    check(&before, &targets)?;
    println!("lost_release_transport_and_cancelled_host_recovery=true");
    hook.pause(true)?;
    check(&before, &[])?;
    // Explorer's own AddObject/selection/menu code can send this while paused.
    // It must not turn painting back on until filtered membership is restored.
    unsafe {
        SendMessageW(view as _, WM_SETREDRAW, 1, 0);
    }
    hook.pause(false)?;
    check(&before, &targets)?;
    println!("menu_pause_resume=true");
    refresh()?;
    std::thread::sleep(Duration::from_millis(1600));
    check(&before, &targets)?;
    assert!(hook.is_alive());
    println!("refresh_reapplies_filter=true");
    hook.set_hidden(&targets[..1])?;
    check(&before, &targets[..1])?;
    hook.set_hidden(&[])?;
    std::thread::sleep(Duration::from_millis(250));
    check(&before, &[])?;
    let after = native_desktop_snapshot()?;
    let positions = before.items.iter().all(|(old, x, y)| {
        after
            .items
            .iter()
            .any(|(new, nx, ny)| old.identity.equivalent_to(&new.identity) && x == nx && y == ny)
    });
    println!("normal_restore_positions={positions}");
    if !positions {
        for (old, x, y) in &before.items {
            if let Some((_, nx, ny)) = after
                .items
                .iter()
                .find(|(new, _, _)| old.identity.equivalent_to(&new.identity))
                && (x != nx || y != ny)
            {
                println!("moved {}: {x},{y} -> {nx},{ny}", old.display_name);
            }
        }
    }
    assert!(positions);
    if let Some((item, _, _)) = before
        .items
        .iter()
        .find(|(item, _, _)| item.identity.file_system_path().is_none())
    {
        let names = vec![
            item.identity
                .activation_name()
                .to_string_lossy()
                .into_owned(),
        ];
        hook.set_hidden(&names)?;
        check(&before, &names)?;
        hook.set_hidden(&[])?;
        check(&before, &[])?;
        println!("namespace_remove_restore=true");
    }
    drop(hook);
    let child = std::process::Command::new(exe)
        .arg("--crash-child")
        .creation_flags(0x08000000)
        .output()?;
    print!("{}", String::from_utf8_lossy(&child.stdout));
    if !child.status.success() {
        return Err(String::from_utf8_lossy(&child.stderr).into_owned().into());
    }
    let started = Instant::now();
    loop {
        if native_desktop_snapshot()?.items.len() == before.items.len() {
            break;
        }
        if started.elapsed() > Duration::from_secs(5) {
            return Err("Owner watchdog did not restore desktop".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    check(&before, &[])?;
    println!("owner_exit_restore=true; PASS");
    Ok(())
}

fn check(
    before: &NativeDesktopSnapshot,
    hidden: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let after = native_desktop_snapshot()?;
    let list = desktop_hook::desktop_view()?;
    let count = unsafe {
        SendMessageW(
            list as _,
            windows_sys::Win32::UI::Controls::LVM_GETITEMCOUNT,
            0,
            0,
        )
    };
    assert_eq!(after.items.len(), before.items.len() - hidden.len());
    assert_eq!(count, after.items.len() as isize);
    for (old, _, _) in &before.items {
        let excluded = hidden.iter().any(|name| {
            old.identity
                .activation_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        });
        assert_eq!(
            after
                .items
                .iter()
                .any(|(item, _, _)| item.identity.equivalent_to(&old.identity)),
            !excluded
        );
    }
    assert!(
        hidden
            .iter()
            .all(|name| name.starts_with("::{") || std::path::Path::new(name).exists())
    );
    Ok(())
}

fn refresh() -> windows::core::Result<()> {
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let mut raw = 0;
        let dispatch = shell.FindWindowSW(
            &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
            &VARIANT::default(),
            SWC_DESKTOP,
            &raw mut raw,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        browser.QueryActiveShellView()?.Refresh()
    }
}

fn restore_saved_baseline() -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::{Foundation::POINT, System::Com::CoTaskMemFree};
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/desktop-filter-baseline.txt"
    ))?;
    let mut positions = std::collections::BTreeMap::new();
    let mut saved_order = Vec::new();
    for line in text.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() != 5 || columns[4].len() % 2 != 0 {
            return Err("Invalid baseline".into());
        }
        let bytes: Vec<u8> = columns[4]
            .as_bytes()
            .chunks_exact(2)
            .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16))
            .collect::<Result<_, _>>()?;
        saved_order.push(bytes.clone());
        positions.insert(
            bytes,
            POINT {
                x: columns[0].parse()?,
                y: columns[1].parse()?,
            },
        );
    }
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let mut raw = 0;
        let dispatch = shell.FindWindowSW(
            &VARIANT::from(CSIDL_DESKTOP.cast_signed()),
            &VARIANT::default(),
            SWC_DESKTOP,
            &raw mut raw,
            SWFO_NEEDDISPATCH,
        )?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let folder: IFolderView2 = browser.QueryActiveShellView()?.cast()?;
        let parent: IShellFolder = folder.GetFolder()?;
        let mut named_positions = std::collections::BTreeMap::new();
        let mut named_order = Vec::new();
        for bytes in &saved_order {
            let point = positions.get(bytes).ok_or("Missing baseline item")?;
            let mut at = 0;
            loop {
                let size = bytes.get(at..at + 2).ok_or("Malformed saved PIDL")?;
                let length = u16::from_le_bytes([size[0], size[1]]) as usize;
                if length == 0 {
                    if at + 2 != bytes.len() {
                        return Err("Trailing PIDL bytes".into());
                    }
                    break;
                }
                if length < 2 {
                    return Err("Malformed saved PIDL size".into());
                }
                at += length;
            }
            let item: IShellItem = SHCreateItemWithParent(None, &parent, bytes.as_ptr().cast())?;
            let raw = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?;
            let name = raw.to_string();
            CoTaskMemFree(Some(raw.0.cast()));
            let name = name?.to_lowercase();
            named_order.push(name.clone());
            named_positions.insert(name, *point);
        }
        let mut restored = Vec::new();
        for index in 0..folder.ItemCount(SVGIO_ALLVIEW)? {
            let id = folder.Item(index)?;
            let item: IShellItem = SHCreateItemWithParent(None, &parent, id)?;
            let raw = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?;
            let name = raw.to_string();
            CoTaskMemFree(Some(raw.0.cast()));
            let name = name?.to_lowercase();
            if let Some(point) = named_positions.get(&name) {
                restored.push((
                    named_order.iter().position(|entry| entry == &name).unwrap(),
                    id.cast_const(),
                    *point,
                ));
            } else {
                CoTaskMemFree(Some(id.cast()));
            }
        }
        restored.sort_by_key(|(index, _, _)| *index);
        let ids: Vec<_> = restored.iter().map(|(_, id, _)| *id).collect();
        let points: Vec<_> = restored.iter().map(|(_, _, point)| *point).collect();
        let result = if ids.len() == positions.len()
            && ids.len() == folder.ItemCount(SVGIO_ALLVIEW)? as usize
        {
            folder.SelectAndPositionItems(
                ids.len() as u32,
                ids.as_ptr(),
                Some(points.as_ptr()),
                SVSI_POSITIONITEM.0 as u32,
            )
        } else {
            Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Desktop identities changed; refusing stale baseline",
            ))
        };
        for id in ids {
            CoTaskMemFree(Some(id.cast()));
        }
        result?;
        std::thread::sleep(Duration::from_millis(500));
        println!(
            "saved baseline restored for {} matching identities",
            positions.len()
        );
    }
    Ok(())
}
