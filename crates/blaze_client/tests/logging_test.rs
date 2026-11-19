//! Port of src/test/cpp/util/logging_test.cc.
//!
//! The C++ tests capture the process's stderr around the global log handler. Here each test
//! drives its own `BazelLogHandler` whose `std::cerr` is an in-memory buffer (`SetLogHandler`
//! with a new handler / nullptr becomes constructing / dropping the handler), and the
//! no-handler tests call the no-handler printing path with a buffer. The death tests check the
//! exit code the handler asks for and the output, instead of forking.

use blaze_client::util::logging::*;

const FILE: &str = "src/test/cpp/util/logging_test.cc";

fn handler() -> (BazelLogHandler, SharedBuffer) {
    let stderr = SharedBuffer::new();
    (
        BazelLogHandler::with_stderr(Box::new(stderr.clone())),
        stderr,
    )
}

fn log(h: &mut BazelLogHandler, level: LogLevel, message: &str) -> Option<i32> {
    h.handle_message(level, FILE, 1, message, 37)
}

fn matches_debug_line(output: &str, level: &str, message: &str) -> bool {
    // MatchesRegex(".LEVEL.* message\n")
    output.starts_with(&format!("[{} ", level)) && output.ends_with(&format!("] {}\n", message))
}

// Java: src/test/cpp/util/logging_test.cc:47 LoggingTest.LogLevelNamesMatch
#[test]
fn log_level_names_match() {
    assert_eq!("INFO", log_level_name(LogLevel::Info));
    assert_eq!("USER", log_level_name(LogLevel::User));
    assert_eq!("WARNING", log_level_name(LogLevel::Warning));
    assert_eq!("ERROR", log_level_name(LogLevel::Error));
    assert_eq!("FATAL", log_level_name(LogLevel::Fatal));
}

fn no_handler(level: LogLevel, message: &str) -> String {
    let mut buf = Vec::new();
    print_without_handler(&mut buf, level, message);
    String::from_utf8(buf).unwrap()
}

// Java: src/test/cpp/util/logging_test.cc:57 LoggingTest.NoHandler_InfoLogsIgnored
#[test]
fn no_handler_info_logs_ignored() {
    let teststring = "test that the info log messages get ignored";
    assert!(!no_handler(LogLevel::Info, teststring).contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:70 LoggingTest.NoHandler_UserLogsPrinted
#[test]
fn no_handler_user_logs_printed() {
    let teststring = "test that the user log messages are not ignored";
    assert!(no_handler(LogLevel::User, teststring).contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:83 LoggingTest.NoHandler_WarningsPrinted
#[test]
fn no_handler_warnings_printed() {
    let out = no_handler(LogLevel::Warning, "test that warnings are printed");
    assert!(out.contains("WARNING: test that warnings are printed"));
}

// Java: src/test/cpp/util/logging_test.cc:96 LoggingTest.NoHandler_ErrorsPrinted
#[test]
fn no_handler_errors_printed() {
    let out = no_handler(LogLevel::Error, "test that errors are printed");
    assert!(out.contains("ERROR: test that errors are printed"));
}

// Java: src/test/cpp/util/logging_test.cc:111 LoggingTest.BazelLogHandler_DumpsToCerrAtDestruction
#[test]
fn bazel_log_handler_dumps_to_cerr_at_destruction() {
    let (mut h, stderr) = handler();
    let teststring = "test that log messages get dumped to stderr";
    log(&mut h, LogLevel::Info, teststring);
    // Check that stderr isn't getting anything yet.
    assert!(!stderr.contents().contains(teststring));
    // Destruct the log handler and get the stderr remains.
    drop(h);
    assert!(stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:135 LoggingTest.BazelLogHandler_DoesNotDumpToStderrIfOuputStreamSetToNull
#[test]
fn bazel_log_handler_does_not_dump_to_stderr_if_ouput_stream_set_to_null() {
    let (mut h, stderr) = handler();
    let teststring = "test that this log message is lost.";
    log(&mut h, LogLevel::Info, teststring);
    h.set_logging_detail(LoggingDetail::User, None);
    drop(h);
    assert!(!stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:153 LoggingTest.BazelLogHandler_DoesNotPrintInfoLogsIfOuputStreamSetToNull
#[test]
fn bazel_log_handler_does_not_print_info_logs_if_ouput_stream_set_to_null() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    let teststring = "test that the log message is lost.";
    log(&mut h, LogLevel::Info, teststring);
    assert!(!stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:168 LoggingTest.BazelLogHandler_PrintsUserLogsEvenIfOuputStreamSetToNull
#[test]
fn bazel_log_handler_prints_user_logs_even_if_ouput_stream_set_to_null() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    let teststring = "some user message";
    log(&mut h, LogLevel::User, teststring);
    assert!(stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:183 LoggingTest.BazelLogHandler_PrintsWarningsEvenIfOuputStreamSetToNull
#[test]
fn bazel_log_handler_prints_warnings_even_if_ouput_stream_set_to_null() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    log(&mut h, LogLevel::Warning, "this is a warning");
    assert!(stderr.contents().contains("WARNING: this is a warning"));
}

// Java: src/test/cpp/util/logging_test.cc:198 LoggingTest.BazelLogHandler_PrintsErrorsEvenIfOuputStreamSetToNull
#[test]
fn bazel_log_handler_prints_errors_even_if_ouput_stream_set_to_null() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    log(&mut h, LogLevel::Error, "this is an error, alert!");
    assert!(
        stderr
            .contents()
            .contains("ERROR: this is an error, alert!")
    );
}

// Java: src/test/cpp/util/logging_test.cc:213 LoggingTest.BazelLogHandler_BufferedInfoLogsGetLostEvenIfOutputStreamSetToNull
#[test]
fn bazel_log_handler_buffered_info_logs_get_lost_even_if_output_stream_set_to_null() {
    let (mut h, stderr) = handler();
    let teststring = "this message should be lost.";
    log(&mut h, LogLevel::Info, teststring);
    h.set_logging_detail(LoggingDetail::User, None);
    drop(h);
    assert!(!stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:235 LoggingTest.BazelLogHandler_BufferedWarningLogsRedirectedAfterOutputStreamSetToNull
#[test]
fn bazel_log_handler_buffered_warning_logs_redirected_after_output_stream_set_to_null() {
    let (mut h, stderr) = handler();
    let teststring = "test that this message gets directed to cerr";
    log(&mut h, LogLevel::Warning, teststring);
    h.set_logging_detail(LoggingDetail::User, None);
    drop(h);
    assert!(
        stderr
            .contents()
            .contains("WARNING: test that this message gets directed to cerr")
    );
}

// Java: src/test/cpp/util/logging_test.cc:261 LoggingTest.BazelLogHandler_DirectingLogsToBufferStreamWorks
#[test]
fn bazel_log_handler_directing_logs_to_buffer_stream_works() {
    let (mut h, stderr) = handler();
    let stringbuf = SharedBuffer::new();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stringbuf.clone())));
    let teststring = "testing log getting directed to a stringbuffer.";
    log(&mut h, LogLevel::Info, teststring);
    h.close();
    assert!(stringbuf.contents().contains(teststring));
    assert!(!stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:287 LoggingTest.BazelLogHandler_BufferedLogsSentToSpecifiedStream
#[test]
fn bazel_log_handler_buffered_logs_sent_to_specified_stream() {
    let (mut h, stderr) = handler();
    let teststring = "test sending logs to the buffer before setting the output stream";
    log(&mut h, LogLevel::Info, teststring);
    assert!(!stderr.contents().contains(teststring));

    let stringbuf = SharedBuffer::new();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stringbuf.clone())));
    h.close();
    assert!(matches_debug_line(
        &stringbuf.contents(),
        "INFO",
        teststring
    ));
    assert!(!stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:321 LoggingTest.BazelLogHandler_WarningsSentToBufferStream
#[test]
fn bazel_log_handler_warnings_sent_to_buffer_stream() {
    let (mut h, stderr) = handler();
    let stringbuf = SharedBuffer::new();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stringbuf.clone())));
    log(&mut h, LogLevel::Warning, "test warning");
    h.close();
    assert!(matches_debug_line(
        &stringbuf.contents(),
        "WARNING",
        "test warning"
    ));
    assert!(!stderr.contents().contains("test warning"));
}

// Java: src/test/cpp/util/logging_test.cc:347 LoggingTest.BazelLogHandler_ErrorsSentToBufferStream
#[test]
fn bazel_log_handler_errors_sent_to_buffer_stream() {
    let (mut h, stderr) = handler();
    let stringbuf = SharedBuffer::new();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stringbuf.clone())));
    log(&mut h, LogLevel::Error, "test error");
    h.close();
    assert!(matches_debug_line(
        &stringbuf.contents(),
        "ERROR",
        "test error"
    ));
    assert!(!stderr.contents().contains("test error"));
}

// Java: src/test/cpp/util/logging_test.cc:375 LoggingTest.BazelLogHandler_DirectingLogsToCerrWorks
#[test]
fn bazel_log_handler_directing_logs_to_cerr_works() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stderr.clone())));
    let teststring = "test that the log messages get directed to cerr";
    log(&mut h, LogLevel::Info, teststring);
    drop(h);
    assert!(stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:396 LoggingTest.BazelLogHandler_BufferedLogsGetDirectedToCerr
#[test]
fn bazel_log_handler_buffered_logs_get_directed_to_cerr() {
    let (mut h, stderr) = handler();
    let teststring = "test that this message gets directed to cerr";
    log(&mut h, LogLevel::Info, teststring);
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stderr.clone())));
    drop(h);
    assert!(stderr.contents().contains(teststring));
}

// Java: src/test/cpp/util/logging_test.cc:423 LoggingDeathTest.NoHandler_FatalStatementUsesInternalErrorCode
#[test]
fn no_handler_fatal_statement_uses_internal_error_code() {
    // BAZEL_LOG(FATAL) uses INTERNAL_ERROR (37); without a handler it prints "FATAL: ...".
    let out = no_handler(LogLevel::Fatal, "something's wrong!");
    assert!(out.contains("FATAL: something's wrong!"));
    assert_eq!(37, blaze_client::util::exit_code::INTERNAL_ERROR);
}

// Java: src/test/cpp/util/logging_test.cc:430 LoggingDeathTest.BazelLogHandler_UnsetOutputStream_FatalStatementUsesInternalErrorCode
#[test]
fn bazel_log_handler_unset_output_stream_fatal_statement_uses_internal_error_code() {
    let (mut h, stderr) = handler();
    assert_eq!(Some(37), log(&mut h, LogLevel::Fatal, "something's wrong!"));
    drop(h); // std::exit runs the handler's destructor
    assert!(matches_debug_line(
        &stderr.contents(),
        "FATAL",
        "something's wrong!"
    ));
}

// Java: src/test/cpp/util/logging_test.cc:442 LoggingDeathTest.BazelLogHandler_Deactivated_FatalStatementUsesInternalErrorCode
#[test]
fn bazel_log_handler_deactivated_fatal_statement_uses_internal_error_code() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    assert_eq!(Some(37), log(&mut h, LogLevel::Fatal, "something's wrong!"));
    drop(h);
    assert!(stderr.contents().contains("FATAL: something's wrong!"));
}

// Java: src/test/cpp/util/logging_test.cc:456 LoggingDeathTest.BazelLogHandler_Stderr_FatalStatementUsesInternalErrorCode
#[test]
fn bazel_log_handler_stderr_fatal_statement_uses_internal_error_code() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stderr.clone())));
    assert_eq!(Some(37), log(&mut h, LogLevel::Fatal, "something's wrong!"));
    drop(h);
    let out = stderr.contents();
    assert!(out.starts_with("[FATAL "));
    assert!(out.contains("] something's wrong!\n"));
}

// Java: src/test/cpp/util/logging_test.cc:470 LoggingDeathTest.NoHandler_BazelDieDiesWithCustomExitCode
#[test]
fn no_handler_bazel_die_dies_with_custom_exit_code() {
    let out = no_handler(LogLevel::Fatal, "dying with exit code 42.");
    assert!(out.contains("FATAL: dying with exit code 42."));
}

// Java: src/test/cpp/util/logging_test.cc:475 LoggingDeathTest.BazelLogHandler_UnsetOutputStream_BazelDieDiesWithCustomExitCode
#[test]
fn bazel_log_handler_unset_output_stream_bazel_die_dies_with_custom_exit_code() {
    let (mut h, stderr) = handler();
    assert_eq!(
        Some(42),
        h.handle_message(LogLevel::Fatal, FILE, 1, "dying with exit code 42.", 42)
    );
    drop(h);
    assert!(matches_debug_line(
        &stderr.contents(),
        "FATAL",
        "dying with exit code 42."
    ));
}

// Java: src/test/cpp/util/logging_test.cc:488 LoggingDeathTest.BazelLogHandler_Deactivated_BazelDieDiesWithCustomExitCode
#[test]
fn bazel_log_handler_deactivated_bazel_die_dies_with_custom_exit_code() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    assert_eq!(
        Some(42),
        h.handle_message(LogLevel::Fatal, FILE, 1, "dying with exit code 42.", 42)
    );
    drop(h);
    assert!(
        stderr
            .contents()
            .contains("FATAL: dying with exit code 42.")
    );
}

// Java: src/test/cpp/util/logging_test.cc:501 LoggingDeathTest.BazelLogHandler_Stderr_BazelDieDiesWithCustomExitCode
#[test]
fn bazel_log_handler_stderr_bazel_die_dies_with_custom_exit_code() {
    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::Debug, Some(Box::new(stderr.clone())));
    assert_eq!(
        Some(42),
        h.handle_message(LogLevel::Fatal, FILE, 1, "dying with exit code 42.", 42)
    );
    drop(h);
    assert!(stderr.contents().contains("] dying with exit code 42.\n"));
}

/// The oracle's debug format: `[FATAL 23:38:58.042 src/main/cpp/blaze.cc:1275] Unknown startup
/// option: '--version'.` printed when the buffer is dumped at exit.
#[test]
fn fatal_before_logging_detail_dumps_debug_format() {
    let (mut h, stderr) = handler();
    let msg =
        "Unknown startup option: '--version'.\n  For more info, run 'bazel help startup_options'.";
    assert_eq!(
        Some(2),
        h.handle_message(LogLevel::Fatal, "src/main/cpp/blaze.cc", 1275, msg, 2)
    );
    drop(h);
    let out = stderr.contents();
    assert!(out.starts_with("[FATAL "), "{}", out);
    let ts = &out["[FATAL ".len().."[FATAL ".len() + 12];
    let b = ts.as_bytes();
    assert!(b[2] == b':' && b[5] == b':' && b[8] == b'.', "{}", ts);
    assert!(out.ends_with(&format!(" src/main/cpp/blaze.cc:1275] {}\n", msg)));
}

// Blaze: messages are internal strings; the C++ writes their raw bytes. `é` (C3 A9), `🐷`
// (F0 9F 90 B7) and a byte FF reach the stream as those bytes, with and without a handler.
#[test]
fn non_ascii_messages_are_written_as_raw_bytes() {
    let message = "\u{c3}\u{a9} \u{f0}\u{9f}\u{90}\u{b7} \u{ff}";
    let mut buf = Vec::new();
    print_without_handler(&mut buf, LogLevel::User, message);
    assert_eq!(buf, b"\xc3\xa9 \xf0\x9f\x90\xb7 \xff\n");

    let (mut h, stderr) = handler();
    h.set_logging_detail(LoggingDetail::User, None);
    log(&mut h, LogLevel::Warning, message);
    assert_eq!(
        *stderr.0.lock().unwrap(),
        b"WARNING: \xc3\xa9 \xf0\x9f\x90\xb7 \xff\n"
    );
    assert_eq!(stderr.contents(), format!("WARNING: {message}\n"));
}
