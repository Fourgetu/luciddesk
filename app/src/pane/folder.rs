//! Live folder sources are separate from Explorer desktop membership.
use super::*;
use std::path::PathBuf;
#[cfg(test)]
use std::time::{Duration, Instant};
use windows::Win32::{
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    UI::Shell::*,
};

pub(super) struct Source {
    pub path: PathBuf,
    root: PathBuf,
    history: Vec<PathBuf>,
    pub sort: (u8, bool),
    request: Arc<Commands>,
    updates: mpsc::Receiver<Result<Vec<Item>, String>>,
    pub items: Vec<Item>,
    pub status: Option<String>,
    pub loading: bool,
}

struct Commands {
    event: isize,
    stop: std::sync::atomic::AtomicBool,
}
impl Commands {
    fn new() -> Result<Self, String> {
        let event = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                std::ptr::null(),
                0,
                0,
                std::ptr::null(),
            )
        };
        if event.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self {
            event: event as isize,
            stop: false.into(),
        })
    }
    fn signal(&self) {
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(self.event as _);
        }
    }
}
impl Drop for Commands {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.event as _);
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.request
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        self.request.signal();
    }
}

struct Watch(windows_sys::Win32::Foundation::HANDLE);

fn wait_for_change(handles: &[windows_sys::Win32::Foundation::HANDLE], timeout: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let started = std::time::Instant::now();
    loop {
        let remaining = if timeout == u32::MAX {
            timeout
        } else {
            timeout.saturating_sub(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
        };
        let result = unsafe {
            MsgWaitForMultipleObjectsEx(
                handles.len() as u32,
                handles.as_ptr(),
                remaining,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if result != handles.len() as u32 {
            return result;
        }
        // Shell can create hidden windows on this STA. Keep servicing their
        // messages while waiting indefinitely for directory or shutdown events.
        unsafe {
            let mut msg = windows_sys::Win32::UI::WindowsAndMessaging::MSG::default();
            for _ in 0..64 {
                if PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
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
}
impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0);
        }
    }
}

impl Source {
    fn start(path: PathBuf, wake: wake::Wake) -> Result<Self, String> {
        let request = Arc::new(Commands::new()?);
        let commands = Arc::clone(&request);
        let (sender, updates) = mpsc::channel();
        let root = path.clone();
        std::thread::Builder::new()
            .name("folder-pane".into())
            .spawn(move || {
                let _apartment = match ShellApartment::initialize_sta() {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = sender.send(Err(error.to_string()));
                        wake.notify();
                        return;
                    }
                };
                let mut watch = Watch::new(&root);
                let mut cache: HashMap<String, (Option<std::time::SystemTime>, Item)> =
                    HashMap::new();
                loop {
                    if commands.stop.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    {
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
                                            details: ItemDetails {
                                                kind: file_type(&entry.identity),
                                                modified: modified_text(entry.modified),
                                                folder: entry.attributes.folder,
                                                modified_time: entry.modified,
                                            },
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
                        wake.notify();
                    }
                    use windows_sys::Win32::System::Threading::*;
                    let handles = [
                        commands.event as _,
                        watch.as_ref().map_or(std::ptr::null_mut(), |w| w.0),
                    ];
                    let result = wait_for_change(
                        &handles[..if watch.is_some() { 2 } else { 1 }],
                        if watch.is_some() { INFINITE } else { 2000 },
                    );
                    if commands.stop.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    if result == 1 {
                        if unsafe {
                            windows_sys::Win32::Storage::FileSystem::FindNextChangeNotification(
                                handles[1],
                            )
                        } == 0
                        {
                            watch = None;
                        }
                        // Merge a burst without indefinitely postponing a visible update.
                        unsafe {
                            WaitForSingleObject(commands.event as _, 100);
                        }
                    } else if result == u32::MAX {
                        let _ = sender.send(Err(std::io::Error::last_os_error().to_string()));
                        wake.notify();
                        break;
                    }
                    if watch.is_none() {
                        watch = Watch::new(&root);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            root: path.clone(),
            history: Vec::new(),
            sort: (0, false),
            path,
            request,
            updates,
            items: Vec::new(),
            status: None,
            loading: true,
        })
    }
    pub fn refresh(&self) {
        self.request.signal();
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
            .is_none_or(|source| source.root != path)
        {
            let mut source = Source::start(path, state.wake.clone())?;
            source.sort = state
                .store
                .preference(&format!("panel_folder_sort:{}", id.get()))
                .map_err(|e| e.to_string())?
                .and_then(|v| {
                    let (column, direction) = v.split_once(':')?;
                    Some((column.parse::<u8>().ok()?.min(2), direction == "desc"))
                })
                .unwrap_or((0, false));
            state.folders.insert(id, source);
        }
    } else {
        state.folders.remove(&id);
    }
    Ok(())
}

pub(super) fn poll(state: &mut PaneApp) {
    let mut changed = false;
    for source in state.folders.values_mut() {
        let mut latest = None;
        while let Ok(result) = source.updates.try_recv() {
            latest = Some(result);
        }
        // Each update is a complete snapshot. Sort and publish only the newest
        // one when the UI was busy while several scans completed.
        if let Some(result) = latest {
            source.loading = false;
            match result {
                Ok(items) => {
                    source.items = items;
                    sort_items(&mut source.items, source.sort);
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

fn sort_items(items: &mut Vec<Item>, sort: (u8, bool)) {
    let mut sorted: Vec<_> = std::mem::take(items)
        .into_iter()
        .map(|item| {
            // Reuse the worker's Shell snapshot; sorting must not touch disk
            // on the UI thread (especially for network folders).
            let folder = item.details.folder;
            let modified = item.details.modified_time;
            let name = item.label.to_lowercase();
            (folder, name, modified, item)
        })
        .collect();
    sorted.sort_by(|(af, an, at, a), (bf, bn, bt, b)| {
        bf.cmp(af).then_with(|| {
            let order = match sort.0 {
                1 => a.details.kind.cmp(&b.details.kind),
                2 => at.cmp(bt),
                _ => an.cmp(bn),
            };
            let order = order.then_with(|| an.cmp(bn));
            if sort.1 { order.reverse() } else { order }
        })
    });
    *items = sorted.into_iter().map(|(_, _, _, item)| item).collect();
}

pub(super) fn sort(state: &mut PaneApp, id: PanelId, column: u8) -> Result<(), String> {
    let Some(source) = state.folders.get_mut(&id) else {
        return Ok(());
    };
    let order = (column.min(2), source.sort.0 == column && !source.sort.1);
    state
        .store
        .save_preference(
            &format!("panel_folder_sort:{}", id.get()),
            &format!("{}:{}", order.0, if order.1 { "desc" } else { "asc" }),
        )
        .map_err(|e| e.to_string())?;
    source.sort = order;
    sort_items(&mut source.items, order);
    refresh_changed_views(state, true);
    Ok(())
}

pub(super) fn navigate(
    state: &mut PaneApp,
    id: PanelId,
    path: Option<PathBuf>,
) -> Result<(), String> {
    let Some(old) = state.folders.get(&id) else {
        return Ok(());
    };
    let mut history = old.history.clone();
    let path = if let Some(path) = path {
        if !path.is_dir() {
            return Err("文件夹不可访问".into());
        }
        history.push(old.path.clone());
        path
    } else {
        let Some(path) = history.pop() else {
            return Ok(());
        };
        path
    };
    let mut source = Source::start(path.clone(), state.wake.clone())?;
    source.root = old.root.clone();
    source.history = history;
    source.sort = old.sort;
    state.folders.insert(id, source);
    if let Some(view) = state.views.iter().find(|v| v.id == id) {
        let mut model = view.model.borrow_mut();
        model.folder = Some(path);
        model.clear_selection();
        model.scroll = 0;
    }
    refresh_changed_views(state, true);
    Ok(())
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

fn file_type(identity: &ShellIdentity) -> String {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_TYPENAME, SHGetFileInfoW};
    let path: Vec<_> = identity
        .activation_name()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut info = SHFILEINFOW::default();
    if unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            0,
            &raw mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_TYPENAME,
        )
    } == 0
    {
        return "—".into();
    }
    let name = info.szTypeName;
    String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())])
}

fn modified_text(value: Option<std::time::SystemTime>) -> String {
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx},
    };
    let Some(time) = value else {
        return "—".into();
    };
    let epoch = 116_444_736_000_000_000u128;
    let ticks = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => epoch.checked_add(duration.as_nanos() / 100),
        Err(error) => epoch.checked_sub(error.duration().as_nanos() / 100),
    }
    .and_then(|ticks| u64::try_from(ticks).ok());
    let Some(ticks) = ticks else {
        return "—".into();
    };
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    if unsafe { FileTimeToSystemTime(&file, &raw mut utc) } == 0
        || unsafe { SystemTimeToTzSpecificLocalTimeEx(std::ptr::null(), &utc, &raw mut local) } == 0
    {
        return "—".into();
    }
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
    )
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
    fn dropping_an_idle_source_wakes_worker_and_releases_handles() {
        let source = Source::start(
            std::env::temp_dir().join(format!(
                "lucidpane-missing-watch-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            Default::default(),
        )
        .unwrap();
        let request = Arc::downgrade(&source.request);
        assert!(
            source
                .updates
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
        drop(source);
        let deadline = Instant::now() + Duration::from_secs(1);
        while request.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            request.upgrade().is_none(),
            "shutdown must interrupt the directory retry wait"
        );
    }

    #[test]
    fn sorting_uses_snapshot_metadata_without_accessing_paths() {
        let make = |label: &str, folder, seconds: Option<u64>| Item {
            identity: ShellIdentity::Namespace {
                parsing_name: format!("test:{label}"),
            },
            label: label.into(),
            image: None,
            details: ItemDetails {
                folder,
                modified_time: seconds.map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s)),
                ..Default::default()
            },
        };
        let mut items = vec![
            make("z-folder", true, None),
            make("a-new", false, Some(20)),
            make("z-old", false, Some(10)),
        ];
        sort_items(&mut items, (2, false));
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["z-folder", "z-old", "a-new"]
        );
        sort_items(&mut items, (2, true));
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["z-folder", "a-new", "z-old"]
        );
        sort_items(&mut items, (0, false));
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["z-folder", "a-new", "z-old"]
        );
    }

    #[test]
    fn navigation_and_sort_keep_the_mapping_and_back_history() {
        let root = std::env::temp_dir().join(format!(
            "lucidpane-navigation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("child")).unwrap();
        std::fs::write(root.join("z.txt"), b"z").unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        let mut state = super::super::tests::test_state();
        let id = PanelId::new(2);
        state
            .workspace
            .panel_mut(id)
            .unwrap()
            .set_folder(Some(root.clone()));
        state.store.save_workspace(&state.workspace).unwrap();
        ensure(&mut state, id).unwrap();
        let items = state.folders[&id]
            .updates
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        state.folders.get_mut(&id).unwrap().items = items;
        sort(&mut state, id, 0).unwrap();
        assert_eq!(
            state.folders[&id]
                .items
                .iter()
                .map(|i| i
                    .identity
                    .file_system_path()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>(),
            ["child", "z.txt", "a.txt"]
        );
        sort(&mut state, id, 0).unwrap();
        assert_eq!(
            state.folders[&id]
                .items
                .iter()
                .map(|i| i
                    .identity
                    .file_system_path()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>(),
            ["child", "a.txt", "z.txt"]
        );
        navigate(&mut state, id, Some(root.join("child"))).unwrap();
        ensure(&mut state, id).unwrap();
        assert_eq!(state.folders[&id].path, root.join("child"));
        assert_eq!(
            state
                .store
                .load_workspace()
                .unwrap()
                .panel(id)
                .unwrap()
                .folder(),
            Some(root.as_path())
        );
        navigate(&mut state, id, None).unwrap();
        assert_eq!(state.folders[&id].path, root);
        state.folders.remove(&id);
        ensure(&mut state, id).unwrap();
        assert_eq!(state.folders[&id].sort, (0, false));
        state.folders.clear();
        std::fs::remove_file(root.join("a.txt")).unwrap();
        std::fs::remove_file(root.join("z.txt")).unwrap();
        std::fs::remove_dir(root.join("child")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

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
        let mut source = Source::start(root.clone(), Default::default()).unwrap();
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
        assert!(!first.details.kind.is_empty());
        assert_ne!(first.details.kind, "—");
        assert_eq!(first.details.modified.len(), 16);
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
