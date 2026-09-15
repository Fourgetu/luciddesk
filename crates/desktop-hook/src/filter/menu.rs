//! Isolated Shell selection for native menus. Never adds items to the desktop.
mod callback;
mod classic;
mod commands;
mod cursor;
mod lifecycle;
mod presenter;
mod selection;

pub mod worker;
use super::{RENAME, wire::MenuContext};
use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, HWND, RECT},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            Ole::IObjectWithSite,
        },
        UI::Shell::*,
    },
    core::{IUnknown, Interface, Result},
};
use windows_sys::Win32::UI::{
    Controls::*,
    Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
    WindowsAndMessaging::*,
};
const SUBCLASS: usize = 0x4c504d48;

pub struct MenuHost {
    browser: IExplorerBrowser,
    hwnd: windows_sys::Win32::Foundation::HWND,
    view: Option<IShellView>,
    presenter: Option<presenter::NativePresenter>,
    callbacks: Option<Rc<MenuCallbacks>>,
    names: Vec<String>,
}
struct MenuCallbacks {
    first: Rc<Cell<Option<u32>>>,
    busy: Cell<bool>,
    view: IShellView,
    desktop: windows_sys::Win32::Foundation::HWND,
    context: Cell<MenuContext>,
    presenter: Option<presenter::NativePresenter>,
    invocation: lifecycle::Invocation,
    retired: Cell<bool>,
    classic: RefCell<Option<IContextMenu>>,
}
impl MenuHost {
    pub fn create(
        desktop: windows_sys::Win32::Foundation::HWND,
        names: &[String],
        context: MenuContext,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::create_impl(desktop, names, context, cancelled, true)
    }
    fn create_impl(
        desktop: windows_sys::Win32::Foundation::HWND,
        names: &[String],
        context: MenuContext,
        cancelled: Arc<AtomicBool>,
        compact_available: bool,
    ) -> Result<Self> {
        unsafe {
            if names.is_empty() || IsWindow(context.owner as _) == 0 {
                return Err(E_INVALIDARG.into());
            }
            let browser: IExplorerBrowser =
                CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
            // A clipped native host supplies DPI/focus/selection context. No desktop
            // icons are drawn here; only the separate system popup is presented.
            let mut instance = null_mut();
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleExW(
                windows_sys::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | windows_sys::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                (host_messages as *const ()).cast(), &raw mut instance);
            let class = windows_sys::w!("LucidPane.IsolatedShellHost.v1");
            let definition = WNDCLASSW {
                lpfnWndProc: Some(host_messages),
                hInstance: instance,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&definition);
            // LUCIDPANE_INSPECT marks the test Pane as APPWINDOW. Carry that
            // inspectability to its menu host without changing its empty region.
            let inspect =
                GetWindowLongPtrW(context.owner as _, GWL_EXSTYLE) & WS_EX_APPWINDOW as isize != 0;
            let hwnd = CreateWindowExW(
                (if inspect {
                    WS_EX_APPWINDOW
                } else {
                    WS_EX_TOOLWINDOW
                }) | WS_EX_NOACTIVATE,
                class,
                windows_sys::w!("LucidPane Menu Host"),
                WS_POPUP,
                context.x.saturating_sub(32),
                context.y.saturating_sub(32),
                640,
                480,
                null_mut(),
                null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            let region = windows_sys::Win32::Graphics::Gdi::CreateRectRgn(0, 0, 0, 0);
            if windows_sys::Win32::Graphics::Gdi::SetWindowRgn(hwnd, region, 0) == 0 {
                windows_sys::Win32::Graphics::Gdi::DeleteObject(region);
                DestroyWindow(hwnd);
                return Err(windows::core::Error::from_thread());
            }
            let mut host = Self {
                browser,
                hwnd,
                view: None,
                presenter: None,
                callbacks: None,
                names: names.to_vec(),
            };
            host.browser
                .Initialize(
                    HWND(hwnd),
                    &RECT {
                        left: 0,
                        top: 0,
                        right: 640,
                        bottom: 480,
                    },
                    Some(&FOLDERSETTINGS {
                        ViewMode: FVM_ICON.0 as u32,
                        fFlags: FWF_AUTOARRANGE.0 as u32,
                    }),
                )
                .map_err(|e| {
                    windows::core::Error::new(e.code(), format!("独立视图初始化失败：{e}"))
                })?;
            host.browser
                .SetOptions(EBO_NAVIGATEONCE | EBO_NOTRAVELLOG)?;
            host.browser
                .FillFromObject(None::<&IUnknown>, EBF_NODROPTARGET)
                .map_err(|e| {
                    windows::core::Error::new(e.code(), format!("独立集合初始化失败：{e}"))
                })?;
            let folder: IFolderView2 = host.browser.GetCurrentView().map_err(|e| {
                windows::core::Error::new(e.code(), format!("获取独立视图失败：{e}"))
            })?;
            let results: IResultsFolder = folder.GetFolder().map_err(|e| {
                windows::core::Error::new(e.code(), format!("获取独立集合失败：{e}"))
            })?;
            let items = selection::resolve(names)?;
            for item in &items {
                results.AddItem(item)?;
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while folder.ItemCount(SVGIO_ALLVIEW)? as usize != items.len() {
                if Instant::now() >= deadline {
                    return Err(windows::core::Error::new(E_FAIL, "独立菜单项目加载超时"));
                }
                let mut message = MSG::default();
                for _ in 0..32 {
                    if PeekMessageW(&raw mut message, null_mut(), 0, 0, PM_REMOVE) == 0 {
                        break;
                    }
                    if message.message == WM_QUIT {
                        PostQuitMessage(message.wParam as i32);
                        return Err(E_FAIL.into());
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                MsgWaitForMultipleObjectsEx(
                    0,
                    std::ptr::null(),
                    5,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
            }
            // Select the isolated result set, including mixed folders and namespace
            // items. Validate identities before any menu can invoke a command.
            selection::select_all(&folder, items.len())?;
            let selected: IShellItemArray = folder.Items(SVGIO_SELECTION)?;
            if selected.GetCount()? as usize != items.len() {
                return Err(windows::core::Error::new(E_FAIL, "独立视图选择数不匹配"));
            }
            if !selection::contains_all(&selected, &items)? {
                return Err(windows::core::Error::new(E_FAIL, "独立菜单目标校验失败"));
            }
            let view: IShellView = folder.cast()?;
            let first = Rc::new(Cell::new(None));
            let invocation = lifecycle::Invocation::new(cancelled);
            let presenter = if compact_available {
                presenter::NativePresenter::create(
                    &view,
                    HWND(hwnd),
                    HWND(desktop),
                    first.clone(),
                    invocation.check_fn(),
                )
                .and_then(|presenter| {
                    let site: IObjectWithSite = host.browser.cast()?;
                    site.SetSite(&presenter.service())?;
                    Ok(presenter)
                })
                .ok()
            } else {
                None
            };
            // A missing private capability affects only the compact presenter.
            // The validated Shell selection still supplies a public classic menu.
            host.presenter = presenter.clone();
            let view_hwnd = view.GetWindow()?.0;
            let callbacks = Rc::new(MenuCallbacks {
                first,
                busy: Cell::new(false),
                view: view.clone(),
                desktop,
                context: Cell::new(context),
                presenter,
                invocation,
                retired: Cell::new(false),
                classic: RefCell::new(None),
            });
            if SetWindowSubclass(
                view_hwnd,
                Some(menu_messages),
                SUBCLASS,
                (&*callbacks as *const MenuCallbacks) as usize,
            ) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            host.callbacks = Some(callbacks);
            host.view = Some(view);
            Ok(host)
        }
    }
    pub fn is_busy(&self) -> bool {
        self.callbacks
            .as_ref()
            .is_some_and(|callbacks| callbacks.busy.get())
    }
    /// Reuse only an idle host whose complete live identity set still matches.
    /// A disappeared/renamed target must never invoke a cached, stale selection.
    pub fn reprepare(
        &self,
        names: &[String],
        context: MenuContext,
        cancelled: Arc<AtomicBool>,
    ) -> Result<bool> {
        if self.is_busy() || self.names != names || self.has_owned_windows() {
            return Ok(false);
        }
        unsafe {
            let callbacks = self.callbacks.as_ref().ok_or(E_FAIL)?;
            if callbacks.retired.get() {
                return Ok(false);
            }
            if callbacks.context.get().owner != context.owner || IsWindow(context.owner as _) == 0 {
                return Ok(false);
            }
            let items = selection::resolve(names)?;
            let folder: IFolderView2 = callbacks.view.cast()?;
            let live: IShellItemArray = folder.Items(SVGIO_ALLVIEW)?;
            if live.GetCount()? as usize != items.len() {
                return Ok(false);
            }
            if !selection::contains_all(&live, &items)? {
                return Ok(false);
            }
            selection::select_all(&folder, items.len())?;
            callbacks.context.set(context);
            callbacks.invocation.reset(cancelled);
            SetWindowPos(
                self.hwnd,
                null_mut(),
                context.x.saturating_sub(32),
                context.y.saturating_sub(32),
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
            Ok(true)
        }
    }
    pub fn finish(&self, cancel: bool) -> Result<()> {
        if let Some(callbacks) = &self.callbacks {
            if cancel {
                callbacks.retired.set(true);
                if let Some(presenter) = &callbacks.presenter {
                    presenter.close()?;
                }
            }
            callbacks.busy.set(false);
        }
        Ok(())
    }
    pub fn has_owned_windows(&self) -> bool {
        struct Search {
            root: windows_sys::Win32::Foundation::HWND,
            found: bool,
        }
        unsafe extern "system" fn visit(
            hwnd: windows_sys::Win32::Foundation::HWND,
            data: isize,
        ) -> i32 {
            unsafe {
                let search = &mut *(data as *mut Search);
                if hwnd != search.root
                    && IsWindowVisible(hwnd) != 0
                    && GetAncestor(hwnd, GA_ROOTOWNER) == search.root
                {
                    search.found = true;
                    return 0;
                }
                1
            }
        }
        let mut search = Search {
            root: self.hwnd,
            found: false,
        };
        unsafe {
            EnumWindows(Some(visit), (&raw mut search) as isize);
        }
        search.found
    }
    pub fn view_hwnd(&self) -> Result<isize> {
        unsafe { Ok(self.view.as_ref().ok_or(E_FAIL)?.GetWindow()?.0 as isize) }
    }
}
impl Drop for MenuHost {
    fn drop(&mut self) {
        unsafe {
            // Close XAML while its Shell callback and windows are still alive.
            if let Some(presenter) = &self.presenter {
                let _ = presenter.close();
            }
            if let Some(view) = &self.view {
                if let Ok(hwnd) = view.GetWindow() {
                    RemoveWindowSubclass(hwnd.0, Some(menu_messages), SUBCLASS);
                }
            }
            if let Ok(site) = self.browser.cast::<IObjectWithSite>() {
                let _ = site.SetSite(None::<&IUnknown>);
            }
            let _ = self.browser.Destroy();
            DestroyWindow(self.hwnd);
        }
    }
}
unsafe extern "system" fn menu_messages(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    data: usize,
) -> isize {
    unsafe {
        let pointer = data as *const MenuCallbacks;
        Rc::increment_strong_count(pointer);
        let callbacks = Rc::from_raw(pointer);
        if callbacks
            .presenter
            .as_ref()
            .is_some_and(|p| p.handle_command_message(msg, wp))
        {
            return 0;
        }
        if msg == WM_CANCELMODE && callbacks.invocation.cancelled() {
            EndMenu();
            return 0;
        }
        if let Some(menu) = callbacks.classic.borrow().clone() {
            if let Some(result) = classic::message(&menu, msg, wp, lp) {
                return result;
            }
        }
        if msg == WM_NOTIFY && lp != 0 {
            let header = &*(lp as *const NMHDR);
            if matches!(header.code, LVN_BEGINLABELEDITA | LVN_BEGINLABELEDITW)
                && GetParent(header.hwndFrom) == hwnd
            {
                if !callbacks.invocation.cancelled() {
                    SetPropW(callbacks.desktop, RENAME, 1usize as _);
                }
                return 1;
            }
        }
        // UI activation must happen on the Shell STA, after the app has installed
        // its popup observer and handed foreground permission to Explorer.
        if msg == RegisterWindowMessageW(windows_sys::w!("LucidPane.IsolatedMenu.Open.v1")) {
            if callbacks.invocation.cancelled() || callbacks.retired.get() {
                return 0;
            }
            if callbacks.busy.replace(true) {
                return 0;
            }
            let started = Instant::now();
            let result = (|| -> Result<()> {
                let context = callbacks.context.get();
                let root = GetAncestor(hwnd, GA_ROOT);
                let style = GetWindowLongPtrW(root, GWL_EXSTYLE);
                SetWindowLongPtrW(root, GWL_EXSTYLE, style & !(WS_EX_NOACTIVATE as isize));
                ShowWindow(root, SW_SHOWNOACTIVATE);
                if SetForegroundWindow(root) == 0 {
                    return Err(windows::core::Error::new(E_FAIL, "无法激活独立菜单宿主"));
                }
                callbacks.view.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32)?;
                cursor::normal_pointer();
                {
                    if let Some(presenter) = &callbacks.presenter {
                        presenter.set_keyboard_invocation(wp != 0);
                    }
                    callbacks.first.set(None);
                    let menu: IContextMenu = callbacks.view.GetItemObject(SVGIO_SELECTION)?;
                    cursor::normal_pointer();
                    SetPropW(
                        hwnd,
                        windows_sys::w!("LucidPane.Menu.GetItemUs"),
                        started.elapsed().as_micros().saturating_add(1) as usize as _,
                    );
                    callbacks.invocation.check()?;
                    let menu = commands::wrap_cancellable(
                        menu,
                        HWND(callbacks.desktop),
                        callbacks.first.clone(),
                        callbacks.invocation.check_fn(),
                    );
                    if callbacks.presenter.is_none() {
                        return classic::show(
                            hwnd,
                            &menu,
                            context,
                            &callbacks.classic,
                            &callbacks.invocation,
                        );
                    }
                    let site: IContextMenuSite = callbacks.view.cast()?;
                    site.DoContextMenuPopup(
                        &menu,
                        CMF_ITEMMENU | CMF_CANRENAME,
                        windows::Win32::Foundation::POINT {
                            x: context.x,
                            y: context.y,
                        },
                    )
                }
            })();
            cursor::normal_pointer();
            SetPropW(
                hwnd,
                windows_sys::w!("LucidPane.Menu.BuildUs"),
                started.elapsed().as_micros().saturating_add(1) as usize as _,
            );
            if let Err(error) = result {
                callbacks.busy.set(false);
                SetPropW(
                    callbacks.desktop,
                    super::MENU_ERROR,
                    error.code().0 as u32 as usize as _,
                );
            }
            return 0;
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn host_messages(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wp: usize,
    lp: isize,
) -> isize {
    unsafe {
        if msg == WM_MOUSEACTIVATE {
            return MA_ACTIVATE as isize;
        }
        if msg == WM_SETCURSOR {
            SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
            return 1;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    #[test]
    #[ignore = "creates a native Shell view; run separately outside the restricted test sandbox"]
    fn unavailable_presenter_keeps_validated_shell_selection_and_rename_verb() {
        unsafe {
            windows::Win32::System::Ole::OleInitialize(None).unwrap();
            let owner = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!(""),
                WS_POPUP,
                0,
                0,
                640,
                480,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!owner.is_null());
            let path = std::env::temp_dir().join(format!(
                "lucidpane-classic-capability-{}.txt",
                std::process::id()
            ));
            std::fs::write(&path, b"owned menu fixture").unwrap();
            {
                let host = MenuHost::create_impl(
                    owner,
                    &[path.to_string_lossy().into_owned()],
                    MenuContext {
                        owner: owner as u64,
                        x: 20,
                        y: 20,
                    },
                    Arc::new(AtomicBool::new(false)),
                    false,
                )
                .unwrap();
                let callbacks = host.callbacks.as_ref().unwrap();
                assert!(callbacks.presenter.is_none());
                let menu: IContextMenu = callbacks.view.GetItemObject(SVGIO_SELECTION).unwrap();
                let menu = commands::wrap(menu, HWND(owner), callbacks.first.clone());
                let popup = windows::Win32::UI::WindowsAndMessaging::CreatePopupMenu().unwrap();
                let count = menu.QueryContextMenu(popup, 0, 1, 0x7fff, CMF_NORMAL | CMF_CANRENAME);
                count.ok().unwrap();
                assert!(
                    (0..(count.0 as usize & 0xffff)).any(|id| commands::is_rename(&menu, id)),
                    "public fallback must keep native rename without a private Presenter"
                );
                windows::Win32::UI::WindowsAndMessaging::DestroyMenu(popup).unwrap();
            }
            std::fs::remove_file(path).unwrap();
            DestroyWindow(owner);
            windows::Win32::System::Ole::OleUninitialize();
        }
    }
}
