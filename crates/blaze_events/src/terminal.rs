//! Ports of `lib/util/io/AnsiTerminal`, `AnsiTerminalPrinter` and `AnsiTerminalWriter`.
//!
//! PORT: these belong to Bazel's `lib/util/io` package; see the note in [`crate::io`].

use std::io;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::io::OutputStream;

/// An enumeration of all terminal colors, containing the escape sequences for both background and
/// foreground settings. Port of `AnsiTerminal.Color`.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:29 AnsiTerminal.Color
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    Default,
}

impl Color {
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:49 Color.getEscapeSeq
    pub fn get_escape_seq(self) -> &'static [u8] {
        match self {
            Color::Red => b"\x1b[31m",
            Color::Green => b"\x1b[32m",
            Color::Yellow => b"\x1b[33m",
            Color::Blue => b"\x1b[34m",
            Color::Magenta => b"\x1b[35m",
            Color::Cyan => b"\x1b[36m",
            Color::Gray => b"\x1b[37m",
            Color::Default => b"\x1b[0m",
        }
    }

    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:53 Color.getBackgroundEscapeSeq
    pub fn get_background_escape_seq(self) -> &'static [u8] {
        match self {
            Color::Red => b"\x1b[41m",
            Color::Green => b"\x1b[42m",
            Color::Yellow => b"\x1b[43m",
            Color::Blue => b"\x1b[44m",
            Color::Magenta => b"\x1b[45m",
            Color::Cyan => b"\x1b[46m",
            Color::Gray => b"\x1b[47m",
            Color::Default => b"\x1b[0m",
        }
    }
}

// Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:58 AnsiTerminal.ESC
const ESC: &[u8] = &[27, b'['];
const BEL: u8 = 7;
const UP: u8 = b'A';
const ERASE_LINE: u8 = b'K';
const SET_GRAPHICS: u8 = b'm';
const TEXT_BOLD: u8 = b'1';
const SET_TERM_TITLE: &[u8] = &[27, b']', b'0', b';'];

/// A class which encapsulates the fancy curses-type stuff that you can do using standard ANSI
/// terminal control sequences. Port of `AnsiTerminal`.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:24 AnsiTerminal
#[derive(Clone)]
pub struct AnsiTerminal {
    out: Arc<dyn OutputStream>,
}

impl AnsiTerminal {
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:66 AnsiTerminal.CR
    pub const CR: &'static [u8] = &[13];

    /// Creates an AnsiTerminal object wrapping an output stream which is going to be displayed in
    /// an ANSI compatible terminal or shell window.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:76 AnsiTerminal.AnsiTerminal
    pub fn new(out: Arc<dyn OutputStream>) -> Self {
        AnsiTerminal { out }
    }

    /// Moves the cursor upwards by a specified number of lines. This will not cause any scrolling
    /// if it tries to move above the top of the terminal window.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:85 AnsiTerminal.cursorUp
    pub fn cursor_up(&self, num_lines: i32) -> io::Result<()> {
        self.write_all(&[ESC, num_lines.to_string().as_bytes(), &[UP]])
    }

    /// Clear the current terminal line from the cursor position to the end.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:92 AnsiTerminal.clearLine
    pub fn clear_line(&self) -> io::Result<()> {
        self.write_escape_sequence(&[ERASE_LINE])
    }

    /// Makes any text output to the terminal appear in bold.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:99 AnsiTerminal.textBold
    pub fn text_bold(&self) -> io::Result<()> {
        self.write_escape_sequence(&[TEXT_BOLD, SET_GRAPHICS])
    }

    /// Set the color of the foreground of the terminal.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:108 AnsiTerminal.setTextColor
    pub fn set_text_color(&self, color: Color) -> io::Result<()> {
        self.write_bytes(color.get_escape_seq())
    }

    /// Resets the terminal colors and fonts to defaults.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:115 AnsiTerminal.resetTerminal
    pub fn reset_terminal(&self) -> io::Result<()> {
        self.write_escape_sequence(b"0m")
    }

    /// Makes text print on the terminal in red.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:122 AnsiTerminal.textRed
    pub fn text_red(&self) -> io::Result<()> {
        self.set_text_color(Color::Red)
    }

    /// Makes text print on the terminal in green.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:129 AnsiTerminal.textGreen
    pub fn text_green(&self) -> io::Result<()> {
        self.set_text_color(Color::Green)
    }

    /// Makes text print on the terminal in magenta.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:136 AnsiTerminal.textMagenta
    pub fn text_magenta(&self) -> io::Result<()> {
        self.set_text_color(Color::Magenta)
    }

    /// Set the terminal title.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:143 AnsiTerminal.setTitle
    pub fn set_title(&self, title: &str) -> io::Result<()> {
        self.write_all(&[SET_TERM_TITLE, title.as_bytes(), &[BEL]])
    }

    /// Writes a string to the terminal using the current font, color and cursor position
    /// settings.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:153 AnsiTerminal.writeString
    pub fn write_string(&self, text: &str) -> io::Result<()> {
        // PORT: StringEncoding identity (Java's getBytes() of an internal Latin-1 string).
        self.out.write(text.as_bytes())
    }

    /// Writes a byte sequence to the terminal using the current font, color and cursor position
    /// settings.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:163 AnsiTerminal.writeBytes
    pub fn write_bytes(&self, bytes: &[u8]) -> io::Result<()> {
        self.out.write(bytes)
    }

    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:174 AnsiTerminal.writeBytes
    fn write_all(&self, stuff: &[&[u8]]) -> io::Result<()> {
        for bytes in stuff {
            self.out.write(bytes)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:186 AnsiTerminal.writeEscapeSequence
    fn write_escape_sequence(&self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(&[ESC, bytes])
    }

    /// Sends a carriage return to the terminal.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:191 AnsiTerminal.cr
    pub fn cr(&self) -> io::Result<()> {
        self.write_bytes(AnsiTerminal::CR)
    }

    /// Flushes the underlying stream. This class does not do any buffering of its own, but the
    /// underlying OutputStream may.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminal.java:199 AnsiTerminal.flush
    pub fn flush(&self) -> io::Result<()> {
        self.out.flush()
    }
}

/// An append-only interface to access to a terminal. Port of `AnsiTerminalWriter`.
///
/// This interface allows to specify a text to be written to the user using semantical
/// highlighting (like failure) without any knowledge about the nature of the terminal or the
/// position the text is to be written to. It is the seam the progress bar (`UiStateTracker`)
/// writes through; its implementations (line counting, line wrapping, logging) come with the
/// progress bar.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalWriter.java:32 AnsiTerminalWriter
pub trait AnsiTerminalWriter {
    /// Write some text to the user
    fn append(&mut self, text: &str) -> io::Result<&mut dyn AnsiTerminalWriter>;
    /// Start a new line in the way appropriate for the given terminal
    fn newline(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter>;
    /// Tell the terminal that the following text will be a positive status message.
    fn ok_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter>;
    /// Tell the terminal that the following text will be an error-reporting status message.
    fn fail_status(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter>;
    /// Tell the terminal that the following text will be normal text, not indicating a status or
    /// similar.
    fn normal(&mut self) -> io::Result<&mut dyn AnsiTerminalWriter>;
}

/// List of supported coloring modes for the [`AnsiTerminalPrinter`]. Port of
/// `AnsiTerminalPrinter.Mode`.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:54 AnsiTerminalPrinter.Mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// green
    Info,
    /// bold red
    Error,
    /// magenta
    Warning,
    /// default color
    Default,
}

// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:44 AnsiTerminalPrinter.MODE_PREFIX
const MODE_PREFIX: &str = "{#";
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:45 AnsiTerminalPrinter.MODE_SUFFIX
const MODE_SUFFIX: &str = "#}";

impl Mode {
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:67 AnsiTerminalPrinter.MODES
    const MODES: [Mode; 4] = [Mode::Info, Mode::Error, Mode::Warning, Mode::Default];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Info => "INFO",
            Mode::Error => "ERROR",
            Mode::Warning => "WARNING",
            Mode::Default => "DEFAULT",
        }
    }
}

/// The keyword inserted into printed strings, e.g. `{#INFO#}`.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:61 Mode.toString
impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{MODE_PREFIX}{}{MODE_SUFFIX}", self.name())
    }
}

/// Java `PATTERN.split(str)` for `MODE_PATTERN = "\{\#(?=[A-Z]+\#\})"`: splits before each
/// `{#` that is followed by one or more capitals and `#}`, consuming the `{#`. As with
/// `Pattern.split`, trailing empty strings are dropped, a leading empty string is kept (the match
/// is not zero-width), and an input without matches is returned whole.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:49 AnsiTerminalPrinter.MODE_PATTERN
fn split_modes(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut matched = false;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'#' {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_uppercase() {
                j += 1;
            }
            if j > i + 2 && bytes[j..].starts_with(b"#}") {
                parts.push(&s[start..i]);
                matched = true;
                start = i + 2;
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    if !matched {
        return vec![s];
    }
    parts.push(&s[start..]);
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

struct PrinterState {
    use_color: bool,
    last_mode: Mode,
}

/// Allows to print "colored" strings by parsing predefined string keywords, which, depending on
/// the useColor value are either replaced with ANSI terminal coloring sequences (as defined by
/// the [`AnsiTerminal`] class) or stripped. Port of `AnsiTerminalPrinter`.
///
/// Following keywords are supported:
///   INFO  - switches color to green.
///   ERROR - switches color to bold red.
///   WARNING - switches color to magenta.
///   NORMAL - resets terminal to the default state.
///
/// Each keyword is starts with prefix "{#" followed by the enum constant name and suffix "#}".
/// Keywords should not be inserted manually - provided enum constants should be used instead.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:42 AnsiTerminalPrinter
pub struct AnsiTerminalPrinter {
    stream: Arc<dyn OutputStream>,
    terminal: AnsiTerminal,
    state: Mutex<PrinterState>,
}

impl AnsiTerminalPrinter {
    /// Creates new instance using provided OutputStream and sets coloring logic for that
    /// instance.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:80 AnsiTerminalPrinter.AnsiTerminalPrinter
    pub fn new(out: Arc<dyn OutputStream>, use_color: bool) -> Self {
        AnsiTerminalPrinter {
            terminal: AnsiTerminal::new(out.clone()),
            stream: out,
            state: Mutex::new(PrinterState {
                use_color,
                last_mode: Mode::Default,
            }),
        }
    }

    /// Writes the specified string to the output stream while injecting coloring sequences when
    /// appropriate mode keyword is found and flushes.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:95 AnsiTerminalPrinter.print
    pub fn print(&self, str: &str) {
        let mut st = self.state.lock();
        for part in split_modes(str) {
            let mut part = part;
            // Mode name will contain at least one character, so suffix index
            // must be at least 1. If it isn't then there is no match.
            if let Some(index) = part.find(MODE_SUFFIX).filter(|&index| index > 1) {
                for mode in Mode::MODES {
                    if index == mode.name().len() && part.starts_with(mode.name()) {
                        self.setup_terminal(&mut st, mode);
                        part = &part[index + MODE_SUFFIX.len()..];
                        break;
                    }
                }
            }
            // PORT: PrintWriter swallows IOExceptions; so do we. StringEncoding identity.
            let _ = self.stream.write(part.as_bytes());
            let _ = self.stream.flush();
        }
    }

    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:114 AnsiTerminalPrinter.printLn
    pub fn print_ln(&self, str: &str) {
        self.print(&format!("{str}\n"));
    }

    /// Returns the underlying OutputStream.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:121 AnsiTerminalPrinter.getOutputStream
    pub fn get_output_stream(&self) -> &Arc<dyn OutputStream> {
        &self.stream
    }

    /// Injects coloring escape sequences if output should be colored and mode has been changed.
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:129 AnsiTerminalPrinter.setupTerminal
    fn setup_terminal(&self, st: &mut PrinterState, mode: Mode) {
        if !st.use_color {
            return;
        }
        let result = (|| -> io::Result<()> {
            if st.last_mode != mode {
                self.terminal.reset_terminal()?;
                st.last_mode = mode;
                match mode {
                    Mode::Default => return Ok(()), // Terminal is already reset - nothing else to do.
                    Mode::Info => self.terminal.text_green()?,
                    Mode::Error => {
                        self.terminal.text_red()?;
                        self.terminal.text_bold()?;
                    }
                    Mode::Warning => self.terminal.text_magenta()?,
                }
            }
            Ok(())
        })();
        if result.is_err() {
            // AnsiTerminal state is now considered to be inconsistent - coloring
            // should be disabled to prevent future use of AnsiTerminal instance.
            // PORT: Java logs "Disabling coloring due to exception" at WARNING; logging is not
            // ported.
            st.use_color = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ByteArrayOutputStream;

    fn printer(use_color: bool) -> (Arc<ByteArrayOutputStream>, AnsiTerminalPrinter) {
        let stream = Arc::new(ByteArrayOutputStream::new());
        let printer = AnsiTerminalPrinter::new(stream.clone(), use_color);
        (stream, printer)
    }

    fn contents(stream: &ByteArrayOutputStream) -> String {
        String::from_utf8(stream.to_byte_array()).unwrap()
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/AnsiTerminalPrinterTest.java AnsiTerminalPrinterTest.testPlainPrinter
    #[test]
    fn test_plain_printer() {
        let (stream, printer) = printer(false);
        printer.print(&format!(
            "1{}2{}3{}4{}5",
            Mode::Info,
            Mode::Error,
            Mode::Warning,
            Mode::Default
        ));
        assert_eq!(contents(&stream), "12345");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/AnsiTerminalPrinterTest.java AnsiTerminalPrinterTest.testDefaultModeIsDefault
    #[test]
    fn test_default_mode_is_default() {
        let (stream, printer) = printer(true);
        printer.print(&format!("1{}2", Mode::Default));
        assert_eq!(contents(&stream), "12");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/AnsiTerminalPrinterTest.java AnsiTerminalPrinterTest.testDuplicateMode
    #[test]
    fn test_duplicate_mode() {
        let (stream, printer) = printer(true);
        printer.print(&format!("_A_{}", Mode::Info));
        printer.print(&format!("_B_{}_C_", Mode::Info));
        let re = regex::Regex::new("(?s)^_A_.+_B__C_$").unwrap();
        assert!(re.is_match(&contents(&stream)), "{:?}", contents(&stream));
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/io/AnsiTerminalPrinterTest.java AnsiTerminalPrinterTest.testModeCodes
    #[test]
    fn test_mode_codes() {
        let (stream, printer) = printer(true);
        printer.print(&format!(
            "{}XXX{}XXX{}XXX{}XXX{}XXX{}XXX{}XXX{}",
            Mode::Info,
            Mode::Error,
            Mode::Warning,
            Mode::Default,
            Mode::Info,
            Mode::Error,
            Mode::Warning,
            Mode::Default
        ));
        let output = contents(&stream);
        let codes: Vec<&str> = output.split("XXX").collect();
        assert_eq!(codes.len(), 8);
        for i in 0..4 {
            assert!(!codes[i].is_empty());
            assert_eq!(codes[i + 4], codes[i]);
        }
        assert_ne!(codes[0], codes[1]);
        assert_ne!(codes[0], codes[2]);
        assert_ne!(codes[0], codes[3]);
        assert_ne!(codes[1], codes[2]);
        assert_ne!(codes[1], codes[3]);
        assert_ne!(codes[2], codes[3]);
    }

    #[test]
    fn split_modes_matches_java_split() {
        assert_eq!(split_modes("a{#INFO#}b"), vec!["a", "INFO#}b"]);
        assert_eq!(split_modes("{#INFO#}b"), vec!["", "INFO#}b"]);
        assert_eq!(split_modes("a{#INFO#}"), vec!["a", "INFO#}"]);
        assert_eq!(split_modes("a{#x#}b"), vec!["a{#x#}b"]);
        assert_eq!(split_modes("a{#"), vec!["a{#"]);
        assert_eq!(split_modes(""), vec![""]);
    }
}
