//! User-visible configuration snapshots; source files are never part of a backup.
use super::*;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn name(label: &str) -> String {
    format!(
        "{label}-{}.db",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

pub(super) fn directory(s: &PaneApp) -> Result<PathBuf, String> {
    s.runtime
        .as_ref()
        .and_then(|r| r.path.parent())
        .map(|p| p.join("backups"))
        .ok_or_else(|| "配置目录不可用".into())
}

pub(super) fn snapshot(s: &PaneApp, label: &str) -> Result<PathBuf, String> {
    let directory = directory(s)?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(name(label));
    s.store.export_backup(&path).map_err(|e| e.to_string())?;
    if label == "auto" {
        let mut entries: Vec<_> = std::fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().is_some_and(|e| e == "db")
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("auto-"))
            })
            .collect();
        entries.sort();
        let excess = entries.len().saturating_sub(10);
        for old in entries.into_iter().take(excess) {
            std::fs::remove_file(old).map_err(|e| e.to_string())?;
        }
    }
    Ok(path)
}

fn choose(owner: isize, export: bool) -> Result<Option<PathBuf>, String> {
    use windows::Win32::{System::Com::*, UI::Shell::*};
    unsafe {
        let dialog: IFileDialog = if export {
            let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            windows::core::Interface::cast(&d).map_err(|e| e.to_string())?
        } else {
            let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            windows::core::Interface::cast(&d).map_err(|e| e.to_string())?
        };
        dialog
            .SetOptions(
                FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR
                    | if export {
                        FOS_OVERWRITEPROMPT
                    } else {
                        FOS_FILEMUSTEXIST
                    },
            )
            .map_err(|e| e.to_string())?;
        dialog
            .SetTitle(if export {
                windows::core::w!("导出 LucidPane 配置")
            } else {
                windows::core::w!("恢复 LucidPane 配置")
            })
            .map_err(|e| e.to_string())?;
        dialog
            .SetFileTypes(&[Common::COMDLG_FILTERSPEC {
                pszName: windows::core::w!("LucidPane 配置 (*.db)"),
                pszSpec: windows::core::w!("*.db"),
            }])
            .map_err(|e| e.to_string())?;
        dialog
            .SetDefaultExtension(windows::core::w!("db"))
            .map_err(|e| e.to_string())?;
        if export {
            dialog
                .SetFileName(&windows::core::HSTRING::from(name("LucidPane")))
                .map_err(|e| e.to_string())?;
        }
        if let Err(e) = dialog.Show(Some(windows::Win32::Foundation::HWND(owner as _))) {
            if e.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(e.to_string());
        }
        let item = dialog.GetResult().map_err(|e| e.to_string())?;
        let raw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(raw.as_wide()));
        CoTaskMemFree(Some(raw.0.cast()));
        Ok(Some(path))
    }
}

pub(super) fn request(state: &Rc<RefCell<PaneApp>>, event: &Event) {
    let event = event.clone();
    let weak = Rc::downgrade(state);
    window::defer_action(move || {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let owner = state
                .borrow()
                .settings
                .as_ref()
                .map_or(0, |w| w.hwnd() as isize);
            if matches!(event, Event::OpenBackups) {
                let path = directory(&state.borrow())?;
                std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                return open_shell_identity(owner, &folder::identity(path))
                    .map_err(|e| e.to_string());
            }
            let export = matches!(event, Event::ExportBackup);
            let Some(path) = choose(owner, export)? else {
                return Ok(());
            };
            if export {
                state
                    .borrow()
                    .store
                    .export_backup(&path)
                    .map_err(|e| e.to_string())?;
            } else {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                let confirmed = unsafe {
                    MessageBoxW(
                        owner as _,
                        windows_sys::w!(
                            "恢复将替换当前布局和设置，不改动文件。当前配置会先自动备份。是否继续？"
                        ),
                        windows_sys::w!("恢复配置"),
                        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                    )
                };
                if confirmed != IDYES {
                    return Ok(());
                }
                snapshot(&state.borrow(), "before-restore")?;
                state
                    .borrow_mut()
                    .store
                    .restore_backup(&path)
                    .map_err(|e| e.to_string())?;
                runtime::reload(&state)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            window::error(&e);
        }
    });
}
