use super::*;
use std::{
    cell::Cell,
    path::Path,
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE, HWND},
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::*},
};

/// Owns one desktop membership session. Dropping it restores removed view items.
pub struct FilterSession {
    view: HWND,
    owner: HWND,
    hook: HHOOK,
    module: HMODULE,
    sequence: Cell<u32>,
    update: Cell<Option<u32>>,
}
impl FilterSession {
    /// # Errors
    /// Fails if Explorer does not expose the required COM interface on its UI thread.
    pub fn connect(view: isize, owner: isize, dll: &Path) -> Result<Self, String> {
        unsafe {
            let view = view as HWND;
            let owner = owner as HWND;
            if IsWindow(view) == 0 || IsWindow(owner) == 0 {
                return Err("桌面或控制窗口无效".into());
            }
            if !GetPropW(view, OWNER).is_null() {
                return Err("桌面过滤已有活动连接".into());
            }
            let path = std::fs::canonicalize(dll).map_err(|e| e.to_string())?;
            let path: Vec<u16> = path
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let module = LoadLibraryExW(
                path.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if module.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let Some(proc) = GetProcAddress(module, windows_sys::s!("LucidPaneFilterHook")) else {
                FreeLibrary(module);
                return Err("Hook DLL 缺少视图过滤入口".into());
            };
            let callback = std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(i32, usize, isize) -> isize,
            >(proc);
            let hook = SetWindowsHookExW(
                WH_GETMESSAGE,
                Some(callback),
                module,
                GetWindowThreadProcessId(view, null_mut()),
            );
            if hook.is_null() {
                FreeLibrary(module);
                return Err(std::io::Error::last_os_error().to_string());
            }
            let session = Self {
                view,
                owner,
                hook,
                module,
                sequence: Cell::new(0),
                update: Cell::new(None),
            };
            RemovePropW(view, ERROR);
            if PostMessageW(view, message(), owner as usize, MAGIC as isize) == 0 {
                return Err("无法请求桌面过滤连接".into());
            }
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                if GetPropW(view, OWNER) == owner {
                    return Ok(session);
                }
                let error = GetPropW(view, ERROR) as usize as u32;
                if error != 0 {
                    return Err(format!("此桌面不支持视图过滤 (0x{error:08x})"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err("Explorer 未确认桌面过滤连接".into())
        }
    }
    #[must_use]
    pub fn is_alive(&self) -> bool {
        unsafe {
            IsWindow(self.view) != 0
                && GetPropW(self.view, OWNER) == self.owner
                && GetPropW(self.view, ERROR).is_null()
        }
    }
    /// # Errors
    /// Returns an error if Explorer cannot apply and acknowledge the complete set.
    pub fn set_hidden(&self, names: &[String]) -> Result<(), String> {
        self.request(wire::SET, names)
    }
    /// # Errors
    /// Fails when the desktop connection is unavailable.
    pub fn clear_selection(&self) -> Result<(), String> {
        self.request(wire::CLEAR_SELECTION, &[])
    }
    /// Freeze presentation while a managed Shell identity changes. Membership
    /// remains filtered; the caller must finish even if the operation fails.
    pub fn begin_update(&self) -> Result<(), String> {
        if self.update.get().is_some() {
            if unsafe { GetPropW(self.view, UPDATE_RELEASE) as usize }
                != self.update.get().unwrap() as usize
            {
                return Err("已有活动身份更新".into());
            }
            self.finish_update()?;
        }
        let sequence = self.next_sequence();
        self.update.set(Some(sequence));
        let bytes = wire::encode(wire::UPDATE_BEGIN, sequence, &[])?;
        self.send(wire::UPDATE_BEGIN, sequence, &bytes)
    }
    pub fn finish_update(&self) -> Result<(), String> {
        let Some(sequence) = self.update.get() else {
            return Ok(());
        };
        unsafe {
            if GetPropW(self.view, OWNER) != self.owner {
                return Err("桌面过滤连接已断开".into());
            }
            // This marker survives a rejected/timed-out WM_COPYDATA. Explorer's
            // timer services it even when the client remains alive. A sequence
            // makes a late release harmless to a subsequent transaction.
            if SetPropW(self.view, UPDATE_RELEASE, sequence as usize as _) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            PostMessageW(self.view, work_message(), 0, 0);
            if GetPropW(self.view, UPDATE_RELEASED) as usize == sequence as usize {
                self.update.set(None);
                return Ok(());
            }
        }
        let result = self.request(wire::UPDATE_END, &[]);
        if result.is_ok() {
            self.update.set(None);
        }
        result
    }
    pub fn replace_identity(&self, old: &str, new: &str) -> Result<(), String> {
        self.request(wire::REPLACE_IDENTITY, &[old.into(), new.into()])
    }
    /// Temporarily restores Shell membership for Explorer preview integrations.
    /// # Errors
    /// Fails when the desktop cannot enter/leave the suspended filtering scope.
    pub fn pause(&self, pause: bool) -> Result<bool, String> {
        self.request(if pause { wire::PAUSE } else { wire::RESUME }, &[])?;
        Ok(!pause && unsafe { !GetPropW(self.view, RENAME).is_null() })
    }
    /// Prepare an independent Shell view. Desktop membership remains filtered.
    /// # Errors
    /// Fails if Explorer cannot provide an isolated native menu host.
    /// Pumps caller UI messages; callers must release app and event borrows first.
    pub fn prepare_menu(
        &self,
        owner: isize,
        names: &[String],
        x: i32,
        y: i32,
    ) -> Result<isize, String> {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(self.view, &raw mut pid);
            AllowSetForegroundWindow(pid);
            let sequence = self.next_sequence();
            let bytes = wire::encode_menu(
                sequence,
                names,
                wire::MenuContext {
                    owner: owner as u64,
                    x,
                    y,
                },
            )?;
            self.send(wire::MENU_PREPARE, sequence, &bytes)?;
            let host = GetPropW(self.view, MENU_HOST);
            if IsWindow(host) == 0 {
                return Err("Explorer 未创建独立菜单窗口".into());
            }
            Ok(host as isize)
        }
    }
    /// Release the menu host and return whether its rename command was chosen.
    /// # Errors
    /// Fails when Explorer does not acknowledge cleanup. Pumps caller UI messages.
    pub fn finish_menu(&self) -> Result<bool, String> {
        self.request(wire::MENU_FINISH, &[])?;
        Ok(unsafe { !GetPropW(self.view, RENAME).is_null() })
    }
    pub fn cancel_menu(&self) -> Result<(), String> {
        self.request(wire::MENU_CANCEL, &[])
    }
    fn next_sequence(&self) -> u32 {
        let sequence = self.sequence.get().wrapping_add(1).max(1);
        self.sequence.set(sequence);
        sequence
    }
    fn request(&self, op: u32, names: &[String]) -> Result<(), String> {
        let sequence = self.next_sequence();
        let bytes = wire::encode(op, sequence, names)?;
        self.send(op, sequence, &bytes)
    }
    fn send(&self, op: u32, sequence: u32, bytes: &[u8]) -> Result<(), String> {
        unsafe {
            if GetPropW(self.view, OWNER) != self.owner {
                return Err("桌面过滤连接已断开".into());
            }
            let data = COPYDATASTRUCT {
                dwData: MAGIC,
                cbData: bytes.len() as u32,
                lpData: bytes.as_ptr().cast_mut().cast(),
            };
            let mut accepted = 0;
            if SendMessageTimeoutW(
                self.view,
                WM_COPYDATA,
                self.owner as usize,
                (&raw const data) as isize,
                SMTO_ABORTIFHUNG,
                1500,
                &raw mut accepted,
            ) == 0
                || accepted != MAGIC
            {
                return Err("Explorer 拒绝桌面过滤请求".into());
            }
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                if op == wire::DETACH && GetPropW(self.view, OWNER).is_null() {
                    return Ok(());
                }
                if GetPropW(self.view, ACK) as usize == sequence as usize {
                    if matches!(
                        op,
                        wire::MENU_PREPARE
                            | wire::MENU_CANCEL
                            | wire::MENU_FINISH
                            | wire::UPDATE_BEGIN
                            | wire::UPDATE_END
                            | wire::REPLACE_IDENTITY
                    ) {
                        let error = GetPropW(self.view, MENU_ERROR) as usize as u32;
                        return if error == 0 {
                            Ok(())
                        } else {
                            Err(format!("独立原生菜单不可用 (0x{error:08x})"))
                        };
                    }
                    let error = GetPropW(self.view, ERROR) as usize as u32;
                    return if error == 0 {
                        let retry = GetPropW(self.view, REQUEST_ERROR) as usize as u32;
                        if retry == 0 {
                            Ok(())
                        } else {
                            Err(format!(
                                "桌面快照暂不可用，保留过滤等待重试 (0x{retry:08x})"
                            ))
                        }
                    } else {
                        Err(format!("桌面过滤失败，已请求恢复原生项目 (0x{error:08x})"))
                    };
                }
                if matches!(
                    op,
                    wire::MENU_PREPARE
                        | wire::MENU_CANCEL
                        | wire::MENU_FINISH
                        | wire::UPDATE_BEGIN
                        | wire::UPDATE_END
                        | wire::REPLACE_IDENTITY
                ) {
                    // Explorer can synchronously query our windows while creating
                    // its Shell view. Menu callers must hold no UI model borrows.
                    let mut message = MSG::default();
                    for _ in 0..32 {
                        if PeekMessageW(&raw mut message, null_mut(), 0, 0, PM_REMOVE) == 0 {
                            break;
                        }
                        if message.message == WM_QUIT {
                            PostQuitMessage(message.wParam as i32);
                            return Err("菜单操作被退出请求中断".into());
                        }
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                if matches!(
                    op,
                    wire::MENU_PREPARE
                        | wire::MENU_CANCEL
                        | wire::MENU_FINISH
                        | wire::UPDATE_BEGIN
                        | wire::UPDATE_END
                        | wire::REPLACE_IDENTITY
                ) {
                    MsgWaitForMultipleObjectsEx(
                        0,
                        std::ptr::null(),
                        5,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Err(
                if matches!(
                    op,
                    wire::MENU_PREPARE
                        | wire::MENU_CANCEL
                        | wire::MENU_FINISH
                        | wire::UPDATE_BEGIN
                        | wire::UPDATE_END
                        | wire::REPLACE_IDENTITY
                ) {
                    "Explorer 菜单准备或释放超时".into()
                } else {
                    "桌面过滤响应超时".into()
                },
            )
        }
    }
}
impl Drop for FilterSession {
    fn drop(&mut self) {
        let _ = self.request(wire::DETACH, &[]);
        unsafe {
            UnhookWindowsHookEx(self.hook);
            FreeLibrary(self.module);
        }
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    #[test]
    fn rejected_release_keeps_token_and_can_be_acknowledged_outside_ipc() {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            SetPropW(hwnd, OWNER, hwnd);
            // STATIC rejects WM_COPYDATA, so this exercises a real transport
            // rejection without injecting into the user's Explorer.
            let session = std::mem::ManuallyDrop::new(FilterSession {
                view: hwnd,
                owner: hwnd,
                hook: null_mut(),
                module: null_mut(),
                sequence: Cell::new(42),
                update: Cell::new(Some(42)),
            });
            assert!(session.finish_update().is_err());
            assert_eq!(GetPropW(hwnd, UPDATE_RELEASE) as usize, 42);
            assert_eq!(session.update.get(), Some(42));
            assert!(session.begin_update().is_err());
            assert_eq!(
                session.update.get(),
                Some(42),
                "must not start a new freeze before release"
            );
            SetPropW(hwnd, UPDATE_RELEASED, 42usize as _);
            session.finish_update().unwrap();
            assert_eq!(session.update.get(), None);
            DestroyWindow(hwnd);
        }
    }
}
