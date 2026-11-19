//! The parts of Gson 2.11.0 that Bzlmod's JSON files depend on: a lenient `JsonReader` that
//! builds a tree (`JsonParser.parseReader`), and `JsonWriter` with Bazel's settings
//! (`disableHtmlEscaping`, optional pretty printing, `serializeNulls` false).
//!
//! Gson is a third-party library, so these are ports of its observable behaviour (output bytes and
//! accepted input), not of Bazel code.

use std::fmt::Write as _;

/// A parsed JSON value (Gson `JsonElement`). Numbers keep their literal text, like Gson's
/// `LazilyParsedNumber`. Object members keep their order; a repeated name replaces the earlier
/// value in place (Gson `JsonObject.add`).
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Gson's `JsonToken` name of this value, for "Expected X but was Y" messages.
    pub fn token_name(&self) -> &'static str {
        match self {
            JsonValue::Null => "NULL",
            JsonValue::Bool(_) => "BOOLEAN",
            JsonValue::Number(_) => "NUMBER",
            JsonValue::String(_) => "STRING",
            JsonValue::Array(_) => "BEGIN_ARRAY",
            JsonValue::Object(_) => "BEGIN_OBJECT",
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Looks up a member of an object.
    pub fn get(&self, name: &str) -> Option<&JsonValue> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }
}

/// A Gson parse error (`MalformedJsonException`/`JsonSyntaxException`), with Gson's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonParseError(pub String);

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsonParseError {}

/// Parses one JSON document the way `Gson.fromJson(String, ...)` does in its default (lenient)
/// mode, then checks that the whole document was consumed. Returns `Ok(None)` for an empty
/// document (Gson returns null).
// PORT: Gson's error messages include a JSONPath; this reader reports line and column and the
// path of enclosing names it can track, which may differ from Gson in corner cases.
pub fn parse_lenient(input: &str) -> Result<Option<JsonValue>, JsonParseError> {
    // Java: Gson 2.11 JsonReader.fillBuffer skips one leading byte order mark (U+FEFF) and
    // counts columns after it (`lineStart++`).
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut r = LenientReader::new(input);
    // Gson skips a non-execute prefix in lenient mode.
    if r.rest().starts_with(")]}'\n") {
        r.pos += 5;
    }
    r.skip_ws()?;
    if r.peek_char().is_none() {
        return Ok(None);
    }
    let value = r.read_value()?;
    r.skip_ws()?;
    if r.peek_char().is_some() {
        return Err(JsonParseError(
            "JSON document was not fully consumed.".to_string(),
        ));
    }
    Ok(Some(value))
}

/// Four UTF-16 units that are all ASCII hex digits, as one unit (Gson's `\u` escape).
fn hex4(units: &[u16]) -> Option<u16> {
    if units.len() != 4 {
        return None;
    }
    let mut result: u16 = 0;
    for &c in units {
        let digit = match c {
            0x30..=0x39 => c - 0x30,
            0x61..=0x66 => c - 0x61 + 10,
            0x41..=0x46 => c - 0x41 + 10,
            _ => return None,
        };
        result = (result << 4) + digit;
    }
    Some(result)
}

struct LenientReader<'a> {
    input: &'a str,
    pos: usize,
    path: Vec<String>,
}

impl<'a> LenientReader<'a> {
    fn new(input: &'a str) -> Self {
        LenientReader {
            input,
            pos: 0,
            path: Vec::new(),
        }
    }

    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn location(&self) -> String {
        let before = &self.input[..self.pos];
        let line = before.matches('\n').count() + 1;
        // Gson counts chars (UTF-16 units) of the text, not UTF-8 bytes.
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let column = before[line_start..].encode_utf16().count() + 1;
        let mut path = String::from("$");
        for p in &self.path {
            path.push_str(p);
        }
        format!(" at line {line} column {column} path {path}")
    }

    fn syntax_error(&self, message: &str) -> JsonParseError {
        JsonParseError(format!(
            "com.google.gson.stream.MalformedJsonException: {message}{}",
            self.location()
        ))
    }

    fn skip_ws(&mut self) -> Result<(), JsonParseError> {
        loop {
            match self.peek_char() {
                Some(' ' | '\t' | '\n' | '\r') => {
                    self.pos += 1;
                }
                Some('#') => self.skip_to_eol(),
                Some('/') => {
                    let rest = self.rest();
                    if rest.starts_with("//") {
                        self.skip_to_eol();
                    } else if let Some(after) = rest.strip_prefix("/*") {
                        match after.find("*/") {
                            Some(i) => self.pos += 2 + i + 2,
                            None => return Err(self.syntax_error("Unterminated comment")),
                        }
                    } else {
                        return Ok(());
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn skip_to_eol(&mut self) {
        match self.rest().find(['\n', '\r']) {
            Some(i) => self.pos += i + 1,
            None => self.pos = self.input.len(),
        }
    }

    fn read_value(&mut self) -> Result<JsonValue, JsonParseError> {
        self.skip_ws()?;
        match self.peek_char() {
            None => Err(JsonParseError(format!(
                "java.io.EOFException: End of input{}",
                self.location()
            ))),
            Some('{') => {
                self.pos += 1;
                self.read_object()
            }
            Some('[') => {
                self.pos += 1;
                self.read_array()
            }
            Some(q @ ('"' | '\'')) => {
                self.pos += 1;
                Ok(JsonValue::String(self.read_quoted(q)?))
            }
            Some('}' | ']' | ':' | ',' | ';' | '=') => Err(self.syntax_error("Expected value")),
            Some(_) => self.read_literal(),
        }
    }

    fn read_object(&mut self) -> Result<JsonValue, JsonParseError> {
        let mut members: Vec<(String, JsonValue)> = Vec::new();
        self.skip_ws()?;
        if self.peek_char() == Some('}') {
            self.pos += 1;
            return Ok(JsonValue::Object(members));
        }
        self.path.push(".".to_string());
        loop {
            self.skip_ws()?;
            let name = match self.peek_char() {
                Some(q @ ('"' | '\'')) => {
                    self.pos += 1;
                    self.read_quoted(q)?
                }
                Some('}') | None => return Err(self.syntax_error("Expected name")),
                Some(_) => {
                    let lit = self.read_unquoted();
                    if lit.is_empty() {
                        return Err(self.syntax_error("Expected name"));
                    }
                    lit
                }
            };
            *self.path.last_mut().unwrap() = format!(".{name}");
            self.skip_ws()?;
            match self.bump() {
                Some(':') => {}
                Some('=') => {
                    if self.peek_char() == Some('>') {
                        self.pos += 1;
                    }
                }
                _ => return Err(self.syntax_error("Expected ':'")),
            }
            let value = self.read_value()?;
            if let Some(slot) = members.iter_mut().find(|(k, _)| *k == name) {
                slot.1 = value;
            } else {
                members.push((name, value));
            }
            self.skip_ws()?;
            match self.bump() {
                Some('}') => break,
                Some(',' | ';') => continue,
                None => {
                    return Err(JsonParseError(format!(
                        "java.io.EOFException: End of input{}",
                        self.location()
                    )));
                }
                _ => return Err(self.syntax_error("Unterminated object")),
            }
        }
        self.path.pop();
        Ok(JsonValue::Object(members))
    }

    fn read_array(&mut self) -> Result<JsonValue, JsonParseError> {
        let mut elements = Vec::new();
        self.skip_ws()?;
        if self.peek_char() == Some(']') {
            self.pos += 1;
            return Ok(JsonValue::Array(elements));
        }
        self.path.push("[0]".to_string());
        loop {
            *self.path.last_mut().unwrap() = format!("[{}]", elements.len());
            self.skip_ws()?;
            // Lenient: a missing value before ',', ';' or ']' is null.
            match self.peek_char() {
                Some(',' | ';') => {
                    self.pos += 1;
                    elements.push(JsonValue::Null);
                    continue;
                }
                Some(']') => {
                    self.pos += 1;
                    elements.push(JsonValue::Null);
                    break;
                }
                _ => {}
            }
            elements.push(self.read_value()?);
            self.skip_ws()?;
            match self.bump() {
                Some(']') => break,
                Some(',' | ';') => {
                    self.skip_ws()?;
                    if self.peek_char() == Some(']') {
                        self.pos += 1;
                        elements.push(JsonValue::Null);
                        break;
                    }
                }
                None => {
                    return Err(JsonParseError(format!(
                        "java.io.EOFException: End of input{}",
                        self.location()
                    )));
                }
                _ => return Err(self.syntax_error("Unterminated array")),
            }
        }
        self.path.pop();
        Ok(JsonValue::Array(elements))
    }

    fn read_quoted(&mut self, quote: char) -> Result<String, JsonParseError> {
        let mut out = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err(self.syntax_error("Unterminated string"));
            };
            if c == quote {
                return Ok(out);
            }
            if c == '\\' {
                let Some(e) = self.bump() else {
                    return Err(self.syntax_error("Unterminated escape sequence"));
                };
                match e {
                    'u' => {
                        // Java: Gson 2.11 JsonReader.readEscapeCharacter: the next four chars
                        // (UTF-16 units), each an ASCII hex digit.
                        let units: Vec<u16> = self.rest().encode_utf16().take(4).collect();
                        if units.len() < 4 {
                            return Err(self.syntax_error("Unterminated escape sequence"));
                        }
                        let Some(unit) = hex4(&units) else {
                            // PORT: a surrogate pair cut by the fourth unit prints as U+FFFD
                            // (Java keeps the lone unit).
                            return Err(self.syntax_error(&format!(
                                "Malformed Unicode escape \\u{}",
                                String::from_utf16_lossy(&units)
                            )));
                        };
                        self.pos += 4;
                        // Combine surrogate pairs.
                        if (0xD800..0xDC00).contains(&unit) && self.rest().starts_with("\\u") {
                            let low = self.rest()[2..].encode_utf16().take(4).collect::<Vec<_>>();
                            if let Some(low) = hex4(&low)
                                && (0xDC00..0xE000).contains(&low)
                            {
                                self.pos += 6;
                                out.push_str(&String::from_utf16_lossy(&[unit, low]));
                                continue;
                            }
                        }
                        // PORT: byte-strings: Java keeps an unpaired surrogate escape as its
                        // UTF-16 code unit in the String; a Rust String cannot hold one, so it
                        // becomes U+FFFD.
                        out.push_str(&String::from_utf16_lossy(&[unit]));
                    }
                    't' => out.push('\t'),
                    'b' => out.push('\u{8}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    'f' => out.push('\u{c}'),
                    '\n' => out.push('\n'),
                    '\'' | '"' | '\\' | '/' => out.push(e),
                    _ => return Err(self.syntax_error("Invalid escape sequence")),
                }
                continue;
            }
            out.push(c);
        }
    }

    /// An unquoted literal (lenient): up to the next delimiter.
    fn read_unquoted(&mut self) -> String {
        let rest = self.rest();
        let end = rest
            .find(|c: char| {
                matches!(
                    c,
                    '/' | '\\'
                        | ';'
                        | '#'
                        | '='
                        | '{'
                        | '}'
                        | '['
                        | ']'
                        | ':'
                        | ','
                        | ' '
                        | '\t'
                        | '\u{c}'
                        | '\r'
                        | '\n'
                )
            })
            .unwrap_or(rest.len());
        self.pos += end;
        rest[..end].to_string()
    }

    fn read_literal(&mut self) -> Result<JsonValue, JsonParseError> {
        let lit = self.read_unquoted();
        if lit.is_empty() {
            return Err(self.syntax_error("Expected value"));
        }
        if lit.eq_ignore_ascii_case("true") {
            return Ok(JsonValue::Bool(true));
        }
        if lit.eq_ignore_ascii_case("false") {
            return Ok(JsonValue::Bool(false));
        }
        if lit.eq_ignore_ascii_case("null") {
            return Ok(JsonValue::Null);
        }
        if is_json_number(&lit) {
            return Ok(JsonValue::Number(lit));
        }
        Ok(JsonValue::String(lit))
    }
}

/// Gson's `peekNumber` grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    if i >= b.len() {
        return false;
    }
    if b[i] == b'0' {
        i += 1;
    } else if b[i].is_ascii_digit() {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    EmptyDocument,
    NonemptyDocument,
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    DanglingName,
    NonemptyObject,
}

/// Port of Gson's `JsonWriter` with `htmlSafe = false` (Bazel always disables HTML escaping).
/// With `pretty`, uses `FormattingStyle.PRETTY` (newline `\n`, indent two spaces, `": "`).
#[derive(Debug)]
pub struct JsonWriter {
    out: String,
    stack: Vec<Scope>,
    deferred_name: Option<String>,
    serialize_nulls: bool,
    pretty: bool,
}

impl JsonWriter {
    pub fn new(pretty: bool) -> Self {
        JsonWriter {
            out: String::new(),
            stack: vec![Scope::EmptyDocument],
            deferred_name: None,
            serialize_nulls: false,
            pretty,
        }
    }

    pub fn get_serialize_nulls(&self) -> bool {
        self.serialize_nulls
    }

    pub fn set_serialize_nulls(&mut self, serialize_nulls: bool) {
        self.serialize_nulls = serialize_nulls;
    }

    pub fn into_string(self) -> String {
        self.out
    }

    fn peek(&self) -> Scope {
        *self.stack.last().expect("JsonWriter is closed.")
    }

    fn replace_top(&mut self, scope: Scope) {
        *self.stack.last_mut().unwrap() = scope;
    }

    pub fn begin_object(&mut self) -> &mut Self {
        self.write_deferred_name();
        self.open(Scope::EmptyObject, '{');
        self
    }

    pub fn end_object(&mut self) -> &mut Self {
        self.close(Scope::EmptyObject, Scope::NonemptyObject, '}');
        self
    }

    pub fn begin_array(&mut self) -> &mut Self {
        self.write_deferred_name();
        self.open(Scope::EmptyArray, '[');
        self
    }

    pub fn end_array(&mut self) -> &mut Self {
        self.close(Scope::EmptyArray, Scope::NonemptyArray, ']');
        self
    }

    fn open(&mut self, empty: Scope, bracket: char) {
        self.before_value();
        self.stack.push(empty);
        self.out.push(bracket);
    }

    fn close(&mut self, empty: Scope, nonempty: Scope, bracket: char) {
        let context = self.peek();
        assert!(
            context == empty || context == nonempty,
            "IllegalStateException: Nesting problem."
        );
        assert!(
            self.deferred_name.is_none(),
            "IllegalStateException: Dangling name: {:?}",
            self.deferred_name
        );
        self.stack.pop();
        if context == nonempty {
            self.newline();
        }
        self.out.push(bracket);
    }

    pub fn name(&mut self, name: &str) -> &mut Self {
        assert!(self.deferred_name.is_none(), "IllegalStateException");
        let context = self.peek();
        assert!(
            context == Scope::EmptyObject || context == Scope::NonemptyObject,
            "IllegalStateException: Please begin an object before writing a name."
        );
        self.deferred_name = Some(name.to_string());
        self
    }

    fn write_deferred_name(&mut self) {
        if let Some(name) = self.deferred_name.take() {
            self.before_name();
            self.string(&name);
        }
    }

    pub fn value_str(&mut self, value: &str) -> &mut Self {
        self.write_deferred_name();
        self.before_value();
        self.string(value);
        self
    }

    pub fn value_bool(&mut self, value: bool) -> &mut Self {
        self.write_deferred_name();
        self.before_value();
        self.out.push_str(if value { "true" } else { "false" });
        self
    }

    /// `value(Number)`: writes the number's string form verbatim.
    pub fn value_number(&mut self, literal: &str) -> &mut Self {
        self.write_deferred_name();
        self.before_value();
        self.out.push_str(literal);
        self
    }

    pub fn null_value(&mut self) -> &mut Self {
        if self.deferred_name.is_some() {
            if self.serialize_nulls {
                self.write_deferred_name();
            } else {
                // Skip the name and the value.
                self.deferred_name = None;
                return self;
            }
        }
        self.before_value();
        self.out.push_str("null");
        self
    }

    /// `Streams.write(JsonElement, JsonWriter)` (`TypeAdapters.JSON_ELEMENT.write`).
    pub fn json_value(&mut self, value: &JsonValue) -> &mut Self {
        match value {
            JsonValue::Null => {
                self.null_value();
            }
            JsonValue::Bool(b) => {
                self.value_bool(*b);
            }
            JsonValue::Number(n) => {
                self.value_number(n);
            }
            JsonValue::String(s) => {
                self.value_str(s);
            }
            JsonValue::Array(a) => {
                self.begin_array();
                for e in a {
                    self.json_value(e);
                }
                self.end_array();
            }
            JsonValue::Object(o) => {
                self.begin_object();
                for (k, v) in o {
                    self.name(k);
                    self.json_value(v);
                }
                self.end_object();
            }
        }
        self
    }

    /// `Gson.toJson(JsonElement, JsonWriter)` with a Gson that has default `serializeNulls`
    /// (false): nulls inside objects are dropped together with their names.
    pub fn json_value_with_gson_defaults(&mut self, value: &JsonValue) -> &mut Self {
        let old = self.serialize_nulls;
        self.serialize_nulls = false;
        self.json_value(value);
        self.serialize_nulls = old;
        self
    }

    fn string(&mut self, value: &str) {
        self.out.push('"');
        for c in value.chars() {
            match c {
                '"' => self.out.push_str("\\\""),
                '\\' => self.out.push_str("\\\\"),
                '\t' => self.out.push_str("\\t"),
                '\u{8}' => self.out.push_str("\\b"),
                '\n' => self.out.push_str("\\n"),
                '\r' => self.out.push_str("\\r"),
                '\u{c}' => self.out.push_str("\\f"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(self.out, "\\u{:04x}", c as u32);
                }
                '\u{2028}' => self.out.push_str("\\u2028"),
                '\u{2029}' => self.out.push_str("\\u2029"),
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }

    fn newline(&mut self) {
        if !self.pretty {
            return;
        }
        self.out.push('\n');
        for _ in 1..self.stack.len() {
            self.out.push_str("  ");
        }
    }

    fn before_name(&mut self) {
        let context = self.peek();
        if context == Scope::NonemptyObject {
            self.out.push(',');
        } else {
            assert!(
                context == Scope::EmptyObject,
                "IllegalStateException: Nesting problem."
            );
        }
        self.newline();
        self.replace_top(Scope::DanglingName);
    }

    fn before_value(&mut self) {
        match self.peek() {
            Scope::NonemptyDocument => {
                panic!("IllegalStateException: JSON must have only one top-level value.")
            }
            Scope::EmptyDocument => self.replace_top(Scope::NonemptyDocument),
            Scope::EmptyArray => {
                self.replace_top(Scope::NonemptyArray);
                self.newline();
            }
            Scope::NonemptyArray => {
                self.out.push(',');
                self.newline();
            }
            Scope::DanglingName => {
                self.out.push_str(if self.pretty { ": " } else { ":" });
                self.replace_top(Scope::NonemptyObject);
            }
            Scope::EmptyObject | Scope::NonemptyObject => {
                panic!("IllegalStateException: Nesting problem.")
            }
        }
    }
}
