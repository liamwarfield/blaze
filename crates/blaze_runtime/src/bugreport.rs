//! Ports of `lib/bugreport` (`BugReport`, `Crash`, `CrashContext`) and
//! `lib/util/CrashFailureDetails`, for Rust panics.
//!
//! PORT: Java's crashes are uncaught `Throwable`s, Blaze's are panics. A process-wide panic hook
//! ([`install_panic_hook`], in place of Java's default uncaught exception handler) records each
//! panic's thread, location, message and backtrace instead of printing them. The catches at the
//! command boundary (`BlazeCommandDispatcher.execExclusively`'s `catch (Throwable)` and
//! `BlazeRuntime.main`) turn the panic that reached them into a [`Crash`], and
//! [`BugReport::handle_crash`] prints Bazel's message with the recorded panic where Java prints
//! the stack trace: `thread '<name>' panicked at <file>:<line>:<column>:`, the panic message
//! (for example `blaze: not ported yet: Class.method`), and the backtrace. The text of a stack
//! trace cannot match Java's.
//!
//! A panic is caught where it reaches a command boundary, and, in the server, where it escapes
//! a thread the server starts: `blaze_server::uncaught_exception` (the `grpc-command-N`,
//! `grpc-timeout-and-memory` and `interrupt-watcher-N` threads) hands it to the handler that
//! `BlazeRuntime` installs (Java's default uncaught exception handler), which halts with the
//! crash ([`BugReport::handle_crash`] with [`CrashContext::halt`]). Rust has no process-wide
//! counterpart for an exception that escapes any other thread nobody joins (Java halts there
//! too); such a panic stays recorded and ends only its thread.

use std::any::Any;
use std::backtrace::Backtrace;
use std::sync::{Arc, Once};

use blaze_events::{Event, EventHandler, OutErr};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use parking_lot::{Mutex, ReentrantMutex};

use crate::exit_code::{DetailedExitCode, ExitCode};

/// One panic as the hook saw it.
pub struct PanicRecord {
    thread: String,
    location: String,
    message: String,
    backtrace: Backtrace,
}

impl PanicRecord {
    /// `thread '<name>' panicked at <location>:` and the message: the first lines of Rust's own
    /// panic report (what tools grep for), then the backtrace.
    fn stack_trace(&self) -> String {
        format!(
            "thread '{}' panicked at {}:\n{}\nstack backtrace:\n{}",
            self.thread,
            self.location,
            self.message,
            short_backtrace(&self.backtrace.to_string())
        )
    }
}

/// The frames of `backtrace` (its `Display` text) from the panicking function to the thread's
/// entry, as Rust's own short backtrace: without the frames of the panic machinery and this hook
/// before it (`std::panicking`, `core::panicking`, `rust_begin_unwind`) and without the runtime's
/// frames after `__rust_begin_short_backtrace`.
fn short_backtrace(backtrace: &str) -> String {
    // A frame is a line "<spaces><n>: <symbol>" and the "at <file>" lines that follow it.
    let mut frames: Vec<String> = Vec::new();
    for line in backtrace.lines() {
        let trimmed = line.trim_start();
        let is_header = trimmed
            .split_once(": ")
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if is_header || frames.is_empty() {
            frames.push(line.to_owned());
        } else {
            let last = frames.last_mut().expect("not empty");
            last.push('\n');
            last.push_str(line);
        }
    }
    let symbol = |frame: &str| -> String {
        let header = frame.lines().next().unwrap_or("");
        header
            .trim_start()
            .split_once(": ")
            .map(|(_, s)| s.to_owned())
            .unwrap_or_default()
    };
    let machinery = |f: &String| {
        let s = symbol(f);
        s.contains("install_panic_hook")
            || s.contains("PanicHookInfo")
            || s.contains("__rust_end_short_backtrace")
            || s.contains("rust_begin_unwind")
            || s.starts_with("std::backtrace")
            || s.starts_with("std::sys::backtrace")
            || s.starts_with("core::panicking::")
            || s.starts_with("std::panicking::")
            || s.starts_with("core::option::expect_failed")
            || s.starts_with("core::result::unwrap_failed")
    };
    let start = frames
        .iter()
        .position(|f| !machinery(f))
        .unwrap_or(frames.len());
    let end = frames
        .iter()
        .position(|f| symbol(f).contains("__rust_begin_short_backtrace"))
        .unwrap_or(frames.len())
        .max(start);
    frames[start..end].join("\n")
}

/// The last panics recorded by the hook, oldest first.
static PANICS: Mutex<Vec<Arc<PanicRecord>>> = Mutex::new(Vec::new());
/// Caught panics (for example the ones `blaze_starlark` turns into evaluation errors) are
/// recorded too; only the last ones are kept.
const MAX_RECORDED_PANICS: usize = 64;

/// The text of a panic payload (`panic!` with a message gives a `&str` or a `String`).
pub fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_owned()
    }
}

/// Installs the process-wide panic hook that records panics for [`Crash::from_panic`] (once per
/// process).
///
/// Java: `BlazeRuntime.setupUncaughtHandlerAtStartup` and, in `newRuntime`,
/// `Thread.setDefaultUncaughtExceptionHandler`. PORT: the hook only records; see the module
/// documentation.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1610 BlazeRuntime.setupUncaughtHandlerAtStartup
pub fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // In this crate's unit tests the default hook still prints, so that a failing test
        // shows its panic.
        #[cfg(test)]
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            #[cfg(test)]
            previous(info);
            let thread = std::thread::current();
            let record = PanicRecord {
                thread: thread.name().unwrap_or("<unnamed>").to_owned(),
                location: info
                    .location()
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                message: payload_message(info.payload()),
                backtrace: Backtrace::force_capture(),
            };
            let mut panics = PANICS.lock();
            if panics.len() >= MAX_RECORDED_PANICS {
                panics.remove(0);
            }
            panics.push(Arc::new(record));
        }));
    });
}

/// The recorded panic that `message` (the message of a caught payload) comes from: the newest
/// record with that message, or whose message ends `message` as its cause (Skyframe wraps a
/// function's panic as "<context>\nCaused by: <message>"); else the newest record.
fn find_record(message: &str) -> Option<Arc<PanicRecord>> {
    let panics = PANICS.lock();
    panics
        .iter()
        .rev()
        .find(|r| r.message == message || message.ends_with(&format!("\nCaused by: {}", r.message)))
        .or_else(|| panics.last())
        .cloned()
}

/// Encapsulates the panic and [`DetailedExitCode`] for a crash.
// Java: com/google/devtools/build/lib/bugreport/Crash.java:24 Crash
pub struct Crash {
    /// The message of the panic that reached the catch (Java: the throwable).
    message: String,
    record: Option<Arc<PanicRecord>>,
    detailed_exit_code: DetailedExitCode,
}

impl Crash {
    /// Creates a crash caused by the given panic payload.
    ///
    /// The exit code is generated by [`CrashFailureDetails::detailed_exit_code_for_throwable`].
    // Java: com/google/devtools/build/lib/bugreport/Crash.java:36 Crash.from
    pub fn from_panic(payload: &(dyn Any + Send)) -> Crash {
        Self::from_message(payload_message(payload))
    }

    /// [`Crash::from_panic`] for a payload already turned into its message (for example by
    /// `blaze_server::uncaught_exception`, whose handler gets the message).
    // Java: com/google/devtools/build/lib/bugreport/Crash.java:36 Crash.from
    pub fn from_message(message: String) -> Crash {
        let record = find_record(&message);
        let detailed_exit_code = CrashFailureDetails::detailed_exit_code_for_throwable(&message);
        Crash {
            message,
            record,
            detailed_exit_code,
        }
    }

    // Java: com/google/devtools/build/lib/bugreport/Crash.java:60 Crash.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }

    /// `Throwables.getStackTraceAsString(throwable)`.
    ///
    /// PORT: the recorded panic (thread, location, message, backtrace) stands for the Java stack
    /// trace. When the caught message wraps the recorded one (`<context>\nCaused by:
    /// <message>`), the context comes first and the panic follows "Caused by: ", as Java prints
    /// a cause.
    fn get_stack_trace_as_string(&self) -> String {
        let Some(record) = &self.record else {
            return self.message.clone();
        };
        if record.message == self.message {
            return record.stack_trace();
        }
        let context = self
            .message
            .strip_suffix(&format!("\nCaused by: {}", record.message))
            .unwrap_or(&self.message);
        format!("{}\nCaused by: {}", context, record.stack_trace())
    }
}

/// Context describing when a [`Crash`] occurred and how it should be handled.
// Java: com/google/devtools/build/lib/bugreport/CrashContext.java:25 CrashContext
pub struct CrashContext {
    halt_jvm: bool,
    args: Vec<String>,
    extra_oom_info: String,
    event_handler: Arc<dyn EventHandler>,
    return_if_crash_in_progress: bool,
}

/// The default event handler: `System.err.println(event.getKind() + ": " + event.getMessage())`.
// Java: com/google/devtools/build/lib/bugreport/CrashContext.java:65 CrashContext.eventHandler
struct SystemErrEventHandler;

impl EventHandler for SystemErrEventHandler {
    fn handle(&self, event: &Event) {
        OutErr::system_out_err().print_err_ln(&format!(
            "{}: {}",
            event.get_kind(),
            event.get_message()
        ));
    }
}

impl CrashContext {
    fn new(halt_jvm: bool, return_if_crash_in_progress: bool) -> CrashContext {
        CrashContext {
            halt_jvm,
            args: Vec::new(),
            extra_oom_info: String::new(),
            event_handler: Arc::new(SystemErrEventHandler),
            return_if_crash_in_progress,
        }
    }

    /// Creates a [`CrashContext`] that halts the process when handling a crash.
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:34 CrashContext.halt
    pub fn halt() -> CrashContext {
        CrashContext::new(true, false)
    }

    /// Creates a [`CrashContext`] that does not halt the process; the caller is responsible
    /// for terminating the server with an appropriate exit code.
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:44 CrashContext.keepAlive
    pub fn keep_alive() -> CrashContext {
        CrashContext::new(false, false)
    }

    /// Halts if there is no other crash in progress, and returns otherwise.
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:56 CrashContext.haltOrReturnIfCrashInProgress
    pub fn halt_or_return_if_crash_in_progress() -> CrashContext {
        CrashContext::new(true, true)
    }

    /// Sets the arguments to include with the bug report.
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:76 CrashContext.withArgs
    pub fn with_args(mut self, args: &[String]) -> CrashContext {
        self.args = args.to_vec();
        self
    }

    /// Sets a custom additional message for an out-of-memory crash.
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:100 CrashContext.withExtraOomInfo
    pub fn with_extra_oom_info(&mut self, extra_oom_info: String) -> &mut CrashContext {
        self.extra_oom_info = extra_oom_info;
        self
    }

    /// Sets the [`EventHandler`] notified about the `FATAL` crash event (default: stderr).
    // Java: com/google/devtools/build/lib/bugreport/CrashContext.java:123 CrashContext.reportingTo
    pub fn reporting_to(&mut self, event_handler: Arc<dyn EventHandler>) -> &mut CrashContext {
        self.event_handler = event_handler;
        self
    }

    pub fn get_args(&self) -> &[String] {
        &self.args
    }
}

/// The narrow interface of `BugReport`'s usage of the runtime.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:87 BugReport.BlazeRuntimeInterface
pub trait BlazeRuntimeInterface: Send + Sync {
    fn get_product_name(&self) -> String;

    fn fill_in_crash_context(&self, ctx: &mut CrashContext);

    /// Perform all possible clean-up before crashing, posting events etc. so long as crashing
    /// isn't significantly delayed or another crash isn't triggered.
    fn clean_up_for_crash(&self, exit_code: &DetailedExitCode);
}

static RUNTIME: Mutex<Option<Arc<dyn BlazeRuntimeInterface>>> = Mutex::new(None);

/// Global lock held while reporting a crash, so that concurrent crashes produce coherent
/// reports. (Java: a fair `ReentrantLock`.)
static LOCK: ReentrantMutex<()> = ReentrantMutex::new(());

/// Utility methods for handling crashes.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:53 BugReport
pub struct BugReport;

impl BugReport {
    /// PORT: Java checks that the runtime is set once outside tests; Blaze creates a runtime per
    /// in-process server, and the last one wins.
    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:99 BugReport.setRuntime
    pub fn set_runtime(new_runtime: Arc<dyn BlazeRuntimeInterface>) {
        *RUNTIME.lock() = Some(new_runtime);
    }

    fn runtime() -> Option<Arc<dyn BlazeRuntimeInterface>> {
        RUNTIME.lock().clone()
    }

    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:106 BugReport.getProductName
    fn get_product_name() -> String {
        Self::runtime()
            .map(|r| r.get_product_name())
            .unwrap_or_else(|| "<unknown>".to_owned())
    }

    /// Handles a [`Crash`] according to the given [`CrashContext`]. With
    /// [`CrashContext::halt`] the process exits; with [`CrashContext::keep_alive`] this returns
    /// and the caller is responsible for shutting down the server.
    ///
    /// PORT: skipped, logging: the server log lines and `sendBugReport` (which reports to a
    /// remote logging service only from a released Google-internal binary).
    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:257 BugReport.handleCrash
    pub fn handle_crash(crash: &Crash, mut ctx: CrashContext) {
        let numeric_exit_code = crash
            .get_detailed_exit_code()
            .get_exit_code()
            .get_numeric_exit_code();
        let runtime = Self::runtime();
        if let Some(runtime) = &runtime {
            runtime.fill_in_crash_context(&mut ctx);
        }

        let _guard = if ctx.return_if_crash_in_progress {
            match LOCK.try_lock() {
                Some(g) => g,
                // Crash already in progress, not reporting to avoid deadlock.
                None => return,
            }
        } else {
            LOCK.lock()
        };
        let reported = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let is_oom = crash.get_detailed_exit_code().get_exit_code() == ExitCode::OOM_ERROR;
            let mut crash_msg = if is_oom {
                // PORT: no heap dump (--heap_dump_on_oom has no JVM heap to dump).
                Self::construct_oom_exit_message(&ctx.extra_oom_info)
            } else {
                format!(
                    "{} crashed due to an internal error.",
                    Self::get_product_name()
                )
            };
            crash_msg.push_str(" Printing stack trace:\n");
            crash_msg.push_str(&crash.get_stack_trace_as_string());
            ctx.event_handler.handle(&Event::fatal(crash_msg));

            Self::emit_exit_data(crash, &ctx, numeric_exit_code, runtime.as_deref());
        }));
        if let Err(t) = reported {
            let product_name = Self::get_product_name();
            let err = OutErr::system_out_err();
            err.print_err_ln(&format!(
                "ERROR: A crash occurred while {product_name} was trying to handle a crash! \
                 Please file a bug against {product_name} and include the information below."
            ));
            err.print_err_ln("Original uncaught exception:");
            err.print_err_ln(&crash.get_stack_trace_as_string());
            err.print_err_ln("Exception encountered during BugReport#handleCrash:");
            err.print_err_ln(&payload_message(t.as_ref()));
        }
        if ctx.halt_jvm {
            Self::halt(numeric_exit_code);
        }
    }

    /// `Runtime.getRuntime().halt(numericExitCode)`.
    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:349 BugReport.halt
    fn halt(numeric_exit_code: i32) -> ! {
        let _ = OutErr::system_out_err().get_error_stream().flush();
        let _ = OutErr::system_out_err().get_output_stream().flush();
        // SAFETY: _exit ends the process without running destructors or exit handlers, as
        // Runtime.halt does.
        unsafe { libc::_exit(numeric_exit_code) }
    }

    /// Writes exit status files and calls [`BlazeRuntimeInterface::clean_up_for_crash`].
    ///
    /// PORT: no heap dump (no `--heap_dump_on_oom` crash, see [`BugReport::handle_crash`]);
    /// skipped, logging.
    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:363 BugReport.emitExitData
    fn emit_exit_data(
        crash: &Crash,
        ctx: &CrashContext,
        numeric_exit_code: i32,
        runtime: Option<&dyn BlazeRuntimeInterface>,
    ) {
        // Writing the exit code status file is only necessary if we are halting. Otherwise, the
        // caller is responsible for an orderly shutdown with the proper exit code.
        if ctx.halt_jvm {
            crate::custom_exit_code_publisher::CustomExitCodePublisher::maybe_write_exit_status_file(
                numeric_exit_code,
            );
        }

        if let Some(failure_detail) = crash.get_detailed_exit_code().get_failure_detail() {
            crate::custom_exit_code_publisher::CustomFailureDetailPublisher::maybe_write_failure_detail_file(
                failure_detail,
            );
        }

        if let Some(runtime) = runtime {
            runtime.clean_up_for_crash(crash.get_detailed_exit_code());
        }
    }

    // Java: com/google/devtools/build/lib/bugreport/BugReport.java:400 BugReport.constructOomExitMessage
    pub fn construct_oom_exit_message(extra_info: &str) -> String {
        let msg = format!(
            "{} ran out of memory and crashed.",
            Self::get_product_name()
        );
        if extra_info.is_empty() {
            msg
        } else {
            format!("{msg} {extra_info}")
        }
    }
}

/// Factory methods for producing `Crash`-type [`FailureDetail`] messages.
// Java: com/google/devtools/build/lib/util/CrashFailureDetails.java:31 CrashFailureDetails
pub struct CrashFailureDetails;

impl CrashFailureDetails {
    /// Max message length in a `FailureDetails.Throwable` submessage; anything beyond this is
    /// truncated.
    const MAX_THROWABLE_MESSAGE_LENGTH: usize = 2000;

    // Java: com/google/devtools/build/lib/util/CrashFailureDetails.java:67 CrashFailureDetails.detailedExitCodeForThrowable
    pub fn detailed_exit_code_for_throwable(message: &str) -> DetailedExitCode {
        DetailedExitCode::of(Self::for_throwable(message))
    }

    /// Returns a `Crash`-type [`FailureDetail`] with its cause chain filled out.
    ///
    /// PORT: a Rust allocation failure aborts the process, so there is no out-of-memory crash
    /// (`CRASH_OOM`, exit code 33) to classify and no OOM detector; every crash is
    /// `CRASH_UNKNOWN` (exit code 37). A panic has no class and no cause chain: the cause list
    /// has one entry whose class is `panic`, and no stack trace elements.
    // Java: com/google/devtools/build/lib/util/CrashFailureDetails.java:72 CrashFailureDetails.forThrowable
    pub fn for_throwable(message: &str) -> FailureDetail {
        let throwable = failure_details::Throwable {
            throwable_class: "panic".to_owned(),
            message: ascii_truncate(message, Self::MAX_THROWABLE_MESSAGE_LENGTH, "[truncated]"),
            stack_trace: Vec::new(),
        };
        let summarized = format!("({}) {}", throwable.throwable_class, throwable.message);
        FailureDetail {
            message: format!("Crashed: {summarized}"),
            category: Some(Category::Crash(failure_details::Crash {
                code: failure_details::crash::Code::CrashUnknown as i32,
                causes: vec![throwable],
                oom_cause_category: failure_details::crash::OomCauseCategory::None as i32,
            })),
        }
    }
}

/// Guava `Ascii.truncate(seq, maxLength, truncationIndicator)` (lengths in chars).
fn ascii_truncate(seq: &str, max_length: usize, truncation_indicator: &str) -> String {
    let len = seq.chars().count();
    if len <= max_length {
        return seq.to_owned();
    }
    let keep = max_length - truncation_indicator.chars().count();
    let mut out: String = seq.chars().take(keep).collect();
    out.push_str(truncation_indicator);
    out
}

/// Posted during `BlazeRuntime.cleanUpForCrash` just prior to [`CommandCompleteEvent`].
// Java: com/google/devtools/build/lib/runtime/CrashEvent.java:19 CrashEvent
#[derive(Debug)]
pub struct CrashEvent;

impl blaze_events::Postable for CrashEvent {}

/// This event is fired when the Blaze command is complete (clean, build, test, etc.). It is
/// fired even if the command terminated abnormally.
// Java: com/google/devtools/build/lib/runtime/CommandCompleteEvent.java:24 CommandCompleteEvent
#[derive(Debug)]
pub struct CommandCompleteEvent {
    detailed_exit_code: DetailedExitCode,
}

impl CommandCompleteEvent {
    // Java: com/google/devtools/build/lib/runtime/CommandCompleteEvent.java:27 CommandCompleteEvent.<init>
    pub fn new(detailed_exit_code: DetailedExitCode) -> Self {
        CommandCompleteEvent { detailed_exit_code }
    }

    /// Returns the exit code of the blaze command.
    // Java: com/google/devtools/build/lib/runtime/CommandCompleteEvent.java:32 CommandCompleteEvent.getExitCode
    pub fn get_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

impl blaze_events::Postable for CommandCompleteEvent {}

/// `BlazeCommandDispatcher.execExclusively`'s `catch (Throwable e)`: runs the command; a panic
/// that reaches here is handled as a crash (`bugReporter.handleCrash(crash,
/// CrashContext.keepAlive().withArgs(args))`) and gives `BlazeCommandResult.createShutdown(crash)`
/// as the `Err`.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:862 BlazeCommandDispatcher.execExclusively
pub(crate) fn catch_crash(
    args: &[String],
    exec: impl FnOnce() -> crate::command::BlazeCommandResult,
) -> Result<crate::command::BlazeCommandResult, crate::command::BlazeCommandResult> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(exec)).map_err(|payload| {
        let crash = Crash::from_panic(payload.as_ref());
        BugReport::handle_crash(&crash, CrashContext::keep_alive().with_args(args));
        crate::command::BlazeCommandResult::create_shutdown(crash.get_detailed_exit_code().clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runtime whose crash context reports to a collector (Java: `fillInCrashContext` with the
    /// command's reporter).
    struct TestRuntime {
        events: Arc<blaze_events::StoredEventHandler>,
        cleaned_up: Mutex<Vec<i32>>,
    }

    impl BlazeRuntimeInterface for TestRuntime {
        fn get_product_name(&self) -> String {
            "bazel".to_owned()
        }
        fn fill_in_crash_context(&self, ctx: &mut CrashContext) {
            ctx.reporting_to(self.events.clone());
        }
        fn clean_up_for_crash(&self, exit_code: &DetailedExitCode) {
            self.cleaned_up
                .lock()
                .push(exit_code.get_exit_code().get_numeric_exit_code());
        }
    }

    fn fatal_events(events: &blaze_events::StoredEventHandler) -> Vec<String> {
        events
            .get_events()
            .iter()
            .filter(|e| e.get_kind() == blaze_events::EventKind::Fatal)
            .map(|e| e.get_message().to_string())
            .collect()
    }

    /// One test: the runtime and the panic hook are process-wide.
    #[test]
    fn crash_in_command_and_on_worker_thread() {
        let _serial = SERIAL.lock();
        install_panic_hook();
        let runtime = Arc::new(TestRuntime {
            events: Arc::new(blaze_events::StoredEventHandler::new()),
            cleaned_up: Mutex::new(Vec::new()),
        });
        BugReport::set_runtime(runtime.clone());
        let args = vec!["build".to_owned()];

        // A panic on the command thread (an `unported!` site).
        let result = catch_crash(&args, || {
            blaze_util::unported!("Foo.bar");
        })
        .unwrap_err();
        assert_eq!(result.get_exit_code().get_numeric_exit_code(), 37);
        assert!(result.shutdown());
        assert_eq!(
            result.get_failure_detail().map(|f| f.message.as_str()),
            Some("Crashed: (panic) blaze: not ported yet: Foo.bar")
        );
        let fatal = fatal_events(&runtime.events);
        assert_eq!(fatal.len(), 1);
        let lines: Vec<&str> = fatal[0].lines().collect();
        assert_eq!(
            lines[0],
            "bazel crashed due to an internal error. Printing stack trace:"
        );
        assert!(
            lines[1].starts_with("thread '") && lines[1].contains("' panicked at crates/"),
            "{}",
            lines[1]
        );
        assert_eq!(lines[2], "blaze: not ported yet: Foo.bar");
        assert_eq!(lines[3], "stack backtrace:");
        // The backtrace starts at the panicking function, not in the hook.
        assert!(
            lines[4].contains("crash_in_command_and_on_worker_thread"),
            "{}",
            lines[4]
        );
        assert_eq!(*runtime.cleaned_up.lock(), [37]);

        // A panic on a worker thread, re-raised on the command thread where the worker is
        // joined (as the Skyframe evaluator and the subprocess threads do).
        let result = catch_crash(&args, || {
            let worker = std::thread::Builder::new()
                .name("worker".to_owned())
                .spawn(|| panic!("IllegalStateException: on a worker"))
                .unwrap();
            match worker.join() {
                Ok(()) => unreachable!(),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })
        .unwrap_err();
        assert_eq!(result.get_exit_code().get_numeric_exit_code(), 37);
        let fatal = fatal_events(&runtime.events);
        assert_eq!(fatal.len(), 2);
        let lines: Vec<&str> = fatal[1].lines().collect();
        assert_eq!(
            lines[0],
            "bazel crashed due to an internal error. Printing stack trace:"
        );
        assert!(
            lines[1].starts_with("thread 'worker' panicked at crates/"),
            "{}",
            lines[1]
        );
        assert_eq!(lines[2], "IllegalStateException: on a worker");

        // A worker's panic that Skyframe wrapped with its context: the context, then the panic
        // as its cause.
        let result = catch_crash(&args, || {
            let worker = std::thread::Builder::new()
                .name("skyframe-evaluator".to_owned())
                .spawn(|| panic!("blaze: not ported yet: Baz.qux"))
                .unwrap();
            let payload = worker.join().unwrap_err();
            let cause = payload_message(payload.as_ref());
            std::panic::resume_unwind(Box::new(format!(
                "Unrecoverable error while evaluating node 'K' (requested by nodes )\nCaused by: {cause}"
            )))
        })
        .unwrap_err();
        assert_eq!(result.get_exit_code().get_numeric_exit_code(), 37);
        let fatal = fatal_events(&runtime.events);
        let lines: Vec<&str> = fatal[2].lines().collect();
        assert_eq!(
            lines[1],
            "Unrecoverable error while evaluating node 'K' (requested by nodes )"
        );
        assert!(
            lines[2].starts_with("Caused by: thread 'skyframe-evaluator' panicked at crates/"),
            "{}",
            lines[2]
        );
        assert_eq!(lines[3], "blaze: not ported yet: Baz.qux");
    }

    #[test]
    fn short_backtrace_drops_panic_machinery_and_runtime_frames() {
        let bt = "   0: blaze_runtime::bugreport::install_panic_hook::{closure#0}\n\
                  \x20            at src/bugreport.rs:1:1\n\
                  \x20  1: std::panicking::panic_with_hook\n\
                  \x20  2: core::panicking::panic_fmt\n\
                  \x20  3: my_crate::f\n\
                  \x20            at src/f.rs:2:3\n\
                  \x20  4: std::panicking::catch_unwind::<x>\n\
                  \x20  5: my_crate::main\n\
                  \x20  6: std::sys::backtrace::__rust_begin_short_backtrace::<x>\n\
                  \x20  7: std::rt::lang_start\n";
        assert_eq!(
            short_backtrace(bt),
            "   3: my_crate::f\n\
             \x20            at src/f.rs:2:3\n\
             \x20  4: std::panicking::catch_unwind::<x>\n\
             \x20  5: my_crate::main"
        );
    }

    /// The tests that set the process-wide runtime run one at a time.
    use crate::custom_exit_code_publisher::TEST_SERIAL as SERIAL;

    /// Java's mock runtime: product name "myProductName", no crash context.
    struct MockRuntime {
        cleaned_up: Mutex<Vec<DetailedExitCode>>,
        handler: Option<Arc<dyn EventHandler>>,
    }

    impl BlazeRuntimeInterface for MockRuntime {
        fn get_product_name(&self) -> String {
            "myProductName".to_owned()
        }
        fn fill_in_crash_context(&self, ctx: &mut CrashContext) {
            if let Some(handler) = &self.handler {
                ctx.with_extra_oom_info("Build fewer targets!".to_owned())
                    .reporting_to(handler.clone());
            }
        }
        fn clean_up_for_crash(&self, exit_code: &DetailedExitCode) {
            self.cleaned_up.lock().push(exit_code.clone());
        }
    }

    fn mock_runtime(handler: Option<Arc<dyn EventHandler>>) -> Arc<MockRuntime> {
        let runtime = Arc::new(MockRuntime {
            cleaned_up: Mutex::new(Vec::new()),
            handler,
        });
        BugReport::set_runtime(runtime.clone());
        runtime
    }

    /// `CrashType.CRASH.createThrowable()`: `new IllegalStateException("Crashed")`, as a panic.
    fn crash_from_panic() -> Crash {
        install_panic_hook();
        let payload = std::panic::catch_unwind(|| {
            blaze_util::check_state!(false, "Crashed");
        })
        .unwrap_err();
        Crash::from_panic(payload.as_ref())
    }

    // Java: src/test/java/com/google/devtools/build/lib/bugreport/BugReportTest.java:261 BugReportTest.keepAlive
    #[test]
    fn keep_alive() {
        let _serial = SERIAL.lock();
        let runtime = mock_runtime(None);
        let crash = crash_from_panic();
        let expected = crash.get_detailed_exit_code().clone();
        assert_eq!(
            expected.get_failure_detail().map(|f| f.message.as_str()),
            Some("Crashed: (panic) IllegalStateException: Crashed")
        );

        BugReport::handle_crash(&crash, CrashContext::keep_alive());

        assert_eq!(*runtime.cleaned_up.lock(), [expected]);
        assert_eq!(
            runtime.cleaned_up.lock()[0]
                .get_exit_code()
                .get_numeric_exit_code(),
            ExitCode::BLAZE_INTERNAL_ERROR.get_numeric_exit_code()
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/bugreport/BugReportTest.java:332 BugReportTest.customContext_setUpFront
    #[test]
    fn custom_context_set_up_front() {
        let _serial = SERIAL.lock();
        mock_runtime(None);
        let crash = crash_from_panic();
        let handler = Arc::new(blaze_events::StoredEventHandler::new());
        let mut ctx = CrashContext::keep_alive();
        ctx.with_extra_oom_info("Build fewer targets!".to_owned())
            .reporting_to(handler.clone());

        BugReport::handle_crash(&crash, ctx);

        let events = handler.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get_kind(), blaze_events::EventKind::Fatal);
        assert!(
            events[0]
                .get_message()
                .contains(&crash.get_stack_trace_as_string())
        );
        assert!(!events[0].get_message().contains("Build fewer targets!"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/bugreport/BugReportTest.java:354 BugReportTest.customContext_filledInByRuntime
    #[test]
    fn custom_context_filled_in_by_runtime() {
        let _serial = SERIAL.lock();
        let handler = Arc::new(blaze_events::StoredEventHandler::new());
        mock_runtime(Some(handler.clone()));
        let crash = crash_from_panic();

        BugReport::handle_crash(&crash, CrashContext::keep_alive());

        let events = handler.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get_kind(), blaze_events::EventKind::Fatal);
        assert!(events[0].get_message().starts_with(
            "myProductName crashed due to an internal error. Printing stack trace:\n"
        ));
        assert!(
            events[0]
                .get_message()
                .contains(&crash.get_stack_trace_as_string())
        );
        assert!(!events[0].get_message().contains("Build fewer targets!"));
    }

    #[test]
    #[ignore = "needs an OOM detector: a Rust allocation failure aborts, there is no OOM crash"]
    fn maybe_set_oom_detector() {}

    #[test]
    #[ignore = "logging only: logException reports to a remote logging service in released Google builds"]
    fn log_exception() {}

    #[test]
    #[ignore = "needs Runtime.halt interception: halting ends the test process"]
    fn convenience_method() {}

    #[test]
    #[ignore = "needs Runtime.halt interception: halting ends the test process"]
    fn halt() {}

    #[test]
    #[ignore = "needs Runtime.halt interception: halting ends the test process"]
    fn halt_or_return_if_crash_in_progress_other_crash_in_progress_returns_eagerly() {}

    // Java: src/test/java/com/google/devtools/build/lib/util/CrashFailureDetailsTest.java:102 CrashFailureDetailsTest.testMessageLimit
    #[test]
    fn test_message_limit() {
        let detail = CrashFailureDetails::for_throwable(&"x".repeat(5000));
        let Some(Category::Crash(crash)) = &detail.category else {
            panic!("not a crash");
        };
        let crash_message = &crash.causes[0].message;
        assert_eq!(crash_message.chars().count(), 2000);
        assert!(crash_message.ends_with("[truncated]"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/CrashFailureDetailsTest.java:163 CrashFailureDetailsTest.detailedExitConstruction_otherCrash
    #[test]
    fn detailed_exit_construction_other_crash() {
        let detailed_exit_code =
            CrashFailureDetails::detailed_exit_code_for_throwable("IllegalStateException");
        assert_eq!(
            detailed_exit_code.get_exit_code(),
            ExitCode::BLAZE_INTERNAL_ERROR
        );
        let Some(Category::Crash(crash)) = &detailed_exit_code
            .get_failure_detail()
            .expect("detail")
            .category
        else {
            panic!("not a crash");
        };
        assert_eq!(
            crash.oom_cause_category,
            failure_details::crash::OomCauseCategory::None as i32
        );
    }

    #[test]
    #[ignore = "a panic has no cause chain or stack trace elements (CrashFailureDetails PORT)"]
    fn nested_throwables() {}

    #[test]
    #[ignore = "a panic has no cause chain (CrashFailureDetails PORT)"]
    fn cause_limit() {}

    #[test]
    #[ignore = "a panic has no cause chain (CrashFailureDetails PORT)"]
    fn cause_cycle() {}

    #[test]
    #[ignore = "a panic has no stack trace elements (CrashFailureDetails PORT)"]
    fn deep_stack() {}

    #[test]
    #[ignore = "needs OutOfMemoryError: a Rust allocation failure aborts"]
    fn detailed_exit_construction_oom() {}

    #[test]
    #[ignore = "needs OutOfMemoryError: a Rust allocation failure aborts"]
    fn detailed_exit_construction_wrapped_oom() {}

    #[test]
    #[ignore = "needs an OOM detector: a Rust allocation failure aborts"]
    fn detail_exit_construction_crash_with_oom_detector_returns_oom_crash() {}
}
