//! The text Bazel prints to the terminal for events: the event path of
//! `lib/runtime/UiEventHandler`, `lib/runtime/LocationPrinter`, and the `--color=no` stripping
//! that `BlazeCommandDispatcher` puts in front of the UI.
//!
//! The seam: everything that needs the progress bar (`UiStateTracker`, the update thread, the
//! `@Subscribe` build-event methods, curses redraws) is not ported. Where the Java would consult
//! the state tracker, the code calls `unported!`, so it fails loudly instead of guessing. With
//! `--noshow_progress` or without cursor control (the default when not on a tty) the event path
//! runs completely. [`UiEventHandler::set_build_running`] stands in for the build-event
//! subscribers that flip `buildRunning` until they are ported.
//!
//! Formatting facts from the Java:
//! - An event prints as `<timestamp?><colour>KIND: <reset><location>: <message>` and then a
//!   newline unless the message already ends with `\n`. Multi-line messages (including Starlark
//!   tracebacks, which are part of the message text) are written verbatim; continuation lines are
//!   not indented or prefixed.
//! - Colours: FATAL/ERROR/FAIL red+bold, WARNING/CANCELLED magenta, INFO green, DEBUG yellow,
//!   SUBCOMMAND blue. `--color=no` writes the same bytes through [`AnsiStrippingOutputStream`].
//! - `--show_timestamps` prefixes `(HH:mm:ss) ` (local time) and, once a build is running, emits
//!   one `INFO: Current date is yyyy-MM-dd` first.
//! - "(cached)" is not event text: it comes from `TestSummaryPrinter`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_util::{check_argument, unported};
use parking_lot::{Mutex, RwLock};

use crate::event::Event;
use crate::event_kind::{EventKind, EventKindSet};
use crate::handler::EventHandler;
use crate::io::{AnsiStrippingOutputStream, ByteArrayOutputStream, OutErr, OutputStream};
use crate::location::Location;
use crate::terminal::{AnsiTerminal, Color};

/// Port of `com.google.devtools.build.lib.clock.Clock`.
///
/// PORT: belongs to `lib/clock`; lives here until that package is ported.
// Java: com/google/devtools/build/lib/clock/Clock.java:19 Clock
pub trait Clock: Send + Sync {
    /// Returns the current time in milliseconds since the epoch.
    fn current_time_millis(&self) -> i64;
    /// Returns the current time in nanoseconds, for measuring elapsed time.
    fn nano_time(&self) -> i64;
}

/// The wall clock (`JavaClock`).
pub struct JavaClock;

impl Clock for JavaClock {
    fn current_time_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn nano_time(&self) -> i64 {
        static START: std::sync::LazyLock<std::time::Instant> =
            std::sync::LazyLock::new(std::time::Instant::now);
        START.elapsed().as_nanos() as i64
    }
}

fn local_time(millis: i64) -> libc::tm {
    // Java: Instant.ofEpochMilli(millis).atZone(ZoneId.systemDefault()).
    // PORT: the system default zone is the C library's local time zone (TZ / /etc/localtime).
    let secs = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: `tm` is plain data; localtime_r only writes into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm
}

/// `TIMESTAMP_FORMAT.format(...)`: `(HH:mm:ss) ` in local time.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:104 UiEventHandler.TIMESTAMP_FORMAT
pub fn format_timestamp(millis: i64) -> String {
    let tm = local_time(millis);
    format!("({:02}:{:02}:{:02}) ", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// `DATE_FORMAT.format(...)`: `yyyy-MM-dd` in local time.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:106 UiEventHandler.DATE_FORMAT
pub fn format_date(millis: i64) -> String {
    let tm = local_time(millis);
    format!(
        "{:04}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday
    )
}

/// The colour for an event kind.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:630 UiEventHandler.setEventKindColor
pub fn set_event_kind_color(terminal: &AnsiTerminal, kind: EventKind) -> io::Result<()> {
    match kind {
        EventKind::Fatal | EventKind::Error | EventKind::Fail => {
            terminal.set_text_color(Color::Red)?;
            terminal.text_bold()
        }
        EventKind::Warning | EventKind::Cancelled => terminal.set_text_color(Color::Magenta),
        EventKind::Info => terminal.set_text_color(Color::Green),
        EventKind::Debug => terminal.set_text_color(Color::Yellow),
        EventKind::Subcommand => terminal.set_text_color(Color::Blue),
        _ => terminal.reset_terminal(),
    }
}

/// Terminate the line in the way appropriate for the operating system.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:1135 UiEventHandler.crlf
fn crlf(terminal: &AnsiTerminal) -> io::Result<()> {
    // System.lineSeparator() on Linux.
    terminal.write_string("\n")
}

/// Writes the text of a FATAL/ERROR/FAIL/WARNING/CANCELLED/INFO/DEBUG/SUBCOMMAND event: the
/// optional timestamp, the coloured `KIND: ` prefix, the location, the message, and a newline
/// unless the message already ends with one.
///
/// PORT: lines 463-483 of `UiEventHandler.handleLocked`, extracted unchanged so the text can be
/// produced without the rest of the UI. `timestamp` is `format_timestamp(now)` when
/// `--show_timestamps` is set.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:463 UiEventHandler.handleLocked
pub fn write_event_text(
    terminal: &AnsiTerminal,
    event: &Event,
    location_printer: &LocationPrinter,
    timestamp: Option<&str>,
) -> io::Result<()> {
    if let Some(timestamp) = timestamp {
        terminal.write_string(timestamp)?;
    }
    set_event_kind_color(terminal, event.get_kind())?;
    terminal.write_string(&format!("{}: ", event.get_kind()))?;
    terminal.reset_terminal()?;
    if let Some(location) = event.get_location() {
        terminal.write_string(&format!(
            "{}: ",
            location_printer.get_location_string(location)
        ))?;
    }
    // Java's message is never null.
    let message = event.get_message();
    terminal.write_string(&message)?;
    let incomplete_line = !message.ends_with('\n');
    if incomplete_line {
        crlf(terminal)?;
    }
    Ok(())
}

/// The bytes the UI prints for one event (see [`write_event_text`]), with colour codes when
/// `use_color` (`--color=yes`), or stripped as `--color=no` does.
///
/// PORT: a convenience composition of `UiEventHandler.handleLocked`'s event text and
/// `BlazeCommandDispatcher.ansiStripErr`; it has no Java counterpart of its own.
pub fn format_event(
    event: &Event,
    location_printer: &LocationPrinter,
    timestamp: Option<&str>,
    use_color: bool,
) -> Vec<u8> {
    let buffer = Arc::new(ByteArrayOutputStream::new());
    let sink: Arc<dyn OutputStream> = if use_color {
        buffer.clone()
    } else {
        Arc::new(AnsiStrippingOutputStream::new(buffer.clone()))
    };
    write_event_text(&AnsiTerminal::new(sink), event, location_printer, timestamp)
        .expect("writing to memory cannot fail");
    buffer.to_byte_array()
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:969 BlazeCommandDispatcher.ansiStripOut
pub fn ansi_strip_out(out_err: &OutErr) -> OutErr {
    let wrapped_out = Arc::new(AnsiStrippingOutputStream::new(
        out_err.get_output_stream().clone(),
    ));
    OutErr::create(wrapped_out, out_err.get_error_stream().clone())
}

// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:974 BlazeCommandDispatcher.ansiStripErr
pub fn ansi_strip_err(out_err: &OutErr) -> OutErr {
    let wrapped_err = Arc::new(AnsiStrippingOutputStream::new(
        out_err.get_error_stream().clone(),
    ));
    OutErr::create(out_err.get_output_stream().clone(), wrapped_err)
}

fn is_absolute(p: &str) -> bool {
    p.starts_with('/')
}

/// `PathFragment.create(path).getPathString()` for paths that are already normalized.
///
/// PORT: PathFragment normalization (`.`, `..`, `//`, trailing `/`) is not ported here; a path
/// that needs it fails loudly.
fn path_fragment_create(p: &str) -> &str {
    let normalized = p.is_empty()
        || p == "/"
        || (!p.ends_with('/')
            && p.strip_prefix('/')
                .unwrap_or(p)
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."));
    if !normalized {
        unported!("PathFragment.create", "normalizing {p:?}");
    }
    p
}

fn drive_str_length(p: &str) -> usize {
    if is_absolute(p) { 1 } else { 0 }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:412 PathFragment.startsWith
fn fragment_starts_with(p: &str, other: &str) -> bool {
    if other.len() > p.len() {
        return false;
    }
    if drive_str_length(p) != drive_str_length(other) {
        return false;
    }
    if !p.starts_with(other) {
        return false;
    }
    p.len() == other.len()
        || other.len() == drive_str_length(p)
        || p.as_bytes()[other.len()] == b'/'
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:369 PathFragment.relativeTo
fn fragment_relative_to<'a>(p: &'a str, base: &str) -> &'a str {
    check_argument!(
        is_absolute(p) == is_absolute(base),
        "Cannot relativize an absolute and a non-absolute path pair"
    );
    check_argument!(
        p.starts_with(base),
        "Path '{}' is not under '{}', cannot relativize",
        p,
        base
    );
    let bn = base.len();
    if bn == 0 {
        return p;
    }
    if p.len() == bn {
        return "";
    }
    let last_slash_index = if base.ends_with('/') { bn - 1 } else { bn };
    check_argument!(
        p.as_bytes()[last_slash_index] == b'/',
        "Path '{}' is not under '{}', cannot relativize",
        p,
        base
    );
    &p[last_slash_index + 1..]
}

/// Port of `com.google.devtools.build.lib.runtime.LocationPrinter`.
///
/// PORT: package path roots are given as their absolute path strings (`Root.fromPath`); the
/// `PathPackageLocator` that supplies them is not ported.
// Java: com/google/devtools/build/lib/runtime/LocationPrinter.java:25 LocationPrinter
pub struct LocationPrinter {
    attempt_to_print_relative_paths: bool,
    workspace_path_fragment: Option<String>,
    package_path_roots_ref: RwLock<Vec<String>>,
}

impl LocationPrinter {
    // Java: com/google/devtools/build/lib/runtime/LocationPrinter.java:31 LocationPrinter.LocationPrinter
    pub fn new(
        attempt_to_print_relative_paths: bool,
        workspace_path_fragment: Option<String>,
    ) -> Self {
        LocationPrinter {
            attempt_to_print_relative_paths,
            workspace_path_fragment,
            package_path_roots_ref: RwLock::new(Vec::new()),
        }
    }

    /// PORT: takes `packageLocator.getPathEntries()` as path strings.
    // Java: com/google/devtools/build/lib/runtime/LocationPrinter.java:38 LocationPrinter.packageLocatorCreated
    pub fn package_locator_created(&self, path_entries: Vec<String>) {
        *self.package_path_roots_ref.write() = path_entries;
    }

    // Java: com/google/devtools/build/lib/runtime/LocationPrinter.java:42 LocationPrinter.getLocationString
    pub fn get_location_string(&self, location: &Location) -> String {
        if self.attempt_to_print_relative_paths {
            LocationPrinter::get_relative_location_string(
                location,
                self.workspace_path_fragment.as_deref(),
                &self.package_path_roots_ref.read(),
            )
        } else {
            location.to_string()
        }
    }

    // Java: com/google/devtools/build/lib/runtime/LocationPrinter.java:49 LocationPrinter.getRelativeLocationString
    pub fn get_relative_location_string(
        location: &Location,
        workspace_path_fragment: Option<&str>,
        package_path_roots: &[String],
    ) -> String {
        let mut relative_path_to_use: Option<&str> = None;
        let location_path_fragment = path_fragment_create(location.file());
        if is_absolute(location_path_fragment) {
            match workspace_path_fragment.map(path_fragment_create) {
                Some(workspace) if fragment_starts_with(location_path_fragment, workspace) => {
                    relative_path_to_use =
                        Some(fragment_relative_to(location_path_fragment, workspace));
                }
                _ => {
                    for package_path_root in package_path_roots {
                        let root = path_fragment_create(package_path_root);
                        // Java: com/google/devtools/build/lib/vfs/Root.java:118 PathRoot.contains
                        if is_absolute(location_path_fragment)
                            && fragment_starts_with(location_path_fragment, root)
                        {
                            // Java: com/google/devtools/build/lib/vfs/Root.java:107 PathRoot.relativize
                            relative_path_to_use =
                                Some(fragment_relative_to(location_path_fragment, root));
                            break;
                        }
                    }
                }
            }
        }

        let mut b = String::new();
        b.push_str(relative_path_to_use.unwrap_or(location_path_fragment));
        let line = location.line();
        if line != 0 {
            b.push(':');
            b.push_str(&line.to_string());
            let column = location.column();
            if column != 0 {
                b.push(':');
                b.push_str(&column.to_string());
            }
        }
        b
    }
}

/// The `UiOptions` values the event path reads.
///
/// PORT: `UiOptions` is transcribed with the options crate; until then callers fill this from
/// it. `Default` holds the option defaults at the pin (`--show_progress=true`, `--curses=auto`
/// with `--isatty=false` so no cursor control, `--show_timestamps=false`,
/// `--experimental_ui_debug_all_events=false`, `--attempt_to_print_relative_paths=false`, no
/// `--ui_event_filters`, `--experimental_ui_max_stdouterr_bytes=1048576`).
#[derive(Clone, Debug)]
pub struct UiEventHandlerOptions {
    pub show_progress: bool,
    /// `UiOptions.useCursorControl()`.
    pub use_cursor_control: bool,
    pub show_timestamp: bool,
    pub experimental_ui_debug_all_events: bool,
    pub attempt_to_print_relative_paths: bool,
    /// `UiOptions.getFilteredEventKinds()`.
    pub filtered_event_kinds: EventKindSet,
    pub max_stdout_err_bytes: i32,
}

impl Default for UiEventHandlerOptions {
    fn default() -> Self {
        UiEventHandlerOptions {
            show_progress: true,
            use_cursor_control: false,
            show_timestamp: false,
            experimental_ui_debug_all_events: false,
            attempt_to_print_relative_paths: false,
            filtered_event_kinds: EventKindSet::EMPTY,
            max_stdout_err_bytes: 1048576,
        }
    }
}

/// An output stream that wraps another output stream and that fully buffers writes until
/// flushed.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:148 UiEventHandler.FullyBufferedOutputStream
struct FullyBufferedOutputStream {
    buffer: Mutex<Vec<u8>>,
    /// The (possibly unbuffered) stream wrapped by this one.
    wrapped: Arc<dyn OutputStream>,
}

impl OutputStream for FullyBufferedOutputStream {
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.buffer.lock().extend_from_slice(bytes);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:162 FullyBufferedOutputStream.flush
    fn flush(&self) -> io::Result<()> {
        // If we failed to write our current buffered contents to the output, there is not much
        // we can do because reporting an error would require another write, and that write would
        // probably fail. So, instead, we silently discard whatever was previously buffered in the
        // hopes that the data itself was what caused the problem.
        let buffered = std::mem::take(&mut *self.buffer.lock());
        self.wrapped.write(&buffered)?;
        self.wrapped.flush()
    }
}

/// The state Java guards with `synchronized (this)`.
struct UiState {
    show_progress: bool,
    build_running: bool,
    num_lines_progress_bar: i32,
    stdout_line_buffer: Vec<u8>,
    stderr_line_buffer: Vec<u8>,
}

/// Presents events to the user in the terminal. Port of the event path of `UiEventHandler`; see
/// the module documentation for the seam.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:91 UiEventHandler
pub struct UiEventHandler {
    quiet: bool,
    cursor_control: bool,
    clock: Arc<dyn Clock>,
    terminal: AnsiTerminal,
    debug_all_events: bool,
    location_printer: LocationPrinter,
    show_timestamp: bool,
    out_err: OutErr,
    filtered_event_kinds: EventKindSet,
    date_shown: AtomicBool,
    shutdown: AtomicBool,
    max_stdout_err_bytes: i32,
    state: Mutex<UiState>,
}

impl UiEventHandler {
    /// PORT: Java also takes the `EventBus`, skymeld mode, `newStatsSummary` and a
    /// `TerminalSizeMonitor`; they only feed the progress bar, which is not ported.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:198 UiEventHandler.UiEventHandler
    pub fn new(
        out_err: OutErr,
        options: &UiEventHandlerOptions,
        quiet: bool,
        clock: Arc<dyn Clock>,
        workspace_path_fragment: Option<String>,
    ) -> Self {
        let out_err = OutErr::create(
            Arc::new(FullyBufferedOutputStream {
                buffer: Mutex::new(Vec::new()),
                wrapped: out_err.get_output_stream().clone(),
            }),
            Arc::new(FullyBufferedOutputStream {
                buffer: Mutex::new(Vec::new()),
                wrapped: out_err.get_error_stream().clone(),
            }),
        );
        UiEventHandler {
            quiet,
            cursor_control: options.use_cursor_control,
            terminal: AnsiTerminal::new(out_err.get_error_stream().clone()),
            show_timestamp: options.show_timestamp,
            clock,
            debug_all_events: options.experimental_ui_debug_all_events,
            location_printer: LocationPrinter::new(
                options.attempt_to_print_relative_paths,
                workspace_path_fragment,
            ),
            out_err,
            filtered_event_kinds: options.filtered_event_kinds,
            date_shown: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            max_stdout_err_bytes: options.max_stdout_err_bytes,
            state: Mutex::new(UiState {
                show_progress: options.show_progress,
                build_running: false,
                num_lines_progress_bar: 0,
                stdout_line_buffer: Vec::new(),
                stderr_line_buffer: Vec::new(),
            }),
        }
    }

    /// Seam: sets `buildRunning` the way the unported `@Subscribe` methods do
    /// (`mainRepoMappingComputationStarted`, `noBuild`, `buildComplete`, ...). Remove once those
    /// methods are ported.
    pub fn set_build_running(&self, build_running: bool) {
        self.state.lock().build_running = build_running;
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:772 UiEventHandler.packageLocatorCreated
    pub fn package_locator_created(&self, path_entries: Vec<String>) {
        self.location_printer.package_locator_created(path_entries);
    }

    /// Disables progress, clearing the progress bar if it is currently shown. Returns false if
    /// progress was not shown, true if it was enabled before this call.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:317 UiEventHandler.disableProgress
    pub fn disable_progress(&self) -> io::Result<bool> {
        let mut st = self.state.lock();
        if !st.show_progress {
            return Ok(false);
        }
        self.clear_progress_bar(&mut st)?;
        self.terminal.flush()?;
        st.show_progress = false;
        Ok(true)
    }

    /// Enables progress and writes the progress bar. This is a no-op if progress is already
    /// enabled.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:332 UiEventHandler.enableProgress
    pub fn enable_progress(&self) -> io::Result<()> {
        let mut st = self.state.lock();
        if st.show_progress {
            return Ok(());
        }
        st.show_progress = true;
        self.add_progress_bar(&mut st)?;
        self.terminal.flush()
    }

    /// Flush buffers for stdout and stderr. Return if either of them flushed a non-zero number
    /// of symbols.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:345 UiEventHandler.flushStdOutStdErrBuffers
    fn flush_std_out_std_err_buffers(&self, st: &mut UiState) -> bool {
        let mut did_flush = false;
        let result = (|| -> io::Result<()> {
            if !st.stdout_line_buffer.is_empty() {
                self.out_err
                    .get_output_stream()
                    .write(&st.stdout_line_buffer)?;
                self.out_err.get_output_stream().flush()?;
                // Re-initialize the stream not to retain allocated memory.
                st.stdout_line_buffer = Vec::new();
                did_flush = true;
            }
            if !st.stderr_line_buffer.is_empty() {
                self.out_err
                    .get_error_stream()
                    .write(&st.stderr_line_buffer)?;
                self.out_err.get_error_stream().flush()?;
                // Re-initialize the stream not to retain allocated memory.
                st.stderr_line_buffer = Vec::new();
                did_flush = true;
            }
            Ok(())
        })();
        // PORT: Java logs "IO Error writing to output stream" at WARNING; logging is not ported.
        let _ = result;
        did_flush
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:368 UiEventHandler.maybeAddDate
    fn maybe_add_date(&self, st: &mut UiState) {
        if !self.show_timestamp || !st.build_running || self.date_shown.swap(true, Ordering::SeqCst)
        {
            return;
        }
        // Java calls handle() while holding the monitor (re-entrant); we pass the held state.
        let event = Event::info(format!(
            "Current date is {}",
            format_date(self.clock.current_time_millis())
        ));
        self.handle_with_state(&event, Some(st));
    }

    /// Process events in debug mode, which causes all events to be dumped to the terminal.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:390 UiEventHandler.handleLockedDebug
    fn handle_locked_debug(
        &self,
        st: &mut UiState,
        event: &Event,
        stdout: Option<&[u8]>,
        stderr: Option<&[u8]>,
    ) -> io::Result<()> {
        // Debugging only: show all events visible to the new UI.
        self.clear_progress_bar(st)?;
        self.terminal.flush()?;
        let stream = self.out_err.get_output_stream();
        // `(event + "\n").getBytes(StandardCharsets.ISO_8859_1)`
        stream.write(&blaze_util::string_encoding::latin1_encode_lossy(&format!(
            "{event}\n"
        )))?;
        if let Some(stdout) = stdout {
            stream.write(b"... with STDOUT: ")?;
            stream.write(stdout)?;
            stream.write(b"\n")?;
        }
        if let Some(stderr) = stderr {
            stream.write(b"... with STDERR: ")?;
            stream.write(stderr)?;
            stream.write(b"\n")?;
        }
        stream.flush()?;
        self.add_progress_bar(st)?;
        self.terminal.flush()
    }

    /// Process events in non-debug mode, which filters out and pretty-prints some events.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:424 UiEventHandler.handleLocked
    fn handle_locked(
        &self,
        st: &mut UiState,
        event: &Event,
        stdout: Option<&[u8]>,
        stderr: Option<&[u8]>,
    ) -> io::Result<()> {
        self.maybe_add_date(st);
        match event.get_kind() {
            EventKind::Stdout | EventKind::Stderr => {
                let stream = if event.get_kind() == EventKind::Stdout {
                    self.out_err.get_output_stream()
                } else {
                    self.out_err.get_error_stream()
                };
                if !st.build_running {
                    stream.write(event.get_message_bytes())?;
                    stream.flush()?;
                } else {
                    let cleared_progress = self.write_to_stream(
                        st,
                        stream,
                        event.get_kind(),
                        event.get_message_bytes(),
                    )?;
                    if cleared_progress && st.show_progress && self.cursor_control {
                        self.add_progress_bar(st)?;
                    }
                    self.terminal.flush()?;
                }
            }
            EventKind::Fatal
            | EventKind::Error
            | EventKind::Fail
            | EventKind::Warning
            | EventKind::Cancelled
            | EventKind::Info
            | EventKind::Debug
            | EventKind::Subcommand => {
                if st.show_progress && st.build_running {
                    self.clear_progress_bar(st)?;
                }
                let incomplete_line = self.flush_std_out_std_err_buffers(st);
                if incomplete_line {
                    crlf(&self.terminal)?;
                }
                let timestamp = if self.show_timestamp {
                    Some(format_timestamp(self.clock.current_time_millis()))
                } else {
                    None
                };
                write_event_text(
                    &self.terminal,
                    event,
                    &self.location_printer,
                    timestamp.as_deref(),
                )?;
                if let Some(stderr) = stderr {
                    let err = self.out_err.get_error_stream();
                    self.write_to_stream(st, err, EventKind::Stderr, stderr)?;
                    err.flush()?;
                }
                if let Some(stdout) = stdout {
                    let out = self.out_err.get_output_stream();
                    self.write_to_stream(st, out, EventKind::Stdout, stdout)?;
                    out.flush()?;
                }
                if st.show_progress && st.build_running && self.cursor_control {
                    self.add_progress_bar(st)?;
                }
                self.terminal.flush()?;
            }
            kind @ (EventKind::Progress
            | EventKind::Start
            | EventKind::Finish
            | EventKind::Pass
            | EventKind::Timeout
            | EventKind::Depchecker) => {
                if kind == EventKind::Progress {
                    // Java: if (stateTracker.progressBarTimeDependent()) refresh();
                    // PORT: refresh() only redraws when showProgress and buildRunning; otherwise
                    // the answer cannot affect output.
                    if st.show_progress && st.build_running {
                        unported!("UiStateTracker.progressBarTimeDependent");
                    }
                }
                // Fall through.
                if stdout.is_some() || stderr.is_some() {
                    unported!(
                        "BugReport.sendBugReport",
                        "IllegalStateException: stdout/stderr should not be present for this \
                         event {}",
                        event
                    );
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:518 UiEventHandler.getContentIfSmallEnough
    fn get_content_if_small_enough(
        &self,
        name: &str,
        size: i64,
        get_content: impl FnOnce() -> Vec<u8>,
        get_path: impl FnOnce() -> String,
    ) -> Option<Vec<u8>> {
        if size == 0 {
            // Avoid any possible I/O when we know it'll be empty anyway.
            return None;
        }

        if size <= self.max_stdout_err_bytes as i64 {
            Some(get_content())
        } else {
            // `String.format(...).getBytes(StandardCharsets.ISO_8859_1)`
            Some(
                blaze_util::string_encoding::latin1_encode_lossy(&format!(
                    "{} ({}) {} exceeds maximum size of \
                     --experimental_ui_max_stdouterr_bytes={} bytes; skipping\n",
                    name,
                    get_path(),
                    size,
                    self.max_stdout_err_bytes
                ))
                .into_owned(),
            )
        }
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:536 UiEventHandler.handleInternal
    fn handle_internal(&self, event: &Event, held: Option<&mut UiState>) {
        let event_kind = event.get_kind();
        if self.quiet {
            match event_kind {
                EventKind::Error | EventKind::Fatal | EventKind::Stdout | EventKind::Stderr => {}
                _ => return,
            }
        }

        if self.filtered_event_kinds.contains(event_kind) {
            return;
        }
        let result = (|| -> io::Result<()> {
            // stdout and stderr may be files. Buffer them in memory to avoid doing I/O in the
            // critical sections of handleLocked*, at the expense of having to cap their size to
            // avoid using too much memory.
            let mut stdout = None;
            let mut stderr = None;
            if let Some(process_output) = event.get_process_output() {
                stdout = self.get_content_if_small_enough(
                    "stdout",
                    process_output.get_std_out_size()?,
                    || process_output.get_std_out(),
                    || process_output.get_std_out_path(),
                );
                stderr = self.get_content_if_small_enough(
                    "stderr",
                    process_output.get_std_err_size()?,
                    || process_output.get_std_err(),
                    || process_output.get_std_err_path(),
                );
            }

            let run = |st: &mut UiState| {
                if self.debug_all_events {
                    self.handle_locked_debug(st, event, stdout.as_deref(), stderr.as_deref())
                } else {
                    self.handle_locked(st, event, stdout.as_deref(), stderr.as_deref())
                }
            };
            match held {
                Some(st) => run(st),
                None => run(&mut self.state.lock()),
            }
        })();
        // PORT: Java logs "IO Error writing to output stream" at WARNING; logging is not ported.
        let _ = result;
    }

    /// `handle(event)` for a caller that already holds the state (Java's re-entrant monitor).
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:583 UiEventHandler.handle
    fn handle_with_state(&self, event: &Event, held: Option<&mut UiState>) {
        if !self.debug_all_events
            && !self.show_timestamp
            && matches!(
                event.get_kind(),
                EventKind::Start
                    | EventKind::Finish
                    | EventKind::Pass
                    | EventKind::Timeout
                    | EventKind::Depchecker
            )
        {
            // Keep this in sync with the list of no-op event kinds in handleLocked above.
            return;
        }

        let held = match held {
            Some(st) => {
                // Ensure that default progress messages are not displayed after a FATAL event.
                if event.get_kind() == EventKind::Fatal {
                    st.build_running = false;
                    self.stop_update_thread();
                }
                Some(st)
            }
            None => {
                if event.get_kind() == EventKind::Fatal {
                    self.state.lock().build_running = false;
                    self.stop_update_thread();
                }
                None
            }
        };

        self.handle_internal(event, held);
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:606 UiEventHandler.writeToStream
    fn write_to_stream(
        &self,
        st: &mut UiState,
        stream: &Arc<dyn OutputStream>,
        event_kind: EventKind,
        message: &[u8],
    ) -> io::Result<bool> {
        let eol_index = message.iter().rposition(|&b| b == b'\n');
        let Some(eol_index) = eol_index else {
            let out_line_buffer = if event_kind == EventKind::Stdout {
                &mut st.stdout_line_buffer
            } else {
                &mut st.stderr_line_buffer
            };
            out_line_buffer.extend_from_slice(message);
            return Ok(false);
        };

        self.clear_progress_bar(st)?;
        self.terminal.flush()?;

        let out_line_buffer = if event_kind == EventKind::Stdout {
            &mut st.stdout_line_buffer
        } else {
            &mut st.stderr_line_buffer
        };
        // Write the buffer so far + the rest of the line (including newline).
        stream.write(out_line_buffer)?;
        out_line_buffer.clear();

        stream.write(&message[..=eol_index])?;
        stream.flush()?;

        out_line_buffer.extend_from_slice(&message[eol_index + 1..]);
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:738 UiEventHandler.completeBuild
    fn complete_build(&self) {
        let mut st = self.state.lock();
        if !st.build_running {
            return;
        }
        st.build_running = false;
        // Have to set this, otherwise there's a lingering "checking cached actions" message for
        // the `mod` command, which doesn't even run any actions.
        unported!("UiStateTracker.setBuildComplete");
    }

    /// PORT: Java's `@Subscribe afterCommand(AfterCommandEvent)`; the event carries nothing the
    /// handler reads.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:793 UiEventHandler.afterCommand
    pub fn after_command(&self) {
        self.state.lock().build_running = false;
        self.complete_build();
        let result = (|| -> io::Result<()> {
            self.flush_std_out_std_err_buffers(&mut self.state.lock());
            self.terminal.reset_terminal()?;
            self.terminal.flush()
        })();
        // PORT: Java logs "IO Error writing to user terminal" at WARNING.
        let _ = result;
    }

    /// Stop the update thread and wait for it to terminate.
    ///
    /// PORT: the update thread only redraws the progress bar and is not ported, so there is no
    /// thread to join.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:1112 UiEventHandler.stopUpdateThread
    fn stop_update_thread(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:1122 UiEventHandler.clearProgressBar
    fn clear_progress_bar(&self, st: &mut UiState) -> io::Result<()> {
        if !self.cursor_control {
            return Ok(());
        }
        for _ in 0..st.num_lines_progress_bar {
            self.terminal.cr()?;
            self.terminal.cursor_up(1)?;
            self.terminal.clear_line()?;
        }
        st.num_lines_progress_bar = 0;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:1139 UiEventHandler.addProgressBar
    fn add_progress_bar(&self, _st: &mut UiState) -> io::Result<()> {
        if self.quiet {
            return Ok(());
        }
        unported!("UiStateTracker.writeProgressBar");
    }
}

impl EventHandler for UiEventHandler {
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:583 UiEventHandler.handle
    fn handle(&self, event: &Event) {
        self.handle_with_state(event, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI64;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java UiEventHandlerTest.BUILD_DID_NOT_COMPLETE_MESSAGE
    const BUILD_DID_NOT_COMPLETE_MESSAGE: &str =
        "\x1b[31m\x1b[1mERROR: \x1b[0mBuild did NOT complete successfully\n";

    /// `ManualClock` from lib/testutil.
    #[derive(Default)]
    struct ManualClock(AtomicI64);

    impl Clock for ManualClock {
        fn current_time_millis(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
        fn nano_time(&self) -> i64 {
            self.0.load(Ordering::SeqCst) * 1_000_000
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java UiEventHandlerTest.FlushCollectingOutputStream
    #[derive(Default)]
    struct FlushCollectingOutputStream {
        state: Mutex<(Vec<String>, String)>,
    }

    impl OutputStream for FlushCollectingOutputStream {
        fn write(&self, bytes: &[u8]) -> io::Result<()> {
            self.state
                .lock()
                .1
                .push_str(std::str::from_utf8(bytes).unwrap());
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            let mut st = self.state.lock();
            // Ignore inconsequential extra flushes.
            if !st.1.is_empty() {
                let written = std::mem::take(&mut st.1);
                st.0.push(written);
            }
            Ok(())
        }
    }

    impl FlushCollectingOutputStream {
        fn assert_flushed(&self, messages: &[&str]) {
            let st = self.state.lock();
            assert!(st.1.is_empty(), "written since flush: {:?}", st.1);
            assert_eq!(st.0, messages);
        }
    }

    /// Stands in for `mock(OutputStream.class)`.
    struct DiscardingOutputStream;

    impl OutputStream for DiscardingOutputStream {
        fn write(&self, _bytes: &[u8]) -> io::Result<()> {
            Ok(())
        }
    }

    /// `createUiEventHandler(outputKind)`. PORT: the Java helper then calls
    /// `mainRepoMappingComputationStarted` and `buildStarted`; their only effect the event path
    /// sees is `buildRunning = true` (the state tracker has no activities, so nothing is drawn),
    /// which the seam sets directly. The `skymeldMode` parameter only selects the state tracker.
    fn create_ui_event_handler(
        output_kind: EventKind,
        options: &UiEventHandlerOptions,
    ) -> (Arc<FlushCollectingOutputStream>, UiEventHandler) {
        let output = Arc::new(FlushCollectingOutputStream::default());
        let out_err = match output_kind {
            EventKind::Stdout => OutErr::create(output.clone(), Arc::new(DiscardingOutputStream)),
            EventKind::Stderr => OutErr::create(Arc::new(DiscardingOutputStream), output.clone()),
            other => panic!("AssertionError: {other}"),
        };
        let handler = UiEventHandler::new(
            out_err,
            options,
            /* quiet= */ false,
            Arc::new(ManualClock::default()),
            /* workspacePathFragment= */ None,
        );
        handler.set_build_running(true);
        (output, handler)
    }

    /// One `#[test]` per value of the `outputKind` test parameter of `StdoutAndStderrTest`.
    macro_rules! stdout_and_stderr_test {
        ($(#[$attr:meta])* $name:ident, $body:expr) => {
            mod $name {
                use super::*;

                $(#[$attr])*
                #[test]
                fn stdout() {
                    let body: fn(EventKind) = $body;
                    body(EventKind::Stdout);
                }

                $(#[$attr])*
                #[test]
                fn stderr() {
                    let body: fn(EventKind) = $body;
                    body(EventKind::Stderr);
                }
            }
        };
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_outputsBuildFailedOnStderr
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.buildComplete (progress bar UI)"]
        build_complete_outputs_build_failed_on_stderr,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_flushesBufferedMessage
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.buildComplete (progress bar UI)"]
        build_complete_flushes_buffered_message,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_successfulBuild
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.buildComplete (progress bar UI)"]
        build_complete_successful_build,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.buildComplete_emptyBuffer_outputsBuildFailedOnStderr
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.buildComplete (progress bar UI)"]
        build_complete_empty_buffer_outputs_build_failed_on_stderr,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_buffersWithoutNewline
    stdout_and_stderr_test!(handle_output_event_buffers_without_newline, |kind| {
        let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
        h.handle(&Event::of(kind, "hello"));
        output.assert_flushed(&[]);
    });

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_concatenatesInBuffer
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.buildComplete (progress bar UI)"]
        handle_output_event_concatenates_in_buffer,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_flushesOnNewline
    stdout_and_stderr_test!(handle_output_event_flushes_on_newline, |kind| {
        let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
        h.handle(&Event::of(kind, "hello\n"));
        output.assert_flushed(&["hello\n"]);
    });

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_flushesOnlyUntilNewline
    stdout_and_stderr_test!(handle_output_event_flushes_only_until_newline, |kind| {
        let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
        h.handle(&Event::of(kind, "hello\nworld"));
        output.assert_flushed(&["hello\n"]);
    });

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_flushesUntilLastNewline
    stdout_and_stderr_test!(handle_output_event_flushes_until_last_newline, |kind| {
        let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
        h.handle(&Event::of(kind, "hello\nto\neveryone"));
        output.assert_flushed(&["hello\nto\n"]);
    });

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_flushesMultiLineMessageAtOnce
    stdout_and_stderr_test!(
        handle_output_event_flushes_multi_line_message_at_once,
        |kind| {
            let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
            h.handle(&Event::of(kind, "hello\neveryone\n"));
            output.assert_flushed(&["hello\neveryone\n"]);
        }
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.handleOutputEvent_concatenatesBufferBeforeFlushingOnNewline
    stdout_and_stderr_test!(
        handle_output_event_concatenates_buffer_before_flushing_on_newline,
        |kind| {
            let (output, h) = create_ui_event_handler(kind, &UiEventHandlerOptions::default());
            h.handle(&Event::of(kind, "hello"));
            h.handle(&Event::of(kind, " there!\nmore text"));
            output.assert_flushed(&["hello there!\n"]);
        }
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutAndStderrTest.noChangeOnUnflushedWrite
    stdout_and_stderr_test!(
        #[ignore = "needs UiStateTracker.writeProgressBar (curses progress bar)"]
        no_change_on_unflushed_write,
        |_| {}
    );

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StdoutOnlyTest.handleOutputEvent_flushesRemainingLines
    #[test]
    fn handle_output_event_flushes_remaining_lines() {
        let (output, h) =
            create_ui_event_handler(EventKind::Stdout, &UiEventHandlerOptions::default());
        h.handle(&Event::of(EventKind::Stdout, "hello\nto\neveryone"));
        output.assert_flushed(&["hello\nto\n"]);
        h.after_command();
        output.assert_flushed(&["hello\nto\n", "everyone"]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.buildCompleteMessageDoesntOverrideError
    #[test]
    #[ignore = "needs UiStateTracker (curses progress bar, buildComplete)"]
    fn build_complete_message_doesnt_override_error() {}

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.temporarilyDisableProgress
    #[test]
    #[ignore = "needs UiStateTracker and action events (progress bar)"]
    fn temporarily_disable_progress() {}

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.terminalSizeChangeRefreshesProgressBarWithNewWidth
    #[test]
    #[ignore = "needs TerminalSizeMonitor and UiStateTracker (progress bar)"]
    fn terminal_size_change_refreshes_progress_bar_with_new_width() {}

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.progressOff_disableProgressReturnsFalse
    #[test]
    fn progress_off_disable_progress_returns_false() {
        let options = UiEventHandlerOptions {
            show_progress: false,
            ..UiEventHandlerOptions::default()
        };
        let (_output, h) = create_ui_event_handler(EventKind::Stderr, &options);
        assert!(!h.disable_progress().unwrap());
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/UiEventHandlerTest.java StderrOnlyTest.progressAlreadyDisabled_disableProgressReturnsFalse
    #[test]
    fn progress_already_disabled_disable_progress_returns_false() {
        let options = UiEventHandlerOptions {
            show_progress: true,
            ..UiEventHandlerOptions::default()
        };
        let (_output, h) = create_ui_event_handler(EventKind::Stderr, &options);
        assert!(h.disable_progress().unwrap());
        assert!(!h.disable_progress().unwrap());
    }

    // Expected text: UiEventHandlerTest.BUILD_DID_NOT_COMPLETE_MESSAGE, which buildComplete
    // prints through handleLocked as an ERROR event without location.
    #[test]
    fn error_event_text_matches_ui_event_handler_test_constant() {
        let (output, h) =
            create_ui_event_handler(EventKind::Stderr, &UiEventHandlerOptions::default());
        h.handle(&Event::error("Build did NOT complete successfully"));
        output.assert_flushed(&[BUILD_DID_NOT_COMPLETE_MESSAGE]);

        let printer = LocationPrinter::new(false, None);
        let event = Event::error("Build did NOT complete successfully");
        assert_eq!(
            format_event(&event, &printer, None, true),
            BUILD_DID_NOT_COMPLETE_MESSAGE.as_bytes()
        );
        assert_eq!(
            format_event(&event, &printer, None, false),
            b"ERROR: Build did NOT complete successfully\n"
        );
    }

    // Expected text: UiEventHandlerTest.buildComplete_successfulBuild.
    #[test]
    fn info_event_text_matches_ui_event_handler_test_constant() {
        let printer = LocationPrinter::new(false, None);
        assert_eq!(
            format_event(
                &Event::info("Build completed successfully, 0 total actions"),
                &printer,
                None,
                true
            ),
            b"\x1b[32mINFO: \x1b[0mBuild completed successfully, 0 total actions\n"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/LocationPrinterTest.java LocationPrinterTest.getRelativeLocationString_PathIsAlreadyRelative
    #[test]
    fn get_relative_location_string_path_is_already_relative() {
        assert_eq!(
            LocationPrinter::get_relative_location_string(
                &Location::from_file_line_column("relative/path", 4, 2),
                Some("/this/is/the/workspace"),
                &["/this/is/a/package/path/root".to_string()]
            ),
            "relative/path:4:2"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/LocationPrinterTest.java LocationPrinterTest.getRelativeLocationString_PathIsAbsoluteAndWorkspaceIsNull
    #[test]
    fn get_relative_location_string_path_is_absolute_and_workspace_is_null() {
        assert_eq!(
            LocationPrinter::get_relative_location_string(
                &Location::from_file_line_column("/absolute/path", 4, 2),
                None,
                &["/this/is/a/package/path/root".to_string()]
            ),
            "/absolute/path:4:2"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/LocationPrinterTest.java LocationPrinterTest.getRelativeLocationString_PathIsAbsoluteButNotUnderWorkspaceOrPackagePathRoots
    #[test]
    fn get_relative_location_string_path_is_absolute_but_not_under_workspace_or_package_path_roots()
    {
        assert_eq!(
            LocationPrinter::get_relative_location_string(
                &Location::from_file_line_column("/absolute/path", 4, 2),
                Some("/this/is/the/workspace"),
                &["/this/is/a/package/path/root".to_string()]
            ),
            "/absolute/path:4:2"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/LocationPrinterTest.java LocationPrinterTest.getRelativeLocationString_PathIsAbsoluteAndUnderWorkspace
    #[test]
    fn get_relative_location_string_path_is_absolute_and_under_workspace() {
        assert_eq!(
            LocationPrinter::get_relative_location_string(
                &Location::from_file_line_column("/this/is/the/workspace/blah.txt", 4, 2),
                Some("/this/is/the/workspace"),
                &["/this/is/a/package/path/root".to_string()]
            ),
            "blah.txt:4:2"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/LocationPrinterTest.java LocationPrinterTest.getRelativeLocationString_PathIsAbsoluteAndUnderPackagePathRoot
    #[test]
    fn get_relative_location_string_path_is_absolute_and_under_package_path_root() {
        assert_eq!(
            LocationPrinter::get_relative_location_string(
                &Location::from_file_line_column("/this/is/a/package/path/root3/blah.txt", 4, 2),
                Some("/this/is/the/workspace"),
                &[
                    "/this/is/a/package/path/root1".to_string(),
                    "/this/is/a/package/path/root2".to_string(),
                    "/this/is/a/package/path/root3".to_string(),
                ]
            ),
            "blah.txt:4:2"
        );
    }
}
