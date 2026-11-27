//! The `java.util.logging` loggers of the server's own classes (`GoogleLogger.forEnclosingClass()`
//! in the code below the runtime), as far as their records reach the server log.
//!
//! PORT: Blaze has no `java.util.logging`. The runtime owns the server log (`blaze_runtime`'s
//! `server_log`); crates below it log through this sink, which the runtime sets when it opens
//! the log. Without a sink (tests, tools) a record is dropped, as with no handler in Java.

use std::sync::OnceLock;

/// The `java.util.logging.Level` of a record (`intValue()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Severe = 1000,
    Warning = 900,
    Info = 800,
}

/// `(level, source class name, source method name, message, cause)` of a record; the cause is
/// the `toString()` of the throwable Java attaches with `withCause`.
pub type Sink = fn(Level, &str, &str, &str, Option<&str>);

static SINK: OnceLock<Sink> = OnceLock::new();

/// Sets the sink of the records (once per process).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// `logger.at<level>().withCause(cause).log(message)` of class `source_class_name`, method
/// `source_method_name` (`cause`: `None` without `withCause`).
pub fn log(
    level: Level,
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    if let Some(sink) = SINK.get() {
        sink(level, source_class_name, source_method_name, message, cause);
    }
}

/// `logger.atInfo().log(message)` of class `source_class_name`, method `source_method_name`.
pub fn info(source_class_name: &str, source_method_name: &str, message: &str) {
    log(
        Level::Info,
        source_class_name,
        source_method_name,
        message,
        None,
    );
}

/// `logger.atWarning().withCause(cause).log(message)`.
pub fn warning(
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    log(
        Level::Warning,
        source_class_name,
        source_method_name,
        message,
        cause,
    );
}

/// `logger.atSevere().withCause(cause).log(message)`.
pub fn severe(
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    log(
        Level::Severe,
        source_class_name,
        source_method_name,
        message,
        cause,
    );
}

/// The `--client_env=NAME=` prefix of `s` when NAME looks private (Java
/// `SafeRequestLogging.suppressFromLog`, `--client_env=([^=]*(?:auth|pass|cookie|token|api_key)[^=]*)=`
/// with `CASE_INSENSITIVE`, matched with `lookingAt`): the match, which keeps `s`'s case.
fn suppressed_prefix(s: &str) -> Option<&str> {
    const FLAG: &str = "--client_env=";
    if s.len() < FLAG.len() || !s.as_bytes()[..FLAG.len()].eq_ignore_ascii_case(FLAG.as_bytes()) {
        return None;
    }
    let name_end = FLAG.len() + s[FLAG.len()..].find('=')?;
    let name = s[FLAG.len()..name_end].to_ascii_lowercase();
    ["auth", "pass", "cookie", "token", "api_key"]
        .iter()
        .any(|word| name.contains(word))
        .then(|| &s[..=name_end])
}

/// Generates a string form of a request to be written to the logs, filtering the user environment
/// to remove anything that looks private. The current filter criteria removes any variable whose
/// name includes "auth", "pass", "cookie" or "token".
// Java: com/google/devtools/build/lib/runtime/SafeRequestLogging.java:35 SafeRequestLogging.getRequestLogString
// PORT: skipped, the private constructor of a static-only class: SafeRequestLogging.<init>
pub fn safe_request_log_string(request_strings: &[String]) -> String {
    let mut buf = String::from("[");
    let mut sep = "";
    for s in request_strings {
        buf.push_str(sep);
        match suppressed_prefix(s) {
            Some(prefix) => {
                buf.push_str(prefix);
                buf.push_str("__private_value_removed__");
            }
            None => buf.push_str(s),
        }
        sep = ", ";
    }
    buf.push(']');
    buf
}

#[cfg(test)]
mod safe_request_logging_tests {
    use super::safe_request_log_string;

    fn log(args: &[&str]) -> String {
        safe_request_log_string(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringPassesThroughNonSensitiveClientEnv
    #[test]
    fn test_get_request_log_string_passes_through_non_sensitive_client_env() {
        assert_eq!(
            log(&["--client_env=A=B", "--client_env=C=D"]),
            "[--client_env=A=B, --client_env=C=D]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringToleratesNonsensicalClientEnv
    #[test]
    fn test_get_request_log_string_tolerates_nonsensical_client_env() {
        assert_eq!(log(&["--client_env=BROKEN"]), "[--client_env=BROKEN]");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringStripsApparentAuthValues
    #[test]
    fn test_get_request_log_string_strips_apparent_auth_values() {
        assert_eq!(
            log(&[
                "--client_env=auth=notprinted",
                "--client_env=other=isprinted"
            ]),
            "[--client_env=auth=__private_value_removed__, --client_env=other=isprinted]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringStripsApparentCookieValues
    #[test]
    fn test_get_request_log_string_strips_apparent_cookie_values() {
        assert_eq!(
            log(&[
                "--client_env=MY_COOKIE=notprinted",
                "--client_env=other=isprinted"
            ]),
            "[--client_env=MY_COOKIE=__private_value_removed__, --client_env=other=isprinted]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringStripsApparentPasswordValues
    #[test]
    fn test_get_request_log_string_strips_apparent_password_values() {
        assert_eq!(
            log(&[
                "--client_env=dont_paSS_ME=notprinted",
                "--client_env=other=isprinted"
            ]),
            "[--client_env=dont_paSS_ME=__private_value_removed__, --client_env=other=isprinted]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringStripsApparentTokenValues
    #[test]
    fn test_get_request_log_string_strips_apparent_token_values() {
        assert_eq!(
            log(&[
                "--client_env=service_ToKEn=notprinted",
                "--client_env=other=isprinted"
            ]),
            "[--client_env=service_ToKEn=__private_value_removed__, --client_env=other=isprinted]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogStringStripsApparentApiKeyValues
    #[test]
    fn test_get_request_log_string_strips_apparent_api_key_values() {
        assert_eq!(
            log(&[
                "--client_env=MY_API_KEY=notprinted",
                "--client_env=other=isprinted"
            ]),
            "[--client_env=MY_API_KEY=__private_value_removed__, --client_env=other=isprinted]"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogIgnoresSensitiveTermsInValues
    #[test]
    fn test_get_request_log_ignores_sensitive_terms_in_values() {
        assert_eq!(log(&["--client_env=ok=COOKIE"]), "[--client_env=ok=COOKIE]");
    }

    // Java: src/test/java/com/google/devtools/build/lib/runtime/SafeRequestLoggingTest.java SafeRequestLoggingTest.testGetRequestLogForStandardCommandLine
    #[test]
    fn test_get_request_log_for_standard_command_line() {
        assert_eq!(
            log(&[
                "blaze",
                "build",
                "--client_env=FOO=BAR",
                "--client_env=FOOPASS=mypassword",
                "--package_path=./MY_PASSWORD/foo",
                "--client_env=SOMEAuThCode=something",
            ]),
            "[blaze, build, --client_env=FOO=BAR, --client_env=FOOPASS=__private_value_removed__, \
             --package_path=./MY_PASSWORD/foo, \
             --client_env=SOMEAuThCode=__private_value_removed__]"
        );
    }
}
