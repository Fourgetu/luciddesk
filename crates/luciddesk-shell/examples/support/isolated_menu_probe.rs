//! In-Explorer isolated Shell host: show and cancel a menu, never invoke verbs.
use std::{fmt::Write, time::{Duration, Instant}};
use windows::{core::{Interface, IUnknown, HSTRING, Result}, Win32::{
    Foundation::{HWND, RECT}, System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER}, UI::Shell::*
}};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
#[path = "isolated_presenter.rs"]
mod isolated_presenter;

struct Host { browser: IExplorerBrowser, hwnd: windows_sys::Win32::Foundation::HWND, presenter: Option<windows::Win32::System::Com::IServiceProvider> }
impl Drop for Host {
    fn drop(&mut self) { unsafe {
        KillTimer(self.hwnd, 0x4c504d50);
        if let Some(presenter) = &self.presenter { isolated_presenter::close(presenter); }
        if let Ok(site) = self.browser.cast::<windows::Win32::System::Ole::IObjectWithSite>() { let _ = site.SetSite(None::<&IUnknown>); }
        let _ = self.browser.Destroy(); DestroyWindow(self.hwnd);
    } }
}
unsafe extern "system" fn cancel(hwnd: windows_sys::Win32::Foundation::HWND, _: u32, _: usize, _: u32) {
    unsafe {
        let mut popups: Vec<(isize, String)> = Vec::new();
        EnumWindows(Some(collect), (&raw mut popups) as isize);
        OBSERVED.with(|seen| seen.borrow_mut().extend(popups.into_iter().map(|(_,class)| class)));
        if STARTED.with(|time| time.get().is_some_and(|time| time.elapsed() > Duration::from_millis(2500))) {
            KillTimer(hwnd, 0x4c504d50); EndMenu(); SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
        }
    }
}
thread_local! {
    static OBSERVED: std::cell::RefCell<std::collections::BTreeSet<String>> = const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
    static STARTED: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}
unsafe extern "system" fn collect(hwnd: windows_sys::Win32::Foundation::HWND, data: isize) -> i32 {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &raw mut pid);
        if pid == windows_sys::Win32::System::Threading::GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            let mut name = [0u16; 128];
            let len = GetClassNameW(hwnd, name.as_mut_ptr(), 128);
            let class = String::from_utf16_lossy(&name[..len.max(0) as usize]);
            if class == "Microsoft.UI.Content.PopupWindowSiteBridge" || class == "#32768" {
                (*(data as *mut Vec<(isize, String)>)).push((hwnd as isize, class));
            }
        }
    }
    1
}
fn pump() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT { PostQuitMessage(msg.wParam as i32); break; }
            TranslateMessage(&msg); DispatchMessageW(&msg);
        }
    }
}
pub fn run(desktop: &IFolderView2, mode: isize, log: &mut String) -> Result<()> {
    unsafe {
        let before = luciddesk_shell::native_desktop_snapshot().map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
        let source = luciddesk_shell::enumerate_desktop_source().map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
        let target = source.iter().find(|item| item.identity.file_system_path().is_some() && !before.items.iter().any(|(v,_,_)| v.identity.equivalent_to(&item.identity)))
            .or_else(|| source.iter().find(|item| item.identity.file_system_path().is_some())).unwrap();
        writeln!(log, "target_is_filtered={}", !before.items.iter().any(|(v,_,_)| v.identity.equivalent_to(&target.identity))).ok();
        let browser: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, windows_sys::w!("STATIC"), windows_sys::w!("LucidDesk isolated menu probe"), WS_POPUP | WS_VISIBLE,
            600, 450, 1, 1, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
        let mut host = Host { browser, hwnd, presenter: None };
        host.browser.Initialize(HWND(hwnd), &RECT {left:0,top:0,right:640,bottom:480}, Some(&FOLDERSETTINGS {ViewMode:FVM_ICON.0 as u32, fFlags:FWF_AUTOARRANGE.0 as u32}))?;
        host.browser.SetOptions(EBO_NAVIGATEONCE | EBO_NOTRAVELLOG)?;
        host.browser.FillFromObject(None::<&IUnknown>, EBF_NODROPTARGET)?;
        let folder: IFolderView2 = host.browser.GetCurrentView()?;
        let results: IResultsFolder = folder.GetFolder()?;
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(target.identity.activation_name().to_string_lossy().as_ref()), None)?;
        results.AddItem(&item)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while folder.ItemCount(SVGIO_ALLVIEW)? != 1 && Instant::now() < deadline { pump(); std::thread::sleep(Duration::from_millis(10)); }
        folder.SelectItem(0, (SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_DESELECTOTHERS.0) as u32)?;
        writeln!(log, "isolated_count={} selected={}", folder.ItemCount(SVGIO_ALLVIEW)?, folder.ItemCount(SVGIO_SELECTION)?).ok();
        let view: IShellView = folder.cast()?;
        if mode == 5 {
            let presenter = isolated_presenter::create(&view, HWND(hwnd))?;
            let site: windows::Win32::System::Ole::IObjectWithSite = host.browser.cast()?;
            site.SetSite(&presenter)?;
            host.presenter = Some(presenter);
            writeln!(log, "isolated_presenter_site=true").ok();
        }
        let popup_view: IShellView = if mode == 4 { desktop.cast()? } else { view.clone() };
        let site: IContextMenuSite = popup_view.cast()?;
        let old_focus = GetForegroundWindow();
        writeln!(log, "foreground={}", SetForegroundWindow(GetAncestor(popup_view.GetWindow()?.0, GA_ROOT))).ok();
        popup_view.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32)?;
        let menu: IContextMenu = view.GetItemObject(SVGIO_SELECTION)?;
        OBSERVED.with(|seen| seen.borrow_mut().clear());
        STARTED.set(Some(Instant::now()));
        SetTimer(hwnd, 0x4c504d50, 50, Some(cancel));
        writeln!(log, "popup_result={:?}", site.DoContextMenuPopup(&menu, CMF_ITEMMENU | CMF_CANRENAME, windows::Win32::Foundation::POINT{x:600,y:450})).ok();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut seen = std::collections::BTreeSet::new();
        while Instant::now() < deadline {
            pump();
            let mut popups: Vec<(isize, String)> = Vec::new();
            EnumWindows(Some(collect), (&raw mut popups) as isize);
            for (popup, class) in popups {
                if seen.insert(class) {
                    let mut rect = windows_sys::Win32::Foundation::RECT::default();
                    GetWindowRect(popup as _, &raw mut rect);
                    writeln!(log, "popup_rect={},{},{},{}", rect.left,rect.top,rect.right,rect.bottom).ok();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        writeln!(log, "popup_classes={seen:?}").ok();
        let mut final_popups: Vec<(isize, String)> = Vec::new();
        EnumWindows(Some(collect), (&raw mut final_popups) as isize);
        for (popup, _) in final_popups {
            let mut rect = windows_sys::Win32::Foundation::RECT::default(); GetWindowRect(popup as _, &raw mut rect);
            writeln!(log, "final_popup_rect={},{},{},{}",rect.left,rect.top,rect.right,rect.bottom).ok();
        }
        OBSERVED.with(|seen| writeln!(log, "popup_classes_during_modal={:?}", seen.borrow()).ok());
        SendMessageW(hwnd, WM_CANCELMODE, 0, 0);
        SendMessageW(view.GetWindow()?.0, WM_CANCELMODE, 0, 0);
        SetForegroundWindow(old_focus);
        pump();
        writeln!(log, "desktop_count_after={}", desktop.ItemCount(SVGIO_ALLVIEW)?).ok();
        let after = luciddesk_shell::native_desktop_snapshot().map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
        let unchanged = before.items.len() == after.items.len() && before.items.iter().all(|(old,x,y)| after.items.iter().any(|(new,nx,ny)| old.identity.equivalent_to(&new.identity) && x==nx && y==ny));
        writeln!(log,"desktop_members_and_positions_unchanged={unchanged}").ok();
    }
    Ok(())
}
