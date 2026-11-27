//! Port of `lib/util/io/FileOutErr.java`: an `OutErr` backed by two files.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_vfs::{Path, PathFragment};
use parking_lot::Mutex;

/// Stdout and stderr of a spawn, recorded in files.
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:46 FileOutErr
///
/// PORT: a clone is the same Java object (it shares `childCount` and both streams).
#[derive(Clone, Debug)]
pub struct FileOutErr {
    out: Arc<FileRecordingOutputStream>,
    err: Arc<FileRecordingOutputStream>,
    /// `childCount`.
    child_count: Arc<AtomicI32>,
}

// PORT: the one-file constructor `FileOutErr(Path)` (both streams one object) has no caller at
// the pin and is not ported.
// PORT: skipped, OutErr's stream accessors (Rust callers use write_out, write_err): FileOutErr.getFileOutputStream, FileOutErr.getFileErrorStream
// PORT: skipped, the abstract stream class is FileRecordingOutputStream with an optional file: FileOutErr.hadError, FileOutErr.getFile, FileOutErr.getFileUnsafe
// PORT: skipped, the abstract stream class is FileRecordingOutputStream with an optional file: FileOutErr.hasRecordedOutput, FileOutErr.getRecordedOutput, FileOutErr.clear
// PORT: skipped, the abstract stream class is FileRecordingOutputStream with an optional file: FileOutErr.getRecordedOutputSize, FileOutErr.dumpOut
// PORT: skipped, Java's single-byte write is the slice write of one byte: FileOutErr.write

impl FileOutErr {
    /// `new FileOutErr(Path stdout, Path stderr)`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:56 FileOutErr.<init>
    pub fn new(stdout: Path, stderr: Path) -> FileOutErr {
        FileOutErr {
            out: Arc::new(FileRecordingOutputStream::new(Some(stdout))),
            err: Arc::new(FileRecordingOutputStream::new(Some(stderr))),
            child_count: Arc::new(AtomicI32::new(0)),
        }
    }

    /// `new FileOutErr()`: records nothing (both streams are one `NullFileRecordingOutputStream`).
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:81 FileOutErr.<init>
    pub fn new_null() -> FileOutErr {
        let stream = Arc::new(FileRecordingOutputStream::new(None));
        FileOutErr {
            out: stream.clone(),
            err: stream,
            child_count: Arc::new(AtomicI32::new(0)),
        }
    }

    /// `childOutErr()`: a `FileOutErr` recording into `<stdout>-<n>` / `<stderr>-<n>`, `n`
    /// counting the children of this one from 0 (a null `FileOutErr` for a null one).
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:232 FileOutErr.childOutErr
    pub fn child_out_err(&self) -> FileOutErr {
        let index = self.child_count.fetch_add(1, Ordering::SeqCst);
        let (Some(out_path), Some(err_path)) =
            (self.out.get_file_unsafe(), self.err.get_file_unsafe())
        else {
            return FileOutErr::new_null();
        };
        FileOutErr::new(
            out_path
                .get_parent_directory()
                .expect("NullPointerException: getParentDirectory()")
                .get_relative_str(&format!("{}-{}", out_path.get_base_name(), index)),
            err_path
                .get_parent_directory()
                .expect("NullPointerException: getParentDirectory()")
                .get_relative_str(&format!("{}-{}", err_path.get_base_name(), index)),
        )
    }

    /// `FileOutErr.dump(from, to)`: writes the captured content of `from` to `to`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:218 FileOutErr.dump
    pub fn dump(from: &FileOutErr, to: &FileOutErr) {
        from.dump_out_as_latin1(&*to.out);
        from.dump_err_as_latin1(&*to.err);
    }

    /// `dumpOutAsLatin1(out)`: writes the recorded stdout (and a recorded error) to `out`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:202 FileOutErr.dumpOutAsLatin1
    pub fn dump_out_as_latin1(&self, out: &dyn OutputSink) {
        self.out.dump_out(out);
    }

    /// `dumpErrAsLatin1(out)`: writes the recorded stderr (and a recorded error) to `out`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:210 FileOutErr.dumpErrAsLatin1
    pub fn dump_err_as_latin1(&self, out: &dyn OutputSink) {
        self.err.dump_out(out);
    }

    /// `getOutputPath()`: the stdout file; the caller may write it, so its size is reread.
    ///
    /// PORT: Java returns null for a null `FileOutErr`; this one panics there.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:121 FileOutErr.getOutputPath
    pub fn get_output_path(&self) -> &Path {
        self.out
            .get_file()
            .expect("NullPointerException: getOutputPath() of a FileOutErr without files")
    }

    /// `getOutputPathFragment()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:132 FileOutErr.getOutputPathFragment
    pub fn get_output_path_fragment(&self) -> &PathFragment {
        self.out
            .get_file_unsafe()
            .expect("NullPointerException: getOutputPathFragment() of a FileOutErr without files")
            .as_fragment()
    }

    /// `getErrorPath()`: the stderr file; the caller may write it, so its size is reread.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:147 FileOutErr.getErrorPath
    pub fn get_error_path(&self) -> &Path {
        self.err
            .get_file()
            .expect("NullPointerException: getErrorPath() of a FileOutErr without files")
    }

    /// `getErrorPathFragment()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:156 FileOutErr.getErrorPathFragment
    pub fn get_error_path_fragment(&self) -> &PathFragment {
        self.err
            .get_file_unsafe()
            .expect("NullPointerException: getErrorPathFragment() of a FileOutErr without files")
            .as_fragment()
    }

    /// `hasRecordedOutput()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:95 FileOutErr.hasRecordedOutput
    pub fn has_recorded_output(&self) -> bool {
        self.out.has_recorded_output() || self.err.has_recorded_output()
    }

    /// `hasRecordedStdout()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:102 FileOutErr.hasRecordedStdout
    pub fn has_recorded_stdout(&self) -> bool {
        self.out.has_recorded_output()
    }

    /// `hasRecordedStderr()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:109 FileOutErr.hasRecordedStderr
    pub fn has_recorded_stderr(&self) -> bool {
        self.err.has_recorded_output()
    }

    /// `outSize()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:137 FileOutErr.outSize
    pub fn out_size(&self) -> io::Result<i64> {
        self.out.get_recorded_output_size()
    }

    /// `errSize()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:179 FileOutErr.errSize
    pub fn err_size(&self) -> io::Result<i64> {
        self.err.get_recorded_output_size()
    }

    /// `outAsBytes()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:160 FileOutErr.outAsBytes
    pub fn out_as_bytes(&self) -> Vec<u8> {
        self.out.get_recorded_output()
    }

    /// `errAsBytes()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:169 FileOutErr.errAsBytes
    pub fn err_as_bytes(&self) -> Vec<u8> {
        self.err.get_recorded_output()
    }

    /// `outAsLatin1()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:164 FileOutErr.outAsLatin1
    pub fn out_as_latin1(&self) -> String {
        blaze_util::string_encoding::latin1_decode(&self.out_as_bytes()).into_owned()
    }

    /// `errAsLatin1()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:173 FileOutErr.errAsLatin1
    pub fn err_as_latin1(&self) -> String {
        blaze_util::string_encoding::latin1_decode(&self.err_as_bytes()).into_owned()
    }

    /// `getErrorStream().write(bytes)`: writes to the stderr file, which the first write after
    /// creation or `clearErr()` truncates.
    pub fn write_err(&self, bytes: &[u8]) -> io::Result<()> {
        self.err.write(bytes)
    }

    /// `getOutputStream().write(bytes)`: writes to the stdout file, which the first write after
    /// creation or `clearOut()` truncates.
    pub fn write_out(&self, bytes: &[u8]) -> io::Result<()> {
        self.out.write(bytes)
    }

    /// `getErrorStream().flush()`.
    pub fn flush_err(&self) -> io::Result<()> {
        self.err.flush()
    }

    /// `getOutputStream().flush()`.
    pub fn flush_out(&self) -> io::Result<()> {
        self.out.flush()
    }

    /// `close()` of both streams (`OutErr.close`).
    pub fn close(&self) -> io::Result<()> {
        self.out.close()?;
        self.err.close()
    }

    /// `clearOut()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:193 FileOutErr.clearOut
    pub fn clear_out(&self) -> io::Result<()> {
        self.out.clear()
    }

    /// `clearErr()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:186 FileOutErr.clearErr
    pub fn clear_err(&self) -> io::Result<()> {
        self.err.clear()
    }
}

/// Where `dumpOut` writes: a Java `OutputStream` (the stream of another `FileOutErr`, or bytes in
/// memory).
pub trait OutputSink {
    /// `write(b, off, len)` with `len > 0`: errors are the sink's to handle (a
    /// `FileRecordingOutputStream` records them).
    fn write_recording(&self, bytes: &[u8]);
    /// `flush()`.
    fn flush_sink(&self);
}

impl OutputSink for FileRecordingOutputStream {
    fn write_recording(&self, bytes: &[u8]) {
        self.write_recording_error(bytes);
    }
    fn flush_sink(&self) {
        // Java's `transferTo` caller ignores nothing: `out.flush()` throws into `dumpOut`'s catch,
        // which records the error on the *source* stream. A flush of a `FileOutputStream` does
        // nothing, so it cannot fail here.
        let _ = self.flush();
    }
}

impl OutputSink for Mutex<Vec<u8>> {
    fn write_recording(&self, bytes: &[u8]) {
        self.lock().extend_from_slice(bytes);
    }
    fn flush_sink(&self) {}
}

/// `FileOutErr.FileRecordingOutputStream` (with a file) and
/// `FileOutErr.NullFileRecordingOutputStream` (without one).
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:363 FileOutErr.FileRecordingOutputStream
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:288 FileOutErr.NullFileRecordingOutputStream
pub struct FileRecordingOutputStream {
    output_file: Option<Path>,
    state: Mutex<StreamState>,
}

struct StreamState {
    output_stream: Option<Box<dyn Write + Send>>,
    error: Option<String>,
    /// `cachedSize`; `None` when the file may have changed.
    cached_size: Option<i64>,
}

impl std::fmt::Debug for FileRecordingOutputStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRecordingOutputStream")
            .field("output_file", &self.output_file)
            .finish_non_exhaustive()
    }
}

impl FileRecordingOutputStream {
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:370 FileOutErr.FileRecordingOutputStream.<init>
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:290 FileOutErr.NullFileRecordingOutputStream.<init>
    pub fn new(output_file: Option<Path>) -> FileRecordingOutputStream {
        FileRecordingOutputStream {
            output_file,
            state: Mutex::new(StreamState {
                output_stream: None,
                error: None,
                cached_size: Some(0),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:376 FileOutErr.FileRecordingOutputStream.hadError
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:294 FileOutErr.NullFileRecordingOutputStream.hadError
    pub fn had_error(&self) -> bool {
        self.state.lock().error.is_some()
    }

    /// The file; the caller may write it, so the cached size is dropped.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:381 FileOutErr.FileRecordingOutputStream.getFile
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:299 FileOutErr.NullFileRecordingOutputStream.getFile
    pub fn get_file(&self) -> Option<&Path> {
        if self.output_file.is_some() {
            self.mark_dirty();
        }
        self.output_file.as_ref()
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:389 FileOutErr.FileRecordingOutputStream.getFileUnsafe
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:304 FileOutErr.NullFileRecordingOutputStream.getFileUnsafe
    pub fn get_file_unsafe(&self) -> Option<&Path> {
        self.output_file.as_ref()
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:393 FileOutErr.FileRecordingOutputStream.markDirty
    fn mark_dirty(&self) {
        self.state.lock().cached_size = None;
    }

    /// Opens the file (truncating) on the first write after creation or `clear()`.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:397 FileOutErr.FileRecordingOutputStream.getOutputStream
    fn get_output_stream<'a>(
        file: &Path,
        state: &'a mut StreamState,
    ) -> io::Result<&'a mut Box<dyn Write + Send>> {
        if state.output_stream.is_none() {
            state.output_stream = Some(file.get_output_stream()?);
        }
        Ok(state.output_stream.as_mut().unwrap())
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:405 FileOutErr.FileRecordingOutputStream.hasOutputStream
    fn has_output_stream(state: &StreamState) -> bool {
        state.output_stream.is_some()
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:410 FileOutErr.FileRecordingOutputStream.clear
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:329 FileOutErr.NullFileRecordingOutputStream.clear
    pub fn clear(&self) -> io::Result<()> {
        let Some(file) = &self.output_file else {
            return Ok(());
        };
        let mut state = self.state.lock();
        Self::close_locked(&mut state)?;
        state.output_stream = None;
        file.delete()?;
        state.cached_size = Some(0);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:420 FileOutErr.FileRecordingOutputStream.recordError
    ///
    /// PORT: the text is the Rust error's (`io::Error`'s `Display`), not Java's exception message.
    fn record_error(state: &mut StreamState, exception: &io::Error) {
        let new_error_text = exception.to_string();
        state.error = Some(match state.error.take() {
            None => new_error_text,
            Some(error) => format!("{error}\n{new_error_text}"),
        });
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:426 FileOutErr.FileRecordingOutputStream.hasRecordedOutput
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:309 FileOutErr.NullFileRecordingOutputStream.hasRecordedOutput
    pub fn has_recorded_output(&self) -> bool {
        if self.output_file.is_none() {
            return false;
        }
        match self.get_recorded_output_size() {
            Ok(size) => size > 0,
            // Error already recorded by getRecordedOutputSize().
            Err(_) => true,
        }
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:436 FileOutErr.FileRecordingOutputStream.getRecordedOutput
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:314 FileOutErr.NullFileRecordingOutputStream.getRecordedOutput
    pub fn get_recorded_output(&self) -> Vec<u8> {
        let Some(file) = &self.output_file else {
            return Vec::new();
        };
        let mut state = self.state.lock();
        let mut bytes = None;
        match read_content(file) {
            Ok(content) => {
                state.cached_size = Some(content.len() as i64);
                bytes = Some(content);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => state.cached_size = Some(0),
            Err(e) => Self::record_error(&mut state, &e),
        }
        if let Some(error) = &state.error {
            let error_bytes = blaze_util::string_encoding::latin1_encode_lossy(error);
            match &mut bytes {
                None => bytes = Some(error_bytes.into_owned()),
                Some(bytes) => bytes.extend_from_slice(&error_bytes),
            }
        }
        bytes.unwrap_or_default()
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:461 FileOutErr.FileRecordingOutputStream.getRecordedOutputSize
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:319 FileOutErr.NullFileRecordingOutputStream.getRecordedOutputSize
    pub fn get_recorded_output_size(&self) -> io::Result<i64> {
        let Some(file) = &self.output_file else {
            return Ok(0);
        };
        let mut state = self.state.lock();
        if let Some(error) = &state.error {
            // `error.length()`: UTF-16 code units.
            return Ok(error.encode_utf16().count() as i64);
        }
        if state.cached_size.is_none() {
            match file.get_file_size() {
                Ok(size) => state.cached_size = Some(size),
                Err(e) if e.kind() == io::ErrorKind::NotFound => state.cached_size = Some(0),
                Err(e) => {
                    Self::record_error(&mut state, &e);
                    return Err(e);
                }
            }
        }
        Ok(state.cached_size.unwrap())
    }

    /// Copies the file to `out`, followed by the recorded error text.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:479 FileOutErr.FileRecordingOutputStream.dumpOut
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:324 FileOutErr.NullFileRecordingOutputStream.dumpOut
    pub fn dump_out(&self, out: &dyn OutputSink) {
        let Some(file) = &self.output_file else {
            return;
        };
        let error = {
            let mut state = self.state.lock();
            match file.get_input_stream() {
                Ok(mut input) => {
                    // `in.transferTo(out)`: 8 KiB reads, each written with `write(b, 0, n)`.
                    let mut buffer = vec![0u8; 8192];
                    loop {
                        match input.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => out.write_recording(&buffer[..n]),
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(e) => {
                                Self::record_error(&mut state, &e);
                                break;
                            }
                        }
                    }
                    out.flush_sink();
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => state.cached_size = Some(0),
                Err(e) => Self::record_error(&mut state, &e),
            }
            state.error.clone()
        };
        if let Some(error) = error {
            // `new PrintStream(out).print(error)` in the default charset (ISO-8859-1).
            let bytes = blaze_util::string_encoding::latin1_encode_lossy(&error);
            if !bytes.is_empty() {
                out.write_recording(&bytes);
            }
            out.flush_sink();
        }
    }

    /// `write(b, off, len)`: an error is recorded, not returned.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:499 FileOutErr.FileRecordingOutputStream.write
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:333 FileOutErr.NullFileRecordingOutputStream.write
    fn write_recording_error(&self, bytes: &[u8]) {
        let Some(file) = &self.output_file else {
            return;
        };
        if !bytes.is_empty() {
            let mut state = self.state.lock();
            state.cached_size = None;
            let result = Self::get_output_stream(file, &mut state)
                .and_then(|stream| stream.write_all(bytes));
            if let Err(e) = result {
                Self::record_error(&mut state, &e);
            }
        }
    }

    /// `write(byte[] b)`: an error is returned.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:521 FileOutErr.FileRecordingOutputStream.write
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:341 FileOutErr.NullFileRecordingOutputStream.write
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let Some(file) = &self.output_file else {
            return Ok(());
        };
        if !bytes.is_empty() {
            let mut state = self.state.lock();
            state.cached_size = None;
            Self::get_output_stream(file, &mut state)?.write_all(bytes)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:529 FileOutErr.FileRecordingOutputStream.flush
    pub fn flush(&self) -> io::Result<()> {
        let mut state = self.state.lock();
        if let Some(stream) = state.output_stream.as_mut() {
            stream.flush()?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:536 FileOutErr.FileRecordingOutputStream.close
    pub fn close(&self) -> io::Result<()> {
        Self::close_locked(&mut self.state.lock())
    }

    /// `close()` under the stream's lock: the file stream is flushed and closed; a later write
    /// fails, as on a closed `FileOutputStream`.
    fn close_locked(state: &mut StreamState) -> io::Result<()> {
        if Self::has_output_stream(state)
            && let Some(stream) = state.output_stream.as_mut()
        {
            stream.flush()?;
            state.output_stream = Some(Box::new(ClosedStream));
        }
        Ok(())
    }
}

/// A closed `FileOutputStream`: every write fails with Java's message.
struct ClosedStream;

impl Write for ClosedStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("Stream Closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `FileSystemUtils.readContent(file)`.
fn read_content(file: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.get_input_stream()?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
