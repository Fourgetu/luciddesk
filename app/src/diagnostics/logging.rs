//! Lazy, bounded diagnostic files. No heartbeat or database writes.
use std::{
    collections::VecDeque,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
impl Level {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "warn" => Self::Warn,
            "info" => Self::Info,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Error,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}
struct Recent {
    level: Level,
    component: String,
    message: String,
    at: Instant,
    suppressed: u64,
}
struct Logger {
    path: PathBuf,
    level: Level,
    recent: VecDeque<Recent>,
}
impl Logger {
    fn new(database: &Path, level: Level) -> Self {
        Self {
            path: database.with_file_name("logs").join("diagnostic.log"),
            level,
            recent: VecDeque::new(),
        }
    }
    fn write(
        &mut self,
        level: Level,
        component: &str,
        message: &str,
        now: Instant,
    ) -> io::Result<()> {
        if level > self.level {
            return Ok(());
        }
        let message: String = message.chars().take(16_384).collect();
        let component: String = component.chars().take(128).collect();
        let index = self
            .recent
            .iter()
            .position(|r| r.level == level && r.component == component && r.message == message);
        let mut suppressed = 0;
        if let Some(index) = index {
            let recent = &mut self.recent[index];
            if now.duration_since(recent.at) < Duration::from_secs(30) {
                recent.suppressed += 1;
                return Ok(());
            }
            suppressed = recent.suppressed;
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Debug escaping keeps each entry one line even for multiline or hostile messages.
        let entry = format!(
            "timestamp_unix_ms={time} level={} pid={} version={} build={} component={component:?} suppressed={suppressed} message={message:?}\n",
            level.label(),
            std::process::id(),
            env!("CARGO_PKG_VERSION"),
            env!("LUCIDDESK_BUILD_REVISION")
        );
        let parent = self.path.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        let length = match std::fs::metadata(&self.path) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        let header = format!("# {:?}\n", super::report());
        let rotate =
            length + entry.len() as u64 + if length == 0 { header.len() as u64 } else { 0 }
                > 256 * 1024;
        if rotate && length > 0 {
            let previous = parent.join("diagnostic.previous.log");
            match std::fs::remove_file(&previous) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
            std::fs::rename(&self.path, previous)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        if length == 0 || rotate {
            file.write_all(header.as_bytes())?;
        }
        file.write_all(entry.as_bytes())?;
        file.flush()?;
        if let Some(index) = index {
            self.recent.remove(index);
        }
        if self.recent.len() == 64 {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            level,
            component,
            message,
            at: now,
            suppressed: 0,
        });
        Ok(())
    }
}
static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();
pub fn init_logging(database: &Path) {
    LOGGER.get_or_init(|| {
        Mutex::new(Logger::new(
            database,
            Level::Error,
        ))
    });
}
pub fn level() -> Level {
    LOGGER.get().and_then(|logger| logger.lock().ok().map(|logger| logger.level)).unwrap_or(Level::Error)
}
pub fn set_level(level: Level) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut logger) = logger.lock() {
            if logger.level != level {
                logger.level = level;
                logger.recent.clear();
            }
        }
    }
}
pub fn log(level: Level, component: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut logger) = logger.lock() {
            if let Err(error) = logger.write(level, component, message, Instant::now()) {
                eprintln!("ERROR diagnostics: {error}; original={message:?}");
            }
        }
    } else if level == Level::Error {
        eprintln!("ERROR {component}: {message}");
    }
}
pub fn desktop_connection_log(database: &Path, error: Option<&str>) -> io::Result<PathBuf> {
    init_logging(database);
    let mut logger = LOGGER
        .get()
        .unwrap()
        .lock()
        .map_err(|_| io::Error::other("diagnostic logger poisoned"))?;
    logger.write(
        if error.is_some() {
            Level::Error
        } else {
            Level::Info
        },
        "desktop.connection",
        error.unwrap_or("connection recovered"),
        Instant::now(),
    )?;
    Ok(logger.path.clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_error_filters_before_any_file_io() {
        let dir = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(&dir.path().join("workspace.db"), Level::parse(""));
        for level in [Level::Warn, Level::Info, Level::Debug, Level::Trace] {
            logger
                .write(level, "test", "filtered", Instant::now())
                .unwrap();
        }
        assert!(!dir.path().join("logs").exists());
        logger
            .write(Level::Error, "test", "failure", Instant::now())
            .unwrap();
        assert!(
            std::fs::read_to_string(&logger.path)
                .unwrap()
                .contains("level=ERROR")
        );
        assert!(!dir.path().join("workspace.db").exists());
        assert_eq!(Level::parse("typo"), Level::Error);
        assert_eq!(Level::parse(" DEBUG "), Level::Debug);
    }
    #[test]
    fn duplicate_errors_are_suppressed_and_files_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let mut logger = Logger::new(&dir.path().join("workspace.db"), Level::Info);
        let now = Instant::now();
        logger
            .write(Level::Error, "test", "failure\nnext", now)
            .unwrap();
        let before = std::fs::read(&logger.path).unwrap();
        for _ in 0..20 {
            logger
                .write(Level::Error, "test", "failure\nnext", now)
                .unwrap();
        }
        assert_eq!(std::fs::read(&logger.path).unwrap(), before);
        logger
            .write(
                Level::Error,
                "test",
                "failure\nnext",
                now + Duration::from_secs(31),
            )
            .unwrap();
        assert!(
            std::fs::read_to_string(&logger.path)
                .unwrap()
                .contains("suppressed=20")
        );
        std::fs::write(&logger.path, vec![b'x'; 256 * 1024]).unwrap();
        logger
            .write(Level::Info, "desktop.connection", "recovered", now)
            .unwrap();
        assert!(
            logger
                .path
                .with_file_name("diagnostic.previous.log")
                .exists()
        );
        assert!(
            std::fs::read_to_string(&logger.path)
                .unwrap()
                .contains("level=INFO")
        );
    }
}
