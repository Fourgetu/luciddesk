//! Live folder sources are separate from Explorer desktop membership.
use super::*;
use std::path::PathBuf;
use std::sync::Mutex;
pub(super) mod entry_mode;
mod images;
mod preferences;
pub(super) use entry_mode::EntryMode;
pub(super) use preferences::{Defaults, save_columns, saved_columns, toggle_column, visible_columns};
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
    cache: images::SharedCache,
    updates: mpsc::Receiver<Result<Vec<Item>, String>>,
    images: mpsc::Receiver<Vec<Item>>,
    pub items: Vec<Item>,
    pub status: Option<String>,
    pub loading: bool,
}

struct Commands {
    event: isize,
    stop: std::sync::atomic::AtomicBool,
    active: std::sync::atomic::AtomicBool,
    images_pending: std::sync::atomic::AtomicBool,
    priority: Mutex<Vec<String>>,
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
            active: true.into(),
            images_pending: false.into(),
            priority: Mutex::new(Vec::new()),
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
    pub(super) fn set_active(&self, active: bool) {
        self.request.active.store(active, std::sync::atomic::Ordering::Release);
        self.request.signal();
    }
    pub(super) fn navigation(&self) -> [bool; 2] {
        [!self.history.is_empty(), self.path != self.root]
    }

    #[cfg(test)]
    fn start(path: PathBuf, wake: wake::Wake) -> Result<Self, String> {
        Self::start_with(path, wake, Arc::default(), (0, false))
    }

    fn start_with(path: PathBuf, wake: wake::Wake, cache: images::SharedCache, sort: (u8, bool)) -> Result<Self, String> {
        let request = Arc::new(Commands::new()?);
        let commands = Arc::clone(&request);
        let (sender, updates) = mpsc::sync_channel(1);
        let (image_sender, image_updates) = mpsc::sync_channel(2);
        let worker_cache = Arc::clone(&cache);
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
                let cache = worker_cache;
                loop {
                    if commands.stop.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    if commands.active.load(std::sync::atomic::Ordering::Acquire) {
                        let mut jobs = Vec::new();
                        let result = desktop_shell::enumerate_folder(&root)
                            .map(|entries| {
                                let mut cache = cache.lock().unwrap();
                                let mut items: Vec<Item> = entries.into_iter().map(|entry| {
                                    let mut item = Item {
                                        details: ItemDetails {
                                            modified: modified_text(entry.modified),
                                            folder: entry.attributes.folder,
                                            modified_time: entry.modified,
                                            size: entry.size,
                                            ..Default::default()
                                        },
                                        identity: entry.identity,
                                        label: entry.display_name,
                                        image: None,
                                    };
                                    if !cache.restore(&mut item) { jobs.push(item.clone()); }
                                    item
                                }).collect();
                                cache.retain_folder(&root, &items.iter().map(|item| item.identity.persistent_key()).collect());
                                drop(cache);
                                sort_items(&mut items, sort);
                                sort_items(&mut jobs, sort);
                                items
                            })
                            .map_err(|error| {
                                format!("无法读取文件夹，请检查路径或访问权限。\n{error}")
                            });
                        if result.is_err() {
                            cache.lock().unwrap().retain_folder(&root, &Default::default());
                            watch = None;
                        }
                        commands.images_pending.store(!jobs.is_empty(), std::sync::atomic::Ordering::Release);
                        if sender.send(result.clone()).is_err() {
                            break;
                        }
                        wake.notify();
                        // Send only completed image/type changes while loading. A final
                        // snapshot also supports consumers waiting for a complete scan.
                        if let Ok(mut items) = result {
                            if !jobs.is_empty() {
                                images::enrich(&mut items, jobs, &commands, &cache, &image_sender, &wake);
                                commands.images_pending.store(false, std::sync::atomic::Ordering::Release);
                                if commands.stop.load(std::sync::atomic::Ordering::Acquire) { return; }
                                if sender.send(Ok(items)).is_err() { return; }
                                wake.notify();
                            }
                        }
                    }
                    use windows_sys::Win32::System::Threading::*;
                    let handles = [
                        commands.event as _,
                        watch.as_ref().map_or(std::ptr::null_mut(), |w| w.0),
                    ];
                    let result = wait_for_change(
                        &handles[..if watch.is_some() { 2 } else { 1 }],
                        if watch.is_some() || !commands.active.load(std::sync::atomic::Ordering::Acquire) { INFINITE } else { 2000 },
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
            sort,
            path,
            request,
            cache,
            updates,
            images: image_updates,
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
            let sort = state
                .store
                .preference(&format!("panel_folder_sort:{}", id.get()))
                .map_err(|e| e.to_string())?
                .and_then(|v| {
                    let (column, direction) = v.split_once(':')?;
                    Some((column.parse::<u8>().ok()?.min(3), direction == "desc"))
                })
                .unwrap_or((0, false));
            let source = Source::start_with(path, state.wake.clone(), Arc::default(), sort)?;
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
        let mut patches = HashMap::new();
        while let Ok(batch) = source.images.try_recv() {
            for item in batch { patches.insert(item.identity.persistent_key(), item); }
        }
        if !patches.is_empty() {
            let mut patched = false;
            for item in &mut source.items {
                if let Some(update) = patches.remove(&item.identity.persistent_key()) {
                    patched |= apply_image_patch(item, update);
                }
            }
            if patched && source.sort.0 == 1 { sort_items(&mut source.items, source.sort); }
            changed |= patched;
        }
    }
    if changed {
        refresh_views(state);
    }
    // Re-evaluate the viewport while work arrives, including after scrolling or
    // changing the sort order. Unseen files remain queued behind these entries.
    for view in &state.views {
        let Some(source) = state.folders.get(&view.id) else { continue; };
        if !changed && !source.request.images_pending.load(std::sync::atomic::Ordering::Acquire) { continue; }
        let model = view.model.borrow();
        let mut bounds = RECT::default();
        let hwnd = view.window.hwnd().cast();
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &raw mut bounds); }
        let scale = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
        let grid = model.grid(bounds.right as f32 / scale, bounds.bottom as f32 / scale);
        let start = model.scroll.saturating_mul(grid.columns).min(model.items.len());
        let count = (grid.visible_rows + 2).saturating_mul(grid.columns);
        let priority: Vec<_> = model.items.iter().skip(start).take(count).map(|item| item.identity.persistent_key()).collect();
        source.cache.lock().unwrap().touch(&priority);
        *source.request.priority.lock().unwrap() = priority;
    }
}

fn apply_image_patch(item: &mut Item, update: Item) -> bool {
    // Unknown timestamps cannot establish that a delayed result still belongs
    // to this scan. Such items receive images through the full final snapshot.
    if item.identity != update.identity
        || item.details.modified_time.is_none()
        || item.details.modified_time != update.details.modified_time
        || item.details.folder != update.details.folder
        || item.details.size != update.details.size
    {
        return false;
    }
    let image_changed = match (&item.image, &update.image) {
        (Some(a), Some(b)) => !Arc::ptr_eq(a, b)
            && (a.width != b.width || a.height != b.height || a.data != b.data),
        (None, None) => false,
        _ => true,
    };
    let changed = image_changed || item.details.kind != update.details.kind;
    if image_changed { item.image = update.image; }
    item.details.kind = update.details.kind;
    changed
}

fn sort_items(items: &mut Vec<Item>, sort: (u8, bool)) {
    let mut sorted: Vec<_> = std::mem::take(items)
        .into_iter()
        .map(|item| {
            // Reuse the worker's directory snapshot; sorting must not touch disk
            // on the UI thread (especially for network folders).
            let folder = item.details.folder;
            let modified = item.details.modified_time;
            let name = item.label.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let kind = if sort.0 == 1 {
                item.details.kind.encode_utf16().chain(Some(0)).collect::<Vec<_>>()
            } else { Vec::new() };
            (folder, name, kind, modified, item)
        })
        .collect();
    let text_order = |a: &[u16], b: &[u16]| unsafe {
        windows_sys::Win32::UI::Shell::StrCmpLogicalW(a.as_ptr(), b.as_ptr()).cmp(&0)
    };
    let direction = |order: std::cmp::Ordering| if sort.1 { order.reverse() } else { order };
    sorted.sort_by(|(af, an, ak, at, a), (bf, bn, bk, bt, b)| {
        let name = || text_order(an, bn);
        match sort.0 {
            // Explorer reverses the complete name order, including folder grouping.
            0 => direction(bf.cmp(af).then_with(name)),
            // Type keeps folders first and names ascending in either direction.
            1 => bf.cmp(af).then_with(|| {
                if *af { name() } else { direction(text_order(ak, bk)).then_with(name) }
            }),
            // Preserve the existing mixed timeline for modified-date sorting.
            2 => {
                if at.is_none() || bt.is_none() {
                    at.is_none().cmp(&bt.is_none()).then_with(name)
                } else {
                    direction(at.cmp(bt).then_with(name))
                }
            }
            // Empty sizes precede files ascending and follow them descending;
            // equal sizes (including folders) always use ascending names.
            3 => direction(bf.cmp(af)).then_with(|| {
                if *af { name() } else {
                    direction(a.details.size.cmp(&b.details.size)).then_with(name)
                }
            }),
            _ => name(),
        }
    });
    *items = sorted.into_iter().map(|(_, _, _, _, item)| item).collect();
}

pub(super) fn sort(state: &mut PaneApp, id: PanelId, column: u8) -> Result<(), String> {
    let Some(source) = state.folders.get_mut(&id) else {
        return Ok(());
    };
    let order = (column.min(3), if source.sort.0 == column { !source.sort.1 } else { column == 3 });
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
    let mut source = Source::start_with(path.clone(), state.wake.clone(), Arc::clone(&old.cache), old.sort)?;
    source.root = old.root.clone();
    source.history = history;
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

pub(super) fn home(state: &mut PaneApp, id: PanelId) -> Result<(), String> {
    let Some(source) = state.folders.get(&id) else { return Ok(()); };
    if source.path == source.root { return Ok(()); }
    let root = source.root.clone();
    navigate(state, id, Some(root))?;
    state.folders.get_mut(&id).unwrap().history.clear();
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

pub(super) fn size_text(bytes: Option<u64>, folder: bool) -> String {
    if folder { return String::new(); }
    let Some(bytes) = bytes else { return "—".into(); };
    if bytes < 1024 { return format!("{bytes} B"); }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KB", "MB", "GB", "TB", "PB", "EB"] {
        value /= 1024.0;
        unit = next;
        if value < 1024.0 { break; }
    }
    format!("{value:.1} {unit}")
}

pub(super) fn modified_text(value: Option<std::time::SystemTime>) -> String {
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

    fn patch_item() -> Item {
        Item {
            identity: identity(PathBuf::from(r"C:\test\image.png")),
            label: "image.png".into(),
            image: Some(Arc::new(assets::Pixels { width: 1, height: 1, data: vec![0; 4] })),
            details: ItemDetails {
                modified_time: Some(std::time::UNIX_EPOCH),
                size: Some(4),
                kind: "image".into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn image_patch_preserves_allocation_and_skips_unchanged_pixels() {
        let mut item = patch_item();
        let original = item.image.clone().unwrap();
        assert!(!apply_image_patch(&mut item, patch_item()));
        assert!(Arc::ptr_eq(&original, item.image.as_ref().unwrap()));
        let mut changed = patch_item();
        changed.image = Some(Arc::new(assets::Pixels { width: 1, height: 1, data: vec![255; 4] }));
        assert!(apply_image_patch(&mut item, changed));
        assert_eq!(item.image.as_ref().unwrap().data, vec![255; 4]);
        let mut kind = item.clone();
        kind.details.kind = "new type".into();
        assert!(apply_image_patch(&mut item, kind));
        assert_eq!(item.details.kind, "new type");
    }

    #[test]
    fn image_patch_rejects_renamed_modified_replaced_and_unknown_items() {
        for case in 0..5 {
            let mut item = patch_item();
            let original = item.image.clone().unwrap();
            let mut delayed = patch_item();
            delayed.details.kind = "stale".into();
            match case {
                0 => item.identity = identity(PathBuf::from(r"C:\test\renamed.png")),
                1 => item.details.modified_time = Some(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                2 => item.details.size = Some(8),
                3 => item.details.folder = true,
                _ => { item.details.modified_time = None; delayed.details.modified_time = None; }
            }
            assert!(!apply_image_patch(&mut item, delayed), "case {case}");
            assert!(Arc::ptr_eq(&original, item.image.as_ref().unwrap()));
            assert_eq!(item.details.kind, "image");
        }
    }

    #[test]
    fn inactive_tab_waits_for_activation_before_rescanning() {
        let root = tempfile::tempdir().unwrap();
        let source = Source::start(root.path().to_path_buf(), Default::default()).unwrap();
        assert!(source.updates.recv_timeout(Duration::from_secs(5)).unwrap().unwrap().is_empty());
        source.set_active(false);
        std::fs::write(root.path().join("new.txt"), b"new").unwrap();
        assert!(matches!(source.updates.recv_timeout(Duration::from_millis(400)), Err(mpsc::RecvTimeoutError::Timeout)));
        source.set_active(true);
        let items = source.updates.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "new.txt");
    }

    #[test]
    fn refresh_event_reads_an_unchanged_folder_again() {
        let root = std::env::temp_dir().join(format!("lucidpane-manual-refresh-{}-{}",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        let state = Rc::new(RefCell::new(super::super::tests::test_state()));
        let id = PanelId::new(1);
        state.borrow_mut().folders.insert(id, Source::start(root.clone(), Default::default()).unwrap());
        assert!(state.borrow().folders[&id].updates.recv_timeout(Duration::from_secs(5)).unwrap().unwrap().is_empty());
        for _ in 0..2 {
            // No file writes: only the explicit Refresh event can request a new snapshot.
            assert!(matches!(state.borrow().folders[&id].updates.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
            super::super::events::handle(&state, id, Event::Refresh).unwrap();
            assert!(state.borrow().folders[&id].updates.recv_timeout(Duration::from_secs(5)).unwrap().unwrap().is_empty());
        }
        drop(state);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn sizes_format_and_sort_numerically_with_empty_values() {
        assert_eq!(size_text(Some(0), false), "0 B");
        assert_eq!(size_text(Some(1023), false), "1023 B");
        assert_eq!(size_text(Some(1536), false), "1.5 KB");
        assert_eq!(size_text(Some(1024 * 1024), false), "1.0 MB");
        assert_eq!(size_text(None, false), "—");
        assert_eq!(size_text(None, true), "");
        let make = |name: &str, size, folder| Item {
            identity: ShellIdentity::Namespace { parsing_name: name.into() },
            label: name.into(), image: None,
            details: ItemDetails { size, folder, ..Default::default() },
        };
        let mut items = vec![make("large", Some(1024 * 1024), false), make("unknown", None, false), make("small", Some(9), false), make("folder", None, true)];
        sort_items(&mut items, (3, false));
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["folder", "unknown", "small", "large"]);
        sort_items(&mut items, (3, true));
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["large", "small", "unknown", "folder"]);
    }

    #[test]
    fn name_type_and_size_match_explorer_view_order() {
        // Captured from IFolderView2 with folder_sort_probe on this Windows
        // installation. In particular, descending type and size do not reverse ties.
        let make = |name: &str, kind: &str, size| Item {
            identity: ShellIdentity::Namespace { parsing_name: name.into() },
            label: name.into(), image: None,
            details: ItemDetails { kind: kind.into(), size, folder: size.is_none(), ..Default::default() },
        };
        let original = vec![
            make("dir10", "文件夹", None), make("dir2", "文件夹", None),
            make("file10.txt", "文本文档", Some(3)), make("file2.txt", "文本文档", Some(3)),
            make("a.zip", "ZIP 压缩文件", Some(3)), make("b.txt", "文本文档", Some(3)),
            make("c.txt", "文本文档", Some(3)), make("file1.bin", "BIN 文件", Some(3)),
            make("large.txt", "文本文档", Some(1024)),
        ];
        for (sort, expected) in [
            ((0, false), ["dir2", "dir10", "a.zip", "b.txt", "c.txt", "file1.bin", "file2.txt", "file10.txt", "large.txt"]),
            ((0, true), ["large.txt", "file10.txt", "file2.txt", "file1.bin", "c.txt", "b.txt", "a.zip", "dir10", "dir2"]),
            ((1, false), ["dir2", "dir10", "file1.bin", "a.zip", "b.txt", "c.txt", "file2.txt", "file10.txt", "large.txt"]),
            ((1, true), ["dir2", "dir10", "b.txt", "c.txt", "file2.txt", "file10.txt", "large.txt", "a.zip", "file1.bin"]),
            ((3, false), ["dir2", "dir10", "a.zip", "b.txt", "c.txt", "file1.bin", "file2.txt", "file10.txt", "large.txt"]),
            ((3, true), ["large.txt", "a.zip", "b.txt", "c.txt", "file1.bin", "file2.txt", "file10.txt", "dir2", "dir10"]),
        ] {
            let mut items = original.clone();
            sort_items(&mut items, sort);
            assert_eq!(items.iter().map(|item| item.label.as_str()).collect::<Vec<_>>(), expected, "sort={sort:?}");
        }
    }

    #[test]
    #[ignore = "Read-only thumbnail diagnostic; set LUCIDPANE_TEST_FOLDER"]
    fn real_folder_images_survive_parent_child_navigation() {
        let root = PathBuf::from(std::env::var_os("LUCIDPANE_TEST_FOLDER").expect("test folder"));
        let cache: images::SharedCache = Arc::default();
        let started = Instant::now();
        let source = Source::start_with(root.clone(), Default::default(), Arc::clone(&cache), (2, true)).unwrap();
        let first = source.updates.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        eprintln!("directory membership: {} entries in {:?}", first.len(), started.elapsed());
        let target = first.len().min(32);
        let mut loaded = HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(45);
        while loaded.len() < target {
            let batch = source.images.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("thumbnail batch");
            for item in batch {
                if let Some(image) = item.image { loaded.insert(item.identity.persistent_key(), image); }
            }
        }
        eprintln!("first {} real images: {:?}", loaded.len(), started.elapsed());
        let logical_bytes: usize = loaded.values().map(|image| image.data.len()).sum();
        let unique: HashMap<_, _> = loaded.values().map(|image| (Arc::as_ptr(image), image.data.len())).collect();
        eprintln!("real image pixel storage: {logical_bytes} unshared bytes -> {} shared bytes ({} unique images)",
            unique.values().sum::<usize>(), unique.len());
        let child = first.iter().find(|item| item.details.folder).and_then(|item| item.identity.file_system_path()).map(Path::to_path_buf);
        drop(source);
        if let Some(child) = child {
            let child = Source::start_with(child, Default::default(), Arc::clone(&cache), (2, true)).unwrap();
            child.updates.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
            drop(child);
        }
        let started = Instant::now();
        let returned = Source::start_with(root, Default::default(), cache, (2, true)).unwrap();
        let first = returned.updates.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        let reused = first.iter().filter(|item| {
            item.image.as_ref().zip(loaded.get(&item.identity.persistent_key())).is_some_and(|(image, old)| Arc::ptr_eq(image, old))
        }).count();
        eprintln!("back navigation: {reused}/{} prior image allocations reused in first snapshot ({:?})", loaded.len(), started.elapsed());
        assert_eq!(reused, loaded.len());
    }

    #[test]
    #[ignore = "Read-only diagnostic; set LUCIDPANE_TEST_FOLDER to an existing directory"]
    fn real_folder_snapshot_matches_directory_and_sorts_newest_first() {
        let root = PathBuf::from(std::env::var_os("LUCIDPANE_TEST_FOLDER").expect("test folder"));
        let mut expected: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        expected.sort();
        let started = Instant::now();
        let source = Source::start(root, Default::default()).unwrap();
        let mut items = source
            .updates
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap();
        let mut actual: Vec<_> = items
            .iter()
            .map(|item| item.identity.file_system_path().unwrap().to_path_buf())
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
        sort_items(&mut items, (2, true));
        assert!(
            items
                .windows(2)
                .all(|pair| pair[0].details.modified_time >= pair[1].details.modified_time)
        );
        eprintln!(
            "{} entries, first sorted snapshot in {:?}",
            items.len(),
            started.elapsed()
        );
        for item in items.iter().take(10) {
            eprintln!("{}  {}", item.details.modified, item.label);
        }
    }

    #[test]
    fn first_snapshot_includes_hidden_children_before_loading_images() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, SetFileAttributesW,
        };
        let root = std::env::temp_dir().join(format!(
            "lucidpane-complete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        for index in 0..256 {
            std::fs::write(root.join(format!("file-{index}.txt")), b"").unwrap();
        }
        let hidden = root.join("file-0.txt");
        let wide: Vec<_> = hidden.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(
            unsafe {
                SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)
            },
            0
        );
        std::fs::create_dir(root.join("child")).unwrap();
        std::fs::write(root.join("child/nested.txt"), b"").unwrap();
        let source = Source::start(root.clone(), Default::default()).unwrap();
        let request = Arc::downgrade(&source.request);
        let started = Instant::now();
        let items = source
            .updates
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        eprintln!(
            "first folder snapshot: {} entries in {:?}",
            items.len(),
            started.elapsed()
        );
        assert_eq!(items.len(), 257);
        assert!(items.iter().all(|item| item.details.size == if item.details.folder { None } else { Some(0) }));
        assert!(
            items
                .iter()
                .any(|item| item.identity.file_system_path() == Some(hidden.as_path()))
        );
        assert!(items.iter().all(|item| item.image.is_none()));
        assert!(
            items
                .iter()
                .all(|item| item.identity.file_system_path().unwrap().parent()
                    == Some(root.as_path()))
        );
        drop(source);
        let deadline = Instant::now() + Duration::from_secs(10);
        while request.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(request.upgrade().is_none());
        // Only the uniquely created test directory is removed.
        std::fs::remove_dir_all(root).unwrap();
    }

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
            ["z-old", "a-new", "z-folder"]
        );
        sort_items(&mut items, (2, true));
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["a-new", "z-old", "z-folder"]
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
        std::fs::write(root.join("a.txt"), b"aaa").unwrap();
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
            ["z.txt", "a.txt", "child"]
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
        for expected in [["a.txt", "z.txt", "child"], ["child", "z.txt", "a.txt"]] {
            sort(&mut state, id, 3).unwrap();
            assert_eq!(state.folders[&id].items.iter().map(|item| item.label.as_str()).collect::<Vec<_>>(), expected);
        }
        navigate(&mut state, id, Some(root.join("child"))).unwrap();
        ensure(&mut state, id).unwrap();
        assert_eq!(state.folders[&id].path, root.join("child"));
        assert_eq!(state.folders[&id].navigation(), [true, true]);
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
        navigate(&mut state, id, Some(root.join("child"))).unwrap();
        home(&mut state, id).unwrap();
        assert_eq!(state.folders[&id].path, root);
        assert!(state.folders[&id].history.is_empty());
        assert_eq!(state.folders[&id].navigation(), [false, false]);
        navigate(&mut state, id, None).unwrap();
        assert_eq!(state.folders[&id].path, root);
        state.folders.remove(&id);
        ensure(&mut state, id).unwrap();
        assert_eq!(state.folders[&id].sort, (3, false));
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
                // The UI consumes both bounded channels. Leaving thumbnails
                // unread eventually blocks the worker before its final snapshot.
                while source.images.try_recv().is_ok() {}
                assert!(Instant::now() < deadline, "folder update timed out");
                let result = match source.updates.recv_timeout(Duration::from_millis(25)) {
                    Ok(result) => result,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(error) => panic!("folder worker disconnected: {error}"),
                };
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
            result.as_ref().is_ok_and(|items| {
                items.len() == 2 && items.iter().all(|item| item.image.is_some())
            })
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
