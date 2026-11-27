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
//! PORT: Blaze has no `java.util.logging`. The handler's file naming, opening and symlink, the
//! `SingleLineFormatter` record format and `DebugLoggerConfigurator.setupLogging` are ported;
//! the server writes its records through [`log`] (and crates below the runtime through
//! `blaze_util::server_logger`, whose sinks [`init`] sets). Rotation and the deletion of old
//! logs are not (Bazel's `javalog.properties` sets no limit that `SimpleLogHandler` reads).

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path as StdPath, PathBuf};

use blaze_util::string_encoding::{
    internal_to_platform, internal_to_unicode, platform_to_internal, unicode_to_internal,
};

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
    /// Flushes the current log file.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:460 SimpleLogHandler.flush
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:780 SimpleLogHandler.Output.flush
    pub fn flush(&mut self) {
        use std::io::Write as _;
        if let Some((_, file)) = self.output.as_mut()
            && let Err(e) = file.flush()
        {
            report_error("Failed to flush log file", &e);
        }
    }

    // PORT: the `Output` inner class is the `output` field: `Output.open` is
    // `open_output_if_needed`, `Output.write` is `publish`, `Output.close` is `close`,
    // `Output.getPath` and `Output.isOpen` are `get_current_log_file_path`.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:743 SimpleLogHandler.Output.open
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:770 SimpleLogHandler.Output.write
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:789 SimpleLogHandler.Output.close
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:760 SimpleLogHandler.Output.getPath
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:734 SimpleLogHandler.Output.isOpen
    //
    // Rotation and the deletion of old logs are not reachable at the pin: Bazel's client
    // configures the handler with `javalog.properties`, whose `limit` and `total_limit` are not
    // property names `SimpleLogHandler` reads, so both stay disabled.
    // PORT: skipped, unreachable: SimpleLogHandler.setRotateLimitBytes, SimpleLogHandler.setTotalLimitBytes
    // PORT: skipped, unreachable: SimpleLogHandler.closeIfByteCountAtleast, SimpleLogHandler.deleteOldLogs
    // PORT: skipped, unreachable: SimpleLogHandler.parseLogFileTimestamp, SimpleLogHandler.PathByTimestamp
    // PORT: skipped, unreachable: SimpleLogHandler.getSize, SimpleLogHandler.compareTo, SimpleLogHandler.getPath
    // The handler level and formatter and their `LogManager` properties are java.util.logging
    // plumbing: records reach the handler filtered by the logger level and formatted by
    // `SingleLineFormatter` (see `log`); the `LogHandlerQuerier` service is
    // `get_server_log_path`.
    // PORT: skipped, JVM-only: SimpleLogHandler.setLogLevel, SimpleLogHandler.setFormatter
    // PORT: skipped, JVM-only: SimpleLogHandler.isLoggable, SimpleLogHandler.getConfiguredProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.getConfiguredStringProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.getConfiguredBooleanProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.getConfiguredIntProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.getConfiguredLevelProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.getConfiguredFormatterProperty
    // PORT: skipped, JVM-only: SimpleLogHandler.canQuery, SimpleLogHandler.getLogHandlerFilePath

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
        // Unicode-aware lowercase conversion requires strings to be in Java's regular encoding.
        unicode_to_internal(&internal_to_unicode(&name).to_lowercase()).into_owned()
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

        // Path.of(StringEncoding.internalToPlatform(sb.toString())).toAbsolutePath()
        let p = PathBuf::from(internal_to_platform(&sb).into_owned());
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir().expect("getcwd").join(p)
        }
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:378 SimpleLogHandler.getSymlinkAbsolutePath
    fn get_symlink_absolute_path(log_dir: &StdPath, symlink: &str) -> PathBuf {
        blaze_util::check_argument!(!symlink.is_empty());
        let mut symlink_path = PathBuf::from(internal_to_platform(symlink).into_owned());
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
    /// Writes a formatted record to the current log file, opening it if needed, and flushes.
    ///
    /// PORT: the record is formatted by the caller ([`single_line_format`]); no rotation (Bazel's
    /// `javalog.properties` sets no limit `SimpleLogHandler` reads).
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:423 SimpleLogHandler.publish
    pub fn publish(&mut self, formatted: &str) {
        use std::io::Write as _;
        self.open_output_if_needed();
        if let Some((_, file)) = self.output.as_mut() {
            let bytes = blaze_util::string_encoding::latin1_encode_lossy(formatted);
            if let Err(e) = file.write_all(&bytes).and_then(|()| file.flush()) {
                report_error("Failed to write to log file", &e);
            }
        }
    }

    pub fn open_output_if_needed(&mut self) {
        if self.output.is_some() {
            return;
        }
        // Ensure the log file's directory exists.
        blaze_util::check_state!(self.base_file_path.is_absolute());
        if let Some(parent) = self.base_file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Path.of(baseFilePath + timestamp + extension), on the platform strings.
        let mut path = OsString::from(self.base_file_path.as_os_str());
        path.push(&*internal_to_platform(&format!(
            "{}{}",
            format_timestamp(self.clock),
            self.extension
        )));
        let path = PathBuf::from(path);
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
        // StringEncoding.platformToInternal(System.getProperty("user.name")).
        platform_to_internal(std::os::unix::ffi::OsStrExt::from_bytes(
            std::ffi::CStr::from_ptr(pwd.pw_name).to_bytes(),
        ))
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
    // StringEncoding.platformToInternal(InetAddress.getLocalHost().getHostName()).
    let local =
        platform_to_internal(std::os::unix::ffi::OsStrExt::from_bytes(&buf[..len])).into_owned();
    // InetAddress.getLocalHost() resolves the name and throws UnknownHostException when it
    // cannot ("localhost" is special-cased to the loopback address).
    if local == "localhost" {
        return local;
    }
    use std::net::ToSocketAddrs;
    // The resolver takes the platform string (bytes); a name that is not UTF-8 cannot be
    // resolved here and is "unknown", as an unresolvable name is in Java.
    let Ok(platform_name) = std::str::from_utf8(&buf[..len]) else {
        return "unknown".to_string();
    };
    match (platform_name, 0u16).to_socket_addrs() {
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
    // The loggers of the crates below the runtime write here too.
    blaze_util::server_logger::set_sink(|level, class, method, message, cause| {
        use blaze_options::converters::LogLevel;
        let level = match level {
            blaze_util::server_logger::Level::Severe => LogLevel::SEVERE,
            blaze_util::server_logger::Level::Warning => LogLevel::WARNING,
            blaze_util::server_logger::Level::Info => LogLevel::INFO,
        };
        log_with_cause(level, class, method, message, cause)
    });
}

/// Flushes all loggers at com.google.devtools.build.* or higher (their one handler, the server
/// log's).
// Java: com/google/devtools/build/lib/util/DebugLoggerConfigurator.java:61 DebugLoggerConfigurator.flushServerLog
pub fn flush_server_log() {
    if let Some(handler) = SERVER_LOG_HANDLER.lock().as_mut() {
        handler.flush();
    }
}

// The LogHandlerQuerier service finds the file of the handler installed by the logging
// configuration; Blaze's one handler is the SimpleLogHandler above, which `get_server_log_path`
// asks directly.
// PORT: skipped, JVM-only (java.util.logging plumbing): LogHandlerQuerier.makeConfiguredInstance
// PORT: skipped, JVM-only (java.util.logging plumbing): LogHandlerQuerier.getConfiguredInstance
// PORT: skipped, JVM-only (java.util.logging plumbing): LogHandlerQuerier.getLoggerFilePath
// PORT: skipped, JVM-only (java.util.logging plumbing): LogHandlerQuerier.canQuery
// PORT: skipped, JVM-only (java.util.logging plumbing): LogHandlerQuerier.getLogHandlerFilePath
// PORT: skipped, JVM-only (java.util.logging FileHandler): FileHandlerQuerier.canQuery
// PORT: skipped, JVM-only (java.util.logging FileHandler): FileHandlerQuerier.getLogHandlerFilePath
// PORT: skipped, Google-internal remote logging: LoggingUtil.installRemoteLogger
// PORT: skipped, Google-internal remote logging: LoggingUtil.installRemoteLoggerForTesting
// PORT: skipped, Google-internal remote logging: LoggingUtil.getRemoteLogger
// PORT: skipped, JVM-only (class path scanning, used by docgen only): Classpath.findClasses

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
        .map(|p| platform_to_internal(p.as_os_str()).into_owned()))
}

/// `Level.intValue()` of a standard `java.util.logging.Level`.
fn level_int_value(level: blaze_options::converters::LogLevel) -> i32 {
    use blaze_options::converters::LogLevel;
    match level {
        LogLevel::OFF => i32::MAX,
        LogLevel::SEVERE => 1000,
        LogLevel::WARNING => 900,
        LogLevel::INFO => 800,
        LogLevel::FINE => 500,
        LogLevel::FINER => 400,
        LogLevel::FINEST => 300,
    }
}

/// The level of the `com.google.devtools.build` logger (`templateLogger`); `None` inherits the
/// root logger's `INFO`.
static TEMPLATE_LOGGER_LEVEL: Mutex<Option<blaze_options::converters::LogLevel>> = Mutex::new(None);

/// `DebugLoggerConfigurator.currentVerbosityLevel`.
static CURRENT_VERBOSITY_LEVEL: Mutex<Option<blaze_options::converters::LogLevel>> =
    Mutex::new(None);

/// Configures "com.google.devtools.build.*" loggers to the given `level`.
// Java: com/google/devtools/build/lib/util/DebugLoggerConfigurator.java:52 DebugLoggerConfigurator.setupLogging
pub fn setup_logging(level: blaze_options::converters::LogLevel) {
    let mut current = CURRENT_VERBOSITY_LEVEL.lock();
    if *current != Some(level) {
        *TEMPLATE_LOGGER_LEVEL.lock() = Some(level);
        log(
            blaze_options::converters::LogLevel::INFO,
            "com.google.devtools.build.lib.util.DebugLoggerConfigurator",
            "setupLogging",
            &format!("Log level: {:?}", level),
        );
        *current = Some(level);
    }
}

/// Logs `message` at `level` on a `com.google.devtools.build.*` logger, as
/// `Logger.log(LogRecord)` with the server log's `SimpleLogHandler` and Bazel's
/// `SingleLineFormatter` (the formatter the client's `javalog.properties` names).
///
/// PORT: Blaze has no `java.util.logging`; this is the one logger the server writes through.
/// The thread ID of the record is the OS thread ID (Java's is the JVM's thread number).
pub fn log(
    level: blaze_options::converters::LogLevel,
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
) {
    log_with_cause(level, source_class_name, source_method_name, message, None)
}

/// [`log`] of a record with a throwable (`withCause`): `cause` is the throwable's `toString()`.
///
/// PORT: Java prints the throwable's stack trace after the record (`printStackTrace`); Blaze has
/// no Java stack, so the record carries the throwable's first line only.
pub fn log_with_cause(
    level: blaze_options::converters::LogLevel,
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    // Logger.isLoggable: the effective level of the logger.
    let logger_level = TEMPLATE_LOGGER_LEVEL
        .lock()
        .unwrap_or(blaze_options::converters::LogLevel::INFO);
    if level_int_value(level) < level_int_value(logger_level)
        || level_int_value(logger_level) == i32::MAX
    {
        return;
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // SAFETY: gettid has no preconditions.
    let thread_id = unsafe { libc::gettid() };
    let record = single_line_format(
        millis,
        level,
        thread_id as i64,
        source_class_name,
        source_method_name,
        message,
        cause,
    );
    let mut handler = SERVER_LOG_HANDLER.lock();
    if let Some(handler) = handler.as_mut() {
        handler.publish(&record);
    }
}

/// `SingleLineFormatter.format`.
// Java: com/google/devtools/build/lib/util/SingleLineFormatter.java:49 SingleLineFormatter.format
fn single_line_format(
    millis: i64,
    level: blaze_options::converters::LogLevel,
    thread_id: i64,
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    thrown: Option<&str>,
) -> String {
    let mut buf = String::new();
    // Timestamp: "yyMMdd HH:mm:ss.SSS" in UTC.
    let t = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: tm is plain data; gmtime_r writes into it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&t, &mut tm);
        tm
    };
    buf.push_str(&format!(
        "{:02}{:02}{:02} {:02}:{:02}:{:02}.{:03}:",
        (tm.tm_year + 1900) % 100,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        millis.rem_euclid(1000)
    ));
    // One character code for level (CODES_BY_LEVEL).
    let value = level_int_value(level);
    buf.push(if value <= 500 {
        'D'
    } else if value < 900 {
        'I'
    } else if value < 1000 {
        'W'
    } else {
        'X'
    });
    // The stack trace, if any
    if thrown.is_some() {
        buf.push('T');
    }
    buf.push(' ');
    // Information about the source of the exception
    buf.push_str(&format!(
        "{} [{}.{}] ",
        thread_id, source_class_name, source_method_name
    ));
    // The actual message
    buf.push_str(message);
    buf.push('\n');
    if let Some(thrown) = thrown {
        buf.push_str(thrown);
        buf.push('\n');
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from src/test/java/com/google/devtools/build/lib/util/SingleLineFormatterTest.java.
    // TIMESTAMP: 2017-04-01T17:03:43.142Z.
    const TIMESTAMP_MILLIS: i64 = 1_491_066_223_142;

    fn format_record(
        level: blaze_options::converters::LogLevel,
        millis: i64,
        thrown: Option<&str>,
    ) -> String {
        single_line_format(
            millis,
            level,
            543,
            "SomeSourceClass",
            "aSourceMethod",
            "some message",
            thrown,
        )
    }

    #[test]
    fn test_format() {
        assert_eq!(
            format_record(
                blaze_options::converters::LogLevel::SEVERE,
                TIMESTAMP_MILLIS,
                None
            ),
            "170401 17:03:43.142:X 543 [SomeSourceClass.aSourceMethod] some message\n"
        );
    }

    #[test]
    fn test_level() {
        let formatted = format_record(
            blaze_options::converters::LogLevel::WARNING,
            TIMESTAMP_MILLIS,
            None,
        );
        assert!(formatted.contains('W'));
        assert!(!formatted.contains('X'));
    }

    #[test]
    fn test_time() {
        // 1999-11-30T03:04:05.722Z
        assert!(
            format_record(
                blaze_options::converters::LogLevel::SEVERE,
                943_931_045_722,
                None
            )
            .contains("991130 03:04:05.722")
        );
    }

    /// PORT: Blaze has no Java stack trace; the record carries the throwable's first line.
    #[test]
    fn test_stack_trace() {
        assert!(
            format_record(
                blaze_options::converters::LogLevel::SEVERE,
                TIMESTAMP_MILLIS,
                Some("java.lang.RuntimeException: something wrong")
            )
            .starts_with(
                "170401 17:03:43.142:XT 543 [SomeSourceClass.aSourceMethod] some message\n\
                 java.lang.RuntimeException: something wrong\n"
            )
        );
    }

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
    fn test_publish() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .build();
        // TrivialFormatter: the message and a newline.
        handler.publish("Hello world\n"); // To open the log file.
        let log_path = handler.get_current_log_file_path().unwrap();
        handler.close();
        assert_eq!(std::fs::read_to_string(log_path).unwrap(), "Hello world\n");
    }

    #[test]
    #[ignore = "handler log levels (Builder.setLogLevel, isLoggable) are skipped: Bazel never sets one"]
    fn test_log_level_equal_published() {}

    #[test]
    #[ignore = "handler log levels (Builder.setLogLevel, isLoggable) are skipped: Bazel never sets one"]
    fn test_log_level_higher_published() {}

    #[test]
    #[ignore = "handler log levels (Builder.setLogLevel, isLoggable) are skipped: Bazel never sets one"]
    fn test_log_level_lower_not_published() {}

    #[test]
    #[ignore = "handler log levels (Builder.setLogLevel, isLoggable) are skipped: Bazel never sets one"]
    fn test_log_level_default_all_published() {}

    #[test]
    #[ignore = "needs log rotation (not used by Bazel's javalog.properties)"]
    fn test_rotate_limit_bytes() {}

    #[test]
    fn test_open_in_append_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("hello.20150901-151754.log");
        std::fs::write(&log_path, "Previous logs\n").unwrap();
        // Instant.parse("2015-09-01T15:17:54Z")
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", tmp.path().display()))
            .set_pattern(".")
            .set_extension("log")
            .set_clock_for_testing(1_441_120_674)
            .build();
        handler.publish("New logs\n");
        assert_eq!(handler.get_current_log_file_path().unwrap(), log_path);
        handler.close();
        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap(),
            "Previous logs\nNew logs\n"
        );
    }

    #[test]
    #[ignore = "needs old-log deletion (not used by Bazel's javalog.properties)"]
    fn test_total_limit() {}
}
