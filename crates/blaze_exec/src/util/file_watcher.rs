//! Port of `util/io/FileWatcher.java`.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_vfs::Path;

/// How often we check for updates in the file we watch. (in ms)
// Java: com/google/devtools/build/lib/util/io/FileWatcher.java:36 FileWatcher.WATCH_INTERVAL
const WATCH_INTERVAL: u64 = 100;

/// The FileWatcher dumps the contents of a files into an OutErr. It then stays active and dumps
/// any content to the OutErr that is added to the file, until it is told to stop and all output
/// has been dumped.
///
/// This is useful to emulate streaming test output.
///
/// PORT: Java's class is the `Thread` itself; here it is the state the thread runs with
/// ([`FileWatcher::run`]), and the caller starts the thread (a scoped thread in
/// `StreamedTestOutput`, which borrows the action's event handler). The `OutErr` both of whose
/// streams are the same filter (`OutErr.create(headerFilter, headerFilter)`) is the one writer
/// `W`.
// Java: com/google/devtools/build/lib/util/io/FileWatcher.java:33 FileWatcher
pub struct FileWatcher {
    output_file: Path,
    finish_pumping: Arc<AtomicBool>,
    to_skip: i64,
}

impl FileWatcher {
    /// Creates a FileWatcher that will dump any output that is appended to outputFile onto
    /// output. If skipExisting is true, the watcher will not dump any output that is in
    /// outputFile when we construct the watcher. If skipExisting is false, already existing
    /// output will be dumped, too.
    // Java: com/google/devtools/build/lib/util/io/FileWatcher.java:53 FileWatcher.<init>
    pub fn new(output_file: Path, skip_existing: bool) -> io::Result<FileWatcher> {
        let mut to_skip = 0;
        if output_file.exists() && skip_existing {
            to_skip = output_file.get_file_size()?;
        }
        Ok(FileWatcher {
            output_file,
            finish_pumping: Arc::new(AtomicBool::new(false)),
            to_skip,
        })
    }

    /// The flag [`Self::stop_pumping`] sets, for the thread that owns the watcher.
    pub fn stop_handle(&self) -> FileWatcherStopHandle {
        FileWatcherStopHandle(self.finish_pumping.clone())
    }

    // Java: com/google/devtools/build/lib/util/io/FileWatcher.java:83 FileWatcher.run
    pub fn run<W: Write>(mut self, output: &mut W) {
        let result = (|| -> io::Result<()> {
            // Wait until the file exists, or we have to abort.
            while !self.output_file.exists() && !self.finish_pumping.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(WATCH_INTERVAL));
            }

            // Check that we did not have abort before the file was created.
            if self.output_file.exists() {
                let mut input_stream = self.output_file.get_input_stream()?;
                // PORT: `inputStream.available()` of a FileInputStream: the bytes between the
                // position and the end of the file.
                let mut position: i64 = 0;
                let available = |position: i64| -> io::Result<i64> {
                    Ok((self.output_file.get_file_size()? - position).max(0))
                };
                let mut buffer = [0u8; 1024];
                while !self.finish_pumping.load(Ordering::SeqCst) || available(position)? != 0 {
                    if available(position)? != 0 {
                        if self.to_skip > 0 {
                            let skipped = io::copy(
                                &mut (&mut input_stream).take(self.to_skip as u64),
                                &mut io::sink(),
                            )? as i64;
                            position += skipped;
                            self.to_skip -= skipped;
                        } else {
                            let read = input_stream.read(&mut buffer)?;
                            if read > 0 {
                                position += read as i64;
                                output.write_all(&buffer[..read])?;
                            }
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(WATCH_INTERVAL));
                    }
                }
                // try-with-resources closes the output stream.
                output.flush()?;
            }
            Ok(())
        })();
        if let Err(ex) = result {
            // output.printOutLn("Failure reading or writing: " + ex.getMessage())
            let _ = output.write_all(&blaze_util::string_encoding::latin1_encode_lossy(&format!(
                "Failure reading or writing: {ex}\n"
            )));
            let _ = output.flush();
        }
    }
}

/// `FileWatcher.stopPumping`, callable from another thread.
#[derive(Clone)]
pub struct FileWatcherStopHandle(Arc<AtomicBool>);

impl FileWatcherStopHandle {
    /// Tells the FileWatcher to stop pumping output and finish. The FileWatcher will only finish
    /// until there is no data left to display.
    // Java: com/google/devtools/build/lib/util/io/FileWatcher.java:76 FileWatcher.stopPumping
    pub fn stop_pumping(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
