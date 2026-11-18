//! Port of `StarlarkUtil`: turning file bytes into parser input, and the event plumbing between
//! Blaze's Starlark layer and Skyframe listeners.
//!
//! Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java.

use blaze_events::{Event, EventHandler, Location};
use blaze_starlark::semantics::Utf8EnforcementMode;

/// Message for a file that is not UTF-8.
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:27 StarlarkUtil.INVALID_UTF_8_MESSAGE
pub const INVALID_UTF_8_MESSAGE: &str = "not a valid UTF-8 encoded file; this can lead to \
                                         inconsistent behavior and will be disallowed in a future \
                                         version of Bazel";

/// Exception thrown when a Starlark file is not valid UTF-8.
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:70 StarlarkUtil.InvalidUtf8Exception
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidUtf8Exception(pub String);

/// Creates the parser input for `bytes` read from `file`, enforcing UTF-8 per
/// `--incompatible_enforce_starlark_utf8`.
///
/// PORT: Java parses Latin-1 "internal" strings (UTF-8 bytes in a Latin-1 String,
/// `ParserInput.fromLatin1`); starlark-rust parses `String`s, which for UTF-8 input is the same
/// text. Non-UTF-8 input that Java would accept (enforcement OFF or WARNING) is not supported yet
/// and fails loudly.
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:43 StarlarkUtil.createParserInput
pub fn create_parser_input(
    bytes: Vec<u8>,
    file: &str,
    utf8_enforcement_mode: Utf8EnforcementMode,
    reporter: &dyn EventHandler,
) -> Result<String, InvalidUtf8Exception> {
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(_) => match utf8_enforcement_mode {
            Utf8EnforcementMode::Off => {
                blaze_util::unported!(
                    "ParserInput.fromLatin1 (non-UTF-8 Starlark file)",
                    "{}",
                    file
                )
            }
            Utf8EnforcementMode::Warning => {
                reporter.handle(&Event::warn_at(
                    Some(Location::new(file, 0, 0)),
                    INVALID_UTF_8_MESSAGE,
                ));
                blaze_util::unported!(
                    "ParserInput.fromLatin1 (non-UTF-8 Starlark file)",
                    "{}",
                    file
                )
            }
            Utf8EnforcementMode::Error => {
                reporter.handle(&Event::error_at(
                    Some(Location::new(file, 0, 0)),
                    format!(
                        "{}. For a temporary workaround, see the --{} flag.",
                        INVALID_UTF_8_MESSAGE, "incompatible_enforce_starlark_utf8"
                    ),
                ));
                Err(InvalidUtf8Exception(format!(
                    "{}: {}",
                    file, INVALID_UTF_8_MESSAGE
                )))
            }
        },
    }
}

/// Converts a Starlark location to an event location.
pub fn event_location(location: &blaze_starlark::Location) -> Location {
    Location::new(
        location.file.clone(),
        location.line as i32,
        location.column as i32,
    )
}

/// Replays syntax errors on `handler` as error events (Java `Event.replayEventsOn(handler,
/// errors)` for `SyntaxError`s).
// Java: com/google/devtools/build/lib/events/Event.java:415 Event.replayEventsOn
pub fn replay_syntax_errors_on(handler: &dyn EventHandler, errors: &[blaze_starlark::SyntaxError]) {
    Event::replay_syntax_errors_on(
        handler,
        errors
            .iter()
            .map(|e| (event_location(&e.location), e.message.clone())),
    );
}

/// Java's static error for a use of a predeclared name whose `GuardedValue` is not accessible
/// (a symbol disabled by a flag, restricted to some repositories, or removed), if `ex` is
/// starlark-rust's undefined-variable error for such a name of `compile_env` (the predeclared
/// environment the file is compiled against).
///
/// Java's `Module.resolve` throws `Undefined(guard.getErrorFromAttemptingAccess(name))` and
/// `Resolver.use` reports that message, as is, at the identifier. starlark-rust resolves
/// against a `Globals` without the inaccessible names and reports its own "Variable `X` not
/// found" when the module starts executing: an evaluation error raised before any statement
/// runs, so its stack is the top level alone, at the identifier.
// Java: net/starlark/java/eval/Module.java:237 Module.resolve
// Java: net/starlark/java/syntax/Resolver.java:941 Resolver.use
pub fn guarded_symbol_syntax_error(
    ex: &blaze_starlark::EvalError,
    compile_env: &blaze_packages::builtins::env::StarlarkEnv,
    semantics: &blaze_starlark::StarlarkSemantics,
    compile_label: Option<&blaze_cmdline::Label>,
) -> Option<blaze_starlark::SyntaxError> {
    if ex.kind != blaze_starlark::error::EvalErrorKind::Eval {
        return None;
    }
    let [toplevel] = ex.callstack.as_slice() else {
        return None;
    };
    if toplevel.name != "<toplevel>" {
        return None;
    }
    let location = toplevel.location.clone()?;
    let name = blaze_packages::builtins::env::undefined_variable_name(&ex.message)?;
    let message = blaze_packages::builtins::env::guarded_symbol_error(
        compile_env,
        name,
        semantics,
        compile_label,
    )?;
    Some(blaze_starlark::SyntaxError { location, message })
}

/// Replays the `print()` output and warnings a Starlark evaluation recorded, as Java's thread
/// print handler (`Event.makeDebugPrintHandler`) would have reported them.
// Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
pub fn replay_starlark_events_on(
    handler: &dyn EventHandler,
    events: Vec<blaze_starlark::thread::Event>,
) {
    for e in events {
        let location = e.location.as_ref().map(event_location);
        let event = match e.kind {
            "DEBUG" => Event::debug_at(location, e.message),
            "WARNING" => Event::warn_at(location, e.message),
            other => panic!("unknown Starlark event kind {}", other),
        };
        handler.handle(&event);
    }
}
