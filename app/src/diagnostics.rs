//! Cached operating-system facts for support reports; never used as a capability gate.
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
            "LucidPane {}\r\nBuild: {}\r\nProcess architecture: {}\r\n{}\r\nService pack: {}\r\n",
            env!("CARGO_PKG_VERSION"),
            env!("LUCIDPANE_BUILD_REVISION"),
            std::env::consts::ARCH,
            self.summary(),
            self.version.pack
        )
    }
}
pub fn report() -> String {
    system().report()
}

pub fn copy(owner: isize, text: &str) -> windows::core::Result<()> {
    use windows::Win32::{
        Foundation::*,
        System::{DataExchange::*, Memory::*},
    };
    struct Memory(HGLOBAL);
    impl Drop for Memory {
        fn drop(&mut self) {
            unsafe {
                let _ = GlobalFree(Some(self.0));
            }
        }
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let memory = Memory(GlobalAlloc(GMEM_MOVEABLE, wide.len() * size_of::<u16>())?);
        let target = GlobalLock(memory.0).cast::<u16>();
        if target.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        let _ = GlobalUnlock(memory.0);
        OpenClipboard(Some(HWND(owner as _)))?;
        let _clipboard = Clipboard;
        EmptyClipboard()?;
        SetClipboardData(13, Some(HANDLE(memory.0.0)))?; // CF_UNICODETEXT
        std::mem::forget(memory); // Clipboard owns the allocation after success.
    }
    Ok(())
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
        assert!(info.report().contains(env!("LUCIDPANE_BUILD_REVISION")));
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
