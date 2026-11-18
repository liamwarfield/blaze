//! Port of `src/test/java/com/google/devtools/build/lib/shell/FutureConsumptionTest.java`.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use blaze_shell::thread_interrupt;
use blaze_shell::{Consumers, OutputStream, ThreadInterrupt};

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Keeps producing input until told to stop.
struct Endless(Arc<AtomicBool>);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.0.load(Ordering::SeqCst) || buf.is_empty() {
            return Ok(0);
        }
        buf[0] = 0;
        Ok(1)
    }
}

#[test]
fn test_future_consumption_ignores_interrupted_exceptions() {
    // Set this up so that the consumer actually have to stream stuff into DEV_NULL, which then
    // discards everything.
    let dev_null: OutputStream = Arc::new(Mutex::new(DevNull));
    let mut out_err = Consumers::create_streaming_consumers(dev_null.clone(), dev_null);

    let input_finished = Arc::new(AtomicBool::new(false));
    let err_input: &'static [u8] = b"";
    out_err.register_inputs(
        Box::new(Endless(input_finished.clone())),
        Box::new(err_input),
        false,
    );
    // OK, this is the main test thread, which we need to interrupt *while* it's waiting in
    // outErr.waitForCompletion()
    let test_thread = ThreadInterrupt::current();

    // go into a different thread, wait a bit, interrupt the test thread, wait a bit, and tell the
    // input stream to finish.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1000));
        test_thread.interrupt(); // this is what we're testing; basic
        std::thread::sleep(Duration::from_millis(1000));
        input_finished.store(true, Ordering::SeqCst);
    });

    out_err.wait_for_completion().unwrap();
    // In addition to asserting that we were interrupted, this clears the interrupt bit.
    assert!(thread_interrupt::interrupted());
}
