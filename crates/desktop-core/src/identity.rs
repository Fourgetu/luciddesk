//! Stable panel, monitor, and Shell identifiers.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelId(u64);

impl PanelId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShellIdentity {
    FileSystem {
        path: PathBuf,
        volume_id: Option<u64>,
        file_id: Option<u128>,
    },
    Namespace {
        parsing_name: String,
    },
}

impl ShellIdentity {
    #[must_use]
    pub fn persistent_key(&self) -> String {
        match self {
            Self::FileSystem {
                path,
                volume_id,
                file_id,
            } => match (volume_id, file_id) {
                (Some(volume_id), Some(file_id)) => {
                    format!("fsid:{volume_id:016x}:{file_id:032x}")
                }
                _ => format!("fs:{}", path.as_os_str().to_string_lossy().to_lowercase()),
            },
            Self::Namespace { parsing_name } => {
                format!("shell:{}", parsing_name.to_lowercase())
            }
        }
    }

    /// Compares two Shell identities using file IDs when available and paths as a migration
    /// fallback for records created before stable file identity was captured.
    #[must_use]
    pub fn equivalent_to(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::FileSystem {
                    path: left_path,
                    volume_id: left_volume,
                    file_id: left_file,
                },
                Self::FileSystem {
                    path: right_path,
                    volume_id: right_volume,
                    file_id: right_file,
                },
            ) => {
                let stable_match = match (left_volume, left_file, right_volume, right_file) {
                    (Some(left_volume), Some(left_file), Some(right_volume), Some(right_file)) => {
                        left_volume == right_volume && left_file == right_file
                    }
                    _ => false,
                };
                stable_match || path_key(left_path) == path_key(right_path)
            }
            (
                Self::Namespace { parsing_name: left },
                Self::Namespace {
                    parsing_name: right,
                },
            ) => left.eq_ignore_ascii_case(right),
            _ => false,
        }
    }

    #[must_use]
    pub fn file_system_path(&self) -> Option<&Path> {
        match self {
            Self::FileSystem { path, .. } => Some(path),
            Self::Namespace { .. } => None,
        }
    }

    #[must_use]
    pub fn activation_name(&self) -> &OsStr {
        match self {
            Self::FileSystem { path, .. } => path.as_os_str(),
            Self::Namespace { parsing_name } => OsStr::new(parsing_name),
        }
    }
}

fn path_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonitorId(String);

impl MonitorId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the underlying identifier without allocating or copying it.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<u64> for PanelId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<PanelId> for u64 {
    fn from(value: PanelId) -> Self {
        value.get()
    }
}

impl std::fmt::Display for PanelId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<String> for MonitorId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for MonitorId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for MonitorId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for MonitorId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Default for MonitorId {
    fn default() -> Self {
        Self::new("primary")
    }
}
