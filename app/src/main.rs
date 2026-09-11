#![windows_subsystem = "windows"]

mod hook_runtime;
mod pane;
mod tray;

use desktop_shell::{KnownFolder, ShellApartment, known_folder_path};
use std::{ffi::OsString, fs, path::PathBuf};

fn main() -> Result<(), String> {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let _apartment = ShellApartment::initialize_sta().map_err(|e| e.to_string())?;
    let title = parse_options(arguments)?;
    let path = database_path()?.with_file_name("hook-desktop.db");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    pane::run(&path, title).inspect_err(|error| desktop_window::NativeFrame::show_error(error))
}

// Keep the old explicit hybrid flag compatible with existing shortcuts.
fn parse_options(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<String>, String> {
    let mut arguments = arguments.into_iter();
    let mut title = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--hybrid-desktop") => {}
            Some("--title") => {
                title = Some(
                    arguments
                        .next()
                        .ok_or("--title requires a value")?
                        .into_string()
                        .map_err(|_| "title must be valid Unicode")?,
                );
            }
            _ => {
                return Err(format!(
                    "Unsupported argument {}. Other modes are available in lucidpane-legacy.exe.",
                    argument.to_string_lossy()
                ));
            }
        }
    }
    Ok(title)
}

fn database_path() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("LUCIDPANE_DATA_DIR") {
        return Ok(PathBuf::from(root).join("lucidpane.db"));
    }
    let root = known_folder_path(KnownFolder::LocalAppData)
        .map_err(|error| format!("failed to resolve LocalAppData: {error}"))?;
    Ok(root.join("LucidPane").join("lucidpane.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hybrid_is_the_only_launch_mode() {
        assert_eq!(parse_options([]).unwrap(), None);
        assert_eq!(
            parse_options([OsString::from("--hybrid-desktop")]).unwrap(),
            None
        );
        assert_eq!(
            parse_options([OsString::from("--title"), OsString::from("Work")])
                .unwrap()
                .as_deref(),
            Some("Work")
        );
        for value in [
            "--preview",
            "--desktop",
            "--managed-desktop",
            "--native-desktop",
            "--hook-desktop",
            "--manual",
            "--icon",
            "C:\\folder",
            "--unknown",
            "--title",
        ] {
            assert!(parse_options([OsString::from(value)]).is_err(), "{value}");
        }
    }
}
