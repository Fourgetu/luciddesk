//! Live folder sources are separate from Explorer desktop membership.
use super::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::Win32::{
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    UI::Shell::*,
};

pub(super) struct Source {
    path: PathBuf,
    request: mpsc::SyncSender<()>,
    updates: mpsc::Receiver<Result<Vec<Item>, String>>,
    pub items: Vec<Item>,
    pub status: Option<String>,
    pub loading: bool,
}

struct Watch(windows_sys::Win32::Foundation::HANDLE);
impl Watch {
    fn new(path: &Path) -> Option<Self> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            FindFirstChangeNotificationW(
                path.as_ptr(),
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE,
            )
        };
        (handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE).then_some(Self(handle))
    }
    fn changed(&self) -> bool {
        unsafe {
            if windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, 0) == 0 {
                windows_sys::Win32::Storage::FileSystem::FindNextChangeNotification(self.0);
                true
            } else {
                false
            }
        }
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0);
        }
    }
}

impl Source {
    fn start(path: PathBuf) -> Result<Self, String> {
        let (request, commands) = mpsc::sync_channel(1);
        let (sender, updates) = mpsc::channel();
        let root = path.clone();
        std::thread::Builder::new()
            .name("folder-pane".into())
            .spawn(move || {
                let _apartment = match ShellApartment::initialize_sta() {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                        return;
                    }
                };
                let mut watch = Watch::new(&root);
                let mut refresh = true;
                let mut last_scan = Instant::now();
                let mut cache: HashMap<String, (Option<std::time::SystemTime>, Item)> =
                    HashMap::new();
                loop {
                    if refresh {
                        let result = desktop_shell::enumerate_folder(&root)
                            .map(|entries| {
                                let mut next = HashMap::new();
                                let items = entries
                                    .into_iter()
                                    .map(|entry| {
                                        let key = entry.identity.persistent_key();
                                        let image = cache
                                            .get(&key)
                                            .filter(|(modified, item)| {
                                                *modified == entry.modified
                                                    && item.identity == entry.identity
                                            })
                                            .and_then(|(_, item)| item.image.clone())
                                            .or_else(|| {
                                                assets::load(&entry.identity, 128)
                                                    .ok()
                                                    .map(Arc::new)
                                            });
                                        let item = Item {
                                            identity: entry.identity,
                                            label: entry.display_name,
                                            image,
                                        };
                                        next.insert(key, (entry.modified, item.clone()));
                                        item
                                    })
                                    .collect();
                                cache = next;
                                items
                            })
                            .map_err(|error| {
                                format!("无法读取文件夹，请检查路径或访问权限。\n{error}")
                            });
                        if result.is_err() {
                            cache.clear();
                            watch = None;
                        }
                        if sender.send(result).is_err() {
                            break;
                        }
                        last_scan = Instant::now();
                    }
                    refresh = match commands.recv_timeout(Duration::from_millis(200)) {
                        Ok(()) => true,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                    };
                    if watch.as_ref().is_some_and(Watch::changed) {
                        refresh = true;
                    }
                    if watch.is_none() && last_scan.elapsed() >= Duration::from_secs(2) {
                        watch = Watch::new(&root);
                        refresh = true;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            request,
            updates,
            items: Vec::new(),
            status: None,
            loading: true,
        })
    }
    pub fn refresh(&self) {
        let _ = self.request.try_send(());
    }
}

pub(super) fn ensure(state: &mut PaneApp, id: PanelId) -> Result<(), String> {
    let path = state
        .workspace
        .panel(id)
        .and_then(Panel::folder)
        .map(Path::to_path_buf);
    if let Some(path) = path {
        if state
            .folders
            .get(&id)
            .is_none_or(|source| source.path != path)
        {
            state.folders.insert(id, Source::start(path)?);
        }
    } else {
        state.folders.remove(&id);
    }
    Ok(())
}

pub(super) fn poll(state: &mut PaneApp) {
    let mut changed = false;
    for source in state.folders.values_mut() {
        while let Ok(result) = source.updates.try_recv() {
            source.loading = false;
            match result {
                Ok(items) => {
                    source.items = items;
                    source.status = None;
                }
                Err(error) => {
                    source.items.clear();
                    source.status = Some(error);
                }
            }
            changed = true;
        }
    }
    if changed {
        refresh_views(state);
    }
}

pub(super) fn choose(owner: isize) -> Result<Option<PathBuf>, String> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        dialog
            .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR)
            .map_err(|e| e.to_string())?;
        dialog
            .SetTitle(windows::core::w!("选择要映射的文件夹"))
            .map_err(|e| e.to_string())?;
        if let Err(error) = dialog.Show(Some(windows::Win32::Foundation::HWND(owner as _))) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(error.to_string());
        }
        let item = dialog.GetResult().map_err(|e| e.to_string())?;
        let raw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(raw.as_wide()));
        windows::Win32::System::Com::CoTaskMemFree(Some(raw.0.cast()));
        Ok(Some(path))
    }
}

pub(super) fn identity(path: PathBuf) -> ShellIdentity {
    ShellIdentity::FileSystem {
        path,
        volume_id: None,
        file_id: None,
    }
}

pub(super) fn accepts_copy(items: &[ShellIdentity], destination: &Path) -> bool {
    let destination = destination.to_string_lossy().to_lowercase();
    !items.is_empty()
        && items.iter().all(|item| {
            item.file_system_path().is_some_and(|path| {
                let source = path.to_string_lossy().to_lowercase();
                path.parent()
                    .is_some_and(|parent| parent.to_string_lossy().to_lowercase() != destination)
                    && source != destination
                    && !destination.starts_with(&format!("{source}\\"))
            })
        })
}

pub(super) fn request_picker(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    change: bool,
) -> Result<(), String> {
    let owner = state
        .borrow()
        .views
        .iter()
        .find(|v| v.id == id)
        .map_or(0, |v| v.window.hwnd() as isize);
    let weak = Rc::downgrade(state);
    if !window::defer_action(move || {
        let result = (|| -> Result<(), String> {
            let Some(path) = choose(owner)? else {
                return Ok(());
            };
            let Some(state) = weak.upgrade() else {
                return Ok(());
            };
            handle(
                &state,
                id,
                if change {
                    Event::SetFolder(path)
                } else {
                    Event::MapFolder(path)
                },
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            window::error(&error);
        }
    }) {
        return Err("无法打开文件夹选择器".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_watch_tracks_children_and_recovers_after_missing_directory() {
        let root = std::env::temp_dir().join(format!(
            "lucidpane-folder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut source = Source::start(root.clone()).unwrap();
        let wait = |source: &mut Source, predicate: &dyn Fn(&Result<Vec<Item>, String>) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let result = source
                    .updates
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("folder update timed out");
                if predicate(&result) {
                    return result;
                }
            }
        };
        assert!(
            wait(&mut source, &|result| result
                .as_ref()
                .is_ok_and(Vec::is_empty))
            .is_ok()
        );
        std::fs::write(root.join("first.txt"), b"original").unwrap();
        std::fs::create_dir(root.join("child")).unwrap();
        std::fs::write(root.join("child/nested.txt"), b"nested").unwrap();
        let items = wait(&mut source, &|result| {
            result.as_ref().is_ok_and(|items| items.len() == 2)
        })
        .unwrap();
        assert!(
            items
                .iter()
                .all(|item| item.identity.file_system_path().unwrap().parent()
                    == Some(root.as_path()))
        );
        let first = items
            .iter()
            .find(|item| item.identity.file_system_path() == Some(root.join("first.txt").as_path()))
            .unwrap();
        assert!(first.image.is_some());
        std::fs::rename(root.join("first.txt"), root.join("renamed.txt")).unwrap();
        wait(&mut source, &|result| {
            result.as_ref().is_ok_and(|items| {
                items.iter().any(|item| {
                    item.identity.file_system_path() == Some(root.join("renamed.txt").as_path())
                })
            })
        })
        .unwrap();
        std::fs::remove_file(root.join("renamed.txt")).unwrap();
        wait(&mut source, &|result| {
            result.as_ref().is_ok_and(|items| items.len() == 1)
        })
        .unwrap();
        std::fs::remove_file(root.join("child/nested.txt")).unwrap();
        std::fs::remove_dir(root.join("child")).unwrap();
        std::fs::remove_dir(&root).unwrap();
        source.refresh();
        assert!(wait(&mut source, &Result::is_err).is_err());
        std::fs::create_dir(&root).unwrap();
        wait(&mut source, &|result| {
            result.as_ref().is_ok_and(Vec::is_empty)
        })
        .unwrap();
        drop(source);
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn folder_copy_rejects_self_recursive_and_virtual_sources() {
        let destination = Path::new(r"C:\Data\Folder");
        assert!(accepts_copy(
            &[identity(PathBuf::from(r"C:\Other\file.txt"))],
            destination
        ));
        for path in [r"C:\Data", r"C:\Data\Folder", r"C:\Data\Folder\file.txt"] {
            assert!(!accepts_copy(&[identity(PathBuf::from(path))], destination));
        }
        assert!(!accepts_copy(
            &[ShellIdentity::Namespace {
                parsing_name: "recycle".into()
            }],
            destination
        ));
    }
}
