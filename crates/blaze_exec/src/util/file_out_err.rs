//! Port of `lib/util/io/FileOutErr.java`: an `OutErr` backed by two files.

use std::io::{self, Read, Write};

use blaze_vfs::Path;

/// Stdout and stderr of a spawn, recorded in files.
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:46 FileOutErr
#[derive(Clone, Debug)]
pub struct FileOutErr {
    output_path: Path,
    error_path: Path,
}

impl FileOutErr {
    /// `new FileOutErr(Path stdout, Path stderr)`.
    pub fn new(stdout: Path, stderr: Path) -> FileOutErr {
        FileOutErr {
            output_path: stdout,
            error_path: stderr,
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
        blaze_protos::java::decode_latin1(&self.out_as_bytes())
    }
    /// `errAsLatin1()`.
    pub fn err_as_latin1(&self) -> String {
        blaze_protos::java::decode_latin1(&self.err_as_bytes())
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
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p.get_path_string())?;
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
