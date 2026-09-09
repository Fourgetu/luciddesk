use desktop_core::ShellIdentity;
use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, HWND},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::Shell::{FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName},
    },
    core::{HSTRING, Result},
};

/// Rename through Shell so namespace rules and Explorer change notifications apply.
/// Returns false if the user cancels a Shell conflict or permission dialog.
/// # Errors
/// Returns Shell errors for invalid names, unavailable items, or failed operations.
pub fn rename_shell_identity(owner: HWND, identity: &ShellIdentity, name: &str) -> Result<bool> {
    if name.trim().is_empty() || name.contains(['\0', '/', '\\']) {
        return Err(windows::core::Error::new(
            E_INVALIDARG,
            "名称不能为空或包含路径分隔符",
        ));
    }
    let parsing_name = match identity {
        ShellIdentity::FileSystem { path, .. } => path.to_string_lossy().into_owned(),
        ShellIdentity::Namespace { parsing_name } => parsing_name.clone(),
    };
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(parsing_name), None)?;
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(owner)?;
        operation.RenameItem(&item, &HSTRING::from(name), None)?;
        operation.PerformOperations()?;
        Ok(!operation.GetAnyOperationsAborted()?.as_bool())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_rename_keeps_the_same_file_identity() {
        let _apartment = crate::ShellApartment::initialize_sta().unwrap();
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lucidpane-shell-rename-{suffix}"));
        std::fs::create_dir(&root).unwrap();
        let before = root.join("旧名字.txt");
        let after = root.join("新名字.txt");
        std::fs::write(&before, b"rename fixture").unwrap();
        let file_id = crate::stable_file_identity(&before).unwrap();
        let identity = ShellIdentity::FileSystem {
            path: before.clone(),
            volume_id: Some(file_id.0),
            file_id: Some(file_id.1),
        };
        unsafe {
            use windows::{
                Win32::UI::{
                    Shell::{
                        BHID_SFUIObject, CMF_CANRENAME, CMF_ITEMMENU, GCS_VERBA, IContextMenu,
                    },
                    WindowsAndMessaging::{CreatePopupMenu, DestroyMenu},
                },
                core::PSTR,
            };
            let item: IShellItem = SHCreateItemFromParsingName(
                &HSTRING::from(before.to_string_lossy().as_ref()),
                None,
            )
            .unwrap();
            let context: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject).unwrap();
            let menu = CreatePopupMenu().unwrap();
            let populated =
                context.QueryContextMenu(menu, 0, 1, 0x7fff, CMF_ITEMMENU | CMF_CANRENAME);
            populated.ok().unwrap();
            let mut found = false;
            for command in 0..(populated.0 as u32 & 0xffff) {
                let mut verb = [0u8; 256];
                if context
                    .GetCommandString(
                        command as usize,
                        GCS_VERBA,
                        None,
                        PSTR(verb.as_mut_ptr()),
                        256,
                    )
                    .is_ok()
                {
                    found |= verb.starts_with(b"rename\0");
                }
            }
            DestroyMenu(menu).unwrap();
            assert!(found, "Shell menu did not expose the native rename verb");
        }
        assert!(rename_shell_identity(HWND::default(), &identity, "新名字.txt").unwrap());
        assert!(!before.exists());
        assert_eq!(std::fs::read(&after).unwrap(), b"rename fixture");
        assert_eq!(crate::stable_file_identity(&after).unwrap(), file_id);
        std::fs::remove_file(after).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
