//! Configuration backup history and serialized background operations.
use super::*;
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Policy {
    pub enabled: bool,
    pub minutes: u64,
    pub keep: usize,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            minutes: 5,
            keep: 10,
        }
    }
}
impl Policy {
    pub fn load(store: &WorkspaceStore) -> Self {
        let value = store
            .preference("backup_policy")
            .ok()
            .flatten()
            .unwrap_or_default();
        let parts: Vec<_> = value.split(',').collect();
        if parts.len() != 3 {
            return Self::default();
        }
        Self {
            enabled: parts[0] != "0",
            minutes: parts[1]
                .parse()
                .ok()
                .filter(|v| [5, 15, 30, 60].contains(v))
                .unwrap_or(5),
            keep: parts[2]
                .parse()
                .ok()
                .filter(|v| [10, 20, 50].contains(v))
                .unwrap_or(10),
        }
    }
    pub fn save(self, store: &WorkspaceStore) -> Result<(), String> {
        store
            .save_preference(
                "backup_policy",
                &format!("{},{},{}", u8::from(self.enabled), self.minutes, self.keep),
            )
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub path: PathBuf,
    pub date: String,
    pub kind: &'static str,
    pub bytes: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct View {
    pub records: Arc<[Record]>,
    pub status: String,
    pub busy: bool,
    pub undo: Option<PathBuf>,
}
struct Completed {
    result: Result<Outcome, String>,
    records: Arc<[Record]>,
}
struct RestoreInput {
    original: PathBuf,
    temporary: tempfile::TempDir,
    count: usize,
    version: String,
    missing: Vec<String>,
}
impl RestoreInput {
    fn path(&self) -> PathBuf {
        self.temporary.path().join("restore.db")
    }
}
enum Outcome {
    Saved(String),
    Unchanged,
    Inspect(RestoreInput),
    ReadyRestore(RestoreInput, PathBuf),
}
pub(super) struct Manager {
    pub view: View,
    receiver: Option<mpsc::Receiver<Completed>>,
    observed: u64,
    changed: Instant,
    attempted: Instant,
    initialized: bool,
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            view: View::default(),
            receiver: None,
            observed: 0,
            changed: Instant::now(),
            attempted: Instant::now() - Duration::from_secs(3600),
            initialized: false,
        }
    }
}
pub(super) fn directory(s: &PaneApp) -> Result<PathBuf, String> {
    s.runtime
        .as_ref()
        .and_then(|r| r.path.parent())
        .map(|p| p.join("backups"))
        .ok_or_else(|| "配置目录不可用".into())
}
fn name(label: &str) -> String {
    let mut time = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&raw mut time);
    }
    format!(
        "{label}-{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}.db",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}
fn records(directory: &Path) -> Vec<Record> {
    let mut result: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "db") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = if name.starts_with("auto-") {
                "自动"
            } else if name.starts_with("before-restore-") {
                "恢复前"
            } else {
                "手动"
            };
            Some((
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                Record {
                    path,
                    date: folder::modified_text(metadata.modified().ok()),
                    kind,
                    bytes: metadata.len(),

                },
            ))
        })
        .collect();
    result.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.path.cmp(&a.1.path)));
    result.into_iter().map(|(_, r)| r).collect()
}
fn cleanup(directory: &Path, kind: &str, keep: usize) -> Option<String> {
    let mut failed = 0;
    for record in records(directory)
        .into_iter()
        .filter(|r| r.kind == kind)
        .skip(keep)
    {
        if std::fs::remove_file(record.path).is_err() {
            failed += 1;
        }
    }
    (failed > 0).then(|| format!("；{failed} 份旧备份清理失败"))
}
fn save_snapshot(
    store: &WorkspaceStore,
    directory: &Path,
    label: &str,
    keep: usize,
) -> Result<(PathBuf, String), String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut path = directory.join(name(label));
    // Never overwrite another snapshot, even when the clock moves backwards.
    if path.exists() {
        path = directory.join(format!(
            "{label}-{}.db",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
    }
    store
        .save_preference("backup_manifest_version", env!("CARGO_PKG_VERSION"))
        .map_err(|e| e.to_string())?;
    store.export_backup(&path).map_err(|e| e.to_string())?;
    let warning = match label {
        "auto" => cleanup(directory, "自动", keep),
        "before-restore" => cleanup(directory, "恢复前", 5),
        _ => None,
    }
    .unwrap_or_default();
    Ok((path, warning))
}
fn begin(
    state: &Rc<RefCell<PaneApp>>,
    label: &str,
    work: impl FnOnce(PathBuf) -> Result<Outcome, String> + Send + 'static,
) -> Result<(), String> {
    let mut s = state.borrow_mut();
    let directory = directory(&s)?;
    let wake = s.wake.clone();
    let manager = &mut s.runtime.as_mut().ok_or("运行时不可用")?.backup;
    if manager.view.busy {
        return Err("备份任务正在进行，请稍候。".into());
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("configuration-backup".into())
        .spawn(move || {
            let result = work(directory.clone());
            let _ = tx.send(Completed {
                result,
                records: records(&directory).into(),
            });
            wake.notify();
        })
        .map_err(|e| e.to_string())?;
    manager.receiver = Some(rx);
    manager.view.busy = true;
    manager.view.status = label.into();
    if let Some(settings) = &s.settings {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                settings.hwnd().cast(),
                std::ptr::null(),
                0,
            );
        }
    }
    Ok(())
}
pub(super) fn view(state: &PaneApp) -> View {
    state
        .runtime
        .as_ref()
        .map(|r| r.backup.view.clone())
        .unwrap_or_default()
}
pub(super) fn maintain(state: &Rc<RefCell<PaneApp>>) {
    let completed = {
        let mut s = state.borrow_mut();
        let Some(runtime) = s.runtime.as_mut() else {
            return;
        };
        runtime
            .backup
            .receiver
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(v) => Some(v),
                Err(mpsc::TryRecvError::Disconnected) => Some(Completed {
                    result: Err("备份任务意外结束，请重试。".into()),
                    records: runtime.backup.view.records.clone(),
                }),
                _ => None,
            })
    };
    if let Some(completed) = completed {
        {
            let mut s = state.borrow_mut();
            let m = &mut s.runtime.as_mut().unwrap().backup;
            m.receiver = None;
            m.view.busy = false;
            m.view.records = completed.records;
            if m.view
                .undo
                .as_ref()
                .is_some_and(|path| !m.view.records.iter().any(|r| &r.path == path))
            {
                m.view.undo = None;
            }
        }
        match completed.result {
            Ok(Outcome::Saved(message)) => set_status(state, &message),
            Ok(Outcome::Unchanged) => set_status(state, "配置暂无变化，无需重复备份"),
            Ok(Outcome::Inspect(input)) => {
                state
                    .borrow_mut()
                    .runtime
                    .as_mut()
                    .unwrap()
                    .backup
                    .view
                    .busy = true;
                let weak = Rc::downgrade(state);
                window::defer_action(move || {
                    if let Some(state) = weak.upgrade() {
                        confirm_restore(&state, input);
                    }
                });
            }
            Ok(Outcome::ReadyRestore(input, rollback)) => {
                state
                    .borrow_mut()
                    .runtime
                    .as_mut()
                    .unwrap()
                    .backup
                    .view
                    .busy = true;
                let weak = Rc::downgrade(state);
                window::defer_action(move || {
                    if let Some(state) = weak.upgrade() {
                        apply_restore(&state, input, rollback);
                    }
                });
            }
            Err(error) => set_status(state, &format!("操作失败：{error}")),
        }
        return;
    }
    let due = {
        let mut s = state.borrow_mut();
        let policy = Policy::load(&s.store);
        let changes = s.store.change_count();
        let Some(r) = s.runtime.as_mut() else {
            return;
        };
        let m = &mut r.backup;
        if changes != m.observed {
            m.observed = changes;
            m.changed = Instant::now();
        }
        if m.view.busy {
            false
        } else if !m.initialized {
            m.initialized = true;
            drop(s);
            let _ = begin(state, "读取备份记录…", |directory| {
                Ok(Outcome::Saved(records(&directory).first().map_or_else(
                    || "尚无备份记录".into(),
                    |r| format!("上次备份：{}", r.date),
                )))
            });
            return;
        } else {
            policy.enabled
                && m.changed.elapsed() >= Duration::from_secs(10)
                && m.attempted.elapsed() >= Duration::from_secs(policy.minutes * 60)
        }
    };
    if due {
        if let Some(r) = state.borrow_mut().runtime.as_mut() {
            r.backup.attempted = Instant::now();
        }
        if let Err(error) = create(state, true) {
            set_status(state, &format!("自动备份失败：{error}"));
        }
    }
}
fn set_status(state: &Rc<RefCell<PaneApp>>, message: &str) {
    let mut state = state.borrow_mut();
    if let Some(r) = state.runtime.as_mut() {
        r.backup.view.status = message.into();
    }
    if let Some(settings) = &state.settings {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                settings.hwnd().cast(),
                std::ptr::null(),
                0,
            );
        }
    }
}
pub(super) fn create(state: &Rc<RefCell<PaneApp>>, automatic: bool) -> Result<(), String> {
    if view(&state.borrow()).busy {
        return Err("备份任务正在进行，请稍候。".into());
    }
    let (snapshot, policy) = {
        let s = state.borrow();
        (
            s.store.backup_snapshot().map_err(|e| e.to_string())?,
            Policy::load(&s.store),
        )
    };
    begin(state, "正在创建备份…", move |directory| {
        if automatic {
            if let Some(previous) = records(&directory).first() {
                if WorkspaceStore::backup_file_content(&previous.path)
                    .ok()
                    .as_ref()
                    == Some(&snapshot.backup_content().map_err(|e| e.to_string())?)
                {
                    return Ok(Outcome::Unchanged);
                }
            }
        }
        let (_, warning) = save_snapshot(
            &snapshot,
            &directory,
            if automatic { "auto" } else { "manual" },
            policy.keep,
        )?;
        Ok(Outcome::Saved(format!(
            "{}备份成功{warning}",
            if automatic { "自动" } else { "手动" }
        )))
    })
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

fn confirm_restore(state: &Rc<RefCell<PaneApp>>, input: RestoreInput) {
    let owner = state
        .borrow()
        .settings
        .as_ref()
        .map_or(0, |w| w.hwnd() as isize);
    let missing = if input.missing.is_empty() {
        String::new()
    } else {
        format!(
            "\n以下映射目录当前不可访问，恢复后会保留映射：\n{}\n",
            input.missing.join("\n")
        )
    };
    let text = format!(
        "已通过完整性与兼容性检查。\n包含 {} 个面板 · 应用版本 {}。\n\n恢复将替换当前设置、布局和映射，不改动实际文件。\n当前状态将先备份，可在恢复后撤销。\n\n{}\n\n{missing}\n是否恢复？",
        input.count,
        input.version,
        input.original.display()
    );
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    if unsafe {
        MessageBoxW(
            owner as _,
            wide.as_ptr(),
            windows_sys::w!("恢复备份"),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    } != IDYES
    {
        state
            .borrow_mut()
            .runtime
            .as_mut()
            .unwrap()
            .backup
            .view
            .busy = false;
        set_status(state, "已取消恢复");
        return;
    }
    state
        .borrow_mut()
        .runtime
        .as_mut()
        .unwrap()
        .backup
        .view
        .busy = false;
    let result = (|| {
        let snapshot = state
            .borrow()
            .store
            .backup_snapshot()
            .map_err(|e| e.to_string())?;
        begin(state, "正在保存恢复前备份…", move |directory| {
            let (rollback, _) = save_snapshot(&snapshot, &directory, "before-restore", 5)?;
            Ok(Outcome::ReadyRestore(input, rollback))
        })
    })();
    if let Err(e) = result {
        set_status(state, &format!("恢复未执行：{e}"));
    }
}
fn apply_restore(state: &Rc<RefCell<PaneApp>>, input: RestoreInput, rollback: PathBuf) {
    apply_restore_with(state, input, rollback, runtime::reload);
}
fn apply_restore_with(
    state: &Rc<RefCell<PaneApp>>,
    input: RestoreInput,
    rollback: PathBuf,
    mut reload: impl FnMut(&Rc<RefCell<PaneApp>>) -> Result<(), String>,
) {
    let result = state
        .borrow_mut()
        .store
        .restore_backup(&input.path())
        .map_err(|e| e.to_string());
    let result = result.and_then(|_| reload(state));
    state
        .borrow_mut()
        .runtime
        .as_mut()
        .unwrap()
        .backup
        .view
        .busy = false;
    match result {
        Ok(()) => {
            state
                .borrow_mut()
                .runtime
                .as_mut()
                .unwrap()
                .backup
                .view
                .undo = Some(rollback);
            set_status(state, "恢复成功，可撤销本次恢复");
        }
        Err(error) => {
            let rollback_result = state
                .borrow_mut()
                .store
                .restore_backup(&rollback)
                .map_err(|e| e.to_string());
            let rollback_result = rollback_result.and_then(|_| reload(state));
            set_status(
                state,
                &match rollback_result {
                    Ok(()) => format!("恢复失败，已回退：{error}"),
                    Err(e) => format!(
                        "恢复失败：{error}；回退失败：{e}。恢复前备份：{}",
                        rollback.display()
                    ),
                },
            );
        }
    }
}
pub(super) fn request(state: &Rc<RefCell<PaneApp>>, event: &Event) {
    let event = event.clone();
    let weak = Rc::downgrade(state);
    window::defer_action(move || {
        if let Some(state) = weak.upgrade() {
            if let Err(e) = execute(&state, &event) {
                set_status(&state, &format!("操作失败：{e}"));
                window::error(&e);
            }
        }
    });
}
fn execute(state: &Rc<RefCell<PaneApp>>, event: &Event) -> Result<(), String> {
    if view(&state.borrow()).busy {
        return Err("备份任务正在进行，请稍候。".into());
    }
    let owner = state
        .borrow()
        .settings
        .as_ref()
        .map_or(0, |w| w.hwnd() as isize);
    if matches!(event, Event::ReloadConfig) {
        state
            .borrow()
            .store
            .reload_config()
            .map_err(|e| e.to_string())?;
        return runtime::reload(state);
    }
    if matches!(event, Event::OpenConfigDirectory | Event::OpenBackups) {
        let path = if matches!(event, Event::OpenBackups) {
            directory(&state.borrow())?
        } else {
            state
                .borrow()
                .store
                .config_path()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .ok_or("配置目录不可用")?
        };
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        return open_shell_identity(owner, &folder::identity(path)).map_err(|e| e.to_string());
    }
    match event {
        Event::CreateBackup => create(state, false),
        Event::RestoreBackup | Event::RestoreBackupPath(_) => {
            let path = if let Event::RestoreBackupPath(path) = event {
                path.clone()
            } else {
                let Some(path) = choose(owner, false)? else {
                    return Ok(());
                };
                path
            };
            begin(state, "正在校验备份…", move |_| {
                let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
                let staged = temporary.path().join("restore.db");
                std::fs::copy(&path, &staged).map_err(|e| e.to_string())?;
                let count = WorkspaceStore::inspect_backup(&staged).map_err(|e| e.to_string())?;
                let version = WorkspaceStore::read_backup(&staged)
                    .map_err(|e| e.to_string())?
                    .preference("backup_manifest_version")
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "旧版备份".into());
                let workspace = WorkspaceStore::read_backup(&staged)
                    .map_err(|e| e.to_string())?
                    .load_workspace()
                    .map_err(|e| e.to_string())?;
                let missing = workspace
                    .panels()
                    .iter()
                    .filter_map(|p| p.folder())
                    .filter(|p| !p.is_dir())
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect();
                Ok(Outcome::Inspect(RestoreInput {
                    original: path,
                    temporary,
                    count,
                    version,
                    missing,
                }))
            })
        }
        Event::DeleteBackup(path) => {
            let root = directory(&state.borrow())?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let path = path.canonicalize().map_err(|e| e.to_string())?;
            if path.parent() != Some(root.as_path()) || path.extension().is_none_or(|e| e != "db") {
                return Err("只能删除备份目录中的记录".into());
            }
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            if unsafe {
                MessageBoxW(
                    owner as _,
                    windows_sys::w!("删除这份备份？此操作不会更改当前配置。"),
                    windows_sys::w!("删除备份"),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            } != IDYES
            {
                return Ok(());
            }
            begin(state, "正在删除…", move |_| {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                Ok(Outcome::Saved("备份已删除".into()))
            })
        }
        Event::ExportBackup | Event::ExportBackupPath(_) => {
            let Some(path) = choose(owner, true)? else {
                return Ok(());
            };
            let s = state.borrow();
            if s.runtime.as_ref().is_some_and(|r| r.path == path)
                || path
                    .file_name()
                    .is_some_and(|p| p == "workspace.db" || p == "config.toml")
            {
                return Err("不能覆盖正在使用的配置文件".into());
            }
            let snapshot = if let Event::ExportBackupPath(source) = event {
                WorkspaceStore::read_backup(source).map_err(|e| e.to_string())?
            } else {
                s.store.backup_snapshot().map_err(|e| e.to_string())?
            };
            drop(s);
            begin(state, "正在导出…", move |_| {
                snapshot
                    .export_backup_replace(&path)
                    .map_err(|e| e.to_string())?;
                Ok(Outcome::Saved(format!("已导出：{}", path.display())))
            })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backup_views_share_history_and_keep_previous_snapshot_stable() {
        let mut current = super::View {
            records: (0..1000).map(|index| super::Record {
                path: std::path::PathBuf::from(format!("backup-{index}.db")),
                date: "2026-09-14".into(), kind: "manual", bytes: 1024,
            }).collect(), ..Default::default()
        };
        let previous = current.clone();
        assert!(std::sync::Arc::ptr_eq(&current.records, &previous.records));
        current.records = Default::default();
        assert_eq!(previous.records.len(), 1000);
    }

    use super::*;
    fn state(root: &Path) -> Rc<RefCell<PaneApp>> {
        let mut state = super::super::tests::test_state();
        let path = root.join("workspace.db");
        state.store = WorkspaceStore::open(&path).unwrap();
        state.store.save_workspace(&state.workspace).unwrap();
        state.runtime = Some(runtime::State::new(path));
        Rc::new(RefCell::new(state))
    }
    fn finish(state: &Rc<RefCell<PaneApp>>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while view(&state.borrow()).busy {
            assert!(Instant::now() < deadline, "backup worker timed out");
            maintain(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn worker_serializes_jobs_deduplicates_content_and_rejects_corrupt_restore() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        create(&state, true).unwrap();
        assert!(create(&state, false).is_err());
        finish(&state);
        assert_eq!(view(&state.borrow()).records.len(), 1);
        create(&state, true).unwrap();
        finish(&state);
        assert_eq!(view(&state.borrow()).records.len(), 1);
        assert!(view(&state.borrow()).status.contains("暂无变化"));
        state
            .borrow()
            .store
            .save_preference("test_changed", "yes")
            .unwrap();
        create(&state, true).unwrap();
        finish(&state);
        assert_eq!(view(&state.borrow()).records.len(), 2);
        let corrupt = root.path().join("corrupt.db");
        std::fs::write(&corrupt, b"not sqlite").unwrap();
        execute(&state, &Event::RestoreBackupPath(corrupt)).unwrap();
        finish(&state);
        assert!(view(&state.borrow()).status.contains("失败"));
        assert_eq!(
            state
                .borrow()
                .store
                .preference("test_changed")
                .unwrap()
                .as_deref(),
            Some("yes")
        );
        let policy = Policy { enabled: false, minutes: 30, keep: 20 };
        policy.save(&state.borrow().store).unwrap();
        assert_eq!(Policy::load(&state.borrow().store), policy);
    }
    #[test]
    fn retention_preserves_manual_snapshots_and_reports_cleanup_failure_separately() {
        let root = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::open_in_memory().unwrap();
        for _ in 0..12 {
            save_snapshot(&store, root.path(), "auto", 10).unwrap();
        }
        for _ in 0..7 {
            save_snapshot(&store, root.path(), "before-restore", 5).unwrap();
        }
        for _ in 0..3 {
            save_snapshot(&store, root.path(), "manual", 10).unwrap();
        }
        let rows = records(root.path());
        assert_eq!(rows.iter().filter(|r| r.kind == "自动").count(), 10);
        assert_eq!(rows.iter().filter(|r| r.kind == "恢复前").count(), 5);
        assert_eq!(rows.iter().filter(|r| r.kind == "手动").count(), 3);
        let path = rows.iter().find(|r| r.kind == "自动").unwrap().path.clone();
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(
            cleanup(root.path(), "自动", 0)
                .unwrap()
                .contains("清理失败")
        );
        drop(held);
    }
    #[test]
    fn failed_reload_restores_previous_database_and_configuration() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        state
            .borrow()
            .store
            .save_preference("search_enabled", "0")
            .unwrap();
        let rollback = root.path().join("rollback.db");
        state.borrow().store.export_backup(&rollback).unwrap();
        state
            .borrow()
            .store
            .save_preference("search_enabled", "1")
            .unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let staged = temporary.path().join("restore.db");
        state.borrow().store.export_backup(&staged).unwrap();
        state.borrow_mut().store.restore_backup(&rollback).unwrap();
        let input = RestoreInput {
            original: staged,
            temporary,
            count: 3,
            version: "test".into(),
            missing: vec![],
        };
        let mut attempts = 0;
        apply_restore_with(&state, input, rollback, |state| {
            assert!(state.try_borrow_mut().is_ok(), "reload must not retain a store borrow");
            attempts += 1;
            if attempts == 1 {
                Err("injected reload failure".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(attempts, 2);
        assert_eq!(
            state
                .borrow()
                .store
                .preference("search_enabled")
                .unwrap()
                .as_deref(),
            Some("0")
        );
        assert!(view(&state.borrow()).status.contains("已回退"));
    }
}
