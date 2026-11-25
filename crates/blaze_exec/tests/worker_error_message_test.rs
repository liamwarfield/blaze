//! Port of src/test/java/com/google/devtools/build/lib/worker/ErrorMessageTest.java.

use blaze_exec::worker::error_message::{ErrorMessage, JavaException};
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};

const LOG_TEXT: &str = "\n\nabcdefghijklmnopqrstuvwxyz\n\n";

#[test]
fn test_empty_error_message() {
    let error_message = ErrorMessage::builder().build();
    assert_eq!(error_message.to_string(), "Unknown error");
}

#[test]
fn test_simple_error_message() {
    let error_message = ErrorMessage::builder().message("Error").build();
    assert_eq!(error_message.to_string(), "Error");
}

#[test]
fn test_error_message_with_log_text() {
    let error_message = ErrorMessage::builder()
        .message("Error with log text")
        .log_text(LOG_TEXT)
        .build();
    assert_eq!(
        error_message.to_string(),
        "Error with log text\n\n\
         ---8<---8<--- Start of log ---8<---8<---\n\
         abcdefghijklmnopqrstuvwxyz\n\
         ---8<---8<--- End of log ---8<---8<---"
    );
}

#[test]
fn test_error_message_with_empty_log_text() {
    let error_message = ErrorMessage::builder()
        .message("Error with log text")
        .log_text("")
        .build();
    assert_eq!(
        error_message.to_string(),
        "Error with log text\n\n\
         ---8<---8<--- Start of log ---8<---8<---\n\
         (empty)\n\
         ---8<---8<--- End of log ---8<---8<---"
    );
}

#[test]
fn test_error_message_with_truncated_log_text() {
    let error_message = ErrorMessage::builder()
        .message("Error with log text")
        .log_text(LOG_TEXT)
        .log_size_limit(13)
        .build();
    assert_eq!(
        error_message.to_string(),
        "Error with log text\n\n\
         ---8<---8<--- Start of log snippet ---8<---8<---\n\
         [... truncated ...]\n\
         nopqrstuvwxyz\n\
         ---8<---8<--- End of log snippet, 13 chars omitted ---8<---8<---"
    );
}

fn temp_path(dir: &tempfile::TempDir, name: &str) -> Path {
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    fs.get_path(&format!("{}/{}", dir.path().display(), name))
}

// PORT: Java uses an InMemoryFileSystem at /log.txt; the test uses a temporary directory and
// expects its path.
#[test]
fn test_error_message_with_log_file() {
    let dir = tempfile::tempdir().unwrap();
    let log_file = temp_path(&dir, "log.txt");
    std::fs::write(dir.path().join("log.txt"), LOG_TEXT).unwrap();
    let error_message = ErrorMessage::builder()
        .message("Error with log file")
        .log_file(&log_file)
        .build();
    assert_eq!(
        error_message.to_string(),
        format!(
            "Error with log file\n\n\
             ---8<---8<--- Start of log, file at {} ---8<---8<---\n\
             abcdefghijklmnopqrstuvwxyz\n\
             ---8<---8<--- End of log ---8<---8<---",
            log_file.get_path_string()
        )
    );
}

#[test]
fn test_error_message_with_unreadable_log_file() {
    let dir = tempfile::tempdir().unwrap();
    // This file does not exist.
    let log_file = temp_path(&dir, "nope.txt");
    let error_message = ErrorMessage::builder()
        .message("Error with log file")
        .log_file(&log_file)
        .build();
    let p = log_file.get_path_string();
    assert!(error_message.to_string().starts_with(&format!(
        "Error with log file\n\n\
         ---8<---8<--- Start of log, file at {p} ---8<---8<---\n\
         ERROR: IOException while trying to read log file:\n\
         java.io.FileNotFoundException: {p} (No such file or directory)"
    )));
    assert!(
        error_message
            .to_string()
            .ends_with("---8<---8<--- End of log ---8<---8<---")
    );
}

#[test]
fn test_error_message_with_exception() {
    let error_message = ErrorMessage::builder()
        .message("An exception occurred.")
        .exception(JavaException::new(
            "java.lang.IllegalStateException",
            "Hello World",
        ))
        .build();
    assert!(error_message.to_string().starts_with(
        "An exception occurred.\n\n\
         ---8<---8<--- Exception details ---8<---8<---\n\
         java.lang.IllegalStateException: Hello World"
    ));
}

// Review exec-2, E2-2: an I/O error of a system call carries the JDK's message, strerror(errno)
// alone (`java.io.IOException: Broken pipe`), never Rust's `Broken pipe (os error 32)`.
#[test]
fn from_io_error_uses_the_jdk_message_for_an_errno() {
    let e = std::io::Error::from_raw_os_error(libc_epipe());
    let java = JavaException::from_io_error(&e);
    assert_eq!(java.to_string(), "java.io.IOException: Broken pipe");
    let error_message = ErrorMessage::builder()
        .message(
            "Worker process quit or closed its stdin stream when we tried to send a WorkRequest:",
        )
        .exception(java)
        .build();
    assert!(error_message.to_string().contains(
        "---8<---8<--- Exception details ---8<---8<---\njava.io.IOException: Broken pipe\n"
    ));
    let e = std::io::Error::from_raw_os_error(2);
    assert_eq!(
        JavaException::from_io_error(&e).to_string(),
        "java.io.IOException: No such file or directory"
    );
}

// A message Blaze gave the error is kept; an empty message is Java's null.
#[test]
fn from_io_error_keeps_a_given_message() {
    let e = std::io::Error::other("Stream closed");
    assert_eq!(
        JavaException::from_io_error(&e).to_string(),
        "java.io.IOException: Stream closed"
    );
    let e = std::io::Error::other("");
    assert_eq!(
        JavaException::from_io_error(&e).to_string(),
        "java.io.IOException"
    );
}

fn libc_epipe() -> i32 {
    32
}
