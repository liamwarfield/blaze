//! Port of `src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem` (not ported) and an `OutErr` of two byte arrays; here
//! a temporary directory on the Unix file system, and the event handler the port takes as its
//! `OutErr` (`Reporter.outErrForReporter`), whose STDOUT/STDERR events are collected.

use std::io::Write;

use blaze_events::{EventKind, StoredEventHandler};
use blaze_exec::exec::streamed_test_output::StreamedTestOutput;
use blaze_exec::exec::test_log_helper::HEADER_DELIMITER;
use blaze_shell::thread_interrupt;
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};

struct Fixture {
    _tmp: tempfile::TempDir,
    watched_path: Path,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let watched_path = fs
            .get_path(tmp.path().to_str().unwrap())
            .get_child("myfile");
        Fixture {
            _tmp: tmp,
            watched_path,
        }
    }

    fn write_content(&self, content: &[u8]) {
        let mut out = self.watched_path.get_output_stream().unwrap();
        out.write_all(content).unwrap();
        out.flush().unwrap();
    }

    /// `FileSystemUtils.writeLinesAs`: each line followed by "\n".
    fn write_lines(&self, lines: &[&str]) {
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        self.write_content(content.as_bytes());
    }
}

fn bytes_of(events: &StoredEventHandler, kind: EventKind) -> Vec<u8> {
    events
        .get_events()
        .iter()
        .filter(|e| e.get_kind() == kind)
        .flat_map(|e| e.get_message_bytes().to_vec())
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:38 StreamedTestOutputTest.testEmptyFile
#[test]
fn test_empty_file() {
    let f = Fixture::new();
    f.write_content(&[]);

    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        under_test.close().unwrap();
    });

    assert!(bytes_of(&events, EventKind::Stdout).is_empty());
    assert!(bytes_of(&events, EventKind::Stderr).is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:52 StreamedTestOutputTest.testNoHeaderOutputsEntireFile
#[test]
fn test_no_header_outputs_entire_file() {
    let f = Fixture::new();
    f.write_content(b"random\nlines\n");

    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        under_test.close().unwrap();
    });

    assert_eq!(bytes_of(&events, EventKind::Stdout), b"random\nlines\n");
    assert!(bytes_of(&events, EventKind::Stderr).is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:66 StreamedTestOutputTest.testOnlyOutputsContentsAfterHeaderWhenPresent
#[test]
fn test_only_outputs_contents_after_header_when_present() {
    let f = Fixture::new();
    f.write_lines(&["ignored", "lines", HEADER_DELIMITER, "included", "lines"]);

    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        under_test.close().unwrap();
    });

    assert_eq!(bytes_of(&events, EventKind::Stdout), b"included\nlines\n");
    assert!(bytes_of(&events, EventKind::Stderr).is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:86 StreamedTestOutputTest.testWatcherDoneAfterClose
// PORT: `getFileWatcher().isAlive()` is false once close() returned: close() joins the watcher
// thread, and the scope would not end otherwise.
#[test]
fn test_watcher_done_after_close() {
    let f = Fixture::new();
    f.write_lines(&[HEADER_DELIMITER, &"x".repeat(10 << 20)]);
    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        under_test.close().unwrap();
    });
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:99 StreamedTestOutputTest.testInterruptWaitsForWatcherToClose
#[test]
fn test_interrupt_waits_for_watcher_to_close() {
    let f = Fixture::new();
    f.write_lines(&[HEADER_DELIMITER, &"x".repeat(10 << 20)]);

    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        thread_interrupt::interrupt_current();
        under_test.close().unwrap();
    });
    // Both checks that the interrupt bit was reset and clears it for later tests.
    assert!(thread_interrupt::interrupted());
}

// Java: src/test/java/com/google/devtools/build/lib/exec/StreamedTestOutputTest.java:118 StreamedTestOutputTest.testOutputsFileWithHeaderRegardlessOfInterrupt
#[test]
fn test_outputs_file_with_header_regardless_of_interrupt() {
    let f = Fixture::new();
    f.write_content(b"blahblahblah");

    let events = StoredEventHandler::new();
    std::thread::scope(|scope| {
        let under_test = StreamedTestOutput::new(scope, &events, f.watched_path.clone()).unwrap();
        thread_interrupt::interrupt_current();
        under_test.close().unwrap();
    });
    // Both checks that the interrupt bit was reset and clears it for later tests.
    assert!(thread_interrupt::interrupted());

    assert_eq!(bytes_of(&events, EventKind::Stdout), b"blahblahblah");
    assert!(bytes_of(&events, EventKind::Stderr).is_empty());
}
