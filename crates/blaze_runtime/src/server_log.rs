//! The server log: the file naming and symlink of `SimpleLogHandler`, and
//! `DebugLoggerConfigurator.getServerLogPath`.
//!
//! Bazel's client writes `<output_base>/javalog.properties` configuring
//! `com.google.devtools.build.lib.util.SimpleLogHandler` with `prefix=<output_base>/java.log`
//! (`StartupOptions::AddJVMLoggingArguments`). The other properties it writes (`limit`,
//! `total_limit`) are not names `SimpleLogHandler` reads, so there is no rotation and no
//! deletion of old logs. The handler opens its file on the first log record, which the server
//! writes during startup; Blaze opens it in [`init`].
//!
//! PORT: only the file naming, opening and symlink creation are ported; Blaze writes no log
//! records yet (there is no `java.util.logging` equivalent), so the file stays empty.

use std::fs::{File, OpenOptions};
use std::path::{Path as StdPath, PathBuf};

use parking_lot::Mutex;

/// A log handler that writes to a file whose name is derived from a prefix, a pattern, a
/// timestamp and an extension, with a symbolic link to the current file.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:78 SimpleLogHandler
pub struct SimpleLogHandler {
    /// Log file extension; the current process ID by default.
    extension: String,
    /// Absolute path to symbolic link to current log file, or `None` if the link should not be
    /// created.
    symlink_path: Option<PathBuf>,
    /// Absolute path to common base name of log files.
    base_file_path: PathBuf,
    /// Log file currently in use.
    output: Option<(PathBuf, File)>,
    /// Source for timestamps in filenames: `None` is the system clock in the default time zone;
    /// `Some(seconds)` is a fixed instant formatted in UTC (`setClockForTesting`).
    clock: Option<i64>,
}

// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:98 SimpleLogHandler.DEFAULT_PREFIX_STRING
const DEFAULT_PREFIX_STRING: &str = "java.log";
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:99 SimpleLogHandler.DEFAULT_BASE_FILE_NAME_PATTERN
const DEFAULT_BASE_FILE_NAME_PATTERN: &str = ".%h.%u.log.java.";

/// Builder class for [`SimpleLogHandler`]. All setters are optional.
///
/// PORT: Java falls back to the `java.util.logging` configuration (`LogManager` properties) for
/// unset values; Blaze has no such configuration, so unset values use the defaults directly.
/// Only the settings that affect file naming are ported.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:145 SimpleLogHandler.Builder
#[derive(Default)]
pub struct SimpleLogHandlerBuilder {
    prefix: Option<String>,
    pattern: Option<String>,
    extension: Option<String>,
    symlink_name: Option<String>,
    create_symlink: Option<bool>,
    clock: Option<i64>,
}

impl SimpleLogHandlerBuilder {
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:158 Builder.setPrefix
    pub fn set_prefix(mut self, prefix: &str) -> Self {
        self.prefix = Some(prefix.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:184 Builder.setPattern
    pub fn set_pattern(mut self, pattern: &str) -> Self {
        self.pattern = Some(pattern.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:199 Builder.setExtension
    pub fn set_extension(mut self, extension: &str) -> Self {
        self.extension = Some(extension.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:215 Builder.setSymlinkName
    pub fn set_symlink_name(mut self, symlink_name: &str) -> Self {
        self.symlink_name = Some(symlink_name.to_string());
        self
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:231 Builder.setCreateSymlink
    pub fn set_create_symlink(mut self, create_symlink: bool) -> Self {
        self.create_symlink = Some(create_symlink);
        self
    }

    /// A fixed instant (seconds since the epoch), formatted in UTC.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:311 Builder.setClockForTesting
    pub fn set_clock_for_testing(mut self, epoch_seconds: i64) -> Self {
        self.clock = Some(epoch_seconds);
        self
    }

    /// Panics (Java `IllegalArgumentException`) on an unknown pattern variable or a symlink name
    /// that is not a top-level file in the log directory.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:317 Builder.build
    pub fn build(self) -> SimpleLogHandler {
        SimpleLogHandler::new(self)
    }
}

impl SimpleLogHandler {
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:135 SimpleLogHandler.builder
    pub fn builder() -> SimpleLogHandlerBuilder {
        SimpleLogHandlerBuilder::default()
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:339 SimpleLogHandler.<init>
    fn new(b: SimpleLogHandlerBuilder) -> SimpleLogHandler {
        let prefix = b
            .prefix
            .as_deref()
            .unwrap_or(DEFAULT_PREFIX_STRING)
            .to_string();
        let base_file_path = Self::get_base_file_path(
            &prefix,
            b.pattern
                .as_deref()
                .unwrap_or(DEFAULT_BASE_FILE_NAME_PATTERN),
        );
        let configured_symlink_name = b.symlink_name.clone().unwrap_or_else(|| prefix.clone());
        // OS.getCurrent() != OS.WINDOWS
        let configured_create_symlink = b.create_symlink.unwrap_or(true);
        let symlink_path = if configured_create_symlink {
            Some(Self::get_symlink_absolute_path(
                base_file_path.parent().expect("log directory"),
                &configured_symlink_name,
            ))
        } else {
            None
        };
        let extension = b
            .extension
            .clone()
            .unwrap_or_else(|| std::process::id().to_string());
        SimpleLogHandler {
            extension,
            symlink_path,
            base_file_path,
            output: None,
            clock: b.clock,
        }
    }

    /// Returns the absolute path of the current log file, or `None` if the log is not open.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:405 SimpleLogHandler.getCurrentLogFilePath
    pub fn get_current_log_file_path(&self) -> Option<PathBuf> {
        self.output.as_ref().map(|(p, _)| p.clone())
    }

    /// Returns the expected absolute path for the symbolic link to the current log file, or
    /// `None` if not used.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:413 SimpleLogHandler.getSymbolicLinkPath
    pub fn get_symbolic_link_path(&self) -> Option<&PathBuf> {
        self.symlink_path.as_ref()
    }

    /// Closes the current log file (`LogManager.reset()` closes all handlers).
    /// PORT: the formatter tail is empty for `SingleLineFormatter`; nothing is written.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:479 SimpleLogHandler.close
    pub fn close(&mut self) {
        self.output = None;
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:642 SimpleLogHandler.getLocalHostnameFirstComponent
    pub fn get_local_hostname_first_component() -> String {
        let mut name = get_cached_short_host_name();
        if name.parse::<std::net::IpAddr>().is_err() {
            // Keep only the first component of the name.
            if let Some(first_dot) = name.find('.') {
                name.truncate(first_dot);
            }
        }
        // PORT: StringEncoding identity; Locale.ROOT lowercasing.
        name.to_lowercase()
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:363 SimpleLogHandler.getBaseFilePath
    fn get_base_file_path(prefix: &str, pattern: &str) -> PathBuf {
        let mut sb = String::with_capacity(100); // Typical name is < 100 bytes
        let mut in_var = false;
        let mut username = user_name();
        if username.is_empty() {
            username = "unknown_user".to_string();
        }

        sb.push_str(prefix);

        for c in pattern.chars() {
            if in_var {
                in_var = false;
                match c {
                    '%' => sb.push('%'),
                    'h' => sb.push_str(&Self::get_local_hostname_first_component()),
                    'u' => sb.push_str(&username),
                    _ => panic!(
                        "IllegalArgumentException: Unknown variable {} in {}",
                        c, pattern
                    ),
                }
            } else if c == '%' {
                in_var = true;
            } else {
                sb.push(c);
            }
        }

        // Path.of(...).toAbsolutePath()
        let p = PathBuf::from(sb);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir().expect("getcwd").join(p)
        }
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:378 SimpleLogHandler.getSymlinkAbsolutePath
    fn get_symlink_absolute_path(log_dir: &StdPath, symlink: &str) -> PathBuf {
        blaze_util::check_argument!(!symlink.is_empty());
        let mut symlink_path = PathBuf::from(symlink);
        if !symlink_path.is_absolute() {
            symlink_path = log_dir.join(symlink_path);
        }
        blaze_util::check_argument!(
            symlink_path.parent() == Some(log_dir),
            "symlink is not a top-level file in logDir"
        );
        symlink_path
    }

    /// Opens the log file (and creates the symlink) if it is not open yet. Failures are reported
    /// on stderr and are not fatal (Java `reportError` through the default `ErrorManager`).
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:433 SimpleLogHandler.openOutputIfNeeded
    pub fn open_output_if_needed(&mut self) {
        if self.output.is_some() {
            return;
        }
        // Ensure the log file's directory exists.
        blaze_util::check_state!(self.base_file_path.is_absolute());
        if let Some(parent) = self.base_file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let path = PathBuf::from(format!(
            "{}{}{}",
            self.base_file_path.display(),
            format_timestamp(self.clock),
            self.extension
        ));
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => self.output = Some((path.clone(), file)),
            Err(e) => {
                report_error("Failed to open log file", &e);
                return;
            }
        }

        // Try to create relative symlink from currentLogFile to baseFile, but don't treat a
        // failure as fatal.
        if let Some(symlink_path) = &self.symlink_path {
            blaze_util::check_state!(symlink_path.parent() == path.parent());
            let result = (|| -> std::io::Result<()> {
                if std::fs::symlink_metadata(symlink_path).is_ok() {
                    std::fs::remove_file(symlink_path)?;
                }
                std::os::unix::fs::symlink(path.file_name().expect("file name"), symlink_path)
            })();
            if let Err(e) = result {
                report_error("Failed to create symbolic link to log file", &e);
            }
        }
    }
}

/// `ErrorManager.error`: prints to stderr.
fn report_error(message: &str, e: &std::io::Error) {
    eprintln!("java.util.logging.ErrorManager: {}: {}", message, e);
}

/// `new SimpleDateFormat("yyyyMMdd-HHmmss.").format(...)` in the JVM's default time zone (the
/// one `localtime_r` uses: `TZ`, else `/etc/localtime`), or in UTC for a fixed test clock.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:103 SimpleLogHandler.DEFAULT_TIMESTAMP_FORMAT
fn format_timestamp(clock: Option<i64>) -> String {
    let (secs, utc) = match clock {
        Some(s) => (s, true),
        None => (
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            false,
        ),
    };
    let t = secs as libc::time_t;
    // SAFETY: tm is plain data; the *_r functions write into it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if utc {
            libc::gmtime_r(&t, &mut tm);
        } else {
            libc::localtime_r(&t, &mut tm);
        }
        tm
    };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}.",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// `System.getProperty("user.name")`: the JVM sets it from `getpwuid(getuid())->pw_name`, or "?"
/// when there is no passwd entry.
fn user_name() -> String {
    // SAFETY: getpwuid_r writes into the provided buffers; result is checked before use.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16384];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            libc::getuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        );
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return "?".to_string();
        }
        std::ffi::CStr::from_ptr(pwd.pw_name)
            .to_string_lossy()
            .into_owned()
    }
}

/// `NetUtil.getCachedShortHostName()`: `InetAddress.getLocalHost().getHostName()`, i.e. the name
/// from `gethostname(2)` if the name service can resolve it, else "unknown".
// Java: com/google/devtools/build/lib/util/NetUtil.java:39 NetUtil.getCachedShortHostName
fn get_cached_short_host_name() -> String {
    static HOSTNAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOSTNAME.get_or_init(compute_short_host_name).clone()
}

// Java: com/google/devtools/build/lib/util/NetUtil.java:67 NetUtil.computeShortHostName
fn compute_short_host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most buf.len() bytes.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "unknown".to_string();
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let local = String::from_utf8_lossy(&buf[..len]).into_owned();
    // InetAddress.getLocalHost() resolves the name and throws UnknownHostException when it
    // cannot ("localhost" is special-cased to the loopback address).
    if local == "localhost" {
        return local;
    }
    use std::net::ToSocketAddrs;
    match (local.as_str(), 0u16).to_socket_addrs() {
        Ok(_) => local,
        Err(_) => "unknown".to_string(),
    }
}

/// The server's log handler (the root logger's `SimpleLogHandler`).
static SERVER_LOG_HANDLER: Mutex<Option<SimpleLogHandler>> = Mutex::new(None);

/// Configures the server log as the client's `javalog.properties` does
/// (`prefix=<output_base>/java.log`) and opens it, creating the `java.log` symlink.
pub fn init(output_base: &blaze_vfs::Path) {
    let mut handler = SimpleLogHandler::builder()
        .set_prefix(output_base.get_relative_str("java.log").get_path_string())
        .build();
    handler.open_output_if_needed();
    *SERVER_LOG_HANDLER.lock() = Some(handler);
}

/// `LogManager.getLogManager().reset()`: closes the server log.
pub fn close() {
    if let Some(handler) = SERVER_LOG_HANDLER.lock().as_mut() {
        handler.close();
    }
}

/// Returns the path to the server log, or `None` if the log is not yet open.
///
/// PORT: Java finds the handler through `LogHandlerQuerier`, which throws `IOException` when no
/// handler can be queried; Blaze's handler is always a `SimpleLogHandler`, and a missing one
/// (no [`init`]) means the log is not open.
// Java: com/google/devtools/build/lib/util/DebugLoggerConfigurator.java:44 DebugLoggerConfigurator.getServerLogPath
pub fn get_server_log_path() -> std::io::Result<Option<String>> {
    Ok(SERVER_LOG_HANDLER
        .lock()
        .as_ref()
        .and_then(|h| h.get_current_log_file_path())
        .map(|p| p.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/util/SimpleLogHandlerTest.java
    #[test]
    fn test_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = format!("{}/hello_world_%u%h%%_", tmp.path().display());
        let mut handler = SimpleLogHandler::builder().set_prefix(&prefix).build();
        handler.open_output_if_needed(); // To open the log file.
        assert!(
            handler
                .get_current_log_file_path()
                .unwrap()
                .display()
                .to_string()
                .starts_with(&prefix)
        );
    }

    #[test]
    fn test_pattern_variables() {
        let tmp = tempfile::tempdir().unwrap();
        let mut username = user_name();
        if username.is_empty() {
            username = "unknown_user".to_string();
        }
        let hostname = SimpleLogHandler::get_local_hostname_first_component();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello_", tmp.path().display()))
            .set_pattern("world_%u%%%h_")
            .build();
        handler.open_output_if_needed();
        assert!(
            handler
                .get_current_log_file_path()
                .unwrap()
                .display()
                .to_string()
                .starts_with(&format!(
                    "{}/hello_world_{}%{}_",
                    tmp.path().display(),
                    username,
                    hostname
                ))
        );
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn test_pattern_invalid_variable() {
        SimpleLogHandler::builder().set_pattern("hello_%t").build();
    }

    #[test]
    fn test_extension_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .build();
        handler.open_output_if_needed();
        assert!(
            handler
                .get_current_log_file_path()
                .unwrap()
                .display()
                .to_string()
                .ends_with(&format!(".{}", std::process::id()))
        );
    }

    #[test]
    fn test_extension_setter() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler1 = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_extension("xyz")
            .build();
        handler1.open_output_if_needed();
        assert!(
            handler1
                .get_current_log_file_path()
                .unwrap()
                .display()
                .to_string()
                .ends_with(".xyz")
        );
    }

    #[test]
    fn test_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        // 2015-09-01T15:17:54Z
        let instant = 1441120674;
        assert_eq!(format_timestamp(Some(instant)), "20150901-151754.");
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_clock_for_testing(instant)
            .build();
        handler.open_output_if_needed();
        assert!(
            handler
                .get_current_log_file_path()
                .unwrap()
                .display()
                .to_string()
                .contains("20150901-151754.")
        );
    }

    #[test]
    fn test_symbolic_link_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let symlink_path = tmp.path().join("hello");
        std::fs::write(&symlink_path, "").unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&symlink_path.display().to_string())
            .build();
        handler.open_output_if_needed();
        assert_eq!(handler.get_symbolic_link_path(), Some(&symlink_path));
        assert!(
            std::fs::symlink_metadata(&symlink_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(&symlink_path).unwrap().as_os_str(),
            handler
                .get_current_log_file_path()
                .unwrap()
                .file_name()
                .unwrap()
        );
    }

    #[test]
    fn test_symbolic_link_setter() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_symlink_name("bye")
            .build();
        handler.open_output_if_needed();
        let link = tmp.path().join("bye");
        assert_eq!(handler.get_symbolic_link_path(), Some(&link));
        assert_eq!(
            std::fs::read_link(&link).unwrap().as_os_str(),
            handler
                .get_current_log_file_path()
                .unwrap()
                .file_name()
                .unwrap()
        );
    }

    #[test]
    fn test_symlink_enabling() {
        let tmp = tempfile::tempdir().unwrap();
        let handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_symlink_name("bye")
            .set_create_symlink(true)
            .build();
        assert!(handler.get_symbolic_link_path().is_some());
    }

    #[test]
    fn test_symlink_disabling() {
        let tmp = tempfile::tempdir().unwrap();
        let handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_symlink_name("bye")
            .set_create_symlink(false)
            .build();
        assert!(handler.get_symbolic_link_path().is_none());
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn test_symbolic_link_invalid_path() {
        let tmp = tempfile::tempdir().unwrap();
        SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_symlink_name("bye/bye")
            .set_create_symlink(true)
            .build();
    }

    #[test]
    fn test_symbolic_link_initially_invalid_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let symlink_path = tmp.path().join("hello");
        std::os::unix::fs::symlink("no-such-file", &symlink_path).unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&symlink_path.display().to_string())
            .build();
        handler.open_output_if_needed();
        assert_eq!(handler.get_symbolic_link_path(), Some(&symlink_path));
        assert_eq!(
            std::fs::read_link(&symlink_path).unwrap().as_os_str(),
            handler
                .get_current_log_file_path()
                .unwrap()
                .file_name()
                .unwrap()
        );
    }

    #[test]
    #[ignore = "needs log record publishing (no java.util.logging port)"]
    fn test_publish() {}

    #[test]
    #[ignore = "needs log levels (no java.util.logging port)"]
    fn test_log_level_equal_published() {}

    #[test]
    #[ignore = "needs log levels (no java.util.logging port)"]
    fn test_log_level_higher_published() {}

    #[test]
    #[ignore = "needs log levels (no java.util.logging port)"]
    fn test_log_level_lower_not_published() {}

    #[test]
    #[ignore = "needs log levels (no java.util.logging port)"]
    fn test_log_level_default_all_published() {}

    #[test]
    #[ignore = "needs log rotation (not used by Bazel's javalog.properties)"]
    fn test_rotate_limit_bytes() {}

    #[test]
    #[ignore = "needs log record publishing (no java.util.logging port)"]
    fn test_open_in_append_mode() {}

    #[test]
    #[ignore = "needs old-log deletion (not used by Bazel's javalog.properties)"]
    fn test_total_limit() {}
}
