//! Port of `lib/util/io/AsynchronousMessageOutputStream.java`.

use std::io::{self, BufWriter, Write};
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use blaze_protos::JavaMessage;
use blaze_vfs::Path;
use parking_lot::Mutex;

use super::{FlushingOutputStream, MessageOutputStream, OutputStream};

/// State shared with the writer thread.
struct Shared {
    /// `closeFuture.isDone()`.
    close_done: AtomicBool,
    /// `exception.get() != null`.
    exception_set: AtomicBool,
    /// The outcome of `closeFuture` (an `io::Error` is not `Clone`, so kind and message).
    close_result: Mutex<Option<Result<(), (io::ErrorKind, String)>>>,
}

/// An output stream supporting asynchronous writes of length-delimited protocol buffer messages,
/// backed by a file. Thread-safe.
///
/// The `LinkedBlockingDeque` of Java is an unbounded channel; the `POISON_PILL` is the
/// disconnection of the channel's sender.
// Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:40 AsynchronousMessageOutputStream
pub struct AsynchronousMessageOutputStream<T> {
    sender: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    writer_thread: Mutex<Option<thread::JoinHandle<()>>>,
    shared: Arc<Shared>,
    _marker: PhantomData<fn(&T)>,
}

impl<T: JavaMessage> AsynchronousMessageOutputStream<T> {
    /// `new AsynchronousMessageOutputStream(Path)`: a 100 kB buffered stream to the file.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:52 AsynchronousMessageOutputStream.<init>
    pub fn new_for_path(path: &Path) -> io::Result<AsynchronousMessageOutputStream<T>> {
        Ok(Self::new(
            path.get_path_string(),
            Box::new(FlushingOutputStream(BufWriter::with_capacity(
                // Use a buffer of 100 kByte, scientifically chosen at random.
                100000,
                path.get_output_stream()?,
            ))),
        ))
    }

    /// `new AsynchronousMessageOutputStream(String name, OutputStream out)`.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:59 AsynchronousMessageOutputStream.<init>
    pub fn new(name: &str, mut out: Box<dyn OutputStream>) -> AsynchronousMessageOutputStream<T> {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let shared = Arc::new(Shared {
            close_done: AtomicBool::new(false),
            exception_set: AtomicBool::new(false),
            close_result: Mutex::new(None),
        });
        let thread_shared = Arc::clone(&shared);
        let writer_thread = thread::Builder::new()
            .name(format!("async-file-writer:{name}"))
            .spawn(move || {
                let mut write_error: Option<io::Error> = None;
                while let Ok(data) = receiver.recv() {
                    if let Err(e) = out.write_all(&data) {
                        thread_shared.exception_set.store(true, Ordering::SeqCst);
                        write_error = Some(e);
                        break;
                    }
                }
                // finally: out.close(). A write failure completes the future first.
                let close_result = out.close();
                let result = match write_error {
                    Some(e) => Err(e),
                    None => close_result,
                };
                *thread_shared.close_result.lock() =
                    Some(result.map_err(|e| (e.kind(), e.to_string())));
                thread_shared.close_done.store(true, Ordering::SeqCst);
            })
            .expect("failed to start the async-file-writer thread");
        AsynchronousMessageOutputStream {
            sender: Mutex::new(Some(sender)),
            writer_thread: Mutex::new(Some(writer_thread)),
            shared,
            _marker: PhantomData,
        }
    }

    /// Writes a protocol buffer message in the same format as `MessageLite.writeDelimitedTo`.
    ///
    /// The writes are guaranteed to land in the output file in the same order that they were
    /// called; however, some writes may fail, leaving the file partially corrupted. In case a
    /// write fails, an error will be propagated in close, but remaining writes will be allowed to
    /// continue.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:96 AsynchronousMessageOutputStream.write
    pub fn write(&self, m: &T) {
        if self.shared.close_done.load(Ordering::SeqCst) {
            if self.shared.exception_set.load(Ordering::SeqCst) {
                // There was a previous write failure. Silently return without doing anything.
                return;
            } else {
                // Attempted to write after closing.
                panic!("IllegalStateException");
            }
        }

        let size = m.get_serialized_size();
        let mut bos = Vec::with_capacity(size + 5);
        m.write_delimited_to(&mut bos);

        // Java enqueues even after the poison pill (and the entry is never written); a closed
        // channel drops it the same way.
        if let Some(sender) = self.sender.lock().as_ref() {
            let _ = sender.send(bos);
        }
    }

    /// Closes the stream and blocks until all pending writes are completed.
    ///
    /// Returns an error if any of the writes or the close itself have failed.
    // Java: com/google/devtools/build/lib/util/io/AsynchronousMessageOutputStream.java:129 AsynchronousMessageOutputStream.close
    pub fn close(&self) -> io::Result<()> {
        // closeAsync(): the poison pill.
        self.sender.lock().take();
        let handle = self.writer_thread.lock().take();
        if let Some(handle) = handle
            && handle.join().is_err()
        {
            return Err(io::Error::other("async-file-writer thread panicked"));
        }
        match &*self.shared.close_result.lock() {
            Some(Err((kind, message))) => Err(io::Error::new(*kind, message.clone())),
            _ => Ok(()),
        }
    }
}

impl<T: JavaMessage> MessageOutputStream<T> for AsynchronousMessageOutputStream<T> {
    fn write(&mut self, m: &T) -> io::Result<()> {
        AsynchronousMessageOutputStream::write(self, m);
        Ok(())
    }

    fn close(&mut self) -> io::Result<()> {
        AsynchronousMessageOutputStream::close(self)
    }
}

impl<T> Drop for AsynchronousMessageOutputStream<T> {
    fn drop(&mut self) {
        // PORT: Java leaves the writer thread blocked on the queue forever when a stream is never
        // closed; disconnecting lets it finish the pending writes and exit.
        self.sender.get_mut().take();
    }
}
