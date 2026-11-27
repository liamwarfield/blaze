//! Port of `HttpStreamTest`.

mod downloader_support;

use std::io::{Cursor, Write};
use std::sync::Arc;

use blaze_repo_fetch::cache::KeyType;
use blaze_repo_fetch::downloader::Checksum;
use blaze_repo_fetch::downloader::http_connector::UrlConnection;
use blaze_repo_fetch::downloader::http_stream::{HttpStream, HttpStreamFactory, PRECHECK_BYTES};
use blaze_repo_fetch::downloader::streams::{
    InputStream, ReadInputStream, Reconnector, transfer_to,
};
use blaze_repo_fetch::errors::{IoException, IoExceptionKind};
use blaze_repo_fetch::uri::JavaUri;
use downloader_support::{ManualClock, RecordingHandler};
use parking_lot::Mutex;
use sha2::Digest as _;

const DATA: &[u8] = b"hello";
const AURL: &str = "http://doodle.example";

fn make_checksum(s: &str) -> Checksum {
    Checksum::from_string(KeyType::SHA256, s).unwrap()
}

fn good_checksum() -> Checksum {
    make_checksum("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
}

fn bad_checksum() -> Checksum {
    make_checksum("0000000000000000000000000000000000000000000000000000000000000000")
}

fn aurl() -> JavaUri {
    JavaUri::create(AURL).unwrap()
}

fn factory() -> HttpStreamFactory {
    HttpStreamFactory {
        clock: ManualClock::new(),
        event_handler: RecordingHandler::new(),
    }
}

/// A mocked connection: `getURL()` is `url`, the given headers, and `body` as input stream.
fn connection(url: &str, headers: &[(&str, &str)], body: Box<dyn InputStream>) -> UrlConnection {
    UrlConnection::new(
        JavaUri::create(url).unwrap(),
        200,
        None,
        headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        Ok(body),
    )
}

fn bytes(b: &[u8]) -> Box<dyn InputStream> {
    Box::new(ReadInputStream::new(Cursor::new(b.to_vec())))
}

/// A reconnector mock that must not be called.
fn unused_reconnector() -> Reconnector {
    Arc::new(|_, _| panic!("unexpected reconnect"))
}

fn to_byte_array(stream: &mut HttpStream) -> Result<Vec<u8>, IoException> {
    let mut out = Vec::new();
    transfer_to(stream, &mut out)?;
    Ok(out)
}

/// try-with-resources: read everything, then close.
fn read_and_close(mut stream: HttpStream) -> Result<Vec<u8>, IoException> {
    let r = to_byte_array(&mut stream);
    let c = stream.close();
    let v = r?;
    c?;
    Ok(v)
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn gzip_data(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

#[test]
fn no_checksum_reads_ok() {
    let s = factory()
        .create(
            connection(AURL, &[], bytes(DATA)),
            &aurl(),
            None,
            unused_reconnector(),
            None,
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), DATA);
}

#[test]
fn small_data_with_valid_checksum_reads_ok() {
    let s = factory()
        .create(
            connection(AURL, &[], bytes(DATA)),
            &aurl(),
            Some(&good_checksum()),
            unused_reconnector(),
            None,
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), DATA);
}

/// `mock(ByteArrayInputStream)` whose reads are scripted; shared between "connections".
struct Flaky {
    n_retries: Arc<Mutex<i32>>,
    real: Arc<Mutex<Cursor<Vec<u8>>>>,
    always_fail: bool,
}

impl InputStream for Flaky {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoException> {
        let mut n = self.n_retries.lock();
        let first = *n == 0;
        *n += 1;
        if self.always_fail || first {
            return Err(IoException::without_message(
                IoExceptionKind::SocketTimeoutException,
            ));
        }
        Ok(std::io::Read::read(&mut *self.real.lock(), buf)?)
    }

    fn close(&mut self) -> Result<(), IoException> {
        Ok(())
    }
}

fn flaky_setup(always_fail: bool) -> (UrlConnection, Reconnector, Arc<Mutex<i32>>) {
    let n_retries = Arc::new(Mutex::new(0));
    let real = Arc::new(Mutex::new(Cursor::new(DATA.to_vec())));
    let make = {
        let n_retries = n_retries.clone();
        move || -> Box<dyn InputStream> {
            Box::new(Flaky {
                n_retries: n_retries.clone(),
                real: real.clone(),
                always_fail,
            })
        }
    };
    let make = Arc::new(make);
    let conn = connection(AURL, &[("Accept-Ranges", "bytes")], make());
    let m = make.clone();
    let reconnector: Reconnector =
        Arc::new(move |_, _| Ok(connection(AURL, &[("Accept-Ranges", "bytes")], m())));
    (conn, reconnector, n_retries)
}

#[test]
fn small_data_with_valid_checksum_times_out_in_create_retries_ok() {
    let (conn, reconnector, _) = flaky_setup(false);
    let s = factory()
        .create(conn, &aurl(), Some(&good_checksum()), reconnector, None)
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), DATA);
}

#[test]
fn small_data_with_valid_checksum_times_out_in_create_repeatedly() {
    let (conn, reconnector, n_retries) = flaky_setup(true);
    let e = factory()
        .create(conn, &aurl(), Some(&good_checksum()), reconnector, None)
        .err()
        .unwrap();
    assert_eq!(e.kind, IoExceptionKind::SocketTimeoutException);
    assert!(*n_retries.lock() > 3); // RetryingInputStream.MAX_RESUMES
}

#[test]
fn small_data_with_invalid_checksum_throws_io_exception_in_create_phase() {
    let e = factory()
        .create(
            connection(AURL, &[], bytes(DATA)),
            &aurl(),
            Some(&bad_checksum()),
            unused_reconnector(),
            None,
        )
        .err()
        .unwrap();
    assert!(e.get_message().contains("Checksum"), "{}", e);
}

#[test]
fn big_data_with_valid_checksum_reads_ok() {
    let big_data = random_bytes(PRECHECK_BYTES + 70001);
    let checksum = make_checksum(&hex::encode(sha2::Sha256::digest(&big_data)));
    let s = factory()
        .create(
            connection(AURL, &[], bytes(&big_data)),
            &aurl(),
            Some(&checksum),
            unused_reconnector(),
            None,
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), big_data);
}

#[test]
fn big_data_with_invalid_checksum_throws_io_exception_after_create_on_eof() {
    let big_data = random_bytes(PRECHECK_BYTES + 70001);
    let mut s = factory()
        .create(
            connection(AURL, &[], bytes(&big_data)),
            &aurl(),
            Some(&bad_checksum()),
            unused_reconnector(),
            None,
        )
        .unwrap();
    let e = to_byte_array(&mut s).unwrap_err();
    assert!(e.get_message().contains("Checksum"), "{}", e);
}

fn content_length_case(content_length: usize) -> IoException {
    let big_data = random_bytes(PRECHECK_BYTES + 70001);
    let checksum = make_checksum(&hex::encode(sha2::Sha256::digest(&big_data)));
    let cl = content_length.to_string();
    let s = factory()
        .create(
            connection(AURL, &[("Content-Length", &cl)], bytes(&big_data)),
            &aurl(),
            Some(&checksum),
            unused_reconnector(),
            None,
        )
        .unwrap();
    read_and_close(s).unwrap_err()
}

#[test]
fn big_data_truncated_throws_expected_error() {
    let n = PRECHECK_BYTES + 70001;
    let e = content_length_case(n + 1);
    assert_eq!(e.kind, IoExceptionKind::ContentLengthMismatchException);
    // getActualSize() / getExpectedSize()
    assert_eq!(
        e.get_message(),
        format!("Bytes read {} but wanted {}", n, n + 1)
    );
}

#[test]
fn big_data_overflowed_throws_expected_error() {
    let n = PRECHECK_BYTES + 70001;
    let e = content_length_case(n - 1);
    assert_eq!(e.kind, IoExceptionKind::ContentLengthMismatchException);
    assert_eq!(
        e.get_message(),
        format!("Bytes read {} but wanted {}", n, n - 1)
    );
}

#[test]
fn http_server_said_gzipped_but_not_gzipped_throws_zip_exception_in_create() {
    let e = factory()
        .create(
            connection(AURL, &[("Content-Encoding", "gzip")], bytes(DATA)),
            &aurl(),
            None,
            unused_reconnector(),
            None,
        )
        .err()
        .unwrap();
    assert_eq!(e.kind, IoExceptionKind::ZipException);
}

#[test]
fn javascript_gzipped_in_transit_automatically_gunzips() {
    let s = factory()
        .create(
            connection(
                AURL,
                &[("Content-Encoding", "x-gzip")],
                bytes(&gzip_data(DATA)),
            ),
            &aurl(),
            None,
            unused_reconnector(),
            None,
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), DATA);
}

#[test]
fn server_says_tarball_path_is_gzipped_doesnt_automatically_gunzip() {
    let gz = gzip_data(DATA);
    let s = factory()
        .create(
            connection(
                "http://doodle.example/foo.tar.gz",
                &[("Content-Encoding", "gzip")],
                bytes(&gz),
            ),
            &aurl(),
            None,
            unused_reconnector(),
            None,
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), gz);
}

// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/HttpStreamTest.java:292 HttpStreamTest.threadInterrupted_haltsReadingAndThrowsInterrupt
#[test]
fn thread_interrupted_halts_reading_and_throws_interrupt() {
    let was_interrupted = std::thread::spawn(|| {
        let mut stream = factory()
            .create(
                connection(AURL, &[], bytes(DATA)),
                &aurl(),
                None,
                unused_reconnector(),
                None,
            )
            .unwrap();
        let mut one = [0u8; 1];
        if stream.read(&mut one).is_err() {
            return false;
        }
        blaze_shell::thread_interrupt::interrupt_current();
        let result = stream.read(&mut one);
        let _ = stream.close();
        matches!(result, Err(e) if e.kind == IoExceptionKind::InterruptedIOException)
    })
    .join()
    .unwrap();
    assert!(was_interrupted);
}

#[test]
fn tarball_has_no_format_and_type_is_gzipped_doesnt_automatically_gunzip() {
    let gz = gzip_data(DATA);
    let s = factory()
        .create(
            connection(
                "http://doodle.example/foo",
                &[("Content-Encoding", "gzip")],
                bytes(&gz),
            ),
            &aurl(),
            None,
            unused_reconnector(),
            Some("tgz"),
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), gz);
}

#[test]
fn tarball_has_no_format_and_type_is_gzipped_and_has_multiple_extensions_doesnt_automatically_gunzip()
 {
    let gz = gzip_data(DATA);
    let s = factory()
        .create(
            connection(
                "http://doodle.example/foo",
                &[("Content-Encoding", "gzip")],
                bytes(&gz),
            ),
            &aurl(),
            None,
            unused_reconnector(),
            Some("tar.gz"),
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), gz);
}

#[test]
fn tarball_has_no_format_and_type_is_not_gzipped_automatically_gunzip() {
    let s = factory()
        .create(
            connection(
                "http://doodle.example/foo",
                &[("Content-Encoding", "gzip")],
                bytes(&gzip_data(DATA)),
            ),
            &aurl(),
            None,
            unused_reconnector(),
            Some("tar"),
        )
        .unwrap();
    assert_eq!(read_and_close(s).unwrap(), DATA);
}
