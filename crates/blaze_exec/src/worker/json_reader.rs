//! The part of Gson 2.11.0's `com.google.gson.stream.JsonReader` that the JSON worker protocol
//! uses (`JsonWorkerProtocol`, `JsonWorkerMessageProcessor`), in lenient mode (both call
//! `setLenient(true)`).
//!
//! Gson is a third-party library bundled in Bazel's server jar; its behaviour was read from the
//! bytecode in the oracle's `A-server.jar` (`javap -c -p com/google/gson/stream/JsonReader`).
//! Errors are [`JsonReaderError`]s carrying the Java exception (`MalformedJsonException`,
//! `EOFException`, `IllegalStateException`, `NumberFormatException`) and Gson's message.
//!
//! PORT: Gson's char buffer refills and the non-execute prefix `)]}'\n` (consumed only at the
//! very start of a document) are folded into a byte-at-a-time reader over the worker's stdout,
//! decoded as UTF-8 (`InputStreamReader(..., UTF_8)`).

use std::fmt;
use std::io;

use super::error_message::JavaException;
use super::recording_input_stream::InputStream;

/// `JsonToken`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// An exception thrown by the reader.
#[derive(Debug)]
pub enum JsonReaderError {
    /// `com.google.gson.stream.MalformedJsonException`.
    Malformed(String),
    /// `java.io.EOFException`.
    Eof(String),
    /// `java.lang.IllegalStateException`.
    IllegalState(String),
    /// `java.lang.NumberFormatException` (Java does not catch it in the worker protocol).
    NumberFormat(String),
    /// An `IOException` of the underlying stream.
    Io(io::Error),
}

impl JsonReaderError {
    /// The Java exception.
    pub fn to_java(&self) -> JavaException {
        match self {
            JsonReaderError::Malformed(m) => {
                JavaException::new("com.google.gson.stream.MalformedJsonException", m.clone())
            }
            JsonReaderError::Eof(m) => JavaException::new("java.io.EOFException", m.clone()),
            JsonReaderError::IllegalState(m) => {
                JavaException::new("java.lang.IllegalStateException", m.clone())
            }
            JsonReaderError::NumberFormat(m) => {
                JavaException::new("java.lang.NumberFormatException", m.clone())
            }
            JsonReaderError::Io(e) => JavaException::from_io_error(e),
        }
    }
}

impl From<io::Error> for JsonReaderError {
    fn from(e: io::Error) -> Self {
        JsonReaderError::Io(e)
    }
}

// JsonScope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    DanglingName,
    NonemptyObject,
    EmptyDocument,
    NonemptyDocument,
    Closed,
}

#[derive(Clone, Debug, PartialEq)]
enum Peeked {
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    True,
    False,
    Null,
    SingleQuoted,
    DoubleQuoted,
    Unquoted,
    /// A number literal (its text).
    Number(String),
    SingleQuotedName,
    DoubleQuotedName,
    UnquotedName,
    Eof,
}

/// `com.google.gson.stream.JsonReader` (lenient).
pub struct JsonReader {
    input: Box<dyn FnMut() -> io::Result<Option<char>> + Send>,
    /// One char of look-ahead (Gson's buffer).
    pushback: Vec<char>,
    peeked: Option<Peeked>,
    line_number: i32,
    /// Chars consumed on the current line (Gson's `pos - lineStart`).
    column: i32,
    stack: Vec<Scope>,
    path_names: Vec<Option<String>>,
    path_indices: Vec<i32>,
    closed: bool,
}

/// `TroubleshootingGuide.createUrl(id)`.
fn troubleshooting_url(id: &str) -> String {
    format!("https://github.com/google/gson/blob/main/Troubleshooting.md#{id}")
}

impl JsonReader {
    /// `new JsonReader(new BufferedReader(new InputStreamReader(stream, UTF_8)))`, then
    /// `setLenient(true)`.
    pub fn new(mut stream: Box<dyn InputStream>) -> JsonReader {
        let mut decoder = Utf8Decoder::default();
        JsonReader {
            input: Box::new(move || decoder.next(&mut *stream)),
            pushback: Vec::new(),
            peeked: None,
            line_number: 0,
            column: 0,
            stack: vec![Scope::EmptyDocument],
            path_names: vec![None],
            path_indices: vec![0],
            closed: false,
        }
    }

    fn read_char(&mut self) -> io::Result<Option<char>> {
        if let Some(c) = self.pushback.pop() {
            return Ok(Some(c));
        }
        (self.input)()
    }

    fn consume(&mut self, c: char) {
        if c == '\n' {
            self.line_number += 1;
            self.column = 0;
        } else {
            self.column += 1;
        }
    }

    /// Looks at the next char without consuming it.
    fn peek_char(&mut self) -> io::Result<Option<char>> {
        let c = self.read_char()?;
        if let Some(c) = c {
            self.pushback.push(c);
        }
        Ok(c)
    }

    /// `getPath()`.
    pub fn get_path(&self) -> String {
        let mut result = String::from("$");
        for (i, scope) in self.stack.iter().enumerate() {
            match scope {
                Scope::EmptyArray | Scope::NonemptyArray => {
                    result.push('[');
                    result.push_str(&self.path_indices[i].to_string());
                    result.push(']');
                }
                Scope::EmptyObject | Scope::DanglingName | Scope::NonemptyObject => {
                    result.push('.');
                    if let Some(name) = &self.path_names[i] {
                        result.push_str(name);
                    }
                }
                _ => {}
            }
        }
        result
    }

    /// `locationString()`.
    fn location_string(&self) -> String {
        format!(
            " at line {} column {} path {}",
            self.line_number + 1,
            self.column + 1,
            self.get_path()
        )
    }

    /// `syntaxError(message)`.
    fn syntax_error(&self, message: &str) -> JsonReaderError {
        JsonReaderError::Malformed(format!(
            "{}{}\nSee {}",
            message,
            self.location_string(),
            troubleshooting_url("malformed-json")
        ))
    }

    /// `unexpectedTokenError(expected)`.
    fn unexpected_token_error(&mut self, expected: &str) -> JsonReaderError {
        let peeked = match self.peek() {
            Ok(p) => p,
            Err(e) => return e,
        };
        let troubleshooting_id = if peeked == JsonToken::Null {
            "adapter-not-null-safe"
        } else {
            "unexpected-json-structure"
        };
        JsonReaderError::IllegalState(format!(
            "Expected {} but was {}{}\nSee {}",
            expected,
            peeked,
            self.location_string(),
            troubleshooting_url(troubleshooting_id)
        ))
    }

    fn push(&mut self, scope: Scope) {
        self.stack.push(scope);
        self.path_names.push(None);
        self.path_indices.push(0);
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.path_names.pop();
        self.path_indices.pop();
    }

    fn top(&mut self) -> &mut Scope {
        self.stack.last_mut().expect("scope")
    }

    fn increment_index(&mut self) {
        if let Some(i) = self.path_indices.last_mut() {
            *i += 1;
        }
    }

    // Java (gson): JsonReader.beginArray
    pub fn begin_array(&mut self) -> Result<(), JsonReaderError> {
        if self.do_peek_cached()? == Peeked::BeginArray {
            self.push(Scope::EmptyArray);
            *self.path_indices.last_mut().unwrap() = 0;
            self.peeked = None;
            Ok(())
        } else {
            Err(self.unexpected_token_error("BEGIN_ARRAY"))
        }
    }

    // Java (gson): JsonReader.endArray
    pub fn end_array(&mut self) -> Result<(), JsonReaderError> {
        if self.do_peek_cached()? == Peeked::EndArray {
            self.pop();
            self.peeked = None;
            self.increment_index();
            Ok(())
        } else {
            Err(self.unexpected_token_error("END_ARRAY"))
        }
    }

    // Java (gson): JsonReader.beginObject
    pub fn begin_object(&mut self) -> Result<(), JsonReaderError> {
        if self.do_peek_cached()? == Peeked::BeginObject {
            self.push(Scope::EmptyObject);
            self.peeked = None;
            Ok(())
        } else {
            Err(self.unexpected_token_error("BEGIN_OBJECT"))
        }
    }

    // Java (gson): JsonReader.endObject
    pub fn end_object(&mut self) -> Result<(), JsonReaderError> {
        if self.do_peek_cached()? == Peeked::EndObject {
            self.pop();
            // Free the last path name so that it can be garbage collected!
            self.peeked = None;
            self.increment_index();
            Ok(())
        } else {
            Err(self.unexpected_token_error("END_OBJECT"))
        }
    }

    // Java (gson): JsonReader.hasNext
    pub fn has_next(&mut self) -> Result<bool, JsonReaderError> {
        let p = self.do_peek_cached()?;
        Ok(p != Peeked::EndObject && p != Peeked::EndArray && p != Peeked::Eof)
    }

    // Java (gson): JsonReader.peek
    pub fn peek(&mut self) -> Result<JsonToken, JsonReaderError> {
        Ok(match self.do_peek_cached()? {
            Peeked::BeginObject => JsonToken::BeginObject,
            Peeked::EndObject => JsonToken::EndObject,
            Peeked::BeginArray => JsonToken::BeginArray,
            Peeked::EndArray => JsonToken::EndArray,
            Peeked::SingleQuotedName | Peeked::DoubleQuotedName | Peeked::UnquotedName => {
                JsonToken::Name
            }
            Peeked::True | Peeked::False => JsonToken::Boolean,
            Peeked::Null => JsonToken::Null,
            Peeked::SingleQuoted | Peeked::DoubleQuoted | Peeked::Unquoted => JsonToken::String,
            Peeked::Number(_) => JsonToken::Number,
            Peeked::Eof => JsonToken::EndDocument,
        })
    }

    fn do_peek_cached(&mut self) -> Result<Peeked, JsonReaderError> {
        if let Some(p) = &self.peeked {
            return Ok(p.clone());
        }
        let p = self.do_peek()?;
        self.peeked = Some(p.clone());
        Ok(p)
    }

    // Java (gson): JsonReader.doPeek
    fn do_peek(&mut self) -> Result<Peeked, JsonReaderError> {
        if self.closed {
            return Err(JsonReaderError::IllegalState(
                "JsonReader is closed".to_string(),
            ));
        }
        let peek_stack = *self.top();
        match peek_stack {
            Scope::EmptyArray => {
                *self.top() = Scope::NonemptyArray;
            }
            Scope::NonemptyArray => {
                // Look for a comma before the next element.
                let c = self.next_non_whitespace(true)?;
                match c {
                    ']' => return Ok(Peeked::EndArray),
                    // lenient: ';' as a separator.
                    ';' | ',' => {}
                    _ => return Err(self.syntax_error("Unterminated array")),
                }
            }
            Scope::EmptyObject | Scope::NonemptyObject => {
                *self.top() = Scope::DanglingName;
                // Look for a comma before the next element.
                if peek_stack == Scope::NonemptyObject {
                    let c = self.next_non_whitespace(true)?;
                    match c {
                        '}' => return Ok(Peeked::EndObject),
                        ';' | ',' => {}
                        _ => return Err(self.syntax_error("Unterminated object")),
                    }
                }
                let c = self.next_non_whitespace(true)?;
                return match c {
                    '"' => Ok(Peeked::DoubleQuotedName),
                    '\'' => Ok(Peeked::SingleQuotedName),
                    '}' => {
                        if peek_stack != Scope::NonemptyObject {
                            Ok(Peeked::EndObject)
                        } else {
                            Err(self.syntax_error("Expected name"))
                        }
                    }
                    _ => {
                        self.unread(c);
                        if is_literal(c) {
                            Ok(Peeked::UnquotedName)
                        } else {
                            Err(self.syntax_error("Expected name"))
                        }
                    }
                };
            }
            Scope::DanglingName => {
                *self.top() = Scope::NonemptyObject;
                // Look for a colon before the value.
                let c = self.next_non_whitespace(true)?;
                match c {
                    ':' => {}
                    '=' => {
                        // lenient: '=>' as a separator.
                        if self.peek_char()? == Some('>') {
                            let c = self.read_char()?.unwrap();
                            self.consume(c);
                        }
                    }
                    _ => return Err(self.syntax_error("Expected ':'")),
                }
            }
            Scope::EmptyDocument => {
                // PORT: consumeNonExecutePrefix is not ported (no worker writes it).
                *self.top() = Scope::NonemptyDocument;
            }
            Scope::NonemptyDocument => {
                match self.next_non_whitespace_opt(false)? {
                    None => return Ok(Peeked::Eof),
                    Some(c) => {
                        // lenient: more than one top-level value.
                        self.unread(c);
                    }
                }
            }
            Scope::Closed => {
                return Err(JsonReaderError::IllegalState(
                    "JsonReader is closed".to_string(),
                ));
            }
        }

        let c = self.next_non_whitespace(true)?;
        match c {
            ']' => {
                if peek_stack == Scope::EmptyArray {
                    return Ok(Peeked::EndArray);
                }
                // In lenient mode, a 0-length literal in an array means 'null'.
                if peek_stack == Scope::EmptyArray || peek_stack == Scope::NonemptyArray {
                    self.unread(c);
                    return Ok(Peeked::Null);
                }
                return Err(self.syntax_error("Unexpected value"));
            }
            ';' | ',' => {
                // In lenient mode, a 0-length literal in an array means 'null'.
                if peek_stack == Scope::EmptyArray || peek_stack == Scope::NonemptyArray {
                    self.unread(c);
                    return Ok(Peeked::Null);
                }
                return Err(self.syntax_error("Unexpected value"));
            }
            '\'' => return Ok(Peeked::SingleQuoted),
            '"' => return Ok(Peeked::DoubleQuoted),
            '[' => return Ok(Peeked::BeginArray),
            '{' => return Ok(Peeked::BeginObject),
            _ => self.unread(c),
        }

        // peekKeyword / peekNumber, then an unquoted literal.
        let literal = self.peek_literal_text()?;
        if literal.is_empty() {
            return Err(self.syntax_error("Expected value"));
        }
        let lower = literal.to_ascii_lowercase();
        if lower == "true" {
            self.skip_literal_chars(literal.chars().count())?;
            return Ok(Peeked::True);
        }
        if lower == "false" {
            self.skip_literal_chars(literal.chars().count())?;
            return Ok(Peeked::False);
        }
        if lower == "null" {
            self.skip_literal_chars(literal.chars().count())?;
            return Ok(Peeked::Null);
        }
        if is_json_number(&literal) {
            self.skip_literal_chars(literal.chars().count())?;
            return Ok(Peeked::Number(literal));
        }
        Ok(Peeked::Unquoted)
    }

    fn unread(&mut self, c: char) {
        self.pushback.push(c);
        // The char was counted by next_non_whitespace; Gson's `pos--` undoes it.
        if c == '\n' {
            // next_non_whitespace never returns a newline.
        } else {
            self.column -= 1;
        }
    }

    /// The literal at the current position, without consuming it.
    fn peek_literal_text(&mut self) -> io::Result<String> {
        let mut chars = Vec::new();
        loop {
            match self.read_char()? {
                Some(c) if is_literal(c) => chars.push(c),
                Some(c) => {
                    self.pushback.push(c);
                    break;
                }
                None => break,
            }
        }
        for c in chars.iter().rev() {
            self.pushback.push(*c);
        }
        Ok(chars.into_iter().collect())
    }

    fn skip_literal_chars(&mut self, n: usize) -> io::Result<()> {
        for _ in 0..n {
            if let Some(c) = self.read_char()? {
                self.consume(c);
            }
        }
        Ok(())
    }

    /// `nextNonWhitespace(throwOnEof)`: the next non-whitespace char, consumed; comments are
    /// skipped (lenient).
    fn next_non_whitespace_opt(&mut self, throw_on_eof: bool) -> Result<Option<char>, JsonReaderError> {
        loop {
            let Some(c) = self.read_char()? else {
                if throw_on_eof {
                    return Err(JsonReaderError::Eof(format!(
                        "End of input{}",
                        self.location_string()
                    )));
                }
                return Ok(None);
            };
            self.consume(c);
            match c {
                '\n' | ' ' | '\r' | '\t' => continue,
                '/' => {
                    match self.peek_char()? {
                        Some('*') => {
                            let c = self.read_char()?.unwrap();
                            self.consume(c);
                            // skip a /* c-style comment */
                            if !self.skip_to("*/")? {
                                return Err(self.syntax_error("Unterminated comment"));
                            }
                            continue;
                        }
                        Some('/') => {
                            let c = self.read_char()?.unwrap();
                            self.consume(c);
                            // skip a // end-of-line comment
                            self.skip_to_end_of_line()?;
                            continue;
                        }
                        _ => return Ok(Some(c)),
                    }
                }
                '#' => {
                    // Skip a # hash end-of-line comment. The JSON RFC doesn't specify this
                    // behaviour, but it's required to parse existing documents.
                    self.skip_to_end_of_line()?;
                    continue;
                }
                _ => return Ok(Some(c)),
            }
        }
    }

    fn next_non_whitespace(&mut self, throw_on_eof: bool) -> Result<char, JsonReaderError> {
        match self.next_non_whitespace_opt(throw_on_eof)? {
            Some(c) => Ok(c),
            // Only reachable with throw_on_eof == false, which callers handle via the _opt form.
            None => Err(JsonReaderError::Eof(format!(
                "End of input{}",
                self.location_string()
            ))),
        }
    }

    fn skip_to_end_of_line(&mut self) -> io::Result<()> {
        while let Some(c) = self.read_char()? {
            self.consume(c);
            if c == '\n' || c == '\r' {
                break;
            }
        }
        Ok(())
    }

    fn skip_to(&mut self, to: &str) -> io::Result<bool> {
        let target: Vec<char> = to.chars().collect();
        let mut window: Vec<char> = Vec::new();
        while let Some(c) = self.read_char()? {
            self.consume(c);
            window.push(c);
            if window.len() > target.len() {
                window.remove(0);
            }
            if window == target {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Java (gson): JsonReader.nextName
    pub fn next_name(&mut self) -> Result<String, JsonReaderError> {
        let result = match self.do_peek_cached()? {
            Peeked::UnquotedName => self.next_unquoted_value()?,
            Peeked::SingleQuotedName => self.next_quoted_value('\'')?,
            Peeked::DoubleQuotedName => self.next_quoted_value('"')?,
            _ => return Err(self.unexpected_token_error("a name")),
        };
        self.peeked = None;
        *self.path_names.last_mut().unwrap() = Some(result.clone());
        Ok(result)
    }

    // Java (gson): JsonReader.nextString
    pub fn next_string(&mut self) -> Result<String, JsonReaderError> {
        let result = match self.do_peek_cached()? {
            Peeked::Unquoted => self.next_unquoted_value()?,
            Peeked::SingleQuoted => self.next_quoted_value('\'')?,
            Peeked::DoubleQuoted => self.next_quoted_value('"')?,
            Peeked::Number(text) => text,
            _ => return Err(self.unexpected_token_error("a string")),
        };
        self.peeked = None;
        self.increment_index();
        Ok(result)
    }

    // Java (gson): JsonReader.nextInt
    pub fn next_int(&mut self) -> Result<i32, JsonReaderError> {
        let p = self.do_peek_cached()?;
        let text = match p {
            Peeked::Number(text) => {
                // PEEKED_LONG fast path, else PEEKED_NUMBER parsed as a double.
                if let Ok(v) = text.parse::<i64>() {
                    let result = v as i32;
                    if result as i64 != v {
                        // Make sure no precision was lost casting to 'int'.
                        return Err(JsonReaderError::NumberFormat(format!(
                            "Expected an int but was {}{}",
                            v,
                            self.location_string()
                        )));
                    }
                    self.peeked = None;
                    self.increment_index();
                    return Ok(result);
                }
                text
            }
            Peeked::SingleQuoted | Peeked::DoubleQuoted | Peeked::Unquoted => {
                let text = match p {
                    Peeked::Unquoted => self.next_unquoted_value()?,
                    Peeked::SingleQuoted => self.next_quoted_value('\'')?,
                    _ => self.next_quoted_value('"')?,
                };
                if let Ok(result) = text.parse::<i32>() {
                    self.peeked = None;
                    self.increment_index();
                    return Ok(result);
                }
                text
            }
            _ => return Err(self.unexpected_token_error("an int")),
        };
        // We don't have an int. Parse as a double.
        let as_double: f64 = match text.trim().parse::<f64>() {
            Ok(d) => d,
            Err(_) => {
                return Err(JsonReaderError::NumberFormat(format!(
                    "For input string: \"{text}\""
                )));
            }
        };
        let result = as_double as i32;
        if result as f64 != as_double {
            // Make sure no precision was lost casting to 'int'.
            return Err(JsonReaderError::NumberFormat(format!(
                "Expected an int but was {}{}",
                text,
                self.location_string()
            )));
        }
        self.peeked = None;
        self.increment_index();
        Ok(result)
    }

    // Java (gson): JsonReader.skipValue
    pub fn skip_value(&mut self) -> Result<(), JsonReaderError> {
        let mut count = 0;
        loop {
            match self.do_peek_cached()? {
                Peeked::BeginArray => {
                    self.push(Scope::EmptyArray);
                    count += 1;
                }
                Peeked::BeginObject => {
                    self.push(Scope::EmptyObject);
                    count += 1;
                }
                Peeked::EndArray | Peeked::EndObject => {
                    if count == 0 {
                        // Unlike Gson 2.11 (which pops past the enclosing scope), stop here.
                        self.peeked = None;
                        return Ok(());
                    }
                    self.pop();
                    count -= 1;
                }
                Peeked::UnquotedName | Peeked::Unquoted => {
                    self.next_unquoted_value()?;
                }
                Peeked::SingleQuotedName | Peeked::SingleQuoted => {
                    self.next_quoted_value('\'')?;
                }
                Peeked::DoubleQuotedName | Peeked::DoubleQuoted => {
                    self.next_quoted_value('"')?;
                }
                Peeked::Eof => {
                    // Do nothing.
                    return Ok(());
                }
                _ => {}
            }
            self.peeked = None;
            if count == 0 {
                break;
            }
        }
        self.increment_index();
        *self.path_names.last_mut().unwrap() = Some("<skipped>".to_string());
        Ok(())
    }

    fn next_unquoted_value(&mut self) -> Result<String, JsonReaderError> {
        let mut s = String::new();
        loop {
            match self.read_char()? {
                Some(c) if is_literal(c) => {
                    self.consume(c);
                    s.push(c);
                }
                Some(c) => {
                    self.pushback.push(c);
                    break;
                }
                None => break,
            }
        }
        Ok(s)
    }

    fn next_quoted_value(&mut self, quote: char) -> Result<String, JsonReaderError> {
        let mut s = String::new();
        loop {
            let Some(c) = self.read_char()? else {
                return Err(self.syntax_error("Unterminated string"));
            };
            self.consume(c);
            if c == quote {
                return Ok(s);
            } else if c == '\\' {
                s.push(self.read_escape_character()?);
            } else {
                s.push(c);
            }
        }
    }

    fn read_escape_character(&mut self) -> Result<char, JsonReaderError> {
        let Some(c) = self.read_char()? else {
            return Err(self.syntax_error("Unterminated escape sequence"));
        };
        self.consume(c);
        Ok(match c {
            'u' => {
                let mut hex = String::new();
                for _ in 0..4 {
                    let Some(h) = self.read_char()? else {
                        return Err(self.syntax_error("Unterminated escape sequence"));
                    };
                    self.consume(h);
                    hex.push(h);
                }
                match u32::from_str_radix(&hex, 16) {
                    Ok(v) => char::from_u32(v).unwrap_or('\u{FFFD}'),
                    Err(_) => {
                        return Err(self.syntax_error(&format!("Malformed Unicode escape \\u{hex}")));
                    }
                }
            }
            't' => '\t',
            'b' => '\u{8}',
            'n' => '\n',
            'r' => '\r',
            'f' => '\u{c}',
            '\n' | '\'' | '"' | '\\' | '/' => c,
            _ => return Err(self.syntax_error("Invalid escape sequence")),
        })
    }

    /// `close()`.
    pub fn close(&mut self) {
        self.peeked = None;
        self.stack = vec![Scope::Closed];
        self.closed = true;
    }
}

/// `isLiteral(c)` in lenient mode.
fn is_literal(c: char) -> bool {
    !matches!(
        c,
        '/' | '\\' | ';' | '#' | '=' | '{' | '}' | '[' | ']' | ':' | ',' | ' ' | '\t' | '\u{c}'
            | '\r' | '\n'
    )
}

/// Whether Gson's `peekNumber` accepts `s` as a number.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp {
            return false;
        }
    }
    i == b.len()
}

/// `InputStreamReader(stream, UTF_8)`: decodes bytes into chars, malformed input as U+FFFD.
#[derive(Default)]
struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    fn next(&mut self, stream: &mut dyn InputStream) -> io::Result<Option<char>> {
        let mut byte = [0u8; 1];
        loop {
            if self.pending.is_empty() {
                if stream.read(&mut byte)? == 0 {
                    return Ok(None);
                }
                self.pending.push(byte[0]);
            }
            let first = self.pending[0];
            let needed = match first {
                0x00..=0x7F => 1,
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => {
                    self.pending.clear();
                    return Ok(Some('\u{FFFD}'));
                }
            };
            while self.pending.len() < needed {
                if stream.read(&mut byte)? == 0 {
                    self.pending.clear();
                    return Ok(Some('\u{FFFD}'));
                }
                if byte[0] & 0xC0 != 0x80 {
                    // Malformed: emit a replacement and keep the new byte.
                    self.pending.clear();
                    self.pending.push(byte[0]);
                    return Ok(Some('\u{FFFD}'));
                }
                self.pending.push(byte[0]);
            }
            let s = std::str::from_utf8(&self.pending)
                .ok()
                .and_then(|s| s.chars().next());
            self.pending.clear();
            return Ok(Some(s.unwrap_or('\u{FFFD}')));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::recording_input_stream::ByteArrayInputStream;

    fn reader(s: &str) -> JsonReader {
        JsonReader::new(Box::new(ByteArrayInputStream::new(s.as_bytes().to_vec())))
    }

    #[test]
    fn reads_objects_in_sequence() {
        let mut r = reader("{\"exitCode\":1,\"output\":\"hi\"}\n{'requestId': 3}");
        r.begin_object().unwrap();
        assert_eq!(r.next_name().unwrap(), "exitCode");
        assert_eq!(r.next_int().unwrap(), 1);
        assert_eq!(r.next_name().unwrap(), "output");
        assert_eq!(r.next_string().unwrap(), "hi");
        assert!(!r.has_next().unwrap());
        r.end_object().unwrap();
        r.begin_object().unwrap();
        assert_eq!(r.next_name().unwrap(), "requestId");
        assert_eq!(r.next_int().unwrap(), 3);
        r.end_object().unwrap();
    }

    #[test]
    fn eof_message() {
        let mut r = reader("");
        match r.begin_object() {
            Err(JsonReaderError::Eof(m)) => assert_eq!(m, "End of input at line 1 column 1 path $"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unexpected_token_message() {
        let mut r = reader("garbage");
        match r.begin_object() {
            Err(JsonReaderError::IllegalState(m)) => assert_eq!(
                m,
                "Expected BEGIN_OBJECT but was STRING at line 1 column 1 path $\n\
                 See https://github.com/google/gson/blob/main/Troubleshooting.md#unexpected-json-structure"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn skips_unknown_values() {
        let mut r = reader("{\"x\":{\"a\":[1,2,{\"b\":null}]},\"exitCode\":0}");
        r.begin_object().unwrap();
        assert_eq!(r.next_name().unwrap(), "x");
        r.skip_value().unwrap();
        assert_eq!(r.next_name().unwrap(), "exitCode");
        assert_eq!(r.next_int().unwrap(), 0);
        r.end_object().unwrap();
    }
}
