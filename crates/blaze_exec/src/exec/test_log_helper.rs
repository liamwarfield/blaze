//! Port of `exec/TestLogHelper.java`: a helper for test log handling. It determines whether the
//! test log should be output and formats the test log for console display.

use std::io::{self, Read, Write};

use blaze_optdefs::command::execution_options::TestOutputFormat;
use blaze_vfs::Path;

// Java: com/google/devtools/build/lib/exec/TestLogHelper.java:33 TestLogHelper.HEADER_DELIMITER
pub const HEADER_DELIMITER: &str =
    "-----------------------------------------------------------------------------";

/// Determines whether the test log should be output from the current outputMode and whether the
/// test has passed or not.
// Java: com/google/devtools/build/lib/exec/TestLogHelper.java:40 TestLogHelper.shouldOutputTestLog
pub fn should_output_test_log(output_mode: TestOutputFormat, has_passed: bool) -> bool {
    (output_mode == TestOutputFormat::ALL)
        || (!has_passed && (output_mode == TestOutputFormat::ERRORS))
}

/// Streams the contents of testOutput file to the provided output, adding a new header and
/// footer. The internal test header is elided. The test output is not emitted if its size is
/// greater than the provided threshold.
///
/// `max_test_output_bytes`: Maximum test log size, including header, to output. Negative implies
/// no limit.
// Java: com/google/devtools/build/lib/exec/TestLogHelper.java:53 TestLogHelper.writeTestLog
pub fn write_test_log(
    test_output: &Path,
    test_name: &str,
    out: &mut dyn Write,
    max_test_output_bytes: i32,
) -> io::Result<()> {
    let result = (|| {
        // `new PrintStream(out)`: the default charset, ISO-8859-1.
        out.write_all(&blaze_util::string_encoding::latin1_encode_lossy(&format!(
            "==================== Test output for {test_name}:\n"
        )))?;
        out.flush()?;

        if max_test_output_bytes < 0 {
            // No limit, print it all.
            stream_test_log(test_output, out)?;
        } else {
            let test_output_bytes = test_output.get_file_size()?;
            if test_output_bytes <= max_test_output_bytes as i64 {
                stream_test_log(test_output, out)?;
            } else {
                writeln!(
                    out,
                    "Test log too large ({test_output_bytes} > {max_test_output_bytes}), skipping..."
                )?;
            }
        }

        out.write_all(
            b"================================================================================\n",
        )
    })();
    // finally
    out.flush()?;
    result
}

/// Use this class to filter the streaming output of a test until we see the header delimiter.
// Java: com/google/devtools/build/lib/exec/TestLogHelper.java:94 TestLogHelper.FilterTestHeaderOutputStream
pub struct FilterTestHeaderOutputStream<'a> {
    out: &'a mut dyn Write,
    seen_delimiter: bool,
    line_builder: String,
}

impl<'a> FilterTestHeaderOutputStream<'a> {
    /// Returns an output stream that doesn't write to original until it sees HEADER_DELIMITER by
    /// itself on a line.
    // Java: com/google/devtools/build/lib/exec/TestLogHelper.java:85 TestLogHelper.getHeaderFilteringOutputStream
    pub fn new(out: &'a mut dyn Write) -> Self {
        FilterTestHeaderOutputStream {
            out,
            seen_delimiter: false,
            line_builder: String::new(),
        }
    }

    // Java: com/google/devtools/build/lib/exec/TestLogHelper.java:106 TestLogHelper.FilterTestHeaderOutputStream.write
    fn write_byte(&mut self, b: u8) -> io::Result<()> {
        if self.seen_delimiter {
            self.out.write_all(&[b])?;
        } else if b == b'\n' {
            let line = std::mem::take(&mut self.line_builder);
            if line == HEADER_DELIMITER {
                self.seen_delimiter = true;
            }
        } else if self.line_builder.chars().count() <= HEADER_DELIMITER.len() {
            // (char) b: Latin-1.
            self.line_builder.push(b as char);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/TestLogHelper.java:129 TestLogHelper.FilterTestHeaderOutputStream.foundHeader
    pub fn found_header(&self) -> bool {
        self.seen_delimiter
    }
}

impl Write for FilterTestHeaderOutputStream<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.seen_delimiter {
            self.out.write_all(buf)?;
        } else {
            for &b in buf {
                self.write_byte(b)?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

// Java: com/google/devtools/build/lib/exec/TestLogHelper.java:63 TestLogHelper.streamTestLog
fn stream_test_log(from_path: &Path, out: &mut dyn Write) -> io::Result<()> {
    let found_header = {
        let mut filtering_output_stream = FilterTestHeaderOutputStream::new(out);
        let mut input = from_path.get_input_stream()?;
        io::copy(&mut input, &mut filtering_output_stream)?;
        filtering_output_stream.found_header()
    };

    if !found_header {
        let mut input_again = from_path.get_input_stream()?;
        let mut buf = Vec::new();
        input_again.read_to_end(&mut buf)?;
        out.write_all(&buf)?;
    }
    Ok(())
}
