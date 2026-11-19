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
/// The text is Bazel's internal string form (`ParserInput.fromLatin1`: one char per byte), so a
/// file that is not UTF-8 parses too (with a warning at the default enforcement mode).
// This method is the only one that is supposed to use the deprecated ParserInput.fromLatin1
// method.
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:43 StarlarkUtil.createParserInput
pub fn create_parser_input(
    bytes: Vec<u8>,
    file: &str,
    utf8_enforcement_mode: Utf8EnforcementMode,
    reporter: &dyn EventHandler,
) -> Result<String, InvalidUtf8Exception> {
    match utf8_enforcement_mode {
        Utf8EnforcementMode::Off => {}
        Utf8EnforcementMode::Warning => {
            // Guava `Utf8.isWellFormed`: the same notion as Rust's UTF-8 validity.
            if std::str::from_utf8(&bytes).is_err() {
                reporter.handle(&Event::warn_at(
                    Some(Location::new(file, 0, 0)),
                    INVALID_UTF_8_MESSAGE,
                ));
            }
        }
        Utf8EnforcementMode::Error => {
            if std::str::from_utf8(&bytes).is_err() {
                reporter.handle(&Event::error_at(
                    Some(Location::new(file, 0, 0)),
                    format!(
                        "{}. For a temporary workaround, see the --{} flag.",
                        INVALID_UTF_8_MESSAGE, "incompatible_enforce_starlark_utf8"
                    ),
                ));
                return Err(InvalidUtf8Exception(format!(
                    "{}: {}",
                    file, INVALID_UTF_8_MESSAGE
                )));
            }
        }
    }
    match blaze_util::string_encoding::parser_input_from_latin1(&bytes) {
        Ok(std::borrow::Cow::Borrowed(_)) => {
            // ASCII: the bytes are the text.
            Ok(String::from_utf8(bytes).expect("ASCII"))
        }
        Ok(std::borrow::Cow::Owned(s)) => Ok(s),
        // PORT: only while `STRINGS_ARE_INTERNAL` is false (the seam of
        // blaze_util::string_encoding): a file that is not UTF-8 has no text then.
        Err(_) => blaze_util::unported!(
            "ParserInput.fromLatin1 (non-UTF-8 Starlark file)",
            "{}",
            file
        ),
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

/// Resolves every name of `prog` against `globals` (Java `Program.compileFile`: the Resolver
/// reports every undefined name of the file, in source order) and returns the syntax errors.
///
/// A use of a predeclared name whose `GuardedValue` is not accessible (a symbol disabled by a
/// flag, restricted to some repositories, or removed) is left out of `globals`; Java's
/// `Module.resolve` throws `Undefined(guard.getErrorFromAttemptingAccess(name))` and
/// `Resolver.use` reports that message, as is, at the identifier. `guarded_message` gives that
/// message for a name, if it is such a symbol.
/// PORT: the message for any other undefined name is starlark-rust's ("Variable `x` not found";
/// Java: "name 'x' is not defined"), core-language text (decisions.md 2026-11-17).
// Java: net/starlark/java/syntax/Program.java:101 Program.compileFile
// Java: net/starlark/java/syntax/Resolver.java:941 Resolver.use
// Java: net/starlark/java/eval/Module.java:237 Module.resolve
pub fn resolve_errors(
    prog: &blaze_starlark::eval::CompiledFile,
    globals: &starlark::environment::Globals,
    guarded_message: impl Fn(&str) -> Option<String>,
) -> Vec<blaze_starlark::SyntaxError> {
    let Err(ex) = blaze_starlark::eval::resolve(prog, globals) else {
        return Vec::new();
    };
    let blaze_starlark::error::EvalErrorKind::Syntax(errors) = ex.kind else {
        unreachable!("resolve only fails with syntax errors")
    };
    errors
        .into_iter()
        .map(|mut error| {
            if let Some(message) =
                blaze_packages::builtins::env::undefined_variable_name(&error.message)
                    .and_then(&guarded_message)
            {
                error.message = message;
            }
            error
        })
        .collect()
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
