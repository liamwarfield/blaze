//! Port of `exec/StreamedTestOutput.java`: implements the `--test_output=streamed` option.

use std::io::{self, Write};
use std::thread::{Scope, ScopedJoinHandle};

use blaze_events::{Event, EventHandler, EventKind};
use blaze_vfs::Path;

use crate::exec::test_log_helper::FilterTestHeaderOutputStream;
use crate::util::file_watcher::{FileWatcher, FileWatcherStopHandle};

/// The stdout stream of `Reporter.outErrForReporter(handler)`: every write is one
/// `EventKind.STDOUT` event.
// Java: com/google/devtools/build/lib/events/ReporterStream.java:24 ReporterStream
struct ReporterOut<'a>(&'a dyn EventHandler);

impl Write for ReporterOut<'_> {
    // Java: com/google/devtools/build/lib/events/ReporterStream.java:54 ReporterStream.write
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .handle(&Event::of_bytes_at(EventKind::Stdout, None, buf.to_vec()));
        Ok(buf.len())
    }

    // Java: com/google/devtools/build/lib/events/ReporterStream.java:39 ReporterStream.flush
    fn flush(&mut self) -> io::Result<()> {
        // NOP.
        Ok(())
    }
}

/// Implements the --test_output=streamed option.
///
/// PORT: Java's `Closeable`; [`StreamedTestOutput::close`] is `close()`, and dropping an unclosed
/// one is `TestStrategy.closeSuppressed` (stops and joins the watcher, errors dropped). The
/// watcher runs on a thread of `scope`, which borrows the reporter (`outErr`).
// Java: com/google/devtools/build/lib/exec/StreamedTestOutput.java:29 StreamedTestOutput
pub struct StreamedTestOutput<'scope> {
    test_log_path: Path,
    out_err: &'scope dyn EventHandler,
    stop: FileWatcherStopHandle,
    /// The watcher thread; it returns `headerFilter.foundHeader()`.
    watcher: Option<ScopedJoinHandle<'scope, bool>>,
}

impl<'scope> StreamedTestOutput<'scope> {
    // Java: com/google/devtools/build/lib/exec/StreamedTestOutput.java:37 StreamedTestOutput.<init>
    pub fn new<'env>(
        scope: &'scope Scope<'scope, 'env>,
        out_err: &'scope dyn EventHandler,
        test_log_path: Path,
    ) -> io::Result<StreamedTestOutput<'scope>> {
        let watcher = FileWatcher::new(test_log_path.clone(), false)?;
        let stop = watcher.stop_handle();
        // watcher.start(): "Streaming Test Output Pump".
        let handle = std::thread::Builder::new()
            .name("Streaming Test Output Pump".to_string())
            .spawn_scoped(scope, move || {
                let mut reporter_out = ReporterOut(out_err);
                // TestLogHelper.getHeaderFilteringOutputStream(outErr.getOutputStream())
                let mut header_filter = FilterTestHeaderOutputStream::new(&mut reporter_out);
                watcher.run(&mut header_filter);
                header_filter.found_header()
            })?;
        Ok(StreamedTestOutput {
            test_log_path,
            out_err,
            stop,
            watcher: Some(handle),
        })
    }

    /// Stops the watcher and waits for it.
    fn stop_and_join(&mut self) -> bool {
        self.stop.stop_pumping();
        let handle = self.watcher.take().expect("StreamedTestOutput closed twice");
        // The watcher thread might leak if the following call is interrupted. This is a
        // relatively minor issue since the worst it could do is write one additional line from
        // the test.log to the console later on in the build.
        // PORT: Blaze threads are not interrupted; the join waits for the thread.
        match handle.join() {
            Ok(found_header) => found_header,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    // Java: com/google/devtools/build/lib/exec/StreamedTestOutput.java:46 StreamedTestOutput.close
    pub fn close(mut self) -> io::Result<()> {
        let found_header = self.stop_and_join();

        // It's unclear if writing this after interrupt is desirable, but it's been this way
        // forever.
        if !found_header {
            let mut input = self.test_log_path.get_input_stream()?;
            io::copy(&mut input, &mut ReporterOut(self.out_err))?;
        }
        Ok(())
    }
}

impl Drop for StreamedTestOutput<'_> {
    fn drop(&mut self) {
        if self.watcher.is_some() {
            if std::thread::panicking() {
                // The scope joins the thread once it stops.
                self.stop.stop_pumping();
                return;
            }
            // closeSuppressed(e, streamed): close() with its IOException suppressed.
            let found_header = self.stop_and_join();
            if !found_header && let Ok(mut input) = self.test_log_path.get_input_stream() {
                let _ = io::copy(&mut input, &mut ReporterOut(self.out_err));
            }
        }
    }
}
