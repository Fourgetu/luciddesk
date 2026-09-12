use desktop_storage::WorkspaceStore;
use std::{
    path::PathBuf,
    sync::{LazyLock, RwLock},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Settings {
    pub path: String,
    pub auto_start: bool,
}
static SETTINGS: LazyLock<RwLock<Settings>> = LazyLock::new(|| RwLock::new(Settings::default()));
pub(super) fn settings() -> Settings {
    SETTINGS.read().unwrap().clone()
}
// Older workspaces used the presence of a search panel as the enabled state.
pub(super) fn enabled(store: &WorkspaceStore) -> Result<bool, String> {
    Ok(store
        .preference("search_enabled")
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some("0"))
}
pub(super) fn set_enabled(store: &WorkspaceStore, enabled: bool) -> Result<(), String> {
    store
        .save_preference("search_enabled", if enabled { "1" } else { "0" })
        .map_err(|e| e.to_string())
}
fn decode(raw: &str) -> Option<Settings> {
    let (flag, path) = raw.split_once('\n')?;
    Some(Settings {
        path: path.into(),
        auto_start: match flag {
            "0" => false,
            "1" => true,
            _ => return None,
        },
    })
}
pub(super) fn load(store: &WorkspaceStore) -> Result<(), String> {
    *SETTINGS.write().unwrap() = store
        .preference("everything")
        .map_err(|e| e.to_string())?
        .and_then(|raw| decode(&raw))
        .unwrap_or_default();
    Ok(())
}
pub(super) fn save(store: &WorkspaceStore, value: Settings) -> Result<(), String> {
    store
        .save_preference(
            "everything",
            &format!("{}\n{}", u8::from(value.auto_start), value.path),
        )
        .map_err(|e| e.to_string())?;
    *SETTINGS.write().unwrap() = value;
    Ok(())
}
pub(super) fn detect() -> Option<PathBuf> {
    for (variable, base) in [
        ("ProgramFiles", "Everything"),
        ("ProgramFiles(x86)", "Everything"),
        ("LOCALAPPDATA", "Programs\\Everything"),
    ] {
        if let Some(root) = std::env::var_os(variable) {
            let path = PathBuf::from(root).join(base).join("Everything.exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("Everything.exe")))
        .filter(|p| p.is_file())
}
pub(super) fn resolved(value: &Settings) -> Option<PathBuf> {
    if value.path.is_empty() {
        detect()
    } else {
        Some(PathBuf::from(&value.path))
    }
}
pub(super) fn launch() -> Result<(), String> {
    let path = resolved(&settings())
        .filter(|p| p.is_file())
        .ok_or("未找到 Everything，请在设置中选择 Everything.exe")?;
    std::process::Command::new(path)
        .arg("-startup")
        .spawn()
        .map_err(|e| format!("无法启动 Everything：{e}"))?;
    Ok(())
}
pub(super) fn browse(owner: isize) -> Result<Option<String>, String> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut file = [0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as _,
        lpstrFilter: windows_sys::w!("Everything\0Everything.exe\0\0"),
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
            Err(format!("无法选择 Everything：{error}"))
        };
    }
    let path =
        String::from_utf16_lossy(&file[..file.iter().position(|c| *c == 0).unwrap_or(file.len())]);
    if !PathBuf::from(&path)
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("Everything.exe"))
    {
        return Err("请选择 Everything.exe".into());
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_preserve_unicode_paths_and_reject_invalid_flags() {
        assert_eq!(
            decode("1\nD:\\应用\\Everything.exe"),
            Some(Settings {
                auto_start: true,
                path: "D:\\应用\\Everything.exe".into()
            })
        );
        assert_eq!(decode("0\n"), Some(Settings::default()));
        assert_eq!(decode("2\nx"), None);
        assert_eq!(decode("bad"), None);
    }
}
