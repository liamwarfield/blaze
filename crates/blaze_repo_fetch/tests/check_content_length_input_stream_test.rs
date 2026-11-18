//! Tests of `CheckContentLengthInputStream`.
//!
//! Bazel has no `CheckContentLengthInputStreamTest`; the class is covered by `HttpStreamTest`
//! (`bigDataTruncated_throwsExpectedError`, `bigDataOverflowed_throwsExpectedError`, ported in
//! `http_stream_test.rs`). These tests cover the same Java semantics
//! (`CheckContentLengthInputStream.java:48-73`, `ContentLengthMismatchException.java:25`) for a
//! transport that reports a short body as an error instead of an end of stream, which is what
//! `ureq` does where the JDK's `HttpURLConnection` stream returns -1.

mod downloader_support;

use std::io::{self, Cursor, Read};
use std::sync::Arc;

use blaze_repo_fetch::downloader::http_connector::UrlConnection;
use blaze_repo_fetch::downloader::http_stream::HttpStreamFactory;
use blaze_repo_fetch::downloader::streams::{
    CheckContentLengthInputStream, InputStream, ReadInputStream, Reconnector, RetryingInputStream,
    transfer_to,
};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use downloader_support::{ManualClock, RecordingHandler};

const AURL: &str = "http://doodle.example";

/// A body like `ureq`'s `LimitedRead`: the data, then an `UnexpectedEof` error because the
/// connection closed before `Content-Length` bytes arrived.
struct ShortBody(Cursor<Vec<u8>>);

impl Read for ShortBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf)? {
            0 => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "response body closed before all bytes were read",
            )),
            n => Ok(n),
        }
    }
}

fn short_body(data: &[u8]) -> Box<dyn InputStream> {
    Box::new(ReadInputStream::new(ShortBody(Cursor::new(data.to_vec()))))
}

fn bytes(data: &[u8]) -> Box<dyn InputStream> {
    Box::new(ReadInputStream::new(Cursor::new(data.to_vec())))
}

fn read_all(stream: &mut dyn InputStream) -> Result<Vec<u8>, IoException> {
    let mut out = Vec::new();
    transfer_to(stream, &mut out)?;
    Ok(out)
}

fn assert_mismatch(e: &IoException, actual: usize, expected: usize) {
    assert_eq!(e.kind, IoExceptionKind::ContentLengthMismatchException);
    assert_eq!(
        e.get_message(),
        format!("Bytes read {} but wanted {}", actual, expected)
    );
}

#[test]
fn matching_length_reads_ok() {
    let mut s = CheckContentLengthInputStream::new(bytes(b"hello"), 5);
    assert_eq!(read_all(&mut s).unwrap(), b"hello");
    s.close().unwrap();
}

#[test]
fn truncated_throws_content_length_mismatch_on_end_of_stream() {
    let mut s = CheckContentLengthInputStream::new(bytes(b"hello"), 6);
    assert_mismatch(&read_all(&mut s).unwrap_err(), 5, 6);
    // close() checks again.
    assert_mismatch(&s.close().unwrap_err(), 5, 6);
}

#[test]
fn overflowed_throws_content_length_mismatch_on_end_of_stream() {
    let mut s = CheckContentLengthInputStream::new(bytes(b"hello"), 4);
    assert_mismatch(&read_all(&mut s).unwrap_err(), 5, 4);
}

#[test]
fn short_body_reported_by_transport_throws_content_length_mismatch() {
    let mut s = CheckContentLengthInputStream::new(short_body(b"hello"), 9);
    let mut buf = [0u8; 16];
    assert_eq!(s.read(&mut buf).unwrap(), 5);
    // The transport's "short body" error is Java's end of stream: the bytes read so far are
    // reported against the expected size.
    assert_mismatch(&s.read(&mut buf).unwrap_err(), 5, 9);
    assert_mismatch(&s.close().unwrap_err(), 5, 9);
}

#[test]
fn short_body_with_nothing_read_throws_content_length_mismatch() {
    let mut s = CheckContentLengthInputStream::new(short_body(b""), 3);
    assert_mismatch(&read_all(&mut s).unwrap_err(), 0, 3);
}

#[test]
fn other_read_errors_are_passed_on() {
    struct Reset;
    impl Read for Reset {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "Connection reset",
            ))
        }
    }
    let mut s = CheckContentLengthInputStream::new(Box::new(ReadInputStream::new(Reset)), 3);
    let e = read_all(&mut s).unwrap_err();
    assert_eq!(e.kind, IoExceptionKind::SocketException);
}

/// A reconnector mock that must not be called.
fn unused_reconnector() -> Reconnector {
    Arc::new(|_, _| panic!("unexpected reconnect"))
}

#[test]
fn short_body_is_not_resumed_by_retrying_input_stream_of_fixed_length_body() {
    // HttpStream.Factory.create's stack for `Accept-Ranges: bytes` + `Content-Length`. With the
    // JDK the short body is an end of stream, so RetryingInputStream does not reconnect.
    let retrying = RetryingInputStream::new(short_body(b"hello"), unused_reconnector())
        .with_fixed_length_body();
    let mut s = CheckContentLengthInputStream::new(Box::new(retrying), 9);
    assert_mismatch(&read_all(&mut s).unwrap_err(), 5, 9);
}

fn connection(headers: &[(&str, &str)], body: Box<dyn InputStream>) -> UrlConnection {
    UrlConnection::new(
        JavaUri::create(AURL).unwrap(),
        200,
        None,
        headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        Ok(body),
    )
}

fn http_stream_error(headers: &[(&str, &str)]) -> IoException {
    let factory = HttpStreamFactory {
        clock: ManualClock::new(),
        event_handler: RecordingHandler::new(),
    };
    let mut s = factory
        .create(
            connection(headers, short_body(b"hello")),
            &JavaUri::create(AURL).unwrap(),
            None,
            unused_reconnector(),
            None,
        )
        .unwrap();
    read_all(&mut s).unwrap_err()
}

#[test]
fn http_stream_short_body_throws_content_length_mismatch() {
    assert_mismatch(&http_stream_error(&[("Content-Length", "9")]), 5, 9);
}

#[test]
fn http_stream_short_body_with_accept_ranges_throws_content_length_mismatch() {
    assert_mismatch(
        &http_stream_error(&[("Accept-Ranges", "bytes"), ("Content-Length", "9")]),
        5,
        9,
    );
}
