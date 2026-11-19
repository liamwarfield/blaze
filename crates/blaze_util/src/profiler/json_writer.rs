//! The part of Gson's `com.google.gson.stream.JsonWriter` (2.11.0, the version Bazel 9.3.2 pins in
//! MODULE.bazel) that the profiler's trace writer uses: objects, arrays, names, string, long and
//! double values, and `setIndent` switched between values.
//!
//! PORT: Gson is a third-party library, not under `src/main/java`; the markers name its source
//! file. Bytes leave through the `OutputStreamWriter(..., ISO_8859_1)` Bazel wraps around the
//! stream: one byte per char of an internal string, `?` for a char above U+00FF.

use std::io::{self, Write};

const EMPTY_ARRAY: u8 = 1;
const NONEMPTY_ARRAY: u8 = 2;
const EMPTY_OBJECT: u8 = 3;
const DANGLING_NAME: u8 = 4;
const NONEMPTY_OBJECT: u8 = 5;
const EMPTY_DOCUMENT: u8 = 6;
const NONEMPTY_DOCUMENT: u8 = 7;
const CLOSED: u8 = 8;

/// Writes a JSON encoded value to a stream, one token at a time.
// Java: com/google/gson/stream/JsonWriter.java:142 JsonWriter (gson 2.11.0)
pub struct JsonWriter<W: Write> {
    out: W,
    stack: Vec<u8>,
    /// `formattingStyle.getIndent()`; the newline is "\n" when the indent is not empty.
    indent: String,
    formatted_colon: &'static str,
    uses_empty_newline_and_indent: bool,
    deferred_name: Option<String>,
}

impl<W: Write> JsonWriter<W> {
    // Java: com/google/gson/stream/JsonWriter.java:224 JsonWriter.<init>
    pub fn new(out: W) -> Self {
        let mut w = JsonWriter {
            out,
            stack: vec![EMPTY_DOCUMENT],
            indent: String::new(),
            formatted_colon: ":",
            uses_empty_newline_and_indent: true,
            deferred_name: None,
        };
        // FormattingStyle.COMPACT.
        w.set_indent("");
        w
    }

    /// `setIndent`: an empty indent is `FormattingStyle.COMPACT`, any other is
    /// `FormattingStyle.PRETTY.withIndent(indent)` (newline "\n", space after separators).
    // Java: com/google/gson/stream/JsonWriter.java:249 JsonWriter.setIndent
    // Java: com/google/gson/stream/JsonWriter.java:266 JsonWriter.setFormattingStyle
    pub fn set_indent(&mut self, indent: &str) {
        if indent.is_empty() {
            self.indent = String::new();
            self.formatted_colon = ":";
            self.uses_empty_newline_and_indent = true;
        } else {
            self.indent = indent.to_string();
            self.formatted_colon = ": ";
            self.uses_empty_newline_and_indent = false;
        }
    }

    // Java: com/google/gson/stream/JsonWriter.java:388 JsonWriter.beginArray
    pub fn begin_array(&mut self) -> io::Result<&mut Self> {
        self.write_deferred_name()?;
        self.open_scope(EMPTY_ARRAY, b'[')
    }

    // Java: com/google/gson/stream/JsonWriter.java:399 JsonWriter.endArray
    pub fn end_array(&mut self) -> io::Result<&mut Self> {
        self.close_scope(EMPTY_ARRAY, NONEMPTY_ARRAY, b']')
    }

    // Java: com/google/gson/stream/JsonWriter.java:409 JsonWriter.beginObject
    pub fn begin_object(&mut self) -> io::Result<&mut Self> {
        self.write_deferred_name()?;
        self.open_scope(EMPTY_OBJECT, b'{')
    }

    // Java: com/google/gson/stream/JsonWriter.java:420 JsonWriter.endObject
    pub fn end_object(&mut self) -> io::Result<&mut Self> {
        self.close_scope(EMPTY_OBJECT, NONEMPTY_OBJECT, b'}')
    }

    // Java: com/google/gson/stream/JsonWriter.java:427 JsonWriter.openScope
    fn open_scope(&mut self, empty: u8, open_bracket: u8) -> io::Result<&mut Self> {
        self.before_value()?;
        self.stack.push(empty);
        self.out.write_all(&[open_bracket])?;
        Ok(self)
    }

    // Java: com/google/gson/stream/JsonWriter.java:446 JsonWriter.closeScope
    fn close_scope(&mut self, empty: u8, nonempty: u8, close_bracket: u8) -> io::Result<&mut Self> {
        let context = self.peek();
        if context != nonempty && context != empty {
            panic!("IllegalStateException: Nesting problem.");
        }
        if let Some(name) = &self.deferred_name {
            panic!("IllegalStateException: Dangling name: {name}");
        }
        self.stack.pop();
        if context == nonempty {
            self.newline()?;
        }
        self.out.write_all(&[close_bracket])?;
        Ok(self)
    }

    fn peek(&self) -> u8 {
        match self.stack.last() {
            Some(top) => *top,
            None => panic!("IllegalStateException: JsonWriter is closed."),
        }
    }

    fn replace_top(&mut self, top: u8) {
        *self.stack.last_mut().expect("stack") = top;
    }

    // Java: com/google/gson/stream/JsonWriter.java:496 JsonWriter.name
    pub fn name(&mut self, name: &str) -> io::Result<&mut Self> {
        if self.deferred_name.is_some() {
            panic!("IllegalStateException: Already wrote a name, expecting a value.");
        }
        let context = self.peek();
        if context != EMPTY_OBJECT && context != NONEMPTY_OBJECT {
            panic!("IllegalStateException: Please begin an object before writing a name.");
        }
        self.deferred_name = Some(name.to_string());
        Ok(self)
    }

    // Java: com/google/gson/stream/JsonWriter.java:510 JsonWriter.writeDeferredName
    fn write_deferred_name(&mut self) -> io::Result<()> {
        if let Some(name) = self.deferred_name.take() {
            self.before_name()?;
            self.string(&name)?;
        }
        Ok(())
    }

    /// `value(String)`.
    // Java: com/google/gson/stream/JsonWriter.java:523 JsonWriter.value
    pub fn value_str(&mut self, value: &str) -> io::Result<&mut Self> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.string(value)?;
        Ok(self)
    }

    /// `value(long)`.
    // Java: com/google/gson/stream/JsonWriter.java:605 JsonWriter.value
    pub fn value_long(&mut self, value: i64) -> io::Result<&mut Self> {
        self.write_deferred_name()?;
        self.before_value()?;
        self.out.write_all(value.to_string().as_bytes())?;
        Ok(self)
    }

    /// `value(double)`: `Double.toString(value)`.
    // Java: com/google/gson/stream/JsonWriter.java:589 JsonWriter.value
    pub fn value_double(&mut self, value: f64) -> io::Result<&mut Self> {
        self.write_deferred_name()?;
        if value.is_nan() || value.is_infinite() {
            panic!(
                "IllegalArgumentException: Numeric values must be finite, but was {}",
                blaze_protos::json::java_double_to_string(value)
            );
        }
        self.before_value()?;
        self.out
            .write_all(blaze_protos::json::java_double_to_string(value).as_bytes())?;
        Ok(self)
    }

    // Java: com/google/gson/stream/JsonWriter.java:720 JsonWriter.flush
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    /// `close()`: flushes and checks that the document is complete. Returns the stream (Java
    /// closes it; the caller drops or finishes it).
    // Java: com/google/gson/stream/JsonWriter.java:731 JsonWriter.close
    pub fn close(mut self) -> io::Result<W> {
        self.out.flush()?;
        let size = self.stack.len();
        if size > 1 || (size == 1 && self.stack[size - 1] != NONEMPTY_DOCUMENT) {
            return Err(io::Error::other("Incomplete document"));
        }
        self.stack = vec![CLOSED];
        Ok(self.out)
    }

    // Java: com/google/gson/stream/JsonWriter.java:737 JsonWriter.string
    fn string(&mut self, value: &str) -> io::Result<()> {
        let mut buf: Vec<u8> = Vec::with_capacity(value.len() + 2);
        buf.push(b'"');
        for c in value.chars() {
            let replacement: Option<String> = if (c as u32) < 128 {
                match c {
                    '"' => Some("\\\"".into()),
                    '\\' => Some("\\\\".into()),
                    '\t' => Some("\\t".into()),
                    '\u{8}' => Some("\\b".into()),
                    '\n' => Some("\\n".into()),
                    '\r' => Some("\\r".into()),
                    '\u{c}' => Some("\\f".into()),
                    c if (c as u32) <= 0x1f => Some(format!("\\u{:04x}", c as u32)),
                    _ => None,
                }
            } else if c == '\u{2028}' {
                Some("\\u2028".into())
            } else if c == '\u{2029}' {
                Some("\\u2029".into())
            } else {
                None
            };
            match replacement {
                Some(r) => buf.extend_from_slice(r.as_bytes()),
                // OutputStreamWriter(ISO_8859_1): one byte per char, '?' when unmappable.
                None => buf.push(if (c as u32) <= 0xff {
                    c as u32 as u8
                } else {
                    b'?'
                }),
            }
        }
        buf.push(b'"');
        self.out.write_all(&buf)
    }

    // Java: com/google/gson/stream/JsonWriter.java:769 JsonWriter.newline
    fn newline(&mut self) -> io::Result<()> {
        if self.uses_empty_newline_and_indent {
            return Ok(());
        }
        self.out.write_all(b"\n")?;
        for _ in 1..self.stack.len() {
            self.out.write_all(self.indent.as_bytes())?;
        }
        Ok(())
    }

    // Java: com/google/gson/stream/JsonWriter.java:784 JsonWriter.beforeName
    fn before_name(&mut self) -> io::Result<()> {
        let context = self.peek();
        if context == NONEMPTY_OBJECT {
            // first in object
            self.out.write_all(b",")?;
        } else if context != EMPTY_OBJECT {
            // not in an object!
            panic!("IllegalStateException: Nesting problem.");
        }
        self.newline()?;
        self.replace_top(DANGLING_NAME);
        Ok(())
    }

    // Java: com/google/gson/stream/JsonWriter.java:800 JsonWriter.beforeValue
    fn before_value(&mut self) -> io::Result<()> {
        match self.peek() {
            NONEMPTY_DOCUMENT => {
                panic!("IllegalStateException: JSON must have only one top-level value.")
            }
            EMPTY_DOCUMENT => {
                // first in document
                self.replace_top(NONEMPTY_DOCUMENT);
            }
            EMPTY_ARRAY => {
                // first in array
                self.replace_top(NONEMPTY_ARRAY);
                self.newline()?;
            }
            NONEMPTY_ARRAY => {
                // another in array
                self.out.write_all(b",")?;
                self.newline()?;
            }
            DANGLING_NAME => {
                // value for name
                self.out.write_all(self.formatted_colon.as_bytes())?;
                self.replace_top(NONEMPTY_OBJECT);
            }
            _ => panic!("IllegalStateException: Nesting problem."),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indent_toggling_matches_bazel_profile_layout() {
        let mut w = JsonWriter::new(Vec::new());
        w.begin_object().unwrap();
        w.name("a").unwrap().value_long(1).unwrap();
        w.name("traceEvents").unwrap();
        w.begin_array().unwrap();
        for i in 0..2 {
            w.set_indent("  ");
            w.begin_object().unwrap();
            w.set_indent("");
            w.name("x").unwrap().value_long(i).unwrap();
            w.name("s").unwrap().value_str("q\"\u{1}é").unwrap();
            w.end_object().unwrap();
        }
        w.set_indent("  ");
        w.end_array().unwrap();
        w.end_object().unwrap();
        let out = w.close().unwrap();
        assert_eq!(
            out,
            b"{\"a\":1,\"traceEvents\":[\n    {\"x\":0,\"s\":\"q\\\"\\u0001\xe9\"},\n    {\"x\":1,\"s\":\"q\\\"\\u0001\xe9\"}\n  ]\n}"
                .to_vec()
        );
    }

    #[test]
    fn doubles_use_java_to_string() {
        let mut w = JsonWriter::new(Vec::new());
        w.begin_array().unwrap();
        w.value_double(158.84299999999996).unwrap();
        w.value_double(1.0).unwrap();
        w.value_double(0.0).unwrap();
        w.end_array().unwrap();
        assert_eq!(w.close().unwrap(), b"[158.84299999999996,1.0,0.0]".to_vec());
    }
}
