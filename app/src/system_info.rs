//! Cached system and build information for diagnostic reports.
use std::sync::LazyLock;
use windows_version::OsVersion;

pub struct SystemInfo {
    version: OsVersion,
    revision: u32,
    server: bool,
}
static SYSTEM: LazyLock<SystemInfo> = LazyLock::new(|| SystemInfo {
    version: OsVersion::current(),
    revision: windows_version::revision(),
    server: windows_version::is_server(),
});
pub fn system() -> &'static SystemInfo {
    &SYSTEM
}
impl SystemInfo {
    pub fn summary(&self) -> String {
        if self.version.major == 0 {
            return "Windows 未获取版本".into();
        }
        let v = self.version;
        let product = if self.server {
            "Windows Server"
        } else {
            "Windows"
        };
        let suffix = if self.revision == 0 {
            " · 修订号未确认".into()
        } else {
            format!(".{}", self.revision)
        };
        format!("{product} {}.{}.{}{suffix}", v.major, v.minor, v.build)
    }
    fn report(&self) -> String {
        format!(
            "LucidDesk {}\r\nBuild: {}\r\nProcess architecture: {}\r\n{}\r\nService pack: {}\r\n",
            env!("CARGO_PKG_VERSION"),
            env!("LUCIDDESK_BUILD_REVISION"),
            std::env::consts::ARCH,
            self.summary(),
            self.version.pack
        )
    }
}
pub fn report() -> String {
    system().report()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_preserves_build_and_does_not_guess_windows_product_names() {
        let info = SystemInfo {
            version: OsVersion::new(10, 0, 0, 26100),
            revision: 1234,
            server: false,
        };
        assert_eq!(info.summary(), "Windows 10.0.26100.1234");
        assert!(info.report().contains(env!("LUCIDDESK_BUILD_REVISION")));
        let server = SystemInfo {
            server: true,
            ..info
        };
        assert_eq!(server.summary(), "Windows Server 10.0.26100.1234");
    }
    #[test]
    fn unknown_revision_is_not_reported_as_confirmed_zero() {
        let info = SystemInfo {
            version: OsVersion::new(10, 0, 0, 26100),
            revision: 0,
            server: false,
        };
        assert!(!info.summary().contains("26100.0"));
        assert!(info.summary().contains("修订号未确认"));
    }
}
