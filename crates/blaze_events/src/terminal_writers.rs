//! The `AnsiTerminalWriter` implementations of `lib/util/io` the progress bar writes through:
//! `PositionAwareAnsiTerminalWriter`, `LineCountingAnsiTerminalWriter`,
//! `LineWrappingAnsiTerminalWriter` and `LoggingTerminalWriter`.

use std::io;

use crate::terminal::{AnsiTerminal, AnsiTerminalWriter};

/// Wrap an `AnsiTerminalWriter` into one that is aware of the position within the current line.
/// Newline characters, written separately or as part of a string, are recognized and
/// translated into the newline operation of the underlying writer.
// Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:24 PositionAwareAnsiTerminalWriter
pub struct PositionAwareAnsiTerminalWriter<'a> {
    terminal_writer: &'a mut dyn AnsiTerminalWriter,
    position: i32,
}

impl<'a> PositionAwareAnsiTerminalWriter<'a> {
    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:29 PositionAwareAnsiTerminalWriter.<init>
    pub fn new(terminal_writer: &'a mut dyn AnsiTerminalWriter) -> Self {
        PositionAwareAnsiTerminalWriter {
            terminal_writer,
            position: 0,
        }
    }

    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:84 PositionAwareAnsiTerminalWriter.getPosition
    pub fn get_position(&self) -> i32 {
        self.position
    }
}

impl AnsiTerminalWriter for PositionAwareAnsiTerminalWriter<'_> {
    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:36 PositionAwareAnsiTerminalWriter.append
    fn append(&mut self, text: &str) -> io::Result<&mut dyn AnsiTerminalWriter> {
        // Java's String indices count chars.
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i..].iter().position(|c| *c == '\n') {
                None => {
                    let rest: String = chars[i..].iter().collect();
                    self.terminal_writer.append(&rest)?;
                    self.position += (chars.len() - i) as i32;
                    i = chars.len();
                }
                Some(offset) => {
                    let next = i + offset;
                    let line: String = chars[i..next].iter().collect();
                    self.terminal_writer.append(&line)?;
                    self.terminal_writer.newline()?;
                    i = next + 1;
                    self.position = 0;
                }
            }
        }
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:56 PositionAwareAnsiTerminalWriter.newline
    fn newline(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.newline()?;
        self.position = 0;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:63 PositionAwareAnsiTerminalWriter.okStatus
    fn ok_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.ok_status()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:70 PositionAwareAnsiTerminalWriter.failStatus
    fn fail_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.fail_status()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriter.java:77 PositionAwareAnsiTerminalWriter.normal
    fn normal(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.normal()?;
        Ok(self)
    }
}

/// An `AnsiTerminalWriter` for an `AnsiTerminal` that counts the number of lines written.
// Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:22 LineCountingAnsiTerminalWriter
pub struct LineCountingAnsiTerminalWriter<'a> {
    terminal: &'a AnsiTerminal,
    line_count: i32,
}

impl<'a> LineCountingAnsiTerminalWriter<'a> {
    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:27 LineCountingAnsiTerminalWriter.<init>
    pub fn new(terminal: &'a AnsiTerminal) -> Self {
        LineCountingAnsiTerminalWriter {
            terminal,
            line_count: 0,
        }
    }

    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:67 LineCountingAnsiTerminalWriter.getWrittenLines
    pub fn get_written_lines(&self) -> i32 {
        self.line_count
    }
}

impl AnsiTerminalWriter for LineCountingAnsiTerminalWriter<'_> {
    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:34 LineCountingAnsiTerminalWriter.append
    fn append(&mut self, text: &str) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal.write_string(text)?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:41 LineCountingAnsiTerminalWriter.newline
    fn newline(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        // System.lineSeparator() on Linux.
        self.terminal.write_string("\n")?;
        self.line_count += 1;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:49 LineCountingAnsiTerminalWriter.okStatus
    fn ok_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal.text_green()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:55 LineCountingAnsiTerminalWriter.failStatus
    fn fail_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal.text_red()?;
        self.terminal.text_bold()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineCountingAnsiTerminalWriter.java:62 LineCountingAnsiTerminalWriter.normal
    fn normal(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal.reset_terminal()?;
        Ok(self)
    }
}

/// Wrap an `AnsiTerminalWriter` into one that breaks lines to use at most a first given number
/// of columns of the terminal. In this way, all line breaks are predictable, even if we only
/// have a lower bound on the number of columns of the underlying terminal. To simplify copy and
/// paste of the terminal output, a continuation character is written into the last usable
/// column when we break a line.
// Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:26 LineWrappingAnsiTerminalWriter
pub struct LineWrappingAnsiTerminalWriter<'a> {
    terminal_writer: &'a mut dyn AnsiTerminalWriter,
    width: i32,
    continuation_character: char,
    position: i32,
}

impl<'a> LineWrappingAnsiTerminalWriter<'a> {
    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:33 LineWrappingAnsiTerminalWriter.<init>
    pub fn with_continuation_character(
        terminal_writer: &'a mut dyn AnsiTerminalWriter,
        width: i32,
        continuation_character: char,
    ) -> Self {
        LineWrappingAnsiTerminalWriter {
            terminal_writer,
            width,
            continuation_character,
            position: 0,
        }
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:41 LineWrappingAnsiTerminalWriter.<init>
    pub fn new(terminal_writer: &'a mut dyn AnsiTerminalWriter, width: i32) -> Self {
        Self::with_continuation_character(terminal_writer, width, '\\')
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:45 LineWrappingAnsiTerminalWriter.appendChar
    fn append_char(&mut self, c: char) -> io::Result<()> {
        if c == '\n' {
            self.terminal_writer.newline()?;
            self.position = 0;
        } else if self.position + 1 < self.width {
            self.terminal_writer.append(&c.to_string())?;
            self.position += 1;
        } else {
            // The last usable character of the line was already been written,
            // hence we have to start a continuation before writing the symbol.
            self.terminal_writer
                .append(&self.continuation_character.to_string())?;
            self.terminal_writer.newline()?;
            self.terminal_writer.append(&c.to_string())?;
            self.position = 1;
        }
        Ok(())
    }
}

impl AnsiTerminalWriter for LineWrappingAnsiTerminalWriter<'_> {
    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:62 LineWrappingAnsiTerminalWriter.append
    fn append(&mut self, text: &str) -> io::Result<&mut dyn AnsiTerminalWriter> {
        for c in text.chars() {
            self.append_char(c)?;
        }
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:70 LineWrappingAnsiTerminalWriter.newline
    fn newline(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.newline()?;
        self.position = 0;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:77 LineWrappingAnsiTerminalWriter.okStatus
    fn ok_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.ok_status()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:84 LineWrappingAnsiTerminalWriter.failStatus
    fn fail_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.fail_status()?;
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriter.java:91 LineWrappingAnsiTerminalWriter.normal
    fn normal(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.terminal_writer.normal()?;
        Ok(self)
    }
}

/// An `AnsiTerminalWriter` that records the text written, with the highlighting calls as
/// markers.
// Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:22 LoggingTerminalWriter
#[derive(Default)]
pub struct LoggingTerminalWriter {
    transcript: String,
    discard_highlight: bool,
}

impl LoggingTerminalWriter {
    // Strings for recording the non-append calls
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:24 LoggingTerminalWriter.NEWLINE
    pub const NEWLINE: &'static str = "[NL]";
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:25 LoggingTerminalWriter.OK
    pub const OK: &'static str = "[OK]";
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:26 LoggingTerminalWriter.FAIL
    pub const FAIL: &'static str = "[FAIL]";
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:27 LoggingTerminalWriter.NORMAL
    pub const NORMAL: &'static str = "[NORMAL]";

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:32 LoggingTerminalWriter.<init>
    pub fn new(discard_highlight: bool) -> Self {
        LoggingTerminalWriter {
            transcript: String::new(),
            discard_highlight,
        }
    }

    /// Clears the stored transcript; mostly useful for testing purposes.
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:42 LoggingTerminalWriter.reset
    pub fn reset(&mut self) {
        self.transcript.clear();
    }

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:88 LoggingTerminalWriter.getTranscript
    pub fn get_transcript(&self) -> &str {
        &self.transcript
    }
}

impl AnsiTerminalWriter for LoggingTerminalWriter {
    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:48 LoggingTerminalWriter.append
    fn append(&mut self, text: &str) -> io::Result<&mut dyn AnsiTerminalWriter> {
        self.transcript.push_str(text);
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:55 LoggingTerminalWriter.newline
    fn newline(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        if !self.discard_highlight {
            self.transcript.push_str(Self::NEWLINE);
        } else {
            self.transcript.push('\n');
        }
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:66 LoggingTerminalWriter.okStatus
    fn ok_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        if !self.discard_highlight {
            self.transcript.push_str(Self::OK);
        }
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:74 LoggingTerminalWriter.failStatus
    fn fail_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        if !self.discard_highlight {
            self.transcript.push_str(Self::FAIL);
        }
        Ok(self)
    }

    // Java: com/google/devtools/build/lib/util/io/LoggingTerminalWriter.java:82 LoggingTerminalWriter.normal
    fn normal(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter> {
        if !self.discard_highlight {
            self.transcript.push_str(Self::NORMAL);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NL: &str = LoggingTerminalWriter::NEWLINE;
    const OK: &str = LoggingTerminalWriter::OK;
    const FAIL: &str = LoggingTerminalWriter::FAIL;
    const NORMAL: &str = LoggingTerminalWriter::NORMAL;

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testSimpleLineWrapping
    #[test]
    fn test_simple_line_wrapping() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 5, '+')
            .append("abcdefghij")
            .unwrap();
        assert_eq!(terminal.get_transcript(), format!("abcd+{NL}efgh+{NL}ij"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testAlwaysWrap
    #[test]
    fn test_always_wrap() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 5, '+')
            .append("12345")
            .unwrap()
            .newline()
            .unwrap();
        assert_eq!(terminal.get_transcript(), format!("1234+{NL}5{NL}"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testWrapLate
    #[test]
    fn test_wrap_late() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 5, '+')
            .append("1234")
            .unwrap();
        // Lines are only wrapped, once a character is written that cannot fit in the current
        // line, and not already once the last usable character of a line is used. Hence, in this
        // example, we do not want to see the continuation character.
        assert_eq!(terminal.get_transcript(), "1234");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testNewlineTranslated
    #[test]
    fn test_newline_translated() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 80, '+')
            .append("foo\nbar\n")
            .unwrap();
        assert_eq!(terminal.get_transcript(), format!("foo{NL}bar{NL}"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testNewlineResetsCount
    #[test]
    fn test_newline_resets_count() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 5, '+')
            .append("123")
            .unwrap()
            .newline()
            .unwrap()
            .append("abc")
            .unwrap()
            .newline()
            .unwrap()
            .append("ABC\nABC")
            .unwrap()
            .newline()
            .unwrap();
        assert_eq!(
            terminal.get_transcript(),
            format!("123{NL}abc{NL}ABC{NL}ABC{NL}")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/LineWrappingAnsiTerminalWriterTest.java LineWrappingAnsiTerminalWriterTest.testEventsPassedThrough
    #[test]
    fn test_events_passed_through() {
        let mut terminal = LoggingTerminalWriter::default();
        LineWrappingAnsiTerminalWriter::with_continuation_character(&mut terminal, 80, '+')
            .ok_status()
            .unwrap()
            .append("ok")
            .unwrap()
            .fail_status()
            .unwrap()
            .append("fail")
            .unwrap()
            .normal()
            .unwrap()
            .append("normal")
            .unwrap();
        assert_eq!(
            terminal.get_transcript(),
            format!("{OK}ok{FAIL}fail{NORMAL}normal")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriterTest.java PositionAwareAnsiTerminalWriterTest.positionSimple
    #[test]
    fn position_simple() {
        let sample = "lorem ipsum...";
        let mut logging_terminal_writer = LoggingTerminalWriter::default();
        let mut terminal_writer =
            PositionAwareAnsiTerminalWriter::new(&mut logging_terminal_writer);
        terminal_writer.append(sample).unwrap();
        assert_eq!(terminal_writer.get_position(), sample.len() as i32);
        assert_eq!(logging_terminal_writer.get_transcript(), sample);
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriterTest.java PositionAwareAnsiTerminalWriterTest.positionTwoLines
    #[test]
    fn position_two_lines() {
        let first_line = "lorem ipsum...";
        let second_line = "foo bar baz";
        let mut logging_terminal_writer = LoggingTerminalWriter::default();
        let mut terminal_writer =
            PositionAwareAnsiTerminalWriter::new(&mut logging_terminal_writer);
        terminal_writer.append(first_line).unwrap();
        assert_eq!(terminal_writer.get_position(), first_line.len() as i32);
        terminal_writer.newline().unwrap();
        assert_eq!(terminal_writer.get_position(), 0);
        terminal_writer.append(second_line).unwrap();
        assert_eq!(terminal_writer.get_position(), second_line.len() as i32);
        terminal_writer.newline().unwrap();
        assert_eq!(terminal_writer.get_position(), 0);
        assert_eq!(
            logging_terminal_writer.get_transcript(),
            format!("{first_line}{NL}{second_line}{NL}")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriterTest.java PositionAwareAnsiTerminalWriterTest.positionNewlineTranslated
    #[test]
    fn position_newline_translated() {
        let first_line = "lorem ipsum...";
        let second_line = "foo bar baz";
        let mut logging_terminal_writer = LoggingTerminalWriter::default();
        let mut terminal_writer =
            PositionAwareAnsiTerminalWriter::new(&mut logging_terminal_writer);
        terminal_writer
            .append(&format!("{first_line}\n{second_line}"))
            .unwrap();
        assert_eq!(terminal_writer.get_position(), second_line.len() as i32);
        terminal_writer.append("\n").unwrap();
        assert_eq!(terminal_writer.get_position(), 0);
        assert_eq!(
            logging_terminal_writer.get_transcript(),
            format!("{first_line}{NL}{second_line}{NL}")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriterTest.java PositionAwareAnsiTerminalWriterTest.passThrough
    #[test]
    fn pass_through() {
        let mut logging_terminal_writer = LoggingTerminalWriter::default();
        let mut terminal_writer =
            PositionAwareAnsiTerminalWriter::new(&mut logging_terminal_writer);
        terminal_writer
            .append("abc")
            .unwrap()
            .ok_status()
            .unwrap()
            .append("ok")
            .unwrap()
            .fail_status()
            .unwrap()
            .append("fail")
            .unwrap()
            .normal()
            .unwrap()
            .append("normal")
            .unwrap();
        assert_eq!(
            logging_terminal_writer.get_transcript(),
            format!("abc{OK}ok{FAIL}fail{NORMAL}normal")
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/PositionAwareAnsiTerminalWriterTest.java PositionAwareAnsiTerminalWriterTest.highlightNospace
    #[test]
    fn highlight_nospace() {
        let sample = "lorem ipsum...";
        let mut logging_terminal_writer = LoggingTerminalWriter::default();
        let mut terminal_writer =
            PositionAwareAnsiTerminalWriter::new(&mut logging_terminal_writer);
        terminal_writer.fail_status().unwrap();
        assert_eq!(terminal_writer.get_position(), 0);
        terminal_writer.append(sample).unwrap();
        assert_eq!(terminal_writer.get_position(), sample.len() as i32);
        assert_eq!(
            logging_terminal_writer.get_transcript(),
            format!("{FAIL}{sample}")
        );
    }
}
