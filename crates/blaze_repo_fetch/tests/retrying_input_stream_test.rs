//! Port of `RetryingInputStreamTest`.

use std::collections::VecDeque;
use std::sync::Arc;

use blaze_repo_fetch::downloader::http_connector::UrlConnection;
use blaze_repo_fetch::downloader::streams::{InputStream, Reconnector, RetryingInputStream};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use parking_lot::Mutex;

/// A Mockito-like scripted stream; handles sharing one script count as the same mock.
#[derive(Default)]
struct Script {
    results: VecDeque<Result<usize, IoException>>,
    /// Returned once `results` is exhausted (Mockito repeats the last stub).
    last: Option<Result<usize, IoException>>,
    reads: usize,
    closes: usize,
}

#[derive(Clone)]
struct Mock(Arc<Mutex<Script>>);

impl Mock {
    fn new(results: Vec<Result<usize, IoException>>) -> Mock {
        let last = results.last().cloned();
        Mock(Arc::new(Mutex::new(Script {
            results: results.into(),
            last,
            reads: 0,
            closes: 0,
        })))
    }

    fn reads(&self) -> usize {
        self.0.lock().reads
    }

    fn closes(&self) -> usize {
        self.0.lock().closes
    }
}

impl InputStream for Mock {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let mut s = self.0.lock();
        s.reads += 1;
        let r = match s.results.pop_front() {
            Some(r) => r,
            None => s.last.clone().unwrap_or(Ok(0)),
        };
        r.map(|n| n.min(buf.len()))
    }

    fn close(&mut self) -> Result<(), IoException> {
        self.0.lock().closes += 1;
        Ok(())
    }
}

type Calls = Arc<Mutex<Vec<Vec<(String, Vec<String>)>>>>;

/// A reconnector returning connections whose input stream is `new_delegate`.
fn reconnector(
    new_delegate: Option<Mock>,
    content_range: Option<&str>,
    fail: bool,
) -> (Reconnector, Calls) {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let c = calls.clone();
    let content_range = content_range.map(str::to_string);
    let r: Reconnector = Arc::new(move |_cause, extra| {
        c.lock().push(extra);
        if fail {
            return Err(IoException::without_message(IoExceptionKind::IOException));
        }
        let mut headers = Vec::new();
        if let Some(cr) = &content_range {
            headers.push(("Content-Range".to_string(), cr.clone()));
        }
        Ok(UrlConnection::new(
            JavaUri::create("http://lol.example").unwrap(),
            200,
            None,
            headers,
            Ok(Box::new(new_delegate.clone().unwrap()) as Box<dyn InputStream>),
        ))
    });
    (r, calls)
}

fn read1(s: &mut RetryingInputStream) -> Result<usize, IoException> {
    let mut b = [0u8; 1];
    s.read(&mut b)
}

fn timeout() -> IoException {
    IoException::without_message(IoExceptionKind::SocketTimeoutException)
}

fn io() -> IoException {
    IoException::without_message(IoExceptionKind::IOException)
}

fn range(s: &str) -> Vec<(String, Vec<String>)> {
    vec![("Range".to_string(), vec![s.to_string()])]
}

#[test]
fn close_calls_delegate() {
    let delegate = Mock::new(vec![]);
    let (r, calls) = reconnector(None, None, false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    stream.close().unwrap();
    assert_eq!(delegate.closes(), 1);
    assert_eq!(delegate.reads(), 0);
    assert!(calls.lock().is_empty());
}

#[test]
#[ignore = "Blaze's InputStream has no available(); nothing to delegate"]
fn available_calls_delegate() {}

#[test]
fn read_callsdelegate() {
    let delegate = Mock::new(vec![Ok(1)]);
    let (r, calls) = reconnector(None, None, false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    read1(&mut stream).unwrap();
    assert_eq!(delegate.reads(), 1);
    assert!(calls.lock().is_empty());
}

#[test]
fn buffer_read_callsdelegate() {
    let delegate = Mock::new(vec![Ok(1024)]);
    let (r, calls) = reconnector(None, None, false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    let mut buffer = [0u8; 1024];
    stream.read(&mut buffer).unwrap();
    assert_eq!(delegate.reads(), 1);
    assert!(calls.lock().is_empty());
}

#[test]
fn read_interrupted_always_passes_through() {
    let delegate = Mock::new(vec![Err(IoException::without_message(
        IoExceptionKind::InterruptedIOException,
    ))]);
    let (r, calls) = reconnector(None, None, false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    let e = read1(&mut stream).unwrap_err();
    assert_eq!(e.kind, IoExceptionKind::InterruptedIOException);
    assert_eq!(delegate.reads(), 1);
    assert_eq!(delegate.closes(), 0);
    assert!(calls.lock().is_empty());
}

#[test]
fn read_times_out_retries() {
    let delegate = Mock::new(vec![Ok(1), Err(timeout())]);
    let new_delegate = Mock::new(vec![Ok(1)]);
    let (r, calls) = reconnector(Some(new_delegate.clone()), Some("bytes 1-42/42"), false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    assert_eq!(read1(&mut stream).unwrap(), 1);
    assert_eq!(read1(&mut stream).unwrap(), 1);
    assert_eq!(*calls.lock(), vec![range("bytes=1-")]);
    assert_eq!(delegate.reads(), 2);
    assert_eq!(delegate.closes(), 1);
    assert_eq!(new_delegate.reads(), 1);
}

#[test]
fn failure_when_no_bytes_are_read_doesnt_use_range() {
    let delegate = Mock::new(vec![Err(timeout())]);
    let new_delegate = Mock::new(vec![Ok(1)]);
    let (r, calls) = reconnector(Some(new_delegate.clone()), None, false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    assert_eq!(read1(&mut stream).unwrap(), 1);
    assert_eq!(*calls.lock(), vec![Vec::<(String, Vec<String>)>::new()]);
    assert_eq!(delegate.reads(), 1);
    assert_eq!(delegate.closes(), 1);
    assert_eq!(new_delegate.reads(), 1);
}

#[test]
fn reconnect_fails_always_passes_through() {
    let delegate = Mock::new(vec![Err(io())]);
    let (r, calls) = reconnector(None, None, true);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    assert!(read1(&mut stream).is_err());
    assert_eq!(delegate.reads(), 1);
    assert_eq!(delegate.closes(), 1);
    assert_eq!(calls.lock().len(), 1);
}

#[test]
fn max_retries_gives_up() {
    let delegate = Mock::new(vec![Ok(1), Err(io()), Err(io()), Err(io()), Err(timeout())]);
    let (r, calls) = reconnector(Some(delegate.clone()), Some("bytes 1-42/42"), false);
    let mut stream = RetryingInputStream::new(Box::new(delegate.clone()), r);
    read1(&mut stream).unwrap();
    let e = read1(&mut stream).unwrap_err();
    assert_eq!(e.kind, IoExceptionKind::SocketTimeoutException);
    assert_eq!(e.suppressed.len(), 3);
    assert_eq!(*calls.lock(), vec![range("bytes=1-"); 3]);
    assert_eq!(delegate.reads(), 5);
    assert_eq!(delegate.closes(), 3);
}
