//! The server log: `java.util.logging`'s configuration as far as Bazel's server uses it
//! (`LogManager` reading the client's `javalog.properties`), the `SimpleLogHandler` that the
//! configuration installs on the root logger, and `DebugLoggerConfigurator`.
//!
//! Bazel's client writes `<output_base>/javalog.properties` configuring
//! `com.google.devtools.build.lib.util.SimpleLogHandler` with `prefix=<output_base>/java.log`
//! (`StartupOptions::AddJVMLoggingArguments`) and passes it to the JVM as
//! `-Djava.util.logging.config.file=...`. The JVM reads it before `main`; Blaze reads it in
//! [`configure_from_jvm_flags`], which the `blaze` binary calls before `Bazel.main`. The other
//! properties the client writes (`limit`, `total_limit`) are not names `SimpleLogHandler` reads,
//! so there is no rotation and no deletion of old logs with the client's configuration.
//!
//! PORT: Blaze has no `java.util.logging`. `LogManager.readConfiguration` (the properties, the
//! root logger's level and handlers), the handler's file naming, opening, rotation and symlink,
//! the `SingleLineFormatter` record format and `DebugLoggerConfigurator.setupLogging` are ported;
//! the server writes its records through [`log`] (and crates below the runtime through
//! `blaze_util::server_logger`, whose sink the configuration sets).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path as StdPath, PathBuf};

use blaze_util::string_encoding::{
    internal_to_platform, internal_to_unicode, platform_to_internal, unicode_to_internal,
};

use parking_lot::Mutex;

/// `java.util.logging.Level.ALL.intValue()`.
const LEVEL_ALL: i32 = i32::MIN;

/// The properties of the JVM's logging configuration (`LogManager.props`), `None` while no
/// configuration file was read.
static LOG_MANAGER_PROPERTIES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// `LogManager.getLogManager().getProperty(name)`.
fn get_log_manager_property(name: &str) -> Option<String> {
    LOG_MANAGER_PROPERTIES
        .lock()
        .as_ref()
        .and_then(|props| props.get(name).cloned())
}

/// `java.util.Properties.load(InputStream)`: the key-value pairs of a `.properties` file, read
/// as ISO-8859-1 with the escapes of the format interpreted. Later keys replace earlier ones.
///
/// PORT: a port of the JDK's `Properties.load0` / `LineReader` (not in Bazel's tree).
pub fn load_properties(bytes: &[u8]) -> HashMap<String, String> {
    let text = blaze_util::string_encoding::latin1_decode(bytes);
    let chars: Vec<char> = text.chars().collect();
    let mut props = HashMap::new();
    let mut i = 0;
    while i < chars.len() {
        // LineReader.readLine: one logical line, without its leading white space, comments and
        // blank lines skipped, continuation lines joined.
        let mut line: Vec<char> = Vec::new();
        let mut skip_white_space = true;
        let mut is_comment_line = false;
        let mut is_new_line = true;
        let mut appended_line_begin = false;
        let mut preceding_backslash = false;
        let mut skip_lf = false;
        let mut ended = false;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            if skip_lf {
                skip_lf = false;
                if c == '\n' {
                    continue;
                }
            }
            if skip_white_space {
                if c == ' ' || c == '\t' || c == '\x0c' {
                    continue;
                }
                if !appended_line_begin && (c == '\r' || c == '\n') {
                    continue;
                }
                skip_white_space = false;
                appended_line_begin = false;
            }
            if is_new_line {
                is_new_line = false;
                if c == '#' || c == '!' {
                    is_comment_line = true;
                    continue;
                }
            }
            if c != '\n' && c != '\r' {
                if !is_comment_line {
                    line.push(c);
                }
                // Flip the preceding backslash flag.
                preceding_backslash = c == '\\' && !preceding_backslash;
            } else {
                // Reached EOL.
                if is_comment_line || line.is_empty() {
                    is_comment_line = false;
                    is_new_line = true;
                    skip_white_space = true;
                    line.clear();
                    preceding_backslash = false;
                    if c == '\r' {
                        skip_lf = true;
                    }
                    continue;
                }
                if preceding_backslash {
                    line.pop();
                    skip_white_space = true;
                    appended_line_begin = true;
                    preceding_backslash = false;
                    if c == '\r' {
                        skip_lf = true;
                    }
                } else {
                    ended = true;
                    break;
                }
            }
        }
        if is_comment_line || line.is_empty() {
            if !ended {
                break;
            }
            continue;
        }
        if !ended && preceding_backslash {
            // A trailing backslash at the end of the input is dropped.
            line.pop();
        }
        // Properties.load0: split the line into key and value.
        let limit = line.len();
        let mut key_len = 0;
        let mut value_start = limit;
        let mut has_sep = false;
        let mut preceding_backslash = false;
        while key_len < limit {
            let c = line[key_len];
            if (c == '=' || c == ':') && !preceding_backslash {
                value_start = key_len + 1;
                has_sep = true;
                break;
            } else if (c == ' ' || c == '\t' || c == '\x0c') && !preceding_backslash {
                value_start = key_len + 1;
                break;
            }
            preceding_backslash = c == '\\' && !preceding_backslash;
            key_len += 1;
        }
        while value_start < limit {
            let c = line[value_start];
            if c != ' ' && c != '\t' && c != '\x0c' {
                if !has_sep && (c == '=' || c == ':') {
                    has_sep = true;
                } else {
                    break;
                }
            }
            value_start += 1;
        }
        let key = load_convert(&line[..key_len]);
        let value = load_convert(&line[value_start.min(limit)..]);
        props.insert(key, value);
    }
    props
}

/// `Properties.loadConvert`: interprets `\uXXXX` and the escapes `\t`, `\r`, `\n`, `\f`; any
/// other escaped character stands for itself.
fn load_convert(chars: &[char]) -> String {
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let mut c = chars[i];
        i += 1;
        if c == '\\' && i < chars.len() {
            c = chars[i];
            i += 1;
            if c == 'u' {
                // Read the xxxx.
                if i + 4 > chars.len() {
                    panic!("IllegalArgumentException: Malformed \\uxxxx encoding.");
                }
                let mut value: u32 = 0;
                for _ in 0..4 {
                    let digit = chars[i].to_digit(16).unwrap_or_else(|| {
                        panic!("IllegalArgumentException: Malformed \\uxxxx encoding.")
                    });
                    value = (value << 4) + digit;
                    i += 1;
                }
                out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
            } else {
                out.push(match c {
                    't' => '\t',
                    'r' => '\r',
                    'n' => '\n',
                    'f' => '\x0c',
                    other => other,
                });
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `java.util.logging.Level.parse(name)` for the standard level names and integers; panics
/// (Java `IllegalArgumentException`) on another name.
fn parse_level(name: &str) -> i32 {
    find_level(name).unwrap_or_else(|| panic!("IllegalArgumentException: Bad level \"{name}\""))
}

/// `Level.findLevel(name)`: the standard level names and integers, else `None`.
fn find_level(name: &str) -> Option<i32> {
    Some(match name {
        "OFF" => i32::MAX,
        "SEVERE" => 1000,
        "WARNING" => 900,
        "INFO" => 800,
        "CONFIG" => 700,
        "FINE" => 500,
        "FINER" => 400,
        "FINEST" => 300,
        "ALL" => LEVEL_ALL,
        _ => return name.parse::<i32>().ok(),
    })
}

/// A log handler that writes to a file whose name is derived from a prefix, a pattern, a
/// timestamp and an extension, with a symbolic link to the current file, optionally rotating
/// the file after a size limit and deleting the oldest logs above a total size.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:78 SimpleLogHandler
pub struct SimpleLogHandler {
    /// The handler's level (`Handler.getLevel().intValue()`).
    level: i32,
    /// Log file extension; the current process ID by default.
    extension: String,
    /// True if the log file extension is not the pid.
    is_static_extension: bool,
    /// Absolute path to symbolic link to current log file, or `None` if the link should not be
    /// created.
    symlink_path: Option<PathBuf>,
    /// Absolute path to common base name of log files.
    base_file_path: PathBuf,
    /// Log file currently in use, and the number of bytes written to it.
    output: Option<(PathBuf, File, i64)>,
    /// If the log file size reaches this limit, the file is closed and a new one opened. A value
    /// of 0 or less disables rotation.
    rotate_limit_bytes: i32,
    /// If the total size of log files exceeds this limit, the oldest are deleted. A value of 0 or
    /// less disables the deletion.
    total_limit_bytes: i32,
    /// Source for timestamps in filenames: `None` is the system clock in the default time zone;
    /// `Some(seconds)` is a fixed instant formatted in UTC (`setClockForTesting`).
    clock: Option<i64>,
    /// `ErrorManager.reported`: the default error manager reports only the first error.
    error_reported: bool,
}

// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:98 SimpleLogHandler.DEFAULT_PREFIX_STRING
const DEFAULT_PREFIX_STRING: &str = "java.log";
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:99 SimpleLogHandler.DEFAULT_BASE_FILE_NAME_PATTERN
const DEFAULT_BASE_FILE_NAME_PATTERN: &str = ".%h.%u.log.java.";
/// `SimpleLogHandler.class.getName()`: the prefix of its configuration properties.
const SIMPLE_LOG_HANDLER_CLASS: &str = "com.google.devtools.build.lib.util.SimpleLogHandler";
/// The formatter the handler supports (its default, and the one Bazel's client configures).
const SINGLE_LINE_FORMATTER_CLASS: &str = "com.google.devtools.build.lib.util.SingleLineFormatter";

/// Builder class for [`SimpleLogHandler`]. All setters are optional; unset values fall back to
/// the JVM logging configuration, then to the defaults.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:145 SimpleLogHandler.Builder
#[derive(Default)]
pub struct SimpleLogHandlerBuilder {
    prefix: Option<String>,
    pattern: Option<String>,
    extension: Option<String>,
    symlink_name: Option<String>,
    create_symlink: Option<bool>,
    rotate_limit_bytes: Option<i32>,
    total_limit_bytes: Option<i32>,
    log_level: Option<i32>,
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

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:249 Builder.setRotateLimitBytes
    pub fn set_rotate_limit_bytes(mut self, rotate_limit_bytes: i32) -> Self {
        self.rotate_limit_bytes = Some(rotate_limit_bytes);
        self
    }

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:266 Builder.setTotalLimitBytes
    pub fn set_total_limit_bytes(mut self, total_limit: i32) -> Self {
        self.total_limit_bytes = Some(total_limit);
        self
    }

    /// `level`: a `java.util.logging.Level` (its `intValue()`).
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:280 Builder.setLogLevel
    pub fn set_log_level(mut self, level: blaze_options::converters::LogLevel) -> Self {
        self.log_level = Some(level_int_value(level));
        self
    }

    // PORT: Builder.setFormatter takes a java.util.logging.Formatter object; the one formatter
    // Blaze has is SingleLineFormatter, applied by `log`; handler tests pass formatted records.
    // PORT: skipped, JVM-only (Formatter objects): Builder.setFormatter

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
        let prefix =
            get_configured_string_property(b.prefix.clone(), "prefix", Some(DEFAULT_PREFIX_STRING))
                .expect("prefix");
        let base_file_path = Self::get_base_file_path(
            &prefix,
            &get_configured_string_property(
                b.pattern.clone(),
                "pattern",
                Some(DEFAULT_BASE_FILE_NAME_PATTERN),
            )
            .expect("pattern"),
        );
        let configured_symlink_name =
            get_configured_string_property(b.symlink_name.clone(), "symlink", Some(&prefix))
                .expect("symlink");
        // OS.getCurrent() != OS.WINDOWS
        let configured_create_symlink =
            get_configured_boolean_property(b.create_symlink, "create_symlink", true);
        let symlink_path = if configured_create_symlink {
            Some(Self::get_symlink_absolute_path(
                base_file_path.parent().expect("log directory"),
                &configured_symlink_name,
            ))
        } else {
            None
        };
        let pid = std::process::id().to_string();
        let extension =
            get_configured_string_property(b.extension.clone(), "extension", Some(&pid))
                .expect("extension");
        let is_static_extension =
            get_configured_string_property(b.extension.clone(), "extension", None).is_some();
        let rotate_limit_bytes =
            get_configured_int_property(b.rotate_limit_bytes, "rotate_limit_bytes", 0);
        blaze_util::check_argument!(
            rotate_limit_bytes >= 0,
            "File size limits cannot be negative"
        );
        let total_limit_bytes =
            get_configured_int_property(b.total_limit_bytes, "total_limit_bytes", 0);
        blaze_util::check_argument!(
            total_limit_bytes >= 0,
            "File size limits cannot be negative"
        );
        let level = get_configured_level_property(b.log_level, "level", LEVEL_ALL);
        // getConfiguredFormatterProperty(formatter, "formatter", new SingleLineFormatter())
        if let Some(formatter) =
            get_log_manager_property(&format!("{SIMPLE_LOG_HANDLER_CLASS}.formatter"))
        {
            let formatter = formatter.trim();
            if !formatter.is_empty() && formatter != SINGLE_LINE_FORMATTER_CLASS {
                blaze_util::unported!("SimpleLogHandler.getConfiguredFormatterProperty");
            }
        }
        SimpleLogHandler {
            level,
            extension,
            is_static_extension,
            symlink_path,
            base_file_path,
            output: None,
            rotate_limit_bytes,
            total_limit_bytes,
            clock: b.clock,
            error_reported: false,
        }
    }

    /// Returns the absolute path of the current log file, or `None` if the log is not open.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:405 SimpleLogHandler.getCurrentLogFilePath
    pub fn get_current_log_file_path(&self) -> Option<PathBuf> {
        self.output.as_ref().map(|(p, _, _)| p.clone())
    }

    /// Returns the expected absolute path for the symbolic link to the current log file, or
    /// `None` if not used.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:413 SimpleLogHandler.getSymbolicLinkPath
    pub fn get_symbolic_link_path(&self) -> Option<&PathBuf> {
        self.symlink_path.as_ref()
    }

    /// `Handler.isLoggable`: the record's level is at least the handler's (and the handler is not
    /// off).
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:418 SimpleLogHandler.isLoggable
    pub fn is_loggable(&self, level: i32) -> bool {
        !(level < self.level || self.level == i32::MAX)
    }

    /// Writes a record of level `level` (a `Level.intValue()`), formatted by the caller, to the
    /// current log file, opening it if needed, then rotates the file if it reached the limit.
    ///
    /// PORT: the record is formatted by the caller ([`single_line_format`] for the server log).
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:423 SimpleLogHandler.publish
    pub fn publish(&mut self, level: i32, formatted: &str) {
        if !self.is_loggable(level) {
            // Silently ignore null or filtered records, matching FileHandler behavior.
            return;
        }
        self.open_output_if_needed();
        if let Err(e) = self.output_write(formatted) {
            self.report_error(None, &e, ERROR_MANAGER_WRITE_FAILURE);
            // Failing to log is non-fatal. Continue to try to rotate the log if necessary, which
            // may fix the underlying IO problem with the file.
        }

        if self.rotate_limit_bytes > 0 {
            self.output_close_if_byte_count_atleast(self.rotate_limit_bytes);
            self.open_output_if_needed();
        }
    }

    /// Flushes the current log file.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:460 SimpleLogHandler.flush
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:780 SimpleLogHandler.Output.flush
    pub fn flush(&mut self) {
        use std::io::Write as _;
        let result = match self.output.as_mut() {
            Some((_, file, _)) => file.flush(),
            None => Ok(()),
        };
        if let Err(e) = result {
            self.report_error(None, &e, ERROR_MANAGER_FLUSH_FAILURE);
        }
    }

    /// Closes the current log file (`LogManager.reset()` closes all handlers).
    /// PORT: the formatter tail is empty for `SingleLineFormatter`; nothing is written.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:479 SimpleLogHandler.close
    pub fn close(&mut self) {
        self.output_close();
    }

    // PORT: the `Output` inner class is the `output` field and the `output_*` methods below;
    // `Output.getPath` and `Output.isOpen` are `get_current_log_file_path`. The file is written
    // unbuffered, so the byte count is exact without the flush `closeIfByteCountAtleast` does.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:760 SimpleLogHandler.Output.getPath
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:734 SimpleLogHandler.Output.isOpen

    /// Opens the specified file in append mode, first closing the current file if needed.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:743 SimpleLogHandler.Output.open
    fn output_open(&mut self, path: PathBuf) -> std::io::Result<()> {
        self.output_close();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.output = Some((path, file, 0));
        Ok(())
    }

    /// Writes the string (ISO-8859-1) to the current file. Fails (Java `NullPointerException`
    /// caught as a write failure) when the file is not open.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:770 SimpleLogHandler.Output.write
    fn output_write(&mut self, string: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        let Some((_, file, count)) = self.output.as_mut() else {
            return Err(std::io::Error::other("java.lang.NullPointerException"));
        };
        let bytes = blaze_util::string_encoding::latin1_encode_lossy(string);
        file.write_all(&bytes)?;
        *count += bytes.len() as i64;
        Ok(())
    }

    /// Closes the current file if it is open.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:789 SimpleLogHandler.Output.close
    fn output_close(&mut self) {
        self.output = None;
    }

    /// Closes the current file unless the number of bytes written to it was under the specified
    /// limit.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:807 SimpleLogHandler.Output.closeIfByteCountAtleast
    fn output_close_if_byte_count_atleast(&mut self, limit: i32) {
        if let Some((_, _, count)) = self.output.as_ref()
            && *count >= limit as i64
        {
            self.output_close();
        }
    }

    // The handler level and formatter come from the configuration (see `new`); the
    // `LogHandlerQuerier` service is `get_server_log_path`.
    // PORT: skipped, JVM-only: SimpleLogHandler.setLogLevel, SimpleLogHandler.setFormatter
    // PORT: skipped, JVM-only: SimpleLogHandler.canQuery, SimpleLogHandler.getLogHandlerFilePath

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

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:663 SimpleLogHandler.getBaseFilePath
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

    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:707 SimpleLogHandler.getSymlinkAbsolutePath
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

    /// Opens a new log file if one is not open, updating the symbolic link and deleting old logs
    /// if needed. Failures are reported on stderr and are not fatal.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:829 SimpleLogHandler.openOutputIfNeeded
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
        // The head of SingleLineFormatter is empty: nothing is written after opening.
        if let Err(e) = self.output_open(PathBuf::from(path)) {
            self.output_close();
            self.report_error(
                Some("Failed to open log file"),
                &e,
                ERROR_MANAGER_OPEN_FAILURE,
            );
            return;
        }

        if self.total_limit_bytes > 0 {
            self.delete_old_logs();
        }

        // Try to create relative symlink from currentLogFile to baseFile, but don't treat a
        // failure as fatal.
        if let Some(symlink_path) = self.symlink_path.clone() {
            let path = self.get_current_log_file_path().expect("open");
            blaze_util::check_state!(symlink_path.parent() == path.parent());
            let result = (|| -> std::io::Result<()> {
                if std::fs::symlink_metadata(&symlink_path).is_ok() {
                    std::fs::remove_file(&symlink_path)?;
                }
                std::os::unix::fs::symlink(path.file_name().expect("file name"), &symlink_path)
            })();
            if let Err(e) = result {
                self.report_error(
                    Some("Failed to create symbolic link to log file"),
                    &e,
                    ERROR_MANAGER_GENERIC_FAILURE,
                );
            }
        }
    }

    /// Parses the absolute path of a logfile (e.g from a previous run of the program) and
    /// extracts the timestamp (seconds; the fields read in the default time zone, as
    /// `SimpleDateFormat.parse` does), or `None` (Java `ParseException`) if the path does not
    /// match the expected prefix, resolved pattern, timestamp format, or extension.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:877 SimpleLogHandler.parseLogFileTimestamp
    fn parse_log_file_timestamp(&self, path: &StdPath) -> Option<i64> {
        use std::os::unix::ffi::OsStrExt as _;
        let path_string = path.as_os_str().as_bytes();
        let base = self.base_file_path.as_os_str().as_bytes();
        if !path_string.starts_with(base) {
            return None; // Wrong prefix or pattern
        }
        let (timestamp, rest) = parse_timestamp(&path_string[base.len()..], self.clock.is_some())?;
        let rest = platform_to_internal(std::ffi::OsStr::from_bytes(rest)).into_owned();
        if self.is_static_extension {
            if rest != self.extension {
                return None; // Wrong file extension
            }
        } else if rest.parse::<i64>().is_err() {
            return None; // File extension is not a numeric PID
        }
        Some(timestamp)
    }

    /// Deletes the oldest log files matching the expected prefix, pattern, timestamp format, and
    /// extension, to keep the total size under `total_limit_bytes` (if set to non-0).
    ///
    /// Each log file's timestamp is determined only from the filename. The current log file will
    /// not be deleted.
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:935 SimpleLogHandler.deleteOldLogs
    // Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:901 SimpleLogHandler.PathByTimestamp
    fn delete_old_logs(&mut self) {
        blaze_util::check_state!(self.base_file_path.is_absolute());
        let current = self.get_current_log_file_path();
        let result = (|| -> std::io::Result<()> {
            // PriorityQueue<PathByTimestamp>, ordered by timestamp.
            let mut queue: Vec<(i64, PathBuf, i64)> = Vec::new();
            let mut total_size: i64 = 0;
            let dir = self.base_file_path.parent().expect("log directory");
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if let Some(timestamp) = self.parse_log_file_timestamp(&path) {
                    let size = std::fs::metadata(&path)?.len() as i64;
                    total_size += size;
                    if current.as_ref() != Some(&path) {
                        queue.push((timestamp, path, size));
                    }
                }
                // Ignore files which don't look like our logs.
            }
            queue.sort_by_key(|(timestamp, _, _)| *timestamp);

            if self.total_limit_bytes > 0 {
                let mut queue = queue.into_iter();
                while total_size > self.total_limit_bytes as i64 {
                    let Some((_, path, size)) = queue.next() else {
                        break;
                    };
                    std::fs::remove_file(&path)?;
                    total_size -= size;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.report_error(
                Some("Failed to clean up old log files"),
                &e,
                ERROR_MANAGER_GENERIC_FAILURE,
            );
        }
    }

    /// `Handler.reportError` through the default `ErrorManager`, which prints the first error
    /// only: `java.util.logging.ErrorManager: <code>[: <message>]` and the exception.
    ///
    /// PORT: Java prints the exception's stack trace; Blaze prints the I/O error's text.
    fn report_error(&mut self, message: Option<&str>, e: &std::io::Error, code: i32) {
        if self.error_reported {
            return;
        }
        self.error_reported = true;
        let mut text = format!("java.util.logging.ErrorManager: {code}");
        if let Some(message) = message {
            text.push_str(": ");
            text.push_str(message);
        }
        eprintln!("{text}");
        eprintln!("{e}");
    }
}

// The codes of java.util.logging.ErrorManager.
const ERROR_MANAGER_GENERIC_FAILURE: i32 = 0;
const ERROR_MANAGER_WRITE_FAILURE: i32 = 1;
const ERROR_MANAGER_FLUSH_FAILURE: i32 = 2;
const ERROR_MANAGER_OPEN_FAILURE: i32 = 4;

/// Checks if a value is `None`, and if it is, falls back to the JVM logging configuration, and
/// if that too is missing, to a provided fallback value.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:518 SimpleLogHandler.getConfiguredProperty
fn get_configured_property<T>(
    builder_value: Option<T>,
    configured_name: &str,
    parse: impl FnOnce(&str) -> Option<T>,
    fallback_value: Option<T>,
) -> Option<T> {
    if builder_value.is_some() {
        return builder_value;
    }
    // .properties files are read as Latin-1 by java.util.Properties, with Unicode escape
    // sequences interpreted. Since the Bazel client passes path properties as UTF-8 without
    // escaping, configuredValue already contains a string in Bazel's internal string encoding.
    match get_log_manager_property(&format!("{SIMPLE_LOG_HANDLER_CLASS}.{configured_name}")) {
        Some(configured_value) => parse(&configured_value),
        None => fallback_value,
    }
}

/// Matches java.logging.* configuration behavior; configured strings are trimmed.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:541 SimpleLogHandler.getConfiguredStringProperty
fn get_configured_string_property(
    builder_value: Option<String>,
    configured_name: &str,
    fallback_value: Option<&str>,
) -> Option<String> {
    get_configured_property(
        builder_value,
        configured_name,
        |v| Some(java_trim(v).to_string()),
        fallback_value.map(str::to_string),
    )
}

/// Matches java.logging.* configuration behavior; "true" and "1" are true, "false" and "0" are
/// false. Panics (Java `IllegalArgumentException`) on another value.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:552 SimpleLogHandler.getConfiguredBooleanProperty
fn get_configured_boolean_property(
    builder_value: Option<bool>,
    configured_name: &str,
    fallback_value: bool,
) -> bool {
    get_configured_property(
        builder_value,
        configured_name,
        |val| {
            let val = java_trim(val).to_ascii_lowercase();
            match val.as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                "" => None,
                _ => panic!("IllegalArgumentException: Cannot parse boolean property value"),
            }
        },
        None,
    )
    .unwrap_or(fallback_value)
}

/// Empty configured values are ignored and the fallback is used instead. Panics (Java
/// `NumberFormatException`) on a value that is not an `int`.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:578 SimpleLogHandler.getConfiguredIntProperty
fn get_configured_int_property(
    builder_value: Option<i32>,
    configured_name: &str,
    fallback_value: i32,
) -> i32 {
    get_configured_property(
        builder_value,
        configured_name,
        |val| {
            let val = java_trim(val);
            if val.is_empty() {
                return None;
            }
            // Integer.parseInt: an optional sign and decimal digits, as str::parse::<i32>.
            match val.parse::<i32>() {
                Ok(v) => Some(v),
                Err(_) => panic!("NumberFormatException: For input string: \"{val}\""),
            }
        },
        None,
    )
    .unwrap_or(fallback_value)
}

/// Empty configured values are ignored and the fallback is used instead.
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:597 SimpleLogHandler.getConfiguredLevelProperty
fn get_configured_level_property(
    builder_value: Option<i32>,
    configured_name: &str,
    fallback_value: i32,
) -> i32 {
    get_configured_property(
        builder_value,
        configured_name,
        |val| {
            let val = java_trim(val);
            (!val.is_empty()).then(|| parse_level(val))
        },
        None,
    )
    .unwrap_or(fallback_value)
}

// PORT: the formatter property is checked in SimpleLogHandler::new (SingleLineFormatter only).
// Java: com/google/devtools/build/lib/util/SimpleLogHandler.java:617 SimpleLogHandler.getConfiguredFormatterProperty

/// Java `String.trim()`: strips the chars `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `SimpleDateFormat("yyyyMMdd-HHmmss.").parse(text, pos)`: the timestamp at the start of
/// `text` (seconds since the epoch, the fields in the default time zone or in UTC for a test
/// clock) and the rest of `text`, or `None` if it does not start with one.
///
/// PORT: the adjacent numeric fields are read with their pattern widths and out-of-range values
/// roll over (lenient parsing), as `SimpleDateFormat` does for this pattern.
fn parse_timestamp(text: &[u8], utc: bool) -> Option<(i64, &[u8])> {
    fn digits(text: &[u8], at: usize, n: usize) -> Option<i32> {
        let field = text.get(at..at + n)?;
        if !field.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(field).ok()?.parse().ok()
    }
    let year = digits(text, 0, 4)?;
    let month = digits(text, 4, 2)?;
    let day = digits(text, 6, 2)?;
    if text.get(8) != Some(&b'-') {
        return None;
    }
    let hour = digits(text, 9, 2)?;
    let minute = digits(text, 11, 2)?;
    let second = digits(text, 13, 2)?;
    if text.get(15) != Some(&b'.') {
        return None;
    }
    // SAFETY: tm is plain data; timegm/mktime normalise the fields.
    let t = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month - 1;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_min = minute;
        tm.tm_sec = second;
        tm.tm_isdst = -1;
        if utc {
            libc::timegm(&mut tm)
        } else {
            libc::mktime(&mut tm)
        }
    };
    Some((t as i64, &text[16..]))
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

/// The root logger's handler: the configuration's `handlers`, which in Bazel's
/// `javalog.properties` is one `SimpleLogHandler`.
static SERVER_LOG_HANDLER: Mutex<Option<SimpleLogHandler>> = Mutex::new(None);

/// The root logger's level (`.level` of the configuration; `INFO` by default).
static ROOT_LOGGER_LEVEL: Mutex<i32> = Mutex::new(800);

/// The system property naming the logging configuration file.
const CONFIG_FILE_FLAG: &str = "-Djava.util.logging.config.file=";

/// The JVM's start of `java.util.logging`: with `-Djava.util.logging.config.file=<file>` among
/// the JVM flags (the last one wins, as for any `-D`), `LogManager` reads that configuration
/// before `main`. `jvm_flags` are the JVM flags of the server's command line (the arguments
/// before `-jar`). Without the flag, or when the file cannot be read, nothing is configured.
///
/// PORT: without a configuration file the JVM reads its default `logging.properties` (a
/// `ConsoleHandler` on stderr); Blaze then opens the server log in [`init`] instead.
pub fn configure_from_jvm_flags(jvm_flags: &[String]) {
    let Some(file) = jvm_flags
        .iter()
        .rev()
        .find_map(|flag| flag.strip_prefix(CONFIG_FILE_FLAG))
    else {
        return;
    };
    // LogManager.readConfiguration: an I/O error leaves the default configuration.
    if let Ok(bytes) = std::fs::read(&*internal_to_platform(file)) {
        read_configuration(&bytes);
    }
}

/// `LogManager.readConfiguration(InputStream)` followed by the root logger's lazy handler
/// initialisation: the properties, the root logger's level (`.level`) and its handlers
/// (`handlers`). The handler opens its file on the first record.
pub fn read_configuration(bytes: &[u8]) {
    let props = load_properties(bytes);
    let root_level = props.get(".level").map(|v| java_trim(v).to_string());
    let handler_names: Vec<String> = props
        .get("handlers")
        .map(|v| {
            // LogManager.parseClassNames: comma or white space separated.
            v.split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    *LOG_MANAGER_PROPERTIES.lock() = Some(props);
    // LogManager.getLevelProperty: a bad level name keeps the default.
    if let Some(level) = root_level.as_deref().and_then(find_level) {
        *ROOT_LOGGER_LEVEL.lock() = level;
    }
    let mut handler = None;
    for name in handler_names {
        if name == SIMPLE_LOG_HANDLER_CLASS {
            // new SimpleLogHandler(): every value from the configuration.
            handler = Some(SimpleLogHandler::builder().build());
        } else {
            blaze_util::unported!("LogManager.readConfiguration");
        }
    }
    *SERVER_LOG_HANDLER.lock() = handler;
    CONFIGURED.store(true, std::sync::atomic::Ordering::SeqCst);
    set_sink();
}

/// Whether [`read_configuration`] ran.
static CONFIGURED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The loggers of the crates below the runtime write here too.
fn set_sink() {
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

/// Opens the server log in `<output_base>` when no logging configuration was read
/// ([`configure_from_jvm_flags`]); otherwise does nothing.
///
/// PORT: Blaze only. Every server the `blaze` binary starts reads the client's
/// `javalog.properties`; this keeps a server log (`prefix=<output_base>/java.log`, as the client
/// configures it) for runtimes created without one (tests).
pub fn init(output_base: &blaze_vfs::Path) {
    if CONFIGURED.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let mut handler = SimpleLogHandler::builder()
        .set_prefix(output_base.get_relative_str("java.log").get_path_string())
        .build();
    handler.open_output_if_needed();
    *SERVER_LOG_HANDLER.lock() = Some(handler);
    set_sink();
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
// PORT: skipped, JVM-only (java.util.logging FileHandler): FileHandlerQuerier.<init>
// PORT: skipped, only the doc generator (not part of the bazel binary) throws it: Classpath.ClassPathException.<init>
// PORT: skipped, Google-internal remote logging: LoggingUtil.installRemoteLogger
// PORT: skipped, Google-internal remote logging: LoggingUtil.installRemoteLoggerForTesting
// PORT: skipped, Google-internal remote logging: LoggingUtil.getRemoteLogger
// PORT: skipped, JVM-only (class path scanning, used by docgen only): Classpath.findClasses

/// `LogManager.getLogManager().reset()`: removes the server log's handler from the root logger
/// and closes it. Later records are dropped (no handler), so nothing reopens the log file: after
/// `clean --expunge` the output base stays deleted.
pub fn close() {
    if let Some(mut handler) = SERVER_LOG_HANDLER.lock().take() {
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
    // Logger.isLoggable: the effective level of the logger (the root logger's when
    // setupLogging did not set one).
    let logger_level = TEMPLATE_LOGGER_LEVEL
        .lock()
        .map(level_int_value)
        .unwrap_or_else(|| *ROOT_LOGGER_LEVEL.lock());
    if level_int_value(level) < logger_level || logger_level == i32::MAX {
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
        handler.publish(level_int_value(level), &record);
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
        handler.publish(SEVERE, "Hello world\n"); // To open the log file.
        let log_path = handler.get_current_log_file_path().unwrap();
        handler.close();
        assert_eq!(std::fs::read_to_string(log_path).unwrap(), "Hello world\n");
    }

    const SEVERE: i32 = 1000;
    const WARNING: i32 = 900;
    const INFO: i32 = 800;
    const FINE: i32 = 500;
    const FINEST: i32 = 300;

    #[test]
    fn test_log_level_equal_published() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/info", tmp.path().display()))
            .set_log_level(blaze_options::converters::LogLevel::INFO)
            .build();
        handler.publish(INFO, "Hello\n");
        let log_path = handler.get_current_log_file_path();
        handler.close();
        assert!(std::fs::metadata(log_path.unwrap()).unwrap().len() > 0);
    }

    #[test]
    fn test_log_level_higher_published() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/info", tmp.path().display()))
            .set_log_level(blaze_options::converters::LogLevel::INFO)
            .build();
        handler.publish(WARNING, "Hello\n");
        let log_path = handler.get_current_log_file_path();
        handler.close();
        assert!(std::fs::metadata(log_path.unwrap()).unwrap().len() > 0);
    }

    #[test]
    fn test_log_level_lower_not_published() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/info", tmp.path().display()))
            .set_log_level(blaze_options::converters::LogLevel::INFO)
            .build();
        handler.publish(FINE, "Hello\n");
        let log_path = handler.get_current_log_file_path();
        handler.close();
        assert!(log_path.is_none());
    }

    #[test]
    fn test_log_level_default_all_published() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/all", tmp.path().display()))
            .build();
        handler.publish(FINEST, "Hello\n");
        let log_path = handler.get_current_log_file_path();
        handler.close();
        assert!(std::fs::metadata(log_path.unwrap()).unwrap().len() > 0);
    }

    #[test]
    fn test_rotate_limit_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        // Instant.parse("2018-01-01T12:00:00Z")
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/limits", tmp.path().display()))
            .set_rotate_limit_bytes(16)
            .set_clock_for_testing(1_514_808_000)
            .build();
        let symlink_path = handler.get_symbolic_link_path().cloned();
        handler.publish(SEVERE, "1234567\n"); // 8 bytes including "\n"
        let first_log_path = handler.get_current_log_file_path().unwrap();
        // Ensure the next file has a different name.
        handler.clock = Some(1_514_808_001);
        handler.publish(SEVERE, "1234567\n"); // 8 bytes including "\n"
        let second_log_path = handler.get_current_log_file_path().unwrap();
        handler.publish(SEVERE, "1234567\n"); // 8 bytes including "\n"
        handler.close();

        let symlink_path = symlink_path.unwrap();
        assert!(
            std::fs::symlink_metadata(&symlink_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(&symlink_path).unwrap().as_os_str(),
            second_log_path.file_name().unwrap()
        );
        assert_eq!(std::fs::metadata(&first_log_path).unwrap().len(), 16); // including two "\n"
        assert_eq!(std::fs::metadata(&second_log_path).unwrap().len(), 8); // including "\n"
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 3);
    }

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
        handler.publish(SEVERE, "New logs\n");
        assert_eq!(handler.get_current_log_file_path().unwrap(), log_path);
        handler.close();
        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap(),
            "Previous logs\nNew logs\n"
        );
    }

    fn new_file_of_size(dir: &StdPath, name: &str, size: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "\n".repeat(size)).unwrap();
        path
    }

    /// PORT: Java parses the old logs' timestamps in the default time zone and formats the
    /// fixed test clock in it too; Blaze's test clock is UTC and so is the parse with it.
    #[test]
    fn test_total_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut username = user_name();
        if username.is_empty() {
            username = "unknown_user".to_string();
        }
        let hostname = SimpleLogHandler::get_local_hostname_first_component();
        let base_filename = format!("hello.{hostname}.{username}.log.java.");
        let non_log = new_file_of_size(dir, "non_log", 16);
        let missing_date = new_file_of_size(dir, &format!("{base_filename}.123"), 16);
        let invalid_extension =
            new_file_of_size(dir, &format!("{base_filename}19900101-120000.invalid"), 16);
        let old_deleted1 =
            new_file_of_size(dir, &format!("{base_filename}19900101-120000.123"), 16);
        let old_deleted2 =
            new_file_of_size(dir, &format!("{base_filename}19950101-120000.123"), 16);
        let kept_then_deleted =
            new_file_of_size(dir, &format!("{base_filename}19990101-120000.123"), 16);
        let kept = new_file_of_size(dir, &format!("{base_filename}19990606-060000.123"), 16);

        // Instant.parse("2018-01-01T12:00:00Z")
        let mut handler = SimpleLogHandler::builder()
            .set_prefix(&format!("{}/hello", dir.display()))
            .set_pattern(".%h.%u.log.java.")
            .set_rotate_limit_bytes(16)
            .set_total_limit_bytes(40)
            .set_clock_for_testing(1_514_808_000)
            .build();
        // Print 8 bytes into the log file. Opening the log file triggers deletion of old logs.
        handler.publish(SEVERE, "1234567\n"); // 8 bytes including "\n"

        // We expect handler to delete all but 32 = 40 - 8 bytes worth of old log files.
        assert!(non_log.exists());
        assert!(missing_date.exists());
        assert!(invalid_extension.exists());
        assert!(!old_deleted1.exists());
        assert!(!old_deleted2.exists());
        assert!(kept_then_deleted.exists());
        assert!(kept.exists());

        handler.publish(SEVERE, "1234567\n"); // 8 bytes including "\n"
        let current_log_path = handler.get_current_log_file_path().unwrap();
        handler.close();

        // We expect another old log file to be deleted after rotation.
        assert!(!kept_then_deleted.exists());
        assert!(kept.exists());
        assert!(current_log_path.exists());
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/SimpleLogHandlerTest.java SimpleLogHandlerTest.getLoggerFilePath_onSimpleLogHandler_withFile_returnsPath
    #[test]
    #[ignore = "JVM-only: the LogHandlerQuerier service (Blaze queries its one handler directly)"]
    fn get_logger_file_path_on_simple_log_handler_with_file_returns_path() {}

    #[test]
    #[ignore = "JVM-only: the LogHandlerQuerier service (Blaze queries its one handler directly)"]
    fn get_logger_file_path_on_simple_log_handler_without_file_returns_empty() {}

    #[test]
    #[ignore = "JVM-only: the LogHandlerQuerier service (Blaze queries its one handler directly)"]
    fn get_logger_file_path_on_unsupported_log_handler_fails() {}

    #[test]
    #[ignore = "JVM-only: the LogHandlerQuerier service (Blaze queries its one handler directly)"]
    fn get_logger_file_path_on_missing_log_handler_fails() {}

    #[test]
    #[ignore = "JVM-only: Thread.interrupted() around the handler's I/O"]
    fn publish_handles_interrupt() {}

    #[test]
    fn load_properties_reads_the_clients_javalog_properties() {
        let props = load_properties(
            b"handlers=com.google.devtools.build.lib.util.SimpleLogHandler\n\
              .level=INFO\n\
              com.google.devtools.build.lib.util.SimpleLogHandler.prefix=/o/b\xc3\xa9/java.log\n\
              # comment\n  ! comment\n\
              key\\ with\\=sep : a \\\n    continued\\u0041\\t\n\
              bare\n",
        );
        assert_eq!(
            props.get("handlers").map(String::as_str),
            Some("com.google.devtools.build.lib.util.SimpleLogHandler")
        );
        assert_eq!(props.get(".level").map(String::as_str), Some("INFO"));
        // Read as ISO-8859-1: one char per byte (Bazel's internal string).
        assert_eq!(
            props
                .get("com.google.devtools.build.lib.util.SimpleLogHandler.prefix")
                .map(String::as_str),
            Some("/o/b\u{c3}\u{a9}/java.log")
        );
        assert_eq!(
            props.get("key with=sep").map(String::as_str),
            Some("a continuedA\t")
        );
        assert_eq!(props.get("bare").map(String::as_str), Some(""));
        assert_eq!(props.len(), 5);
    }
}
