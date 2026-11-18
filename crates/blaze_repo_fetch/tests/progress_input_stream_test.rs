//! Port of `ProgressInputStreamTest`.

mod downloader_support;

use std::sync::Arc;

use blaze_events::handler::FetchProgress;
use blaze_repo_fetch::downloader::DownloadProgressEvent;
use blaze_repo_fetch::downloader::streams::{InputStream, ProgressInputStream};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use downloader_support::{ManualClock, RecordingHandler};
use parking_lot::Mutex;

/// Mock `InputStream`: each read returns `ret` bytes (or fails), counting calls.
#[derive(Default)]
struct Calls {
    reads: usize,
    closes: usize,
}

struct MockStream {
    calls: Arc<Mutex<Calls>>,
    ret: Result<usize, IoException>,
}

impl InputStream for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        self.calls.lock().reads += 1;
        match &self.ret {
            Ok(n) => Ok((*n).min(buf.len())),
            Err(e) => Err(e.clone()),
        }
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.calls.lock().closes += 1;
        Ok(())
    }
}

struct Fixture {
    clock: Arc<ManualClock>,
    handler: Arc<RecordingHandler>,
    calls: Arc<Mutex<Calls>>,
    stream: ProgressInputStream,
}

fn fixture_with(url: &str, ret: Result<usize, IoException>) -> Fixture {
    let clock = ManualClock::new();
    let handler = RecordingHandler::new();
    let calls = Arc::new(Mutex::new(Calls::default()));
    let original = JavaUri::create("http://lol.example").unwrap();
    let stream = ProgressInputStream::new(
        clock.clone(),
        handler.clone(),
        1,
        Box::new(MockStream {
            calls: calls.clone(),
            ret,
        }),
        JavaUri::create(url).unwrap(),
        original,
        None,
    );
    Fixture {
        clock,
        handler,
        calls,
        stream,
    }
}

fn fixture(ret: Result<usize, IoException>) -> Fixture {
    fixture_with("http://lol.example", ret)
}

/// `stream.read()` (single byte).
fn read1(s: &mut ProgressInputStream) -> Result<usize, IoException> {
    let mut b = [0u8; 1];
    s.read(&mut b)
}

#[test]
fn close_calls_delegate() {
    let mut f = fixture(Ok(1));
    f.stream.close().unwrap();
    assert_eq!(f.calls.lock().closes, 1);
    assert!(f.handler.messages().is_empty());
}

#[test]
#[ignore = "Blaze's InputStream has no available(); nothing to delegate"]
fn available_calls_delegate() {}

#[test]
fn read_callsdelegate() {
    let mut f = fixture(Ok(1));
    read1(&mut f.stream).unwrap();
    assert_eq!(f.calls.lock().reads, 1);
    assert!(f.handler.messages().is_empty());
}

#[test]
fn read_throws_exception_passes_through() {
    let mut f = fixture(Err(IoException::new("")));
    assert!(read1(&mut f.stream).is_err());
    assert_eq!(f.calls.lock().reads, 1);
    assert!(f.handler.messages().is_empty());
}

#[test]
fn reads_after_interval_emits_progress_once() {
    let mut f = fixture(Ok(1));
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    f.clock.advance_millis(1);
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    assert_eq!(f.calls.lock().reads, 3);
    assert_eq!(
        f.handler.messages(),
        vec!["Downloading http://lol.example: 2 bytes"]
    );
}

#[test]
fn multiple_intervals_elapsed_shows_multiple_progress() {
    let mut f = fixture(Ok(1));
    read1(&mut f.stream).unwrap();
    read1(&mut f.stream).unwrap();
    f.clock.advance_millis(1);
    read1(&mut f.stream).unwrap();
    read1(&mut f.stream).unwrap();
    f.clock.advance_millis(1);
    read1(&mut f.stream).unwrap();
    read1(&mut f.stream).unwrap();
    assert_eq!(f.calls.lock().reads, 6);
    assert_eq!(
        f.handler.messages(),
        vec![
            "Downloading http://lol.example: 3 bytes",
            "Downloading http://lol.example: 5 bytes"
        ]
    );
}

#[test]
fn buffer_reads_after_interval_emits_progress_once() {
    let mut f = fixture(Ok(1024));
    let mut buffer = [0u8; 1024];
    assert_eq!(f.stream.read(&mut buffer).unwrap(), 1024);
    f.clock.advance_millis(1);
    assert_eq!(f.stream.read(&mut buffer).unwrap(), 1024);
    assert_eq!(f.stream.read(&mut buffer).unwrap(), 1024);
    assert_eq!(f.calls.lock().reads, 3);
    assert_eq!(
        f.handler.messages(),
        vec!["Downloading http://lol.example: 2,048 bytes"]
    );
}

#[test]
#[ignore = "PORT: Blaze formats progress with an English locale only (Java uses Locale.getDefault())"]
fn buffer_reads_after_interval_in_germany_uses_period_as_separator() {}

#[test]
fn redirected_to_different_server_shows_original_url_with_via() {
    let mut f = fixture_with("http://cdn.example/foo", Ok(1));
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    f.clock.advance_millis(1);
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    assert_eq!(read1(&mut f.stream).unwrap(), 1);
    assert_eq!(f.calls.lock().reads, 3);
    assert_eq!(
        f.handler.messages(),
        vec!["Downloading http://lol.example via cdn.example: 2 bytes"]
    );
}

#[test]
fn percentual_progress() {
    let url = JavaUri::create("http://lol.example").unwrap();
    let event = DownloadProgressEvent::new(
        url.clone(),
        Some(url),
        25 * 1024 * 1024,
        Some(100 * 1024 * 1024),
        false,
    );
    assert_eq!(event.get_progress(), "25.0 MiB (25.0%)");
}

#[test]
fn percentual_progress_zero_total_bytes() {
    let url = JavaUri::create("http://lol.example").unwrap();
    let event =
        DownloadProgressEvent::new(url.clone(), Some(url), 25 * 1024 * 1024, Some(0), false);
    assert_eq!(event.get_progress(), "25.0 MiB (100.0%)");
}
