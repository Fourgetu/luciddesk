//! Everything IPC2. Queries run on a worker with its own reply window/message pump.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*, System::DataExchange::COPYDATASTRUCT, UI::WindowsAndMessaging::*,
};

pub(super) const PAGE_SIZE: u32 = 200;
const TOKEN: usize = 0x4c504556;
const REQUEST: u32 = 0x4; // FULL_PATH_AND_NAME

#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub folder: bool,
}
#[derive(Debug)]
pub(super) struct Page {
    pub total: u32,
    pub offset: u32,
    pub entries: Vec<Entry>,
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let data = bytes
        .get(offset..offset.checked_add(4).ok_or("数据溢出")?)
        .ok_or("Everything 返回的数据不完整")?;
    Ok(u32::from_le_bytes(data.try_into().unwrap()))
}

fn parse(bytes: &[u8]) -> Result<Page, String> {
    let total = word(bytes, 0)?;
    let count = word(bytes, 4)?;
    let offset = word(bytes, 8)?;
    if word(bytes, 12)? != REQUEST
        || count > PAGE_SIZE
        || (count > 0 && offset.saturating_add(count) > total)
    {
        return Err("Everything 返回的结果格式不受支持".into());
    }
    let data_start = 20 + count as usize * 8;
    if bytes.len() < data_start {
        return Err("Everything 返回的数据不完整".into());
    }
    let mut entries = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let flags = word(bytes, 20 + i * 8)?;
        let start = word(bytes, 24 + i * 8)? as usize;
        if start < data_start {
            return Err("Everything 返回的偏移无效".into());
        }
        let length = word(bytes, start)? as usize;
        let end = start
            .checked_add(4)
            .and_then(|v| {
                length
                    .checked_add(1)
                    .and_then(|l| l.checked_mul(2))
                    .and_then(|l| v.checked_add(l))
            })
            .ok_or("Everything 返回的路径过长")?;
        let text = bytes
            .get(start + 4..end)
            .ok_or("Everything 返回的路径不完整")?;
        if text.len() < 2 || text[text.len() - 2..] != [0, 0] {
            return Err("Everything 返回的路径无效".into());
        }
        let wide: Vec<u16> = text[..text.len() - 2]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if wide.contains(&0) {
            return Err("Everything 返回的路径无效".into());
        }
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(&wide));
        if !path.is_absolute() {
            return Err("Everything 返回的路径不是绝对路径".into());
        }
        entries.push(Entry {
            path,
            folder: flags & 3 != 0,
        });
    }
    Ok(Page {
        total,
        offset,
        entries,
    })
}

unsafe extern "system" fn find_instance(hwnd: HWND, data: LPARAM) -> i32 {
    let mut class = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    let name = String::from_utf16_lossy(&class[..n.max(0) as usize]);
    if name.starts_with("EVERYTHING_TASKBAR_NOTIFICATION_(") {
        unsafe {
            *(data as *mut HWND) = hwnd;
        }
        return 0;
    }
    1
}

unsafe extern "system" fn receive(
    hwnd: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _: usize,
    reference: usize,
) -> LRESULT {
    if message == WM_COPYDATA && lp != 0 {
        let data = unsafe { &*(lp as *const COPYDATASTRUCT) };
        if data.dwData == TOKEN && !data.lpData.is_null() && data.cbData <= 32 * 1024 * 1024 {
            unsafe {
                *(reference as *mut Option<Vec<u8>>) = Some(
                    std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize)
                        .to_vec(),
                );
            }
            return 1;
        }
    }
    unsafe { windows_sys::Win32::UI::Shell::DefSubclassProc(hwnd, message, wp, lp) }
}

fn instance() -> HWND {
    let mut target = unsafe {
        FindWindowW(
            windows_sys::w!("EVERYTHING_TASKBAR_NOTIFICATION"),
            std::ptr::null(),
        )
    };
    if target.is_null() {
        unsafe {
            EnumWindows(Some(find_instance), (&raw mut target) as isize);
        }
    }
    target
}

pub(super) fn query(search: &str, offset: u32) -> Result<Page, String> {
    if search.encode_utf16().count() > 16_384 || search.contains('\0') {
        return Err("搜索内容过长或无效".into());
    }
    let target = instance();
    if target.is_null() {
        return Err("未连接 Everything，请启动 Everything 并启用 IPC，然后刷新。".into());
    }
    let mut reply: Option<Vec<u8>> = None;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            windows_sys::w!("STATIC"),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    struct ReplyWindow(HWND);
    impl Drop for ReplyWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    let _window = ReplyWindow(hwnd);
    unsafe {
        if windows_sys::Win32::UI::Shell::SetWindowSubclass(
            hwnd,
            Some(receive),
            1,
            (&raw mut reply) as usize,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, std::ptr::null_mut());
    }
    let mut payload = Vec::new();
    for value in [
        hwnd as usize as u32,
        TOKEN as u32,
        0,
        offset,
        PAGE_SIZE,
        REQUEST,
        1,
    ] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for value in search.encode_utf16().chain(Some(0)) {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    let data = COPYDATASTRUCT {
        dwData: 18,
        cbData: payload.len() as u32,
        lpData: payload.as_ptr().cast_mut().cast(),
    };
    let mut accepted = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            target,
            WM_COPYDATA,
            hwnd as usize,
            (&raw const data) as isize,
            SMTO_ABORTIFHUNG,
            1500,
            &raw mut accepted,
        )
    };
    if sent == 0 || accepted == 0 {
        return Err(
            "Everything 未响应搜索请求，请确认 IPC 已启用（需要 Everything 1.4 或更新版本）。"
                .into(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(bytes) = reply.take() {
            return parse(&bytes);
        }
        if Instant::now() >= deadline {
            return Err("Everything 搜索超时，请稍后刷新。".into());
        }
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 25, QS_ALLINPUT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(path: &str) -> Vec<u8> {
        let mut data = Vec::new();
        for v in [
            1u32,
            1,
            0,
            REQUEST,
            1,
            0,
            28,
            path.encode_utf16().count() as u32,
        ] {
            data.extend(v.to_le_bytes());
        }
        for c in path.encode_utf16().chain(Some(0)) {
            data.extend(c.to_le_bytes());
        }
        data
    }
    #[test]
    fn parses_unicode_and_rejects_truncated_packets() {
        let bytes = packet("C:\\测试\\文档.txt");
        let page = parse(&bytes).unwrap();
        assert_eq!(page.entries[0].path, PathBuf::from("C:\\测试\\文档.txt"));
        assert!(!page.entries[0].folder);
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end]).is_err(), "end={end}");
        }
    }
    #[test]
    fn rejects_untrusted_offsets_and_lengths() {
        let original = packet("C:\\a.txt");
        for (offset, value) in [
            (4, 201u32),
            (12, 0),
            (24, 0),
            (24, u32::MAX),
            (28, u32::MAX),
        ] {
            let mut bytes = original.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(parse(&bytes).is_err());
        }
    }
    #[test]
    #[ignore = "requires a running Everything instance with this repository indexed"]
    fn live_everything_query() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let page = query(&format!("path:\"{}\" Cargo.toml", root.display()), 0).unwrap();
        assert!(
            page.entries
                .iter()
                .any(|e| e.path == root.join("Cargo.toml")),
            "{page:?}"
        );
        let files = query(
            &format!("file: <path:\"{}\" Cargo.toml>", root.display()),
            0,
        )
        .unwrap();
        assert!(files.entries.iter().all(|e| !e.folder));
        let folders = query(&format!("folder: path:\"{}\"", root.display()), 0).unwrap();
        assert!(!folders.entries.is_empty());
        assert!(folders.entries.iter().all(|e| e.folder));
        let first = query("file:", 0).unwrap();
        if first.total > PAGE_SIZE {
            let second = query("file:", PAGE_SIZE).unwrap();
            assert_eq!(second.offset, PAGE_SIZE);
            assert!(!second.entries.is_empty());
            assert!(
                second
                    .entries
                    .iter()
                    .all(|entry| first.entries.iter().all(|old| old.path != entry.path))
            );
        }
        assert_eq!(
            query("lucidpane-no-match-7af4d18cb03e", 0).unwrap().total,
            0
        );
    }
}
