//! Port of `lib/util/io/FileOutErr.java`: an `OutErr` backed by two files.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_vfs::Path;

/// Stdout and stderr of a spawn, recorded in files.
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:46 FileOutErr
///
/// PORT: a clone is the same Java object (it shares `childCount`).
#[derive(Clone, Debug)]
pub struct FileOutErr {
    output_path: Path,
    error_path: Path,
    /// `childCount`.
    child_count: Arc<AtomicI32>,
}

impl FileOutErr {
    /// `new FileOutErr(Path stdout, Path stderr)`.
    pub fn new(stdout: Path, stderr: Path) -> FileOutErr {
        FileOutErr {
            output_path: stdout,
            error_path: stderr,
            child_count: Arc::new(AtomicI32::new(0)),
        }
    }

    /// `childOutErr()`: a `FileOutErr` recording into `<stdout>-<n>` / `<stderr>-<n>`, `n`
    /// counting the children of this one from 0.
    ///
    /// PORT: the `outPath == null || errPath == null` branch (a `FileOutErr` not backed by
    /// files) does not exist here: this `FileOutErr` always has both files.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:232 FileOutErr.childOutErr
    pub fn child_out_err(&self) -> FileOutErr {
        let index = self.child_count.fetch_add(1, Ordering::SeqCst);
        let out_path = &self.output_path;
        let err_path = &self.error_path;
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
    ///
    /// PORT: Java streams the files and never throws: a failure to read or write is recorded in
    /// the stream (and printed with its output later); this `FileOutErr` has no recorded error,
    /// so such a failure is dropped. Like Java, nothing is written (and the target file is not
    /// created) when there is nothing to copy.
    // Java: com/google/devtools/build/lib/util/io/FileOutErr.java:218 FileOutErr.dump
    pub fn dump(from: &FileOutErr, to: &FileOutErr) {
        // `from.dumpOutAsLatin1(to.getOutputStream())`
        let out = from.out_as_bytes();
        if !out.is_empty() {
            let _ = to.write_out(&out);
        }
        // `from.dumpErrAsLatin1(to.getErrorStream())`
        let err = from.err_as_bytes();
        if !err.is_empty() {
            let _ = to.write_err(&err);
        }
    }

    /// `getOutputPath()`.
    pub fn get_output_path(&self) -> &Path {
        &self.output_path
    }

    /// `getErrorPath()`.
    pub fn get_error_path(&self) -> &Path {
        &self.error_path
    }

    fn has_content(p: &Path) -> bool {
        p.get_file_size().map(|s| s > 0).unwrap_or(false)
    }

    /// `hasRecordedOutput()`.
    pub fn has_recorded_output(&self) -> bool {
        self.has_recorded_stdout() || self.has_recorded_stderr()
    }
    /// `hasRecordedStdout()`.
    pub fn has_recorded_stdout(&self) -> bool {
        FileOutErr::has_content(&self.output_path)
    }
    /// `hasRecordedStderr()`.
    pub fn has_recorded_stderr(&self) -> bool {
        FileOutErr::has_content(&self.error_path)
    }

    fn read(p: &Path) -> Vec<u8> {
        let mut buf = Vec::new();
        if let Ok(mut s) = p.get_input_stream() {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    }

    /// `outAsBytes()`.
    pub fn out_as_bytes(&self) -> Vec<u8> {
        FileOutErr::read(&self.output_path)
    }
    /// `errAsBytes()`.
    pub fn err_as_bytes(&self) -> Vec<u8> {
        FileOutErr::read(&self.error_path)
    }
    /// `outAsLatin1()`.
    pub fn out_as_latin1(&self) -> String {
        blaze_util::string_encoding::latin1_decode(&self.out_as_bytes()).into_owned()
    }
    /// `errAsLatin1()`.
    pub fn err_as_latin1(&self) -> String {
        blaze_util::string_encoding::latin1_decode(&self.err_as_bytes()).into_owned()
    }

    /// `getErrorStream().write(bytes)`: appends to the stderr file.
    pub fn write_err(&self, bytes: &[u8]) -> io::Result<()> {
        self.append(&self.error_path, bytes)
    }

    /// `getOutputStream().write(bytes)`: appends to the stdout file.
    pub fn write_out(&self, bytes: &[u8]) -> io::Result<()> {
        self.append(&self.output_path, bytes)
    }

    fn append(&self, p: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = p.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(
            blaze_util::string_encoding::internal_to_platform(p.get_path_string()),
        )?;
        f.write_all(bytes)?;
        f.flush()
    }

    /// `clearOut()`.
    pub fn clear_out(&self) -> io::Result<()> {
        self.output_path.delete().map(|_| ())
    }
    /// `clearErr()`.
    pub fn clear_err(&self) -> io::Result<()> {
        self.error_path.delete().map(|_| ())
    }
}
