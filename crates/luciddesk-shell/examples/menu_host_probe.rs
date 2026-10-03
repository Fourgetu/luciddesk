//! Isolated research probe: proves Shell results/selection hosting, not modern-menu parity.
//! Run with absolute file paths. Nothing is moved, invoked, or changed on Explorer's desktop.
//! Optional --initialize probes the version-checked initialization without opening a menu.
#![allow(clippy::wildcard_imports)]
use windows::{
    Win32::{
        Foundation::{HWND, RECT},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::*,
    },
    core::{IUnknown, Interface, PCWSTR},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};

struct Window(windows_sys::Win32::Foundation::HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
struct Browser(IExplorerBrowser);
impl Drop for Browser {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.Destroy();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let initialize = std::env::args().any(|arg| arg == "--initialize");
    let names: Vec<_> = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--initialize")
        .collect();
    if names.is_empty() {
        return Err("Supply absolute filesystem paths or Shell parsing names".into());
    }
    let _apartment = luciddesk_shell::ShellApartment::initialize_sta()?;
    unsafe {
        let window = Window(CreateWindowExW(
            0,
            windows_sys::w!("STATIC"),
            windows_sys::w!("LucidDesk isolated Shell host probe"),
            WS_POPUP,
            0,
            0,
            400,
            300,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ));
        if window.0.is_null() {
            return Err(windows::core::Error::from_thread().into());
        }
        let browser = Browser(CoCreateInstance(
            &ExplorerBrowser,
            None,
            CLSCTX_INPROC_SERVER,
        )?);
        println!("browser_created=true");
        browser.0.Initialize(
            HWND(window.0),
            &RECT {
                left: 0,
                top: 0,
                right: 400,
                bottom: 300,
            },
            Some(&FOLDERSETTINGS {
                ViewMode: FVM_ICON.0 as u32,
                fFlags: FWF_AUTOARRANGE.0 as u32,
            }),
        )?;
        println!("browser_initialized=true");
        browser.0.SetOptions(EBO_NAVIGATEONCE | EBO_NOTRAVELLOG)?;
        browser
            .0
            .FillFromObject(None::<&IUnknown>, EBF_NODROPTARGET)?;
        println!("results_folder_created=true");
        let view: IFolderView2 = browser.0.GetCurrentView()?;
        let results: IResultsFolder = view.GetFolder()?;
        for name in &names {
            let text: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(text.as_ptr()), None)?;
            results.AddItem(&item)?;
        }
        pump_results_notifications();
        let count = view.ItemCount(SVGIO_ALLVIEW)?;
        println!("results_count={count}, requested={}", names.len());
        if usize::try_from(count)? != names.len() {
            return Err("Results count mismatch".into());
        }
        view.SelectItem(
            0,
            (SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_DESELECTOTHERS.0).cast_unsigned(),
        )?;
        println!("selection_requested=true");
        let pidl = view.Item(0)?;
        let selection = view.GetSelectionState(pidl);
        CoTaskMemFree(Some(pidl.cast()));
        let selection = selection?;
        println!(
            "selected_count={}, selected_flags=0x{selection:x}",
            view.ItemCount(SVGIO_SELECTION)?
        );
        if selection & SVSI_SELECT.0.cast_unsigned() == 0 {
            return Err("Selection did not stick".into());
        }
        let shell_view: IShellView = view.cast()?;
        let menu: IContextMenu = shell_view.GetItemObject(SVGIO_SELECTION)?;
        println!(
            "selection_IContextMenu_available={}",
            !menu.as_raw().is_null()
        );
        println!("modern_menu_verified=false (no popup invoked by this read-only probe)");
        inspect_native_menu_class(&shell_view, HWND(window.0), initialize);
    }
    Ok(())
}

fn inspect_native_menu_class(view: &IShellView, owner: HWND, initialize: bool) {
    unsafe {
        // Registered as File Explorer Context Menu. Private initialization is optional
        // and only runs after the known implementation has been matched.
        let class = windows::core::GUID::from_u128(0x86ca1aa0_34aa_4e8b_a509_50c905bae2a2);
        match CoCreateInstance::<_, IUnknown>(&raw const class, None, CLSCTX_INPROC_SERVER) {
            Ok(object) => {
                println!("native_class_created=true");
                // IID located in this build's public-symbol-matched QueryInterface code.
                // Only use the standard IUnknown ABI; no private presenter methods invoked.
                let presenter_iid =
                    windows::core::GUID::from_u128(0x37a472f7_63cf_4ccf_a88b_5231a3c7d8b6);
                let mut presenter = std::ptr::null_mut();
                let status = (object.vtable().QueryInterface)(
                    object.as_raw(),
                    &raw const presenter_iid,
                    &raw mut presenter,
                );
                println!("native_class_IContextMenuPresenter={status:?}");
                if status.is_ok() && !presenter.is_null() {
                    if inspect_presenter_vtable(presenter) && initialize {
                        probe_presenter_initialization(presenter, view, owner);
                    }
                    drop(IUnknown::from_raw(presenter));
                }
                println!(
                    "native_class_IContextMenu={}",
                    object.cast::<IContextMenu>().is_ok()
                );
                println!(
                    "native_class_IShellExtInit={}",
                    object.cast::<IShellExtInit>().is_ok()
                );
                println!(
                    "native_class_IInitializeWithItem={}",
                    object.cast::<IInitializeWithItem>().is_ok()
                );
                println!(
                    "native_class_IMenuPopup={}",
                    object.cast::<IMenuPopup>().is_ok()
                );
                println!(
                    "native_class_IObjectWithSelection={}",
                    object.cast::<IObjectWithSelection>().is_ok()
                );
                println!(
                    "native_class_IExecuteCommand={}",
                    object.cast::<IExecuteCommand>().is_ok()
                );
                println!(
                    "native_class_IObjectWithSite={}",
                    object
                        .cast::<windows::Win32::System::Ole::IObjectWithSite>()
                        .is_ok()
                );
                match object.cast::<windows::core::IInspectable>() {
                    Ok(inspectable) => {
                        println!(
                            "native_class_runtime_name={:?}",
                            inspectable.GetRuntimeClassName()
                        );
                        let mut count = 0;
                        let mut ids = std::ptr::null_mut();
                        let status = (inspectable.vtable().GetIids)(
                            inspectable.as_raw(),
                            &raw mut count,
                            &raw mut ids,
                        );
                        println!("native_class_GetIids={status:?}, count={count}");
                        if status.is_ok() && !ids.is_null() {
                            for id in std::slice::from_raw_parts(ids, count as usize) {
                                println!("native_class_iid={id:?}");
                            }
                        }
                        CoTaskMemFree(Some(ids.cast()));
                    }
                    Err(_) => println!("native_class_IInspectable=false"),
                }
            }
            Err(error) => println!("native_class_activation={error}"),
        }
    }
}

// Read only: correlate this process's activated implementation with the matching PDB.
// No function pointers from this diagnostic are called.
fn inspect_presenter_vtable(presenter: *mut std::ffi::c_void) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
    }
    unsafe {
        let base = GetModuleHandleW(windows::core::w!("Windows.UI.FileExplorer.dll").as_ptr());
        if base.is_null() {
            println!("presenter_module_missing=true");
            return false;
        }
        let table = *presenter.cast::<*const usize>();
        for (index, address) in std::slice::from_raw_parts(table, 11).iter().enumerate() {
            println!(
                "presenter_slot_{index}_rva=0x{:x}",
                address.wrapping_sub(base as usize)
            );
        }
        let expected = [
            0xb9f90, 0xa4700, 0xba340, 0xaa5d0, 0xb9bc0, 0xa5fc0, 0xa5d60, 0xace30, 0xac6f0,
            0xace50, 0xa5e70,
        ];
        let matches = std::slice::from_raw_parts(table, expected.len())
            .iter()
            .zip(expected)
            .all(|(actual, rva)| actual.wrapping_sub(base as usize) == rva);
        println!("presenter_known_vtable={matches}");
        matches
    }
}

fn probe_presenter_initialization(
    presenter: *mut std::ffi::c_void,
    view: &IShellView,
    owner: HWND,
) {
    // ABI and arguments verified against the matching shell32 CDesktopBrowser call site.
    // The real Shell view supplies the callback; no commands or popup are requested here.
    type Initialize = unsafe extern "system" fn(
        *mut std::ffi::c_void,
        i32,
        *mut std::ffi::c_void,
        HWND,
        i32,
    ) -> windows::core::HRESULT;
    unsafe {
        let iid = windows::core::GUID::from_u128(0x9a19ddcf_9ed4_4a18_89de_c3b4dd1d7ae3);
        let unknown: IUnknown = match view.cast() {
            Ok(value) => value,
            Err(error) => {
                println!("callback_unknown={error}");
                return;
            }
        };
        let mut raw = std::ptr::null_mut();
        let status =
            (unknown.vtable().QueryInterface)(unknown.as_raw(), &raw const iid, &raw mut raw);
        println!("view_IInvokeContextMenuCommand={status:?}");
        if status.is_err() || raw.is_null() {
            return;
        }
        let callback = IUnknown::from_raw(raw);
        let table = *presenter.cast::<*const usize>();
        let initialize: Initialize = std::mem::transmute(*table.add(3));
        println!(
            "presenter_initialize={:?}",
            initialize(presenter, 1, callback.as_raw(), owner, 1)
        );
        // Diagnostic only: Initialize uses this field in the exact implementation checked above.
        println!(
            "presenter_xaml_enabled={}",
            *presenter.cast::<u8>().add(0xb2)
        );
        // Close before releasing the callback; the native presenter stores a borrowed pointer.
        let presenter_object = IUnknown::from_raw_borrowed(&presenter).expect("non-null presenter");
        if let Ok(closable) = presenter_object.cast::<windows::Foundation::IClosable>() {
            println!(
                "presenter_close_before_callback_release={:?}",
                closable.Close()
            );
        }
    }
}

fn pump_results_notifications() {
    unsafe {
        // Results notifications are asynchronous; let this host's view materialize its items.
        for _ in 0..25 {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
            };
            let mut message = MSG::default();
            while PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
