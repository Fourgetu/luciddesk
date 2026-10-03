//! Current-user login startup; MSIX uses StartupTask, ordinary builds use Run.
//! StartupApproved is an undocumented, read-only hint.
use std::{os::windows::fs::MetadataExt, path::Path, sync::mpsc};
use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows_registry::{CURRENT_USER, Key};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "LucidDesk";
const TASK_ID: &str = "LucidDeskStartup";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Off,
    Enabled,
    DisabledByWindows,
    Unknown,
    OtherLocation,
    DisabledByUser,
    DisabledByPolicy,
    EnabledByPolicy,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Self::Off => "off", Self::Enabled => "enabled", Self::DisabledByWindows => "disabled_by_windows",
            Self::Unknown => "unknown", Self::OtherLocation => "other_location", Self::DisabledByUser => "disabled_by_user",
            Self::DisabledByPolicy => "disabled_by_policy", Self::EnabledByPolicy => "enabled_by_policy",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Off,Self::Enabled,Self::DisabledByWindows,Self::Unknown,Self::OtherLocation,Self::DisabledByUser,Self::DisabledByPolicy,Self::EnabledByPolicy].into_iter().find(|status|status.code()==code)
    }

    pub fn registered(self) -> bool {
        matches!(
            self,
            Self::Enabled | Self::DisabledByWindows | Self::EnabledByPolicy
        )
    }
    pub fn message(self) -> &'static str {
        crate::i18n::text(match self {
            Self::Off => "startup-off",
            Self::Enabled => "startup-enabled",
            Self::DisabledByWindows => "startup-windows-disabled",
            Self::Unknown => "startup-unknown",
            Self::OtherLocation => "startup-other-location",
            Self::DisabledByUser => "startup-user-disabled",
            Self::DisabledByPolicy => "startup-policy-disabled",
            Self::EnabledByPolicy => "startup-policy-enabled",
        })
    }
    pub fn editable(self) -> bool {
        !matches!(
            self,
            Self::Unknown
                | Self::OtherLocation
                | Self::DisabledByUser
                | Self::DisabledByPolicy
                | Self::EnabledByPolicy
        )
    }
}

fn missing(code: i32) -> bool {
    code as u32 == 0x8007_0002
}

fn command(exe: &Path) -> Result<String, String> {
    let path = exe
        .to_str()
        .ok_or_else(|| crate::i18n::text("startup-invalid-path").to_string())?;
    let command = format!("\"{path}\" --startup");
    // Run values have a documented 260-character command-line limit.
    if !exe.is_absolute() || path.contains(['\"', '\0']) || command.encode_utf16().count() > 260 {
        return Err(crate::i18n::text("startup-invalid-path").into());
    }
    Ok(command)
}

fn registration(root: &Key, path: &str) -> windows_registry::Result<Option<String>> {
    let key = match root.open(path) {
        Ok(key) => key,
        Err(error) if missing(error.code().0) => return Ok(None),
        Err(error) => return Err(error),
    };
    match key.get_string(NAME) {
        Ok(value) => Ok(Some(value)),
        Err(error) if missing(error.code().0) => Ok(None),
        Err(error) => Err(error),
    }
}

fn approval(bytes: Option<&[u8]>) -> Status {
    let Some(bytes) = bytes else {
        return Status::Enabled;
    };
    if bytes.len() != 12 {
        return Status::Unknown;
    }
    match u32::from_le_bytes(bytes[..4].try_into().unwrap()) {
        2 | 6 => Status::Enabled,
        3 | 7 => Status::DisabledByWindows,
        _ => Status::Unknown,
    }
}

fn query(root: &Key, run: &str, approved: &str, exe: &Path) -> Result<Status, String> {
    let Some(value) = registration(root, run).map_err(|e| e.to_string())? else {
        return Ok(Status::Off);
    };
    if !value.eq_ignore_ascii_case(&command(exe)?) {
        return Ok(Status::OtherLocation);
    }
    let key = match root.open(approved) {
        Ok(key) => key,
        Err(error) if missing(error.code().0) => return Ok(approval(None)),
        Err(error) => return Err(error.to_string()),
    };
    match key.get_value(NAME) {
        Ok(value) if value.ty() == windows_registry::Type::Bytes => Ok(approval(Some(&value))),
        Ok(_) => Ok(Status::Unknown),
        Err(error) if missing(error.code().0) => Ok(approval(None)),
        Err(error) => Err(error.to_string()),
    }
}

fn packaged() -> Result<bool, String> {
    let mut length = 0;
    let code = unsafe {
        windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName(
            &mut length,
            std::ptr::null_mut(),
        )
    };
    match code as u32 {
        windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE => Ok(false),
        windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER => Ok(true),
        _ => Err(std::io::Error::from_raw_os_error(code as i32).to_string()),
    }
}

fn msix(exe: &Path, has_identity: bool) -> Result<bool, String> {
    match std::fs::symlink_metadata(exe.with_file_name("msix")) {
        Ok(meta)
            if meta.is_file()
                && meta.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    == 0 =>
        {
            Ok(has_identity)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !has_identity => Ok(false),
        _ => Err(crate::i18n::text("startup-package-invalid").into()),
    }
}

fn task_status(state: StartupTaskState) -> Status {
    match state {
        StartupTaskState::Disabled => Status::Off,
        StartupTaskState::Enabled => Status::Enabled,
        StartupTaskState::DisabledByUser => Status::DisabledByUser,
        StartupTaskState::DisabledByPolicy => Status::DisabledByPolicy,
        StartupTaskState::EnabledByPolicy => Status::EnabledByPolicy,
        _ => Status::Unknown,
    }
}

fn package_operation(enabled: Option<bool>) -> Result<Status, String> {
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|error| error.to_string())?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let result = (|| -> windows::core::Result<Status> {
        let task = StartupTask::GetAsync(&TASK_ID.into())?.join()?;
        let current = task_status(task.State()?);
        // Windows owns user/policy disable state. Never try to override it.
        if !current.editable() || current == Status::Unknown {
            return Ok(current);
        }
        match enabled {
            Some(true) => Ok(task_status(task.RequestEnableAsync()?.join()?)),
            Some(false) => {
                task.Disable()?;
                Ok(task_status(task.State()?))
            }
            None => Ok(current),
        }
    })();
    result.map_err(|error| error.to_string())
}

// Serialize GUI and CLI workers, then re-check OS state immediately before writing.
static OPERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn operation(enabled: Option<bool>) -> Result<Status, String> {
    operation_checked(enabled, None).map(|(_, after)| after)
}
pub(crate) fn operation_checked(enabled: Option<bool>, expected: Option<Status>) -> Result<(Status, Status), String> {
    let _guard = OPERATION.lock().map_err(|_| "Startup operation lock unavailable")?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let package = msix(&exe, packaged()?)?;
    let before = if package { package_operation(None)? } else { query(&CURRENT_USER, RUN, APPROVED, &exe)? };
    validate_change(before, enabled, expected)?;
    let Some(enabled) = enabled else {return Ok((before,before));};
    if (enabled && before == Status::Enabled) || (!enabled && before == Status::Off) {return Ok((before,before));}
    let after = if package {package_operation(Some(enabled))?} else {
        update(&CURRENT_USER, RUN, &exe, enabled)?;
        query(&CURRENT_USER, RUN, APPROVED, &exe)?
    };
    Ok((before,after))
}
fn validate_change(before: Status, enabled: Option<bool>, expected: Option<Status>) -> Result<(),String> {
    if expected.is_some_and(|expected| expected != before) {return Err("Startup state changed; query and preview again".into());}
    if enabled.is_some() && !before.editable() {return Err(before.message().into());}
    Ok(())
}

pub(crate) struct Controller {
    status: Status,
    pending: Option<mpsc::Receiver<Result<Status, String>>>,
    report_error: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            status: Status::Unknown,
            pending: None,
            report_error: false,
        }
    }
}
impl Controller {
    pub fn status(&self) -> Status {
        self.status
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn refresh(&mut self) -> Result<(), String> {
        self.start(None)
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.start(Some(enabled))
    }
    fn start(&mut self, enabled: Option<bool>) -> Result<(), String> {
        if self.busy() {
            return Ok(());
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("login-startup".into())
            .spawn(move || {
                let _ = sender.send(operation(enabled));
            })
            .map_err(|error| error.to_string())?;
        self.pending = Some(receiver);
        self.report_error = enabled.is_some();
        Ok(())
    }
    pub fn poll(&mut self) -> Option<String> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("Startup worker disconnected".into()),
        };
        self.pending = None;
        match result {
            Ok(status) => {
                self.status = status;
                None
            }
            Err(error) => {
                self.status = Status::Unknown;
                if self.report_error {
                    Some(error)
                } else {
                    crate::diagnostics::log(crate::diagnostics::Level::Error, "startup", &format!("Startup status: {error}"));
                    None
                }
            }
        }
    }
}

fn update(root: &Key, run: &str, exe: &Path, enabled: bool) -> Result<(), String> {
    let expected = command(exe)?;
    let previous = registration(root, run).map_err(|e| e.to_string())?;
    if previous
        .as_ref()
        .is_some_and(|value| !value.eq_ignore_ascii_case(&expected))
    {
        return Err(crate::i18n::text("startup-other-location").into());
    }
    if enabled && previous.as_deref() == Some(expected.as_str()) {
        return Ok(());
    }
    if enabled {
        root.create(run)
            .and_then(|key| key.set_string(NAME, expected))
            .map_err(|e| e.to_string())
    } else if previous.is_some() {
        root.options()
            .read()
            .write()
            .open(run)
            .and_then(|key| key.remove_value(NAME))
            .map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_changes_reject_stale_or_protected_state() {
        for status in [Status::Unknown,Status::OtherLocation,Status::DisabledByUser,Status::DisabledByPolicy,Status::EnabledByPolicy] {
            assert!(validate_change(status,Some(true),Some(status)).is_err());
            assert!(validate_change(status,Some(false),Some(status)).is_err());
            assert!(validate_change(status,None,None).is_ok());
            assert_eq!(Status::from_code(status.code()),Some(status));
        }
        assert!(validate_change(Status::Enabled,Some(false),Some(Status::Off)).is_err());
        assert!(validate_change(Status::Off,Some(true),Some(Status::Off)).is_ok());
        assert!(validate_change(Status::DisabledByWindows,Some(false),Some(Status::DisabledByWindows)).is_ok());
    }
    #[test]
    fn msix_requires_a_regular_marker_and_package_identity() {
        let directory = tempfile::tempdir().unwrap();
        let exe = directory.path().join("luciddesk.exe");
        assert!(!msix(&exe, false).unwrap());
        assert!(msix(&exe, true).is_err());
        let marker = directory.path().join("msix");
        std::fs::write(&marker, []).unwrap();
        assert!(msix(&exe, true).unwrap());
        assert!(!msix(&exe, false).unwrap());
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(msix(&exe, true).is_err());
        assert!(msix(&exe, false).is_err());
    }
    #[test]
    fn package_state_mapping_respects_user_and_policy_controls() {
        for (state, status, editable, selected) in [
            (StartupTaskState::Disabled, Status::Off, true, false),
            (StartupTaskState::Enabled, Status::Enabled, true, true),
            (
                StartupTaskState::DisabledByUser,
                Status::DisabledByUser,
                false,
                false,
            ),
            (
                StartupTaskState::DisabledByPolicy,
                Status::DisabledByPolicy,
                false,
                false,
            ),
            (
                StartupTaskState::EnabledByPolicy,
                Status::EnabledByPolicy,
                false,
                true,
            ),
        ] {
            assert_eq!(task_status(state), status);
            assert_eq!(status.editable(), editable);
            assert_eq!(status.registered(), selected);
        }
        assert_eq!(task_status(StartupTaskState(99)), Status::Unknown);
        assert!(!Status::Unknown.editable());
        assert!(!Status::Unknown.registered());
    }
    #[test]
    fn approval_handles_disabled_and_unknown_without_guessing() {
        assert_eq!(approval(None), Status::Enabled);
        for (value, expected) in [
            (2_u32, Status::Enabled),
            (6, Status::Enabled),
            (3, Status::DisabledByWindows),
            (7, Status::DisabledByWindows),
            (99, Status::Unknown),
        ] {
            let mut blob = [0; 12];
            blob[..4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(approval(Some(&blob)), expected);
        }
        assert_eq!(approval(Some(&[2])), Status::Unknown);
    }
    #[test]
    fn command_quotes_spaces_and_rejects_invalid_or_long_paths() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\LucidDesk\luciddesk.exe")).unwrap(),
            r#""C:\Program Files\LucidDesk\luciddesk.exe" --startup"#
        );
        assert!(command(Path::new("luciddesk.exe")).is_err());
        assert!(
            command(Path::new(&format!(
                "C:\\{}\\luciddesk.exe",
                "a".repeat(260)
            )))
            .is_err()
        );
    }
    // Isolated HKCU fixture only; never creates a real Run entry or starts an app.
    #[test]
    fn registry_round_trip_preserves_system_approval_and_other_installations() {
        let path = format!(r"Software\LucidDesk.Tests\Startup-{}", std::process::id());
        let root = CURRENT_USER.create(&path).unwrap();
        let exe = Path::new(r"C:\LucidDesk Test\luciddesk.exe");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_eq!(query(&root, "Run", "Approved", exe).unwrap(), Status::Off);
            update(&root, "Run", exe, true).unwrap();
            assert_eq!(
                query(&root, "Run", "Approved", exe).unwrap(),
                Status::Enabled
            );
            let approved = root.create("Approved").unwrap();
            let mut blob = [0; 12];
            blob[0] = 3;
            blob[4] = 42;
            approved
                .set_bytes(NAME, windows_registry::Type::Bytes, &blob)
                .unwrap();
            assert_eq!(
                query(&root, "Run", "Approved", exe).unwrap(),
                Status::DisabledByWindows
            );
            update(&root, "Run", exe, true).unwrap();
            assert_eq!(approved.get_bytes(NAME).unwrap(), blob);
            approved.set_string(NAME, "unexpected format").unwrap();
            assert_eq!(
                query(&root, "Run", "Approved", exe).unwrap(),
                Status::Unknown
            );
            approved
                .set_bytes(NAME, windows_registry::Type::Bytes, &blob)
                .unwrap();
            let other = Path::new(r"C:\Other\luciddesk.exe");
            assert_eq!(
                query(&root, "Run", "Approved", other).unwrap(),
                Status::OtherLocation
            );
            assert!(update(&root, "Run", other, false).is_err());
            assert!(update(&root, "Run", other, true).is_err());
            update(&root, "Run", exe, false).unwrap();
            assert_eq!(query(&root, "Run", "Approved", exe).unwrap(), Status::Off);
            assert_eq!(approved.get_bytes(NAME).unwrap(), blob);
        }));
        drop(root);
        CURRENT_USER.remove_tree(&path).unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }
}
