// Shared Explorer recovery and restore-helper protocol.
const RESTORE_GUARD_OPTION: &str = "--desktop-restore-guard";
const RESTORE_SHELL_OPTION: &str = "--restore-shell";
const TAKEOVER_MARKER_FILE: &str = "shell-takeover.state";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct ManagedDesktopLease {
    original_hidden: bool,
    changed: bool,
    marker_path: Option<PathBuf>,
}

impl ManagedDesktopLease {
    fn acquire(marker_path: PathBuf) -> Result<Self, String> {
        let original_hidden = desktop_icons_hidden();
        if original_hidden {
            return Ok(Self {
                original_hidden,
                changed: false,
                marker_path: None,
            });
        }

        let marker = ShellTakeoverMarker {
            process_id: std::process::id(),
            original_hidden,
        };
        write_shell_takeover_marker(&marker_path, marker)?;
        if let Err(error) = spawn_restore_guard(original_hidden, &marker_path) {
            let _ = remove_shell_takeover_marker(&marker_path);
            return Err(error);
        }
        if let Err(error) = set_desktop_icons_hidden(true) {
            let _ = remove_shell_takeover_marker(&marker_path);
            return Err(format!("failed to hide Explorer desktop icons: {error}"));
        }
        Ok(Self {
            original_hidden,
            changed: true,
            marker_path: Some(marker_path),
        })
    }
}

impl Drop for ManagedDesktopLease {
    fn drop(&mut self) {
        if self.changed {
            match set_desktop_icons_hidden(self.original_hidden) {
                Ok(()) => {
                    if let Some(marker_path) = self.marker_path.as_deref()
                        && let Err(error) = remove_shell_takeover_marker(marker_path)
                    {
                        eprintln!("failed to remove desktop takeover marker: {error}");
                    }
                }
                Err(error) => eprintln!("failed to restore Explorer desktop icons: {error}"),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShellTakeoverMarker {
    process_id: u32,
    original_hidden: bool,
}

impl ShellTakeoverMarker {
    fn encode(self) -> String {
        format!(
            "version=1\npid={}\noriginal_hidden={}\n",
            self.process_id,
            u8::from(self.original_hidden)
        )
    }

    fn decode(value: &str) -> Result<Self, String> {
        let mut version = None;
        let mut process_id = None;
        let mut original_hidden = None;
        for line in value.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err("invalid desktop takeover marker line".to_string());
            };
            match key {
                "version" => version = Some(value),
                "pid" => {
                    process_id = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| "invalid desktop takeover process id".to_string())?,
                    );
                }
                "original_hidden" => {
                    original_hidden = Some(match value {
                        "0" => false,
                        "1" => true,
                        _ => return Err("invalid desktop takeover visibility state".to_string()),
                    });
                }
                _ => return Err(format!("unknown desktop takeover marker key: {key}")),
            }
        }
        if version != Some("1") {
            return Err("unsupported desktop takeover marker version".to_string());
        }
        Ok(Self {
            process_id: process_id
                .ok_or_else(|| "desktop takeover marker is missing its process id".to_string())?,
            original_hidden: original_hidden.ok_or_else(|| {
                "desktop takeover marker is missing its visibility state".to_string()
            })?,
        })
    }
}

fn shell_takeover_marker_path(database_path: &Path) -> PathBuf {
    database_path.with_file_name(TAKEOVER_MARKER_FILE)
}

fn write_shell_takeover_marker(
    marker_path: &Path,
    marker: ShellTakeoverMarker,
) -> Result<(), String> {
    let temporary_path = marker_path.with_extension("state.tmp");
    if temporary_path.exists() {
        fs::remove_file(&temporary_path).map_err(|error| {
            format!(
                "failed to replace stale desktop takeover marker {}: {error}",
                temporary_path.display()
            )
        })?;
    }
    let mut file = fs::File::create(&temporary_path).map_err(|error| {
        format!(
            "failed to create desktop takeover marker {}: {error}",
            temporary_path.display()
        )
    })?;
    file.write_all(marker.encode().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("failed to persist desktop takeover marker: {error}"))?;
    fs::rename(&temporary_path, marker_path).map_err(|error| {
        format!(
            "failed to publish desktop takeover marker {}: {error}",
            marker_path.display()
        )
    })
}

fn read_shell_takeover_marker(marker_path: &Path) -> Result<ShellTakeoverMarker, String> {
    let value = fs::read_to_string(marker_path).map_err(|error| {
        format!(
            "failed to read desktop takeover marker {}: {error}",
            marker_path.display()
        )
    })?;
    ShellTakeoverMarker::decode(&value)
}

fn remove_shell_takeover_marker(marker_path: &Path) -> Result<(), String> {
    match fs::remove_file(marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove desktop takeover marker {}: {error}",
            marker_path.display()
        )),
    }
}

fn restore_shell_takeover(marker_path: &Path) -> Result<bool, String> {
    if !marker_path.exists() {
        return Ok(false);
    }
    let marker = read_shell_takeover_marker(marker_path)?;
    set_desktop_icons_hidden(marker.original_hidden)
        .map_err(|error| format!("failed to restore Explorer desktop icons: {error}"))?;
    remove_shell_takeover_marker(marker_path)?;
    Ok(true)
}

fn recover_stale_shell_takeover(marker_path: &Path) -> Result<(), String> {
    if !marker_path.exists() {
        return Ok(());
    }
    let marker = read_shell_takeover_marker(marker_path).map_err(|error| {
        format!("{error}; run LucidPane with {RESTORE_SHELL_OPTION} to force recovery")
    })?;
    if let Ok(waiter) = ProcessExitWaiter::open(marker.process_id)
        && !waiter
            .has_exited()
            .map_err(|error| format!("failed to inspect the active LucidPane process: {error}"))?
    {
        return Err(format!(
            "Managed Desktop Mode is already active in process {}; close it before starting another instance",
            marker.process_id
        ));
    }
    restore_shell_takeover(marker_path)?;
    Ok(())
}

fn run_restore_shell_if_requested(arguments: &[OsString]) -> Result<bool, String> {
    if arguments.first().and_then(|argument| argument.to_str()) != Some(RESTORE_SHELL_OPTION) {
        return Ok(false);
    }
    if arguments.len() != 1 {
        return Err(format!("{RESTORE_SHELL_OPTION} does not accept arguments"));
    }
    let database_path = database_path()?;
    let marker_path = shell_takeover_marker_path(&database_path);
    if marker_path.exists() {
        match restore_shell_takeover(&marker_path) {
            Ok(true) => {}
            Ok(false) => unreachable!("the takeover marker was checked above"),
            Err(error) => {
                set_desktop_icons_hidden(false).map_err(|restore_error| {
                    format!("{error}; forced Explorer recovery also failed: {restore_error}")
                })?;
                remove_shell_takeover_marker(&marker_path)?;
            }
        }
    } else {
        set_desktop_icons_hidden(false)
            .map_err(|error| format!("failed to show Explorer desktop icons: {error}"))?;
    }
    Ok(true)
}

fn spawn_restore_guard(original_hidden: bool, marker_path: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate the desktop restore helper: {error}"))?;
    let mut child = Command::new(executable)
        .arg(RESTORE_GUARD_OPTION)
        .arg(std::process::id().to_string())
        .arg(if original_hidden { "hidden" } else { "visible" })
        .arg(marker_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("failed to start the desktop restore helper: {error}"))?;
    let mut ready = [0_u8; 6];
    child
        .stdout
        .take()
        .ok_or_else(|| "desktop restore helper did not expose its handshake".to_string())?
        .read_exact(&mut ready)
        .map_err(|error| format!("desktop restore helper did not become ready: {error}"))?;
    if ready != *b"ready\n" {
        return Err("desktop restore helper returned an invalid handshake".to_string());
    }
    Ok(())
}

fn run_restore_guard_if_requested(arguments: &[OsString]) -> Result<bool, String> {
    if arguments.first().and_then(|argument| argument.to_str()) != Some(RESTORE_GUARD_OPTION) {
        return Ok(false);
    }
    if arguments.len() != 4 {
        return Err("invalid desktop restore helper arguments".to_string());
    }
    let process_id = arguments[1]
        .to_string_lossy()
        .parse::<u32>()
        .map_err(|_| "invalid parent process id for desktop restore helper".to_string())?;
    let original_hidden = match arguments[2].to_str() {
        Some("hidden") => true,
        Some("visible") => false,
        _ => return Err("invalid restore state for desktop restore helper".to_string()),
    };
    let marker_path = PathBuf::from(&arguments[3]);
    let waiter = ProcessExitWaiter::open(process_id)
        .map_err(|error| format!("failed to watch the LucidPane process: {error}"))?;
    std::io::stdout()
        .write_all(b"ready\n")
        .and_then(|()| std::io::stdout().flush())
        .map_err(|error| format!("failed to signal desktop restore readiness: {error}"))?;
    waiter
        .wait()
        .map_err(|error| format!("failed while waiting to restore the desktop: {error}"))?;
    set_desktop_icons_hidden(original_hidden)
        .map_err(|error| format!("failed to restore Explorer desktop icons: {error}"))?;
    remove_shell_takeover_marker(&marker_path)?;
    Ok(true)
}
