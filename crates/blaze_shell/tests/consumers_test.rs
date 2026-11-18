//! Port of `src/test/java/com/google/devtools/build/lib/shell/ConsumersTest.java`.

mod common;

use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use parking_lot::Mutex;

use blaze_shell::{Consumers, OutputStream};

const SECRET_MESSAGE: &str = "This is a secret message.";

enum Failure {
    Io,
    Panic,
}

struct FailingStream(Failure);

impl Write for FailingStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match self.0 {
            Failure::Io => Err(io::Error::other(SECRET_MESSAGE)),
            Failure::Panic => panic!("{}", SECRET_MESSAGE),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn register(failure: Failure) -> blaze_shell::OutErrConsumers {
    let out: OutputStream = Arc::new(Mutex::new(FailingStream(failure)));
    let mut out_err = Consumers::create_streaming_consumers(out.clone(), out);
    let out_input: &'static [u8] = b"a";
    let err_input: &'static [u8] = b"";
    out_err.register_inputs(Box::new(out_input), Box::new(err_input), false);
    out_err
}

/// Tests that if an IOException occurs in an output stream, the exception will be recorded and
/// thrown when we call waitForCompletion.
#[test]
fn test_asynchronous_io_exception_in_consumer_output_stream() {
    let mut out_err = register(Failure::Io);
    let e = out_err.wait_for_completion().unwrap_err();
    assert_eq!(e.to_string(), SECRET_MESSAGE);
}

fn assert_panic_propagates() {
    let mut out_err = register(Failure::Panic);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(|| out_err.wait_for_completion()));
    std::panic::set_hook(prev);
    let payload = r.expect_err("expected the worker's panic to propagate");
    assert_eq!(common::payload_string(&*payload), SECRET_MESSAGE);
}

/// Tests that if an OutOfMemoryError occurs in an output stream, it will be recorded and thrown
/// when we call waitForCompletion. PORT: Java errors are Rust panics; the payload must not be
/// masked.
#[test]
fn test_asynchronous_out_of_memory_error_in_consumer_output_stream() {
    assert_panic_propagates();
}

/// Tests that if an Error occurs in an output stream, the error will be recorded and thrown when
/// we call waitForCompletion.
#[test]
fn test_asynchronous_error_in_consumer_output_stream() {
    assert_panic_propagates();
}

/// Tests that if an RuntimeException occurs in an output stream, the exception will be recorded
/// and thrown when we call waitForCompletion.
#[test]
fn test_asynchronous_runtime_exception_in_consumer_output_stream() {
    assert_panic_propagates();
}
