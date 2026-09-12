//! Use the resident Peek Shell entry point for files and virtual desktop items.
use super::keyboard::{self, Modifiers};
use desktop_core::ShellIdentity;
use desktop_storage::WorkspaceStore;
use std::{cell::RefCell, path::PathBuf};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Settings {
    pub enabled: bool,
    pub path: String,
    pub key: u16,
    pub modifiers: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            path: String::new(),
            key: VK_SPACE,
            modifiers: 0,
        }
    }
}
thread_local! { static SETTINGS: RefCell<Settings> = RefCell::new(Settings::default()); }
pub(super) fn settings() -> Settings {
    SETTINGS.with(|s| s.borrow().clone())
}
fn encode(s: &Settings) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        u8::from(s.enabled),
        s.key,
        s.modifiers,
        s.path
    )
}
fn decode(raw: &str) -> Option<Settings> {
    let mut lines = raw.splitn(4, '\n');
    let enabled = match lines.next()? {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    let key = lines.next()?.parse().ok()?;
    let modifiers = lines.next()?.parse().ok()?;
    let path = lines.next()?.to_owned();
    valid_shortcut(key, modifiers).then_some(Settings {
        enabled,
        key,
        modifiers,
        path,
    })
}
pub(super) fn load(store: &WorkspaceStore) -> Result<(), String> {
    let value = store
        .preference("peek")
        .map_err(|e| e.to_string())?
        .and_then(|raw| decode(&raw))
        .unwrap_or_default();
    SETTINGS.with(|s| *s.borrow_mut() = value);
    Ok(())
}
pub(super) fn save(store: &WorkspaceStore, value: Settings) -> Result<(), String> {
    store
        .save_preference("peek", &encode(&value))
        .map_err(|e| e.to_string())?;
    SETTINGS.with(|s| *s.borrow_mut() = value);
    Ok(())
}
pub(super) fn modifier_bits(m: &Modifiers) -> u8 {
    u8::from(m.ctrl)
        | (u8::from(m.shift) << 1)
        | (u8::from(m.alt) << 2)
        | (u8::from(m.windows) << 3)
}
pub(super) fn valid_shortcut(key: u16, bits: u8) -> bool {
    if bits > 7
        || !(key == VK_SPACE
            || (0x30..=0x39).contains(&key)
            || (0x41..=0x5a).contains(&key)
            || (VK_F1..=VK_F24).contains(&key))
    {
        return false;
    }
    if bits & 4 != 0 && matches!(key, VK_F4 | VK_SPACE) {
        return false;
    }
    let mods = Modifiers {
        ctrl: bits & 1 != 0,
        shift: bits & 2 != 0,
        alt: bits & 4 != 0,
        windows: false,
    };
    keyboard::command(key, &mods, false).is_none()
}
pub(super) fn matches(key: u16, mods: &Modifiers, repeat: bool) -> bool {
    let s = settings();
    s.enabled && !repeat && key == s.key && modifier_bits(mods) == s.modifiers
}
pub(super) fn shortcut_label(s: &Settings) -> String {
    let mut parts = Vec::new();
    if s.modifiers & 1 != 0 {
        parts.push("Ctrl".to_owned());
    }
    if s.modifiers & 2 != 0 {
        parts.push("Shift".to_owned());
    }
    if s.modifiers & 4 != 0 {
        parts.push("Alt".to_owned());
    }
    parts.push(match s.key {
        VK_SPACE => "Space".into(),
        VK_F1..=VK_F24 => format!("F{}", s.key - VK_F1 + 1),
        key => char::from_u32(u32::from(key)).unwrap_or('?').to_string(),
    });
    parts.join(" + ")
}
pub(super) fn detect() -> Option<PathBuf> {
    for (var, base) in [
        ("ProgramFiles", "PowerToys"),
        ("LOCALAPPDATA", "PowerToys"),
        ("LOCALAPPDATA", "Programs\\PowerToys"),
    ] {
        if let Some(root) = std::env::var_os(var) {
            for relative in ["WinUI3Apps\\PowerToys.Peek.UI.exe", "PowerToys.Peek.UI.exe"] {
                let path = PathBuf::from(&root).join(base).join(relative);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}
pub(super) fn resolved(s: &Settings) -> Option<PathBuf> {
    if s.path.is_empty() {
        detect()
    } else {
        Some(PathBuf::from(&s.path))
    }
}
pub(super) fn browse(owner: isize) -> Result<Option<String>, String> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut file = [0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as _,
        lpstrFilter: windows_sys::w!("PowerToys Peek\0PowerToys.Peek.UI.exe\0\0"),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&raw mut dialog) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(format!("无法选择 Peek 程序：{error}"))
        };
    }
    let path =
        String::from_utf16_lossy(&file[..file.iter().position(|c| *c == 0).unwrap_or(file.len())]);
    if !PathBuf::from(&path)
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("PowerToys.Peek.UI.exe"))
    {
        return Err("请选择 PowerToys.Peek.UI.exe".into());
    }
    Ok(Some(path))
}

struct Signal(windows_sys::Win32::Foundation::HANDLE);
impl Drop for Signal {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
fn ensure_running(path: &std::path::Path) -> Result<Vec<Signal>, String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{ProcessStatus::EnumProcesses, Threading::*},
    };
    let mut ids = vec![0u32; 65536];
    let mut bytes = 0;
    if unsafe { EnumProcesses(ids.as_mut_ptr(), (ids.len() * 4) as u32, &raw mut bytes) } == 0 {
        return Err("无法检查 Peek 进程".into());
    }
    for id in &ids[..bytes as usize / 4] {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, *id);
            if process.is_null() {
                continue;
            }
            let mut name = [0u16; 32768];
            let mut length = name.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &raw mut length);
            CloseHandle(process);
            if ok != 0
                && String::from_utf16_lossy(&name[..length as usize])
                    .eq_ignore_ascii_case(&path.to_string_lossy())
            {
                return Ok(Vec::new());
            }
        }
    }
    // Keep auto-reset events alive until the newly started listener consumes the request.
    let mut signals = Vec::new();
    for name in [
        windows_sys::w!("Local\\ShowPeekEvent"),
        windows_sys::w!("Local\\TerminatePeekEvent"),
    ] {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, name) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        signals.push(Signal(handle));
    }
    std::process::Command::new(path)
        .arg(std::process::id().to_string())
        .spawn()
        .map_err(|e| format!("无法启动 Peek：{e}"))?;
    Ok(signals)
}

pub(super) fn open(owner: isize, identity: &ShellIdentity) -> Result<(), String> {
    let s = settings();
    if !s.enabled {
        return Ok(());
    }
    let path = resolved(&s)
        .filter(|path| path.is_file())
        .ok_or("未找到 Peek，请在设置中选择 PowerToys.Peek.UI.exe")?;
    let _signals = ensure_running(&path)?;
    desktop_shell::peek_desktop_item(windows::Win32::Foundation::HWND(owner as _), identity)
        .map_err(|error| format!("无法打开 PowerToys Peek：{error}"))
}

pub(super) fn open_path(identity: &ShellIdentity) -> Result<(), String> {
    let value = settings();
    if !value.enabled {
        return Ok(());
    }
    let executable = resolved(&value)
        .filter(|p| p.is_file())
        .ok_or("未找到 Peek，请在设置中选择程序路径")?;
    let path = identity.file_system_path().ok_or("此项目没有文件路径")?;
    std::process::Command::new(executable)
        .arg(path)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_and_shortcut_conflicts() {
        let store = WorkspaceStore::open_in_memory().unwrap();
        let value = Settings {
            enabled: false,
            path: "C:\\应用\\PowerToys.Peek.UI.exe".into(),
            key: 0x50,
            modifiers: 3,
        };
        save(&store, value.clone()).unwrap();
        load(&store).unwrap();
        assert_eq!(settings(), value);
        assert!(!matches(
            0x50,
            &Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
            false
        ));
        assert!(valid_shortcut(VK_SPACE, 0));
        assert!(!valid_shortcut(VK_SPACE, 1));
        assert!(!valid_shortcut(0x43, 1));
        assert!(!valid_shortcut(VK_F4, 4));
        assert!(decode("broken").is_none());
        save(&store, Settings::default()).unwrap();
        assert!(matches(VK_SPACE, &Modifiers::default(), false));
        assert!(!matches(VK_SPACE, &Modifiers::default(), true));
    }
}
