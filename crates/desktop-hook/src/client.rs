// Wire sizes and array counts are bounded by the fixed protocol before conversion.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
use crate::protocol::{
    Area, DETACH, MAGIC, MAX_AREAS, MOVE_ITEM, OK, QUERY, REJECTED, Request, SET_AREAS, name_hash,
};
use std::mem::size_of;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{FreeLibrary, HWND};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetPropW, GetWindowThreadProcessId, HHOOK, IsWindow,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetWindowsHookExW, UnhookWindowsHookEx, WH_CALLWNDPROC,
    WM_COPYDATA,
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// Finds the real desktop `ListView`; never creates or hides a replacement icon layer.
/// # Errors
/// Returns an error when the current Shell desktop cannot be found.
pub fn desktop_view() -> Result<isize, String> {
    unsafe {
        let progman = FindWindowW(windows_sys::w!("Progman"), null());
        let defview = FindWindowExW(
            progman,
            null_mut(),
            windows_sys::w!("SHELLDLL_DefView"),
            null(),
        );
        if !defview.is_null() {
            let view = FindWindowExW(
                defview,
                null_mut(),
                windows_sys::w!("SysListView32"),
                null(),
            );
            if !view.is_null() {
                return Ok(view as isize);
            }
        }
        let mut result: HWND = null_mut();
        EnumWindows(Some(find_view), (&raw mut result) as isize);
        if result.is_null() {
            Err("找不到 Explorer 原生桌面图标视图".into())
        } else {
            Ok(result as isize)
        }
    }
}

unsafe extern "system" fn find_view(hwnd: HWND, lp: isize) -> i32 {
    let defview = unsafe {
        FindWindowExW(
            hwnd,
            null_mut(),
            windows_sys::w!("SHELLDLL_DefView"),
            null(),
        )
    };
    if !defview.is_null() {
        let view = unsafe {
            FindWindowExW(
                defview,
                null_mut(),
                windows_sys::w!("SysListView32"),
                null(),
            )
        };
        if !view.is_null() {
            unsafe {
                *(lp as *mut HWND) = view;
            }
            return 0;
        }
    }
    1
}

/// Detect another active desktop organizer before allowing work-area changes.
#[must_use]
pub fn conflicting_desktop_extension() -> bool {
    // Detection is intentionally limited to known loaded Explorer modules, not window titles.
    // The app may also use this to display a concrete conflict instead of fighting its layout.
    let Ok(view) = desktop_view() else {
        return false;
    };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(view as HWND, &raw mut pid);
    }
    loaded_fences(pid) && fences_running()
}

fn fences_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return true;
        }
        let mut item = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut current = Process32FirstW(snapshot, &raw mut item);
        let mut found = false;
        while current != 0 {
            let end = item
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(item.szExeFile.len());
            if String::from_utf16_lossy(&item.szExeFile[..end]).eq_ignore_ascii_case("Fences.exe") {
                found = true;
                break;
            }
            current = Process32NextW(snapshot, &raw mut item);
        }
        CloseHandle(snapshot);
        found
    }
}

fn loaded_fences(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE,
        TH32CS_SNAPMODULE32,
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..std::mem::zeroed()
        };
        let mut found = false;
        let mut available = Module32FirstW(snapshot, &raw mut entry);
        while available != 0 {
            let end = entry
                .szModule
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szModule.len());
            let name = String::from_utf16_lossy(&entry.szModule[..end]).to_lowercase();
            if name == "desktopdock64.dll" {
                found = true;
                break;
            }
            available = Module32NextW(snapshot, &raw mut entry);
        }
        CloseHandle(snapshot);
        found
    }
}

/// Owns the per-thread Windows hook. Drop asks the target UI thread to restore its baseline.
pub struct HookSession {
    view: HWND,
    owner: HWND,
    hook: HHOOK,
    module: windows_sys::Win32::Foundation::HMODULE,
}

impl HookSession {
    /// Connect the exact-image virtual-icon geometry backend. No work-area messages are used.
    /// # Errors
    /// Fails when the target cannot validate and bind all native geometry functions.
    pub fn connect_geometry(view: isize, owner: isize, dll: &Path) -> Result<Self, String> {
        let bootstrap = crate::protocol::geometry_attach_message();
        let view = view as HWND;
        let owner = owner as HWND;
        unsafe {
            let thread = GetWindowThreadProcessId(view, null_mut());
            if thread == 0 || IsWindow(owner) == 0 {
                return Err("Hook 目标或控制窗口无效".into());
            }
            let path = std::fs::canonicalize(dll)
                .map_err(|e| format!("找不到 Hook DLL {}：{e}", dll.display()))?;
            let path_text = path.to_string_lossy();
            let hook_path = path_text.strip_prefix(r"\\?\").unwrap_or(&path_text);
            let module = LoadLibraryExW(
                wide(hook_path).as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if module.is_null() {
                return Err(format!(
                    "加载 Hook DLL 失败：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let Some(proc) = GetProcAddress(module, windows_sys::s!("LucidPaneDesktopHook")) else {
                FreeLibrary(module);
                return Err("Hook DLL 版本或导出函数不匹配".into());
            };
            let callback = std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(i32, usize, isize) -> isize,
            >(proc);
            let hook = SetWindowsHookExW(WH_CALLWNDPROC, Some(callback), module, thread);
            if hook.is_null() {
                FreeLibrary(module);
                return Err(format!(
                    "安装桌面线程 Hook 失败：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let session = Self {
                view,
                owner,
                hook,
                module,
            };
            // Windows may have captured the original WndProc before WH_CALLWNDPROC installs
            // our subclass. Confirm on a second message, after the bootstrap has returned.
            session.send(bootstrap, owner as usize, MAGIC as isize)?;
            let result = session.send(bootstrap, owner as usize, MAGIC as isize)?;
            if result != OK {
                return Err(format!(
                    "Explorer 未确认 Hook 连接，原桌面保持不变 (stage={}, result={result})",
                    GetPropW(view, windows_sys::w!("LucidPane.Hook.Bootstrap")) as usize
                ));
            }
            session.request(&Request::new(QUERY))?;
            Ok(session)
        }
    }

    fn send(&self, message: u32, wp: usize, lp: isize) -> Result<isize, String> {
        let mut result = 0;
        if unsafe {
            SendMessageTimeoutW(
                self.view,
                message,
                wp,
                lp,
                SMTO_ABORTIFHUNG,
                1500,
                &raw mut result,
            )
        } == 0
        {
            Err(format!(
                "桌面 Hook 请求超时或目标已退出：{}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(result as isize)
        }
    }

    /// Sends a bounded request to the view's UI thread.
    /// # Errors
    /// Fails for malformed requests, rejected operations or an unresponsive target.
    pub fn request(&self, request: &Request) -> Result<isize, String> {
        if !request.valid() {
            return Err("无效的桌面 Hook 请求".into());
        }
        let data = COPYDATASTRUCT {
            dwData: MAGIC,
            cbData: size_of::<Request>() as u32,
            lpData: std::ptr::from_ref(request).cast_mut().cast(),
        };
        let result = self.send(WM_COPYDATA, self.owner as usize, (&raw const data) as isize)?;
        if result == REJECTED {
            Err("原生视图拒绝此布局操作；没有关闭自动排列".into())
        } else {
            Ok(result)
        }
    }

    /// Queue selection cleanup without blocking pane input or painting.
    /// # Errors
    /// Returns an error if the notification could not be registered or queued.
    pub fn post_clear_desktop_selection(&self) -> Result<(), String> {
        let message = crate::protocol::clear_selection_message();
        if message == 0
            || unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.view,
                    message,
                    self.owner as usize,
                    0,
                )
            } == 0
        {
            return Err(format!(
                "无法通知桌面清除选择：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// Sets the validated bounds used by this session's geometry updates.
    /// # Errors
    /// Fails for invalid areas or a rejected geometry update.
    pub fn set_areas(&self, areas: &[Area]) -> Result<(), String> {
        if areas.len() > MAX_AREAS {
            return Err("工作区域数量过多".into());
        }
        let mut request = Request::new(SET_AREAS);
        request.count = areas.len() as u32;
        request.areas[..areas.len()].copy_from_slice(areas);
        if self.request(&request)? != OK {
            return Err("原生工作区设置未得到确认".into());
        }
        Ok(())
    }

    /// Publishes native pane appearance, clipping membership and positions together.
    /// # Errors
    /// Rejects a stale or oversized scene without publishing partial changes.
    pub fn apply_pane_layout(
        &self,
        areas: &[Area],
        positions: &[(i32, i32, i32, String, u32)],
        panes: &[crate::protocol::PaneAppearance],
    ) -> Result<(), String> {
        use crate::protocol::{ItemPosition, LAYOUT_MAGIC, LayoutBatch, MAX_LAYOUT_ITEMS};
        if areas.len() > MAX_AREAS || panes.len() > MAX_AREAS || positions.len() > MAX_LAYOUT_ITEMS
        {
            return Err("分组布局超过单次事务容量".into());
        }
        let mut batch = LayoutBatch {
            areas: Request::new(SET_AREAS),
            count: positions.len() as u32,
            flush: 1,
            pane_count: panes.len() as u32,
            panes: [crate::protocol::PaneAppearance::default(); MAX_AREAS],
            items: [ItemPosition::default(); MAX_LAYOUT_ITEMS],
        };
        batch.areas.count = areas.len() as u32;
        batch.panes[..panes.len()].copy_from_slice(panes);
        batch.areas.areas[..areas.len()].copy_from_slice(areas);
        for (destination, (item, x, y, name, pane)) in batch.items.iter_mut().zip(positions) {
            *destination = ItemPosition {
                item: *item,
                x: *x,
                y: *y,
                name_hash: name_hash(name.encode_utf16()),
                reserved: *pane,
            };
        }
        if !batch.valid() {
            return Err("无效的分组布局事务".into());
        }
        let data = COPYDATASTRUCT {
            dwData: LAYOUT_MAGIC,
            cbData: size_of::<LayoutBatch>() as u32,
            lpData: std::ptr::from_ref(&batch).cast_mut().cast(),
        };
        if self.send(WM_COPYDATA, self.owner as usize, (&raw const data) as isize)? != OK {
            return Err("原生视图拒绝分组布局，项目或可用空间可能已变化".into());
        }
        Ok(())
    }

    /// Uploads a bounded decoded wallpaper atlas once, outside the drag path.
    /// # Errors
    /// Rejects invalid dimensions, mismatched buffers or unavailable native view.
    pub fn set_texture(
        &self,
        header: crate::protocol::TextureHeader,
        pixels: &[u8],
    ) -> Result<(), String> {
        if header.byte_count() != Some(pixels.len()) {
            return Err("无效的材质缓冲区".into());
        }
        let mut bytes =
            Vec::with_capacity(size_of::<crate::protocol::TextureHeader>() + pixels.len());
        bytes.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(&header).cast::<u8>(),
                size_of::<crate::protocol::TextureHeader>(),
            )
        });
        bytes.extend_from_slice(pixels);
        let data = COPYDATASTRUCT {
            dwData: crate::protocol::TEXTURE_MAGIC,
            cbData: bytes.len() as u32,
            lpData: bytes.as_mut_ptr().cast(),
        };
        if self.send(WM_COPYDATA, self.owner as usize, (&raw const data) as isize)? != OK {
            return Err("无法安装分组材质".into());
        }
        Ok(())
    }

    /// Checks the current label before moving, rejecting stale view indices.
    /// # Errors
    /// Also fails if the item label changed since the caller's snapshot.
    pub fn move_item_named(&self, index: i32, x: i32, y: i32, name: &str) -> Result<(), String> {
        self.move_item_checked(index, x, y, name_hash(name.encode_utf16()))
    }

    fn move_item_checked(&self, index: i32, x: i32, y: i32, hash: u64) -> Result<(), String> {
        let mut request = Request::new(MOVE_ITEM);
        request.name_hash = hash;
        request.item = index;
        request.x = x;
        request.y = y;
        if self.request(&request)? != OK {
            return Err("原生图标移动未得到确认".into());
        }
        Ok(())
    }
}

impl Drop for HookSession {
    fn drop(&mut self) {
        let _ = self.request(&Request::new(DETACH));
        unsafe {
            UnhookWindowsHookEx(self.hook);
            FreeLibrary(self.module);
        }
    }
}
