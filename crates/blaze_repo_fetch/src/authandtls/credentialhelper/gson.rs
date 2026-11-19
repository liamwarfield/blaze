//! The subset of Gson (`com.google.gson`) the credential helper protocol uses:
//! `Gson.fromJson(Reader, Class)` / `Gson.toJson(Object, Appendable)` for a class with a
//! `@JsonAdapter`, `stream.JsonReader`, `stream.JsonWriter`, `stream.JsonToken` and
//! `JsonSyntaxException`.
//!
//! PORT: Gson is a third-party library; this is not a port of its sources but of the behaviour
//! the protocol classes rely on, for a `new Gson()` (no strictness set):
//!
//! - `fromJson` reads with `Strictness.LENIENT`, returns `null` for an empty document (only
//!   whitespace), and fails with "JSON document was not fully consumed." when a second value
//!   follows the first. The `@JsonAdapter` adapter is null-safe: a top-level `null` reads as `null`.
//! - The reader reads Gson's lenient syntax: unquoted and single-quoted names and strings,
//!   `#`, `//` and `/* */` comments, `=`/`=>` name separators, `;` separators, the `)]}'\n`
//!   prefix, missing array elements as `null`, case-insensitive `true`/`false`/`null`; and it
//!   skips a leading byte order mark (`JsonReader.fillBuffer`).
//! - Numbers follow JSON's grammar, except that Gson's own number reader also has limits on
//!   `long` values; anything that is not a number is read as an unquoted literal (`NaN`, `01`).
//! - A `\u` escape that leaves an unpaired UTF-16 surrogate becomes U+FFFD (Rust strings cannot
//!   hold one; Java keeps it).
//! - The texts of Gson's own syntax errors (with their `at line N column M path $...` suffix) are
//!   not reproduced; no Bazel output prints them (the adapters' own messages are kept exactly).
//! - The writer is html-safe, as `new Gson()` is: `<`, `>`, `&`, `=` and `'` are written as
//!   `\u003c`, `\u003e`, `\u0026`, `\u003d` and `\u0027`.

use std::fmt;

/// `com.google.gson.JsonSyntaxException`: the JSON text is malformed, or does not match the
/// expected structure.
///
/// PORT: Gson wraps `IllegalStateException`s (the reader's "Expected X but was Y", AutoValue's
/// "Missing required properties") and `IOException`s (`MalformedJsonException`,
/// `EOFException`) thrown while reading in a `JsonSyntaxException`; their text is the message here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonSyntaxException {
    pub message: String,
}

impl JsonSyntaxException {
    pub fn new(message: impl Into<String>) -> JsonSyntaxException {
        JsonSyntaxException {
            message: message.into(),
        }
    }
}

impl fmt::Display for JsonSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonSyntaxException {}

/// `com.google.gson.stream.JsonToken`. `Display` is the Java enum constant name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JsonToken {
    BeginArray,
    EndArray,
    BeginObject,
    EndObject,
    Name,
    String,
    Number,
    Boolean,
    Null,
    EndDocument,
}

impl fmt::Display for JsonToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            JsonToken::BeginArray => "BEGIN_ARRAY",
            JsonToken::EndArray => "END_ARRAY",
            JsonToken::BeginObject => "BEGIN_OBJECT",
            JsonToken::EndObject => "END_OBJECT",
            JsonToken::Name => "NAME",
            JsonToken::String => "STRING",
            JsonToken::Number => "NUMBER",
            JsonToken::Boolean => "BOOLEAN",
            JsonToken::Null => "NULL",
            JsonToken::EndDocument => "END_DOCUMENT",
        })
    }
}

/// `com.google.gson.stream.JsonScope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonScope {
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    DanglingName,
    NonemptyObject,
    EmptyDocument,
    NonemptyDocument,
}

/// A token read by [`JsonReader::peek`] and not consumed yet, with its text.
#[derive(Clone, Debug)]
enum Peeked {
    BeginArray,
    EndArray,
    BeginObject,
    EndObject,
    Name(String),
    String(String),
    Number(String),
    Boolean,
    Null,
    EndDocument,
}

impl Peeked {
    fn token(&self) -> JsonToken {
        match self {
            Peeked::BeginArray => JsonToken::BeginArray,
            Peeked::EndArray => JsonToken::EndArray,
            Peeked::BeginObject => JsonToken::BeginObject,
            Peeked::EndObject => JsonToken::EndObject,
            Peeked::Name(_) => JsonToken::Name,
            Peeked::String(_) => JsonToken::String,
            Peeked::Number(_) => JsonToken::Number,
            Peeked::Boolean => JsonToken::Boolean,
            Peeked::Null => JsonToken::Null,
            Peeked::EndDocument => JsonToken::EndDocument,
        }
    }
}

/// `com.google.gson.stream.JsonReader` in lenient mode, over a complete document.
pub struct JsonReader<'a> {
    input: &'a [u8],
    pos: usize,
    stack: Vec<JsonScope>,
    peeked: Option<Peeked>,
}

/// Gson `JsonReader.isLiteral(c)`: whether `c` may be part of an unquoted literal.
fn is_literal(c: u8) -> bool {
    !matches!(
        c,
        b'/' | b'\\'
            | b';'
            | b'#'
            | b'='
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b':'
            | b','
            | b' '
            | b'\t'
            | 0x0c
            | b'\r'
            | b'\n'
    )
}

fn malformed(message: &str) -> JsonSyntaxException {
    JsonSyntaxException::new(format!(
        "com.google.gson.stream.MalformedJsonException: {message}"
    ))
}

fn end_of_input() -> JsonSyntaxException {
    JsonSyntaxException::new("java.io.EOFException: End of input")
}

fn illegal_state(expected: JsonToken, was: JsonToken) -> JsonSyntaxException {
    JsonSyntaxException::new(format!(
        "java.lang.IllegalStateException: Expected {expected} but was {was}"
    ))
}

impl<'a> JsonReader<'a> {
    pub fn new(input: &'a str) -> JsonReader<'a> {
        // JsonReader.fillBuffer: a byte order mark at the start is skipped.
        let pos = if input.starts_with('\u{feff}') { 3 } else { 0 };
        JsonReader {
            input: input.as_bytes(),
            pos,
            stack: vec![JsonScope::EmptyDocument],
            peeked: None,
        }
    }

    /// Whether the document holds nothing but whitespace and comments (`Gson.fromJson`'s
    /// `isEmpty`, the case in which the first `peek` throws an `EOFException`).
    fn is_empty_document(&self) -> bool {
        let mut probe = JsonReader {
            input: self.input,
            pos: self.pos,
            stack: Vec::new(),
            peeked: None,
        };
        matches!(probe.next_non_whitespace(), Ok(None))
    }

    /// Gson `JsonReader.nextNonWhitespace` (lenient): skips whitespace and `/* */`, `//` and
    /// `#` comments; returns the next character, consumed, or `None` at the end.
    fn next_non_whitespace(&mut self) -> Result<Option<u8>, JsonSyntaxException> {
        while let Some(&c) = self.input.get(self.pos) {
            self.pos += 1;
            match c {
                b' ' | b'\t' | b'\n' | b'\r' => {}
                b'/' => match self.input.get(self.pos) {
                    Some(b'*') => {
                        // skip a /* c-style comment */
                        let end = self.input[self.pos + 1..]
                            .windows(2)
                            .position(|w| w == b"*/")
                            .ok_or_else(|| malformed("Unterminated comment"))?;
                        self.pos += 1 + end + 2;
                    }
                    // skip a // end-of-line comment
                    Some(b'/') => self.skip_to_end_of_line(),
                    _ => return Ok(Some(c)),
                },
                // skip a # hash end-of-line comment
                b'#' => self.skip_to_end_of_line(),
                _ => return Ok(Some(c)),
            }
        }
        Ok(None)
    }

    // Gson JsonReader.skipToEndOfLine
    fn skip_to_end_of_line(&mut self) {
        while let Some(&c) = self.input.get(self.pos) {
            self.pos += 1;
            if c == b'\n' || c == b'\r' {
                break;
            }
        }
    }

    fn scope(&mut self) -> &mut JsonScope {
        self.stack.last_mut().expect("JsonReader scope stack")
    }

    // Gson JsonReader.doPeek
    fn do_peek(&mut self) -> Result<Peeked, JsonSyntaxException> {
        let peek_stack = *self.scope();
        match peek_stack {
            JsonScope::EmptyArray => *self.scope() = JsonScope::NonemptyArray,
            JsonScope::NonemptyArray => match self.next_non_whitespace()? {
                Some(b']') => return Ok(Peeked::EndArray),
                // Lenient: `;` separates elements too.
                Some(b';' | b',') => {}
                Some(_) => return Err(malformed("Unterminated array")),
                None => return Err(end_of_input()),
            },
            JsonScope::EmptyObject | JsonScope::NonemptyObject => {
                *self.scope() = JsonScope::DanglingName;
                if peek_stack == JsonScope::NonemptyObject {
                    match self.next_non_whitespace()? {
                        Some(b'}') => return Ok(Peeked::EndObject),
                        Some(b';' | b',') => {}
                        Some(_) => return Err(malformed("Unterminated object")),
                        None => return Err(end_of_input()),
                    }
                }
                return match self.next_non_whitespace()? {
                    Some(b'"') => Ok(Peeked::Name(self.next_quoted_value(b'"')?)),
                    Some(b'\'') => Ok(Peeked::Name(self.next_quoted_value(b'\'')?)),
                    Some(b'}') if peek_stack != JsonScope::NonemptyObject => Ok(Peeked::EndObject),
                    Some(b'}') => Err(malformed("Expected name")),
                    Some(c) => {
                        self.pos -= 1;
                        if is_literal(c) {
                            Ok(Peeked::Name(self.next_unquoted_value()))
                        } else {
                            Err(malformed("Expected name"))
                        }
                    }
                    None => Err(end_of_input()),
                };
            }
            JsonScope::DanglingName => {
                *self.scope() = JsonScope::NonemptyObject;
                match self.next_non_whitespace()? {
                    Some(b':') => {}
                    // Lenient: `=` or `=>` separates a name from its value.
                    Some(b'=') => {
                        if self.input.get(self.pos) == Some(&b'>') {
                            self.pos += 1;
                        }
                    }
                    Some(_) => return Err(malformed("Expected ':'")),
                    None => return Err(end_of_input()),
                }
            }
            JsonScope::EmptyDocument => {
                // Lenient: JsonReader.consumeNonExecutePrefix.
                if self.next_non_whitespace()?.is_some() {
                    self.pos -= 1;
                }
                if self.input[self.pos..].starts_with(b")]}'\n") {
                    self.pos += 5;
                }
                *self.scope() = JsonScope::NonemptyDocument
            }
            JsonScope::NonemptyDocument => match self.next_non_whitespace()? {
                None => return Ok(Peeked::EndDocument),
                // Lenient: a further top-level value.
                Some(_) => self.pos -= 1,
            },
        }

        let c = match self.next_non_whitespace()? {
            Some(c) => c,
            None => return Err(end_of_input()),
        };
        match c {
            b']' if peek_stack == JsonScope::EmptyArray => Ok(Peeked::EndArray),
            // In an array, a missing element (`[1,,2]`, `[1,]`) reads as null.
            b']' | b';' | b','
                if matches!(peek_stack, JsonScope::EmptyArray | JsonScope::NonemptyArray) =>
            {
                self.pos -= 1;
                Ok(Peeked::Null)
            }
            b']' | b';' | b',' => Err(malformed("Unexpected value")),
            b'\'' => Ok(Peeked::String(self.next_quoted_value(b'\'')?)),
            b'"' => Ok(Peeked::String(self.next_quoted_value(b'"')?)),
            b'[' => Ok(Peeked::BeginArray),
            b'{' => Ok(Peeked::BeginObject),
            _ => {
                self.pos -= 1;
                if let Some(p) = self.peek_keyword() {
                    return Ok(p);
                }
                if let Some(p) = self.peek_number() {
                    return Ok(p);
                }
                if !is_literal(self.input[self.pos]) {
                    return Err(malformed("Expected value"));
                }
                Ok(Peeked::String(self.next_unquoted_value()))
            }
        }
    }

    // Gson JsonReader.nextUnquotedValue: the characters up to the first non-literal one.
    fn next_unquoted_value(&mut self) -> String {
        let start = self.pos;
        while self.input.get(self.pos).is_some_and(|&c| is_literal(c)) {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.input[start..self.pos]).into_owned()
    }

    /// Whether a keyword or number may end before `at` (Gson: the next character is not part of
    /// an unquoted literal).
    fn is_literal_end(&self, at: usize) -> bool {
        self.input.get(at).is_none_or(|&c| !is_literal(c))
    }

    fn peek_keyword(&mut self) -> Option<Peeked> {
        for (word, peeked) in [
            (&b"true"[..], Peeked::Boolean),
            (&b"false"[..], Peeked::Boolean),
            (&b"null"[..], Peeked::Null),
        ] {
            let end = self.pos + word.len();
            if self.input.len() >= end
                && self.input[self.pos..end].eq_ignore_ascii_case(word)
                && self.is_literal_end(end)
            {
                self.pos = end;
                return Some(peeked);
            }
        }
        None
    }

    fn peek_number(&mut self) -> Option<Peeked> {
        let start = self.pos;
        let mut i = start;
        let digits = |i: &mut usize, input: &[u8]| {
            let s = *i;
            while input.get(*i).is_some_and(u8::is_ascii_digit) {
                *i += 1;
            }
            *i > s
        };
        if self.input.get(i) == Some(&b'-') {
            i += 1;
        }
        // Gson: a leading '0' followed by a digit is not a number (it could be octal); the text
        // is then an unquoted literal.
        if self.input.get(i) == Some(&b'0') && self.input.get(i + 1).is_some_and(u8::is_ascii_digit)
        {
            return None;
        }
        if !digits(&mut i, self.input) {
            return None;
        }
        if self.input.get(i) == Some(&b'.') {
            i += 1;
            if !digits(&mut i, self.input) {
                return None;
            }
        }
        if matches!(self.input.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(self.input.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            if !digits(&mut i, self.input) {
                return None;
            }
        }
        if !self.is_literal_end(i) {
            return None;
        }
        self.pos = i;
        Some(Peeked::Number(
            String::from_utf8_lossy(&self.input[start..i]).into_owned(),
        ))
    }

    /// Reads a string quoted with `quote` (`"`, or `'` in lenient mode) whose opening quote was
    /// consumed.
    fn next_quoted_value(&mut self, quote: u8) -> Result<String, JsonSyntaxException> {
        let mut units: Vec<u16> = Vec::new();
        let mut out = String::new();
        let flush = |units: &mut Vec<u16>, out: &mut String| {
            if !units.is_empty() {
                out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{FFFD}')));
            }
        };
        loop {
            let Some(&c) = self.input.get(self.pos) else {
                return Err(malformed("Unterminated string"));
            };
            self.pos += 1;
            match c {
                c if c == quote => {
                    flush(&mut units, &mut out);
                    return Ok(out);
                }
                b'\\' => {
                    let Some(&e) = self.input.get(self.pos) else {
                        return Err(malformed("Unterminated escape sequence"));
                    };
                    self.pos += 1;
                    let unit = match e {
                        b'u' => {
                            let hex = self
                                .input
                                .get(self.pos..self.pos + 4)
                                .ok_or_else(|| malformed("Unterminated escape sequence"))?;
                            let hex = std::str::from_utf8(hex)
                                .ok()
                                .and_then(|h| u16::from_str_radix(h, 16).ok())
                                .ok_or_else(|| malformed("Malformed Unicode escape"))?;
                            self.pos += 4;
                            hex
                        }
                        b't' => b'\t' as u16,
                        b'b' => 0x08,
                        b'n' => b'\n' as u16,
                        b'r' => b'\r' as u16,
                        b'f' => 0x0c,
                        b'\n' | b'\'' | b'"' | b'\\' | b'/' => e as u16,
                        _ => return Err(malformed("Invalid escape sequence")),
                    };
                    units.push(unit);
                }
                _ => {
                    flush(&mut units, &mut out);
                    // The input is a `str`: copy the whole UTF-8 sequence that starts here.
                    let start = self.pos - 1;
                    let len = match c {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let end = (start + len).min(self.input.len());
                    out.push_str(&String::from_utf8_lossy(&self.input[start..end]));
                    self.pos = end;
                }
            }
        }
    }

    // Gson JsonReader.peek
    pub fn peek(&mut self) -> Result<JsonToken, JsonSyntaxException> {
        if self.peeked.is_none() {
            self.peeked = Some(self.do_peek()?);
        }
        Ok(self.peeked.as_ref().unwrap().token())
    }

    fn consume(&mut self, expected: JsonToken) -> Result<Peeked, JsonSyntaxException> {
        let token = self.peek()?;
        if token != expected {
            return Err(illegal_state(expected, token));
        }
        Ok(self.peeked.take().unwrap())
    }

    // Gson JsonReader.beginObject
    pub fn begin_object(&mut self) -> Result<(), JsonSyntaxException> {
        self.consume(JsonToken::BeginObject)?;
        self.stack.push(JsonScope::EmptyObject);
        Ok(())
    }

    // Gson JsonReader.endObject
    pub fn end_object(&mut self) -> Result<(), JsonSyntaxException> {
        self.consume(JsonToken::EndObject)?;
        self.stack.pop();
        Ok(())
    }

    // Gson JsonReader.beginArray
    pub fn begin_array(&mut self) -> Result<(), JsonSyntaxException> {
        self.consume(JsonToken::BeginArray)?;
        self.stack.push(JsonScope::EmptyArray);
        Ok(())
    }

    // Gson JsonReader.endArray
    pub fn end_array(&mut self) -> Result<(), JsonSyntaxException> {
        self.consume(JsonToken::EndArray)?;
        self.stack.pop();
        Ok(())
    }

    // Gson JsonReader.hasNext
    pub fn has_next(&mut self) -> Result<bool, JsonSyntaxException> {
        let token = self.peek()?;
        Ok(!matches!(
            token,
            JsonToken::EndObject | JsonToken::EndArray | JsonToken::EndDocument
        ))
    }

    // Gson JsonReader.nextName
    pub fn next_name(&mut self) -> Result<String, JsonSyntaxException> {
        match self.consume(JsonToken::Name)? {
            Peeked::Name(name) => Ok(name),
            _ => unreachable!(),
        }
    }

    /// Gson `JsonReader.nextString`: a string, or the text of a number.
    pub fn next_string(&mut self) -> Result<String, JsonSyntaxException> {
        let token = self.peek()?;
        match self.peeked.take().unwrap() {
            Peeked::String(s) | Peeked::Number(s) => Ok(s),
            other => {
                self.peeked = Some(other);
                Err(illegal_state(JsonToken::String, token))
            }
        }
    }

    // Gson JsonReader.nextNull
    pub fn next_null(&mut self) -> Result<(), JsonSyntaxException> {
        self.consume(JsonToken::Null)?;
        Ok(())
    }

    // Gson JsonReader.skipValue
    pub fn skip_value(&mut self) -> Result<(), JsonSyntaxException> {
        let mut count = 0usize;
        loop {
            match self.peek()? {
                JsonToken::BeginArray => {
                    self.begin_array()?;
                    count += 1;
                }
                JsonToken::BeginObject => {
                    self.begin_object()?;
                    count += 1;
                }
                JsonToken::EndArray => {
                    self.end_array()?;
                    count = count.saturating_sub(1);
                }
                JsonToken::EndObject => {
                    self.end_object()?;
                    count = count.saturating_sub(1);
                }
                JsonToken::EndDocument => return Ok(()),
                _ => {
                    self.peeked = None;
                }
            }
            if count == 0 {
                return Ok(());
            }
        }
    }
}

/// `Gson.fromJson(json, T.class)` for a class whose `@JsonAdapter` adapter is `read`:
/// `Ok(None)` for an empty document or a top-level `null`.
pub fn from_json<T>(
    json: &str,
    read: impl FnOnce(&mut JsonReader<'_>) -> Result<T, JsonSyntaxException>,
) -> Result<Option<T>, JsonSyntaxException> {
    let mut reader = JsonReader::new(json);
    // Gson.fromJson(JsonReader, TypeToken): an EOFException on the first peek is an empty document.
    if reader.is_empty_document() {
        return Ok(None);
    }
    // TypeAdapter.nullSafe()
    let object = if reader.peek()? == JsonToken::Null {
        reader.next_null()?;
        None
    } else {
        Some(read(&mut reader)?)
    };
    // Gson.assertFullConsumption
    if object.is_some() && reader.peek()? != JsonToken::EndDocument {
        return Err(JsonSyntaxException::new(
            "JSON document was not fully consumed.",
        ));
    }
    Ok(object)
}

/// `com.google.gson.stream.JsonWriter` with Gson's defaults (html-safe, no indentation).
#[derive(Default)]
pub struct JsonWriter {
    out: String,
    /// Whether the current object or array already has an element (Gson's `NONEMPTY_*` scopes).
    nonempty: Vec<bool>,
    /// A name was written and its value not yet.
    dangling_name: bool,
}

impl JsonWriter {
    pub fn new() -> JsonWriter {
        JsonWriter::default()
    }

    fn before_value(&mut self) {
        if self.dangling_name {
            self.dangling_name = false;
            return;
        }
        if let Some(nonempty) = self.nonempty.last_mut() {
            if *nonempty {
                self.out.push(',');
            }
            *nonempty = true;
        }
    }

    pub fn begin_object(&mut self) -> &mut Self {
        self.before_value();
        self.out.push('{');
        self.nonempty.push(false);
        self
    }

    pub fn end_object(&mut self) -> &mut Self {
        self.nonempty.pop();
        self.out.push('}');
        self
    }

    pub fn begin_array(&mut self) -> &mut Self {
        self.before_value();
        self.out.push('[');
        self.nonempty.push(false);
        self
    }

    pub fn end_array(&mut self) -> &mut Self {
        self.nonempty.pop();
        self.out.push(']');
        self
    }

    pub fn name(&mut self, name: &str) -> &mut Self {
        self.before_value();
        write_string(&mut self.out, name);
        self.out.push(':');
        self.dangling_name = true;
        self
    }

    pub fn value(&mut self, value: &str) -> &mut Self {
        self.before_value();
        write_string(&mut self.out, value);
        self
    }

    pub fn into_string(self) -> String {
        self.out
    }
}

/// Gson `JsonWriter.string` with `htmlSafe` set.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{0c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
