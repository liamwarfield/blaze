//! Port of `lib/remote/common/LazyFileOutputStream.java`.

use std::io::{self, Write};

use blaze_vfs::Path;

use super::errors::io_exception;

/// Creates an `OutputStream` backed by a file that isn't actually opened until the first data is
/// written. This is useful to only have as many open file descriptors as necessary at a time to
/// avoid running into system limits.
///
/// PORT: Java's `MaybePathBacked` interface (`common/MaybePathBacked.java`) is the inherent
/// method [`LazyFileOutputStream::maybe_get_path`]; a `&mut dyn Write` cannot be asked for it
/// (Java: `out instanceof MaybePathBacked`).
// Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:28 LazyFileOutputStream
pub struct LazyFileOutputStream {
    path: Path,
    /// `out`; `None` before the first write and after [`LazyFileOutputStream::close`].
    out: Option<Box<dyn Write + Send>>,
    /// Whether `out` was opened and then closed (Java keeps the closed stream in `out`, so the
    /// file is not opened again).
    closed: bool,
}

impl std::fmt::Debug for LazyFileOutputStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyFileOutputStream")
            .field("path", &self.path)
            .field("open", &self.out.is_some())
            .field("closed", &self.closed)
            .finish()
    }
}

impl LazyFileOutputStream {
    /// `new LazyFileOutputStream(path)`.
    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:33 LazyFileOutputStream.<init>
    pub fn new(path: Path) -> LazyFileOutputStream {
        LazyFileOutputStream {
            path,
            out: None,
            closed: false,
        }
    }

    /// `close()`.
    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:63 LazyFileOutputStream.close
    pub fn close(&mut self) -> io::Result<()> {
        if let Some(mut out) = self.out.take() {
            self.closed = true;
            // Dropping the stream closes the file.
            out.flush()?;
        }
        Ok(())
    }

    /// If the output stream is a `FileOutputStream`, call `FileDescriptor#sync` on it. Otherwise,
    /// do nothing.
    ///
    /// PORT: `blaze_vfs::Path::get_output_stream` hands out an opaque `Box<dyn Write + Send>`
    /// (missing hook: a `sync` on the stream of the file system). `fsync` acts on the file, not
    /// on the descriptor, so the port syncs through a second descriptor of the same file; every
    /// `blaze_vfs` file system is the host file system.
    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:73 LazyFileOutputStream.syncIfPossible
    pub fn sync_if_possible(&mut self) -> io::Result<()> {
        self.ensure_open()?;
        match self.out.as_mut() {
            Some(out) => out.flush()?,
            // Java: `FileDescriptor.sync()` on a closed stream.
            None => return Err(io_exception("java.io.SyncFailedException: sync failed")),
        }
        std::fs::File::open(self.path.get_path_string())?.sync_all()
    }

    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:80 LazyFileOutputStream.ensureOpen
    fn ensure_open(&mut self) -> io::Result<()> {
        if self.out.is_none() && !self.closed {
            self.out = Some(self.path.get_output_stream()?);
        }
        Ok(())
    }

    /// `maybeGetPath()` (`MaybePathBacked`).
    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:87 LazyFileOutputStream.maybeGetPath
    pub fn maybe_get_path(&self) -> Option<&Path> {
        Some(&self.path)
    }
}

impl Write for LazyFileOutputStream {
    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:44 LazyFileOutputStream.write
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.ensure_open()?;
        match self.out.as_mut() {
            Some(out) => out.write_all(buf)?,
            // Java: `FileOutputStream.write` on a closed stream.
            None => return Err(io_exception("Stream Closed")),
        }
        Ok(buf.len())
    }

    // Java: com/google/devtools/build/lib/remote/common/LazyFileOutputStream.java:56 LazyFileOutputStream.flush
    fn flush(&mut self) -> io::Result<()> {
        if let Some(out) = self.out.as_mut() {
            out.flush()?;
        }
        Ok(())
    }
}
