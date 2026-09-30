//! Only MSIX packages opt into per-package desktop DLL deployment.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER},
    Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash},
    Storage::{
        FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ},
        Packaging::Appx::GetCurrentPackageFullName,
    },
};

const DLL: &str = "luciddesk_desktop.dll";

pub(crate) struct Component {
    pub path: PathBuf,
    // Prevent replacement/deletion between validation and LoadLibraryEx.
    _file: Option<File>,
}

pub(crate) fn prepare() -> Result<Component, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    resolve(&exe, local_state)
}

fn resolve(
    exe: &Path,
    package_data: impl FnOnce() -> Result<Option<PathBuf>, String>,
) -> Result<Component, String> {
    let source = exe.with_file_name(DLL);
    let marker = exe.with_file_name("msix");
    match fs::symlink_metadata(&marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Component {
                path: source,
                _file: None,
            });
        }
        Err(e) => return Err(format!("无法读取 MSIX 标记: {e}")),
        Ok(m) if !m.is_file() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
            return Err("msix 必须是普通文件".into());
        }
        Ok(_) => {}
    }
    let Some(data) = package_data()? else {
        return Ok(Component {
            path: source,
            _file: None,
        });
    };
    let root = data.join("DesktopComponent");
    let payload = fs::read(source).map_err(|e| format!("无法读取包内桌面组件: {e}"))?;
    deploy(&root, &payload)
}

fn local_state() -> Result<Option<PathBuf>, String> {
    let mut length = 0;
    let code = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    if code == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(None);
    }
    if code != ERROR_INSUFFICIENT_BUFFER {
        return Err(format!("无法查询 MSIX 包身份: {code}"));
    }
    let folder = windows::Storage::ApplicationData::Current()
        .and_then(|data| data.LocalFolder())
        .and_then(|folder| folder.Path())
        .map_err(|e| format!("无法获取 MSIX LocalState: {e}"))?;
    Ok(Some(PathBuf::from(folder.to_os_string())))
}

fn digest(payload: &[u8]) -> Result<String, String> {
    let size = u32::try_from(payload.len()).map_err(|_| "桌面组件过大")?;
    let mut hash = [0u8; 32];
    let status = unsafe {
        BCryptHash(
            BCRYPT_SHA256_ALG_HANDLE,
            std::ptr::null(),
            0,
            payload.as_ptr(),
            size,
            hash.as_mut_ptr(),
            hash.len() as u32,
        )
    };
    if status < 0 {
        return Err(format!("计算桌面组件 SHA256 失败: {status:#x}"));
    }
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect())
}

fn ordinary(path: &Path, directory: bool) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        })
    {
        return Err(std::io::Error::other(
            "桌面组件缓存路径类型异常或包含重解析点",
        ));
    }
    Ok(())
}

fn checked_file(path: &Path, payload: &[u8]) -> std::io::Result<File> {
    ordinary(path, false)?;
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(payload.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes != payload {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "桌面组件缓存内容不匹配",
        ));
    }
    Ok(file)
}

fn deploy(root: &Path, payload: &[u8]) -> Result<Component, String> {
    let hash = digest(payload)?;
    let run = || -> std::io::Result<Component> {
        fs::create_dir_all(root)?;
        ordinary(root, true)?;
        let lock_path = root.join("deploy.lock");
        if lock_path.try_exists()? {
            ordinary(&lock_path, false)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock()
            .map_err(|e| std::io::Error::other(format!("桌面组件部署正在进行或无法锁定: {e}")))?;
        let dir = root.join(&hash);
        fs::create_dir_all(&dir)?;
        ordinary(&dir, true)?;
        let path = dir.join(DLL);
        let file = match checked_file(&path, payload) {
            Ok(file) => file,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
                ) =>
            {
                let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
                temp.write_all(payload)?;
                temp.as_file().sync_all()?;
                temp.persist(&path).map_err(|e| e.error)?;
                checked_file(&path, payload)?
            }
            Err(e) => return Err(e),
        };
        cleanup(root, &hash);
        Ok(Component {
            path,
            _file: Some(file),
        })
    };
    run().map_err(|e| format!("无法部署 MSIX 桌面组件: {e}"))
}

// Best effort: Windows refuses deletion of loaded images or guarded candidates.
// Never recurse into unknown entries or follow reparse points.
fn cleanup(root: &Path, current: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name == current || name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let dir = entry.path();
        if ordinary(&dir, true).is_err() {
            continue;
        }
        let dll = dir.join(DLL);
        if ordinary(&dll, false).is_ok() {
            let _ = fs::remove_file(dll);
        }
        let _ = fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_build_never_queries_package_data() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve(&dir.path().join("luciddesk.exe"), || {
            panic!("ordinary app queried package data")
        })
        .unwrap();
        assert_eq!(result.path, dir.path().join(DLL));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn marker_without_package_uses_adjacent_dll_without_cache() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("msix"), []).unwrap();
        let result = resolve(&dir.path().join("luciddesk.exe"), || Ok(None)).unwrap();
        assert_eq!(result.path, dir.path().join(DLL));
        assert!(result._file.is_none());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn package_query_failure_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("msix"), []).unwrap();
        let result = resolve(&dir.path().join("luciddesk.exe"), || {
            Err("query failed".into())
        });
        assert_eq!(result.err().unwrap(), "query failed");
    }

    #[test]
    fn hash_matches_sha256_vector() {
        assert_eq!(
            digest(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn repairs_corruption_and_reuses_matching_payload() {
        let dir = tempfile::tempdir().unwrap();
        let first = deploy(dir.path(), b"first").unwrap();
        let path = first.path.clone();
        drop(first);
        fs::write(&path, b"broken").unwrap();
        let repaired = deploy(dir.path(), b"first").unwrap();
        assert_eq!(fs::read(&repaired.path).unwrap(), b"first");
        let reused = deploy(dir.path(), b"first").unwrap();
        assert_eq!(reused.path, repaired.path);
    }

    #[test]
    fn upgrade_retains_in_use_version_then_cleans_it_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let old = deploy(dir.path(), b"old").unwrap();
        let old_path = old.path.clone();
        let new = deploy(dir.path(), b"new").unwrap();
        assert_ne!(new.path, old.path);
        assert!(old.path.exists());
        assert!(fs::write(&old.path, b"tamper").is_err());
        drop(old);
        let _again = deploy(dir.path(), b"new").unwrap();
        assert!(!old_path.exists());
        assert!(new.path.exists());
    }

    #[test]
    fn concurrent_deployment_reports_busy_and_preserves_unknown_files() {
        let dir = tempfile::tempdir().unwrap();
        let lock = File::create(dir.path().join("deploy.lock")).unwrap();
        lock.lock().unwrap();
        assert!(deploy(dir.path(), b"x").is_err());
        drop(lock);
        fs::create_dir(dir.path().join("unrelated")).unwrap();
        fs::write(dir.path().join("unrelated/keep.txt"), b"keep").unwrap();
        let _component = deploy(dir.path(), b"x").unwrap();
        assert!(dir.path().join("unrelated/keep.txt").exists());
    }
}
