//! Port of `com.google.protobuf.TextFormat` (protobuf-java 33.4, the version Bazel 9.3.2 builds
//! with: `MODULE.bazel` `bazel_dep(name = "protobuf", version = "33.4")`; Bazel's
//! `third_party/protobuf.patch` does not touch `TextFormat.java`). Paths in the `// Java:`
//! markers are relative to `java/core/src/main/java` of that release.
//!
//! - Printing: [`printer`] (`TextFormat.printer()`), [`Printer`], [`message_to_string`]
//!   (`Message.toString()`), [`escape_bytes`], [`unsigned_to_string_32`] / `_64`.
//! - Parsing: [`get_parser`], [`Parser`] / [`ParserBuilder`], [`merge`] (`TextFormat.merge`),
//!   [`parse`] (`TextFormat.parse`), the `Tokenizer`, [`unescape_bytes`], [`parse_int32`] ...
//! - [`TypeRegistry`] for `google.protobuf.Any`.
//!
//! Both halves are descriptor-driven, as in Java: they walk the runtime descriptors of
//! [`crate::descriptors`] (the `FileDescriptorSet` that `build.rs` compiles, embedded in the
//! crate) over a [`DynamicMessage`]. A generated message enters as the `DynamicMessage` parsed
//! from its Java encoding and leaves as the generated message decoded from the
//! `DynamicMessage`'s encoding. No code is written per message type.
//!
//! PORT: Java's `CharSequence` is UTF-16; the tokenizer works on the UTF-16 code units of the
//! input, so line and column numbers in errors count what Java counts.

use std::collections::BTreeSet;
use std::fmt;

use crate::JavaMessage;
use crate::build::bazel::remote::execution::v2::Platform;
use crate::descriptors::{Descriptor, FieldDescriptor, JavaType, Type, pool};
pub use crate::dynamic_message::{
    DynamicMessage, UnknownField as UnknownFieldSetField, UnknownFieldSet, Value,
};
use crate::json::InvalidProtocolBufferException;

mod printer;

pub use printer::{
    Printer, debug_format_printer, default_format_printer, escape_bytes,
    escape_double_quotes_and_backslashes, escape_text, message_to_string, printer,
    unsigned_to_string_32, unsigned_to_string_64,
};

/// `Parser.Builder.recursionLimit` default.
// Java: com/google/protobuf/TextFormat.java:1941 TextFormat.Parser.Builder.recursionLimit
const RECURSION_LIMIT: i32 = 100;

/// Thrown when parsing an invalid text format message.
// Java: com/google/protobuf/TextFormat.java:1704 TextFormat.ParseException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseException {
    line: i32,
    column: i32,
    message: String,
}

impl ParseException {
    /// Create a new instance.
    // Java: com/google/protobuf/TextFormat.java:1721 TextFormat.ParseException.<init>
    pub fn new(line: i32, column: i32, message: &str) -> ParseException {
        ParseException {
            line,
            column,
            message: format!("{line}:{column}: {message}"),
        }
    }

    /// Return the line where the parse exception occurred, or -1 when none is provided. The value
    /// is specified as 1-offset, so the first line is line 1.
    // Java: com/google/protobuf/TextFormat.java:1731 TextFormat.ParseException.getLine
    pub fn get_line(&self) -> i32 {
        self.line
    }

    /// Return the column where the parse exception occurred, or -1 when none is provided.
    // Java: com/google/protobuf/TextFormat.java:1739 TextFormat.ParseException.getColumn
    pub fn get_column(&self) -> i32 {
        self.column
    }

    /// `getMessage()`: `<line>:<column>: <description>`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseException {}

/// Thrown by `unescapeBytes` when an invalid escape sequence is seen.
// Java: com/google/protobuf/TextFormat.java:2925 TextFormat.InvalidEscapeSequenceException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidEscapeSequenceException(pub String);

impl fmt::Display for InvalidEscapeSequenceException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidEscapeSequenceException {}

/// `java.lang.NumberFormatException` from `parseInteger` and the JDK number parsers it calls.
///
/// The messages of `Long.parseLong`, `Integer.parseInt` and `new BigInteger(String, int)` are
/// JDK 21's; they reach a user through `Couldn't parse integer: ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberFormatException(pub String);

impl fmt::Display for NumberFormatException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------------------------
// TypeRegistry (`com.google.protobuf.TypeRegistry`): the types `Any` may be expanded as.

/// A registry of message types the printer and parser expand `google.protobuf.Any` with.
// Java: com/google/protobuf/TypeRegistry.java TypeRegistry
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    /// `types`: full name to descriptor.
    types: BTreeSet<String>,
}

impl TypeRegistry {
    /// `TypeRegistry.getEmptyTypeRegistry()`.
    pub fn get_empty_type_registry() -> TypeRegistry {
        TypeRegistry::default()
    }

    /// `TypeRegistry.newBuilder()`.
    pub fn new_builder() -> TypeRegistryBuilder {
        TypeRegistryBuilder {
            files: BTreeSet::new(),
            types: BTreeSet::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// `find(name)`.
    // Java: com/google/protobuf/TypeRegistry.java:42 TypeRegistry.find
    pub fn find(&self, name: &str) -> Option<&'static Descriptor> {
        self.types
            .contains(name)
            .then(|| printer::find_message_type(name))
            .flatten()
    }

    /// Find a type by its typeUrl. Returns `None` if it cannot be found in this registry.
    // Java: com/google/protobuf/TypeRegistry.java:49 TypeRegistry.getDescriptorForTypeUrl
    pub fn get_descriptor_for_type_url(
        &self,
        type_url: &str,
    ) -> Result<Option<&'static Descriptor>, InvalidProtocolBufferException> {
        Ok(self.find(&get_type_name(type_url)?))
    }
}

/// `typeUrl.split("/")`'s last part (Java's `split` drops trailing empty strings).
// Java: com/google/protobuf/TypeRegistry.java:60 TypeRegistry.getTypeName
fn get_type_name(type_url: &str) -> Result<String, InvalidProtocolBufferException> {
    let mut parts: Vec<&str> = type_url.split('/').collect();
    while parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() {
        parts.clear();
    }
    if parts.len() <= 1 {
        return Err(InvalidProtocolBufferException::new(format!(
            "Invalid type url found: {type_url}"
        )));
    }
    Ok(parts[parts.len() - 1].to_string())
}

/// `TypeRegistry.Builder`.
// Java: com/google/protobuf/TypeRegistry.java TypeRegistry.Builder
#[derive(Clone, Debug)]
pub struct TypeRegistryBuilder {
    files: BTreeSet<String>,
    types: BTreeSet<String>,
}

impl TypeRegistryBuilder {
    /// `add(Descriptor)`: adds the type's file, its dependencies, and all their messages.
    pub fn add<M: JavaMessage>(mut self) -> Self {
        let descriptor = descriptor_of::<M>();
        self.add_file(&descriptor.get_file().name);
        self
    }

    /// `add(Descriptor)` of a pool descriptor.
    pub fn add_descriptor(mut self, descriptor: &Descriptor) -> Self {
        self.add_file(&descriptor.get_file().name);
        self
    }

    // Java: com/google/protobuf/TypeRegistry.java TypeRegistry.Builder.addFile
    fn add_file(&mut self, name: &str) {
        // Skip the file if it's already added.
        if !self.files.insert(name.to_string()) {
            return;
        }
        let file = pool().find_file_by_name(name).expect("file of the pool");
        for dependency in &file.dependencies {
            self.add_file(dependency);
        }
        for message in file.get_message_types() {
            self.add_message(message);
        }
    }

    // Java: com/google/protobuf/TypeRegistry.java TypeRegistry.Builder.addMessage
    fn add_message(&mut self, message: &'static Descriptor) {
        for nested_type in message.get_nested_types() {
            self.add_message(nested_type);
        }

        if self.types.contains(&message.full_name) {
            // logger.warning("Type " + message.getFullName() + " is added multiple times.")
            return;
        }

        self.types.insert(message.full_name.clone());
    }

    /// `build()`.
    pub fn build(self) -> TypeRegistry {
        TypeRegistry { types: self.types }
    }
}

// ---------------------------------------------------------------------------------------------
// Tokenizer.

/// Represents a stream of tokens parsed from a `CharSequence`.
///
/// PORT: the silent-marker fields (`containsSilentMarkerAfter*Token`) are never set by this
/// version's `nextToken` and only `detectSilentMarker` (a no-op) reads them; they are left out.
// Java: com/google/protobuf/TextFormat.java:1123 TextFormat.Tokenizer
struct Tokenizer<'a> {
    text: &'a [u16],
    current_token: Vec<u16>,

    // The character index within this.text at which the current token begins.
    pos: usize,

    // The line and column numbers of the current token.
    line: i32,
    column: i32,
    line_info_tracking_pos: usize,

    // The line and column numbers of the previous token (allows throwing errors *after*
    // consuming).
    previous_line: i32,
    previous_column: i32,
}

const fn u(c: char) -> u16 {
    c as u16
}

impl<'a> Tokenizer<'a> {
    /// Construct a tokenizer that parses tokens from the given text.
    // Java: com/google/protobuf/TextFormat.java:1150 TextFormat.Tokenizer.<init>
    fn new(text: &'a [u16]) -> Tokenizer<'a> {
        let mut t = Tokenizer {
            text,
            current_token: Vec::new(),
            pos: 0,
            line: 0,
            column: 0,
            line_info_tracking_pos: 0,
            previous_line: 0,
            previous_column: 0,
        };
        t.skip_whitespace();
        t.next_token();
        t
    }

    fn get_previous_line(&self) -> i32 {
        self.previous_line
    }

    fn get_previous_column(&self) -> i32 {
        self.previous_column
    }

    /// `currentToken` as a Java string.
    ///
    /// PORT: a token is a slice of the UTF-16 input; only a single-character token can hold half
    /// of a surrogate pair, which Java would print as is and this prints as U+FFFD.
    fn current_token_string(&self) -> String {
        String::from_utf16_lossy(&self.current_token)
    }

    fn token_equals(&self, token: &str) -> bool {
        self.current_token.iter().copied().eq(token.encode_utf16())
    }

    /// Are we at the end of the input?
    // Java: com/google/protobuf/TextFormat.java:1181 TextFormat.Tokenizer.atEnd
    fn at_end(&self) -> bool {
        self.current_token.is_empty()
    }

    /// Advance to the next token.
    // Java: com/google/protobuf/TextFormat.java:1186 TextFormat.Tokenizer.nextToken
    fn next_token(&mut self) {
        self.previous_line = self.line;
        self.previous_column = self.column;

        // Advance the line counter to the current position.
        while self.line_info_tracking_pos < self.pos {
            if self.text[self.line_info_tracking_pos] == u('\n') {
                self.line += 1;
                self.column = 0;
            } else {
                self.column += 1;
            }
            self.line_info_tracking_pos += 1;
        }

        // Match the next token.
        if self.pos == self.text.len() {
            self.current_token = Vec::new(); // EOF
        } else {
            self.current_token = self.next_token_internal();
            self.skip_whitespace();
        }
    }

    // Java: com/google/protobuf/TextFormat.java:1210 TextFormat.Tokenizer.nextTokenInternal
    fn next_token_internal(&mut self) -> Vec<u16> {
        let text_length = self.text.len();
        let start_pos = self.pos;
        let start_char = self.text[start_pos];

        let mut end_pos = self.pos;
        if is_alpha_under(start_char) {
            // Identifier
            loop {
                end_pos += 1;
                if end_pos == text_length {
                    break;
                }
                let c = self.text[end_pos];
                if !(is_alpha_under(c) || is_digit_plus_minus(c)) {
                    break;
                }
            }
        } else if is_digit_plus_minus(start_char) || start_char == u('.') {
            // Number
            if start_char == u('.') {
                // Optional leading dot
                end_pos += 1;
                if end_pos == text_length {
                    return self.next_token_single_char();
                }

                if !is_digit_plus_minus(self.text[end_pos]) {
                    // Mandatory first digit
                    return self.next_token_single_char();
                }
            }

            loop {
                end_pos += 1;
                if end_pos == text_length {
                    break;
                }
                let c = self.text[end_pos];
                if !(is_digit_plus_minus(c) || is_alpha_under(c) || c == u('.')) {
                    break;
                }
            }
        } else if start_char == u('"') || start_char == u('\'') {
            // String
            loop {
                end_pos += 1;
                if end_pos == text_length {
                    break;
                }
                let c = self.text[end_pos];
                if c == start_char {
                    end_pos += 1;
                    break; // Quote terminates
                } else if c == u('\n') {
                    break; // Newline terminates (error during parsing) (not consumed)
                } else if c == u('\\') {
                    end_pos += 1;
                    if end_pos == text_length {
                        break; // Escape into end-of-text terminates (error during parsing)
                    } else if self.text[end_pos] == u('\n') {
                        break; // Escape into newline terminates (error during parsing) (not consumed)
                    } else {
                        // Otherwise the escaped char is legal and consumed
                    }
                } else {
                    // Otherwise the char is a legal and consumed
                }
            }
        } else {
            return self.next_token_single_char(); // Unrecognized start character
        }

        self.pos = end_pos;
        self.text[start_pos..end_pos].to_vec()
    }

    /// Produce a token for the single char at the current position.
    // Java: com/google/protobuf/TextFormat.java:1291 TextFormat.Tokenizer.nextTokenSingleChar
    fn next_token_single_char(&mut self) -> Vec<u16> {
        let c = self.text[self.pos];
        self.pos += 1;
        vec![c]
    }

    /// Skip over any whitespace so that the matcher region starts at the next token.
    // Java: com/google/protobuf/TextFormat.java:1317 TextFormat.Tokenizer.skipWhitespace
    fn skip_whitespace(&mut self) {
        let text_length = self.text.len();

        let mut end_pos = self.pos;
        while end_pos != text_length {
            let c = self.text[end_pos];
            if c == u('#') {
                loop {
                    end_pos += 1;
                    if end_pos == text_length {
                        break;
                    }
                    if self.text[end_pos] == u('\n') {
                        break; // Consume the newline as whitespace.
                    }
                }
                if end_pos == text_length {
                    break;
                }
            } else if is_whitespace(c) {
                // OK
            } else {
                break;
            }
            end_pos += 1;
        }

        self.pos = end_pos;
    }

    /// If the next token exactly matches `token`, consume it and return `true`. Otherwise,
    /// return `false` without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1347 TextFormat.Tokenizer.tryConsume
    fn try_consume(&mut self, token: &str) -> bool {
        if self.token_equals(token) {
            self.next_token();
            true
        } else {
            false
        }
    }

    /// If the next token exactly matches `token`, consume it. Otherwise, throw a
    /// [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1360 TextFormat.Tokenizer.consume
    fn consume(&mut self, token: &str) -> Result<(), ParseException> {
        if !self.try_consume(token) {
            return Err(self.parse_exception(&format!("Expected \"{token}\".")));
        }
        Ok(())
    }

    // Java: com/google/protobuf/TextFormat.java:1367 TextFormat.Tokenizer.lookingAtInteger
    fn looking_at_integer(&self) -> bool {
        match self.current_token.first() {
            None => false,
            Some(&c) => is_digit_plus_minus(c),
        }
    }

    /// Returns `true` if the current token's text is equal to that specified.
    // Java: com/google/protobuf/TextFormat.java:1376 TextFormat.Tokenizer.lookingAt
    fn looking_at(&self, text: &str) -> bool {
        self.token_equals(text)
    }

    /// If the next token is an identifier, consume it and return its value. Otherwise, throw a
    /// [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1384 TextFormat.Tokenizer.consumeIdentifier
    fn consume_identifier(&mut self) -> Result<String, ParseException> {
        for &c in &self.current_token {
            if is_alpha_under(c) || (u('0') <= c && c <= u('9')) || (c == u('.')) {
                // OK
            } else {
                return Err(self.parse_exception(&format!(
                    "Expected identifier. Found '{}'",
                    self.current_token_string()
                )));
            }
        }

        let result = self.current_token_string();
        self.next_token();
        Ok(result)
    }

    /// If the next token is an identifier, consume it and return `true`. Otherwise, return
    /// `false` without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1403 TextFormat.Tokenizer.tryConsumeIdentifier
    fn try_consume_identifier(&mut self) -> bool {
        self.consume_identifier().is_ok()
    }

    /// If the next token is a 32-bit signed integer, consume it and return its value. Otherwise,
    /// throw a [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1416 TextFormat.Tokenizer.consumeInt32
    fn consume_int32(&mut self) -> Result<i32, ParseException> {
        match parse_int32(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.integer_parse_exception(&e)),
        }
    }

    /// If the next token is a 32-bit unsigned integer, consume it and return its value.
    /// Otherwise, throw a [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1430 TextFormat.Tokenizer.consumeUInt32
    fn consume_uint32(&mut self) -> Result<i32, ParseException> {
        match parse_uint32(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.integer_parse_exception(&e)),
        }
    }

    /// If the next token is a 64-bit signed integer, consume it and return its value. Otherwise,
    /// throw a [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1444 TextFormat.Tokenizer.consumeInt64
    fn consume_int64(&mut self) -> Result<i64, ParseException> {
        match parse_int64(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.integer_parse_exception(&e)),
        }
    }

    /// If the next token is a 64-bit signed integer, consume it and return `true`. Otherwise,
    /// return `false` without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1458 TextFormat.Tokenizer.tryConsumeInt64
    fn try_consume_int64(&mut self) -> bool {
        self.consume_int64().is_ok()
    }

    /// If the next token is a 64-bit unsigned integer, consume it and return its value.
    /// Otherwise, throw a [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1471 TextFormat.Tokenizer.consumeUInt64
    fn consume_uint64(&mut self) -> Result<i64, ParseException> {
        match parse_uint64(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.integer_parse_exception(&e)),
        }
    }

    /// If the next token is a 64-bit unsigned integer, consume it and return `true`. Otherwise,
    /// return `false` without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1485 TextFormat.Tokenizer.tryConsumeUInt64
    fn try_consume_uint64(&mut self) -> bool {
        self.consume_uint64().is_ok()
    }

    /// If the next token is a double, consume it and return its value. Otherwise, throw a
    /// [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1498 TextFormat.Tokenizer.consumeDouble
    fn consume_double(&mut self) -> Result<f64, ParseException> {
        // We need to parse infinity and nan separately because
        // Double.parseDouble() does not accept "inf", "infinity", or "nan".
        // PORT: `toLowerCase(Locale.ROOT)`; only ASCII spellings can match.
        match self.current_token_string().to_ascii_lowercase().as_str() {
            "-inf" | "-infinity" => {
                self.next_token();
                return Ok(f64::NEG_INFINITY);
            }
            "inf" | "infinity" => {
                self.next_token();
                return Ok(f64::INFINITY);
            }
            "nan" => {
                self.next_token();
                return Ok(f64::NAN);
            }
            _ => {
                // fall through
            }
        }

        match java_parse_double(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.float_parse_exception(&e)),
        }
    }

    /// If the next token is a double, consume it and return `true`. Otherwise, return `false`
    /// without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1530 TextFormat.Tokenizer.tryConsumeDouble
    fn try_consume_double(&mut self) -> bool {
        self.consume_double().is_ok()
    }

    /// If the next token is a float, consume it and return its value. Otherwise, throw a
    /// [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1543 TextFormat.Tokenizer.consumeFloat
    fn consume_float(&mut self) -> Result<f32, ParseException> {
        // We need to parse infinity and nan separately because
        // Float.parseFloat() does not accept "inf", "infinity", or "nan".
        match self.current_token_string().to_ascii_lowercase().as_str() {
            "-inf" | "-inff" | "-infinity" | "-infinityf" => {
                self.next_token();
                return Ok(f32::NEG_INFINITY);
            }
            "inf" | "inff" | "infinity" | "infinityf" => {
                self.next_token();
                return Ok(f32::INFINITY);
            }
            "nan" | "nanf" => {
                self.next_token();
                return Ok(f32::NAN);
            }
            _ => {
                // fall through
            }
        }

        match java_parse_float(&self.current_token_string()) {
            Ok(result) => {
                self.next_token();
                Ok(result)
            }
            Err(e) => Err(self.float_parse_exception(&e)),
        }
    }

    /// If the next token is a boolean, consume it and return its value. Otherwise, throw a
    /// [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1591 TextFormat.Tokenizer.consumeBoolean
    fn consume_boolean(&mut self) -> Result<bool, ParseException> {
        if self.token_equals("true")
            || self.token_equals("True")
            || self.token_equals("t")
            || self.token_equals("1")
        {
            self.next_token();
            Ok(true)
        } else if self.token_equals("false")
            || self.token_equals("False")
            || self.token_equals("f")
            || self.token_equals("0")
        {
            self.next_token();
            Ok(false)
        } else {
            Err(self.parse_exception(&format!(
                "Expected \"true\" or \"false\". Found \"{}\".",
                self.current_token_string()
            )))
        }
    }

    /// If the next token is a float, consume it and return `true`. Otherwise, return `false`
    /// without doing anything.
    // Java: com/google/protobuf/TextFormat.java:1580 TextFormat.Tokenizer.tryConsumeFloat
    fn try_consume_float(&mut self) -> bool {
        self.consume_float().is_ok()
    }

    /// If the next token is a string, consume it and return its (unescaped) value. Otherwise,
    /// throw a [`ParseException`].
    ///
    /// PORT: `ByteString.toStringUtf8()` replaces malformed UTF-8 with U+FFFD per maximal
    /// subpart, as `String::from_utf8_lossy` does.
    // Java: com/google/protobuf/TextFormat.java:1615 TextFormat.Tokenizer.consumeString
    fn consume_string(&mut self) -> Result<String, ParseException> {
        Ok(String::from_utf8_lossy(&self.consume_byte_string()?).into_owned())
    }

    /// If the next token is a string, consume it, unescape it as bytes, and return it.
    /// Otherwise, throw a [`ParseException`].
    // Java: com/google/protobuf/TextFormat.java:1624 TextFormat.Tokenizer.consumeByteString
    fn consume_byte_string(&mut self) -> Result<Vec<u8>, ParseException> {
        let mut list: Vec<Vec<u8>> = Vec::new();
        self.consume_byte_string_into(&mut list)?;
        while self.current_token.first() == Some(&u('\''))
            || self.current_token.first() == Some(&u('"'))
        {
            self.consume_byte_string_into(&mut list)?;
        }
        Ok(list.concat())
    }

    /// If the next token is a string, consume it and return true. Otherwise, return false.
    // Java: com/google/protobuf/TextFormat.java:1634 TextFormat.Tokenizer.tryConsumeByteString
    fn try_consume_byte_string(&mut self) -> bool {
        self.consume_byte_string().is_ok()
    }

    /// Like [`Self::consume_byte_string`] but adds each token of the string to the given list.
    /// String literals (whether bytes or text) may come in multiple adjacent tokens which are
    /// automatically concatenated, like in C or Python.
    // Java: com/google/protobuf/TextFormat.java:1648 TextFormat.Tokenizer.consumeByteString
    fn consume_byte_string_into(&mut self, list: &mut Vec<Vec<u8>>) -> Result<(), ParseException> {
        let quote = self.current_token.first().copied().unwrap_or(0);
        if quote != u('"') && quote != u('\'') {
            return Err(self.parse_exception("Expected string."));
        }

        if self.current_token.len() < 2 || self.current_token[self.current_token.len() - 1] != quote
        {
            return Err(self.parse_exception("String missing ending quote."));
        }

        let escaped =
            String::from_utf16_lossy(&self.current_token[1..self.current_token.len() - 1]);
        match unescape_bytes(&escaped) {
            Ok(result) => {
                self.next_token();
                list.push(result);
                Ok(())
            }
            Err(e) => Err(self.parse_exception(&e.0)),
        }
    }

    /// Returns a [`ParseException`] with the current line and column numbers in the
    /// description, suitable for throwing.
    // Java: com/google/protobuf/TextFormat.java:1672 TextFormat.Tokenizer.parseException
    fn parse_exception(&self, description: &str) -> ParseException {
        // Note:  People generally prefer one-based line and column numbers.
        ParseException::new(self.line + 1, self.column + 1, description)
    }

    /// Returns a [`ParseException`] with the line and column numbers of the previous token in
    /// the description, suitable for throwing.
    // Java: com/google/protobuf/TextFormat.java:1681 TextFormat.Tokenizer.parseExceptionPreviousToken
    fn parse_exception_previous_token(&self, description: &str) -> ParseException {
        // Note:  People generally prefer one-based line and column numbers.
        ParseException::new(
            self.previous_line + 1,
            self.previous_column + 1,
            description,
        )
    }

    /// Constructs an appropriate [`ParseException`] for the given `NumberFormatException` when
    /// trying to parse an integer.
    // Java: com/google/protobuf/TextFormat.java:1690 TextFormat.Tokenizer.integerParseException
    fn integer_parse_exception(&self, e: &NumberFormatException) -> ParseException {
        self.parse_exception(&format!("Couldn't parse integer: {}", e.0))
    }

    /// Constructs an appropriate [`ParseException`] for the given `NumberFormatException` when
    /// trying to parse a float or double.
    // Java: com/google/protobuf/TextFormat.java:1698 TextFormat.Tokenizer.floatParseException
    fn float_parse_exception(&self, e: &NumberFormatException) -> ParseException {
        self.parse_exception(&format!("Couldn't parse number: {}", e.0))
    }
}

// Java: com/google/protobuf/TextFormat.java:1268 TextFormat.Tokenizer.isAlphaUnder
fn is_alpha_under(c: u16) -> bool {
    // Defining this char-class with numeric comparisons is much faster than using a regex.
    (u('a') <= c && c <= u('z')) || (u('A') <= c && c <= u('Z')) || c == u('_')
}

// Java: com/google/protobuf/TextFormat.java:1273 TextFormat.Tokenizer.isDigitPlusMinus
fn is_digit_plus_minus(c: u16) -> bool {
    // Defining this char-class with numeric comparisons is much faster than using a regex.
    (u('0') <= c && c <= u('9')) || c == u('+') || c == u('-')
}

// Java: com/google/protobuf/TextFormat.java:1278 TextFormat.Tokenizer.isWhitespace
fn is_whitespace(c: u16) -> bool {
    // Defining this char-class with numeric comparisons is much faster than using a regex.
    c == u(' ') || c == 0x0C || c == u('\n') || c == u('\r') || c == u('\t')
}

// ---------------------------------------------------------------------------------------------
// Parser.

/// `Parser.UnknownField.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnknownFieldType {
    Field,
    Extension,
}

// Java: com/google/protobuf/TextFormat.java:2062 TextFormat.Parser.UnknownField
struct UnknownField {
    message: String,
    type_: UnknownFieldType,
}

/// Determines if repeated values for non-repeated fields and oneofs are permitted.
// Java: com/google/protobuf/TextFormat.java:1879 TextFormat.Parser.SingularOverwritePolicy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingularOverwritePolicy {
    /// Later values are merged with earlier values. For primitive fields or conflicting oneofs,
    /// the last value is retained.
    AllowSingularOverwrites,
    /// An error is issued.
    ForbidSingularOverwrites,
}

/// Parser for text-format proto2 instances. This class is thread-safe. The implementation
/// largely follows google/protobuf/text_format.cc.
///
/// Use [`get_parser`] to obtain the default parser, or [`ParserBuilder`] to control the parser
/// behavior.
///
/// PORT: `parseInfoTreeBuilder` (`TextFormatParseInfoTree`) and extension registries are not
/// ported: Blaze registers no extension, so `[name]` fields are always unknown extensions, as
/// with `ExtensionRegistry.getEmptyRegistry()`. `detectSilentMarker` is empty in this version.
/// The warnings Java logs (`java.util.logging`) when unknown fields or enum values are allowed
/// are not written anywhere.
// Java: com/google/protobuf/TextFormat.java:1859 TextFormat.Parser
#[derive(Clone, Debug)]
pub struct Parser {
    type_registry: TypeRegistry,
    allow_unknown_fields: bool,
    allow_unknown_enum_values: bool,
    allow_unknown_extensions: bool,
    singular_overwrite_policy: SingularOverwritePolicy,
    recursion_limit: i32,
}

/// Builder that can be used to obtain new instances of [`Parser`].
// Java: com/google/protobuf/TextFormat.java:1933 TextFormat.Parser.Builder
#[derive(Clone, Debug)]
pub struct ParserBuilder {
    allow_unknown_fields: bool,
    allow_unknown_enum_values: bool,
    allow_unknown_extensions: bool,
    singular_overwrite_policy: SingularOverwritePolicy,
    type_registry: TypeRegistry,
    recursion_limit: i32,
}

impl ParserBuilder {
    /// Sets the TypeRegistry for resolving Any.
    pub fn set_type_registry(mut self, type_registry: TypeRegistry) -> Self {
        self.type_registry = type_registry;
        self
    }

    /// Set whether this parser will allow unknown fields.
    pub fn set_allow_unknown_fields(mut self, allow_unknown_fields: bool) -> Self {
        self.allow_unknown_fields = allow_unknown_fields;
        self
    }

    /// Set whether this parser will allow unknown extensions.
    pub fn set_allow_unknown_extensions(mut self, allow_unknown_extensions: bool) -> Self {
        self.allow_unknown_extensions = allow_unknown_extensions;
        self
    }

    /// Sets parser behavior when a non-repeated field appears more than once.
    pub fn set_singular_overwrite_policy(mut self, p: SingularOverwritePolicy) -> Self {
        self.singular_overwrite_policy = p;
        self
    }

    /// Set the maximum recursion limit that the parser will allow.
    pub fn set_recursion_limit(mut self, recursion_limit: i32) -> Self {
        self.recursion_limit = recursion_limit;
        self
    }

    /// `build()`.
    pub fn build(self) -> Parser {
        Parser {
            type_registry: self.type_registry,
            allow_unknown_fields: self.allow_unknown_fields,
            allow_unknown_enum_values: self.allow_unknown_enum_values,
            allow_unknown_extensions: self.allow_unknown_extensions,
            singular_overwrite_policy: self.singular_overwrite_policy,
            recursion_limit: self.recursion_limit,
        }
    }
}

/// Return a [`Parser`] instance which can parse text-format messages
/// (`Parser.newBuilder().build()`).
// Java: com/google/protobuf/TextFormat.java:1790 TextFormat.getParser
pub fn get_parser() -> Parser {
    Parser::new_builder().build()
}

/// Parse a text-format message from `input` and merge the contents into `builder`.
// Java: com/google/protobuf/TextFormat.java:1801 TextFormat.merge
pub fn merge<M: JavaMessage>(input: &str, builder: &mut M) -> Result<(), ParseException> {
    get_parser().merge(input, builder)
}

/// Parse a text-format message from `input`.
///
/// `builder.build()` throws the unchecked `UninitializedMessageException` when a required field
/// is missing; that is a panic here.
// Java: com/google/protobuf/TextFormat.java:1811 TextFormat.parse
pub fn parse<M: JavaMessage>(input: &str) -> Result<M, ParseException> {
    let descriptor = descriptor_of::<M>();
    let mut target = DynamicMessage::new(descriptor);
    get_parser().merge_dynamic(input, &mut target)?;
    let missing = target.find_missing_fields();
    if !missing.is_empty() {
        panic!(
            "UninitializedMessageException: Message missing required fields: {}",
            missing.join(", ")
        );
    }
    Ok(target.into_message())
}

fn descriptor_of<M: JavaMessage>() -> &'static Descriptor {
    pool()
        .find_message_type_by_name(M::FULL_NAME)
        .unwrap_or_else(|| panic!("no descriptor for {}", M::FULL_NAME))
}

/// `TextFormat.getParser().merge(input, platformBuilder)` for a
/// `build.bazel.remote.execution.v2.Platform.Builder`: parses a text-format `Platform` from
/// `input` and merges it into `builder` (repeated `properties` are appended).
// Java: com/google/protobuf/TextFormat.java:2019 TextFormat.Parser.merge
pub fn merge_platform(input: &str, builder: &mut Platform) -> Result<(), ParseException> {
    get_parser().merge(input, builder)
}

impl Parser {
    /// Returns a new instance of [`ParserBuilder`].
    pub fn new_builder() -> ParserBuilder {
        ParserBuilder {
            allow_unknown_fields: false,
            allow_unknown_enum_values: false,
            allow_unknown_extensions: false,
            singular_overwrite_policy: SingularOverwritePolicy::AllowSingularOverwrites,
            type_registry: TypeRegistry::get_empty_type_registry(),
            recursion_limit: RECURSION_LIMIT,
        }
    }

    /// Parse a text-format message from `input` and merge the contents into `builder`.
    ///
    /// PORT: the parse goes into a [`DynamicMessage`] holding `builder`'s fields and replaces
    /// `builder` at the end; Java writes into the builder as it goes, so on a `ParseException`
    /// Java's builder holds the fields parsed so far and this one is unchanged.
    // Java: com/google/protobuf/TextFormat.java:2019 TextFormat.Parser.merge
    pub fn merge<M: JavaMessage>(
        &self,
        input: &str,
        builder: &mut M,
    ) -> Result<(), ParseException> {
        let mut target = DynamicMessage::from_message(builder);
        self.merge_dynamic(input, &mut target)?;
        *builder = target.into_message();
        Ok(())
    }

    /// Parse a text-format message from `input` and merge the contents into `target`.
    // Java: com/google/protobuf/TextFormat.java:2119 TextFormat.Parser.merge
    pub fn merge_dynamic(
        &self,
        input: &str,
        target: &mut DynamicMessage,
    ) -> Result<(), ParseException> {
        let text: Vec<u16> = input.encode_utf16().collect();
        let mut tokenizer = Tokenizer::new(&text);
        let mut unknown_fields: Vec<UnknownField> = Vec::new();

        while !tokenizer.at_end() {
            self.merge_field(
                &mut tokenizer,
                target,
                &mut unknown_fields,
                self.recursion_limit,
            )?;
        }
        self.check_unknown_fields(&unknown_fields)
    }

    /// Check both unknown fields and unknown extensions and log warning messages or throw
    /// exceptions according to the flag.
    // Java: com/google/protobuf/TextFormat.java:2079 TextFormat.Parser.checkUnknownFields
    fn check_unknown_fields(&self, unknown_fields: &[UnknownField]) -> Result<(), ParseException> {
        if unknown_fields.is_empty() {
            return Ok(());
        }

        let mut msg = String::from("Input contains unknown fields and/or extensions:");
        for field in unknown_fields {
            msg.push('\n');
            msg.push_str(&field.message);
        }

        if self.allow_unknown_fields {
            // logger.warning(msg)
            return Ok(());
        }

        let mut first_error_index = 0;
        if self.allow_unknown_extensions {
            let mut all_unknown_extensions = true;
            for field in unknown_fields {
                if field.type_ == UnknownFieldType::Field {
                    all_unknown_extensions = false;
                    break;
                }
                first_error_index += 1;
            }
            if all_unknown_extensions {
                // logger.warning(msg)
                return Ok(());
            }
        }

        let line_column: Vec<&str> = unknown_fields[first_error_index]
            .message
            .split(':')
            .collect();
        Err(ParseException::new(
            line_column[0].parse::<i32>().expect("Integer.parseInt"),
            line_column[1].parse::<i32>().expect("Integer.parseInt"),
            &msg,
        ))
    }

    /// Parse a single field from `tokenizer` and merge it into `target`.
    // Java: com/google/protobuf/TextFormat.java:2152 TextFormat.Parser.mergeField
    fn merge_field(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        target: &mut DynamicMessage,
        unknown_fields: &mut Vec<UnknownField>,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        let mut field: Option<&'static FieldDescriptor> = None;
        let name: String;
        let type_ = target.get_descriptor_for_type();

        if type_.full_name == "google.protobuf.Any" && tokenizer.try_consume("[") {
            if recursion_limit < 1 {
                return Err(tokenizer.parse_exception("Message is nested too deep"));
            }
            return self.merge_any_field_value(
                tokenizer,
                target,
                unknown_fields,
                type_,
                recursion_limit - 1,
            );
        }

        if tokenizer.try_consume("[") {
            // An extension.
            let mut name_builder = tokenizer.consume_identifier()?;
            while tokenizer.try_consume(".") {
                name_builder.push('.');
                name_builder.push_str(&tokenizer.consume_identifier()?);
            }
            name = name_builder;

            // `target.findExtensionByName(extensionRegistry, name)`: no extension is registered.
            let message = format!(
                "{}:{}:\t{}.[{}]",
                tokenizer.get_previous_line() + 1,
                tokenizer.get_previous_column() + 1,
                type_.full_name,
                name
            );
            unknown_fields.push(UnknownField {
                message,
                type_: UnknownFieldType::Extension,
            });

            tokenizer.consume("]")?;
        } else {
            name = tokenizer.consume_identifier()?;
            field = type_.find_field_by_name(&name);

            // Group names are expected to be capitalized as they appear in the
            // .proto file, which actually matches their type names, not their field
            // names.
            if field.is_none() {
                // Explicitly specify US locale so that this code does not break when
                // executing in Turkey.
                // PORT: an identifier is ASCII ([A-Za-z0-9_.]), where `toLowerCase(Locale.US)`
                // is ASCII lowercasing.
                let lower_name = name.to_ascii_lowercase();
                field = type_.find_field_by_name(&lower_name);
                // If the case-insensitive match worked but the field is NOT a group,
                if field.is_some_and(|f| !f.is_group_like()) {
                    field = None;
                }
                if field.is_some_and(|f| f.get_message_type().name != name) {
                    field = None;
                }
            }

            if field.is_none() {
                let message = format!(
                    "{}:{}:\t{}.{}",
                    tokenizer.get_previous_line() + 1,
                    tokenizer.get_previous_column() + 1,
                    type_.full_name,
                    name
                );
                unknown_fields.push(UnknownField {
                    message,
                    type_: UnknownFieldType::Field,
                });
            }
        }

        // Skips unknown fields.
        let Some(field) = field else {
            self.guess_field_type_and_skip(tokenizer, type_, recursion_limit)?;
            return Ok(());
        };

        // Handle potential ':'.
        if field.get_java_type() == JavaType::Message {
            tokenizer.try_consume(":"); // optional
            self.consume_field_values(tokenizer, target, field, unknown_fields, recursion_limit)?;
        } else {
            tokenizer.consume(":")?; // required
            self.consume_field_values(tokenizer, target, field, unknown_fields, recursion_limit)?;
        }

        // For historical reasons, fields may optionally be separated by commas or
        // semicolons.
        if !tokenizer.try_consume(";") {
            tokenizer.try_consume(",");
        }
        Ok(())
    }

    /// Parse a one or more field values from `tokenizer` and merge it into `target`.
    // Java: com/google/protobuf/TextFormat.java:2335 TextFormat.Parser.consumeFieldValues
    fn consume_field_values(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        target: &mut DynamicMessage,
        field: &'static FieldDescriptor,
        unknown_fields: &mut Vec<UnknownField>,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        // Support specifying repeated field values as a comma-separated list.
        // Ex."foo: [1, 2, 3]"
        if field.is_repeated() && tokenizer.try_consume("[") {
            if !tokenizer.try_consume("]") {
                // Allow "foo: []" to be treated as empty.
                loop {
                    self.consume_field_value(
                        tokenizer,
                        target,
                        field,
                        unknown_fields,
                        recursion_limit,
                    )?;
                    if tokenizer.try_consume("]") {
                        // End of list.
                        break;
                    }
                    tokenizer.consume(",")?;
                }
            }
        } else {
            self.consume_field_value(tokenizer, target, field, unknown_fields, recursion_limit)?;
        }
        Ok(())
    }

    /// Parse a single field value from `tokenizer` and merge it into `target`.
    // Java: com/google/protobuf/TextFormat.java:2380 TextFormat.Parser.consumeFieldValue
    fn consume_field_value(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        target: &mut DynamicMessage,
        field: &'static FieldDescriptor,
        unknown_fields: &mut Vec<UnknownField>,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        if self.singular_overwrite_policy == SingularOverwritePolicy::ForbidSingularOverwrites
            && !field.is_repeated()
        {
            if target.has_field(field) {
                return Err(tokenizer.parse_exception_previous_token(&format!(
                    "Non-repeated field \"{}\" cannot be overwritten.",
                    field.full_name
                )));
            } else if let Some(oneof) = field.get_containing_oneof()
                && target.has_oneof(oneof.index)
            {
                return Err(tokenizer.parse_exception_previous_token(&format!(
                    "Field \"{}\" is specified along with field \"{}\", another member of oneof \"{}\".",
                    field.full_name,
                    target
                        .get_oneof_field_descriptor(oneof.index)
                        .expect("hasOneof")
                        .full_name,
                    oneof.name
                )));
            }
        }

        let value = if field.get_java_type() == JavaType::Message {
            if recursion_limit < 1 {
                return Err(tokenizer.parse_exception("Message is nested too deep"));
            }

            let end_token = if tokenizer.try_consume("<") {
                ">"
            } else {
                tokenizer.consume("{")?;
                "}"
            };

            let mut sub_field = target.new_merge_target_for_field(field);

            while !tokenizer.try_consume(end_token) {
                if tokenizer.at_end() {
                    return Err(tokenizer.parse_exception(&format!("Expected \"{end_token}\".")));
                }
                self.merge_field(
                    tokenizer,
                    &mut sub_field,
                    unknown_fields,
                    recursion_limit - 1,
                )?;
            }

            Value::Message(sub_field)
        } else {
            match field.type_ {
                Type::Int32 | Type::Sint32 | Type::Sfixed32 => {
                    Value::Int(tokenizer.consume_int32()?)
                }
                Type::Int64 | Type::Sint64 | Type::Sfixed64 => {
                    Value::Long(tokenizer.consume_int64()?)
                }
                Type::Uint32 | Type::Fixed32 => Value::Int(tokenizer.consume_uint32()?),
                Type::Uint64 | Type::Fixed64 => Value::Long(tokenizer.consume_uint64()?),
                Type::Float => Value::Float(tokenizer.consume_float()?),
                Type::Double => Value::Double(tokenizer.consume_double()?),
                Type::Bool => Value::Boolean(tokenizer.consume_boolean()?),
                Type::String => Value::String(tokenizer.consume_string()?.into_bytes()),
                Type::Bytes => Value::Bytes(tokenizer.consume_byte_string()?),
                Type::Enum => {
                    let enum_type = field.get_enum_type();

                    if tokenizer.looking_at_integer() {
                        let number = tokenizer.consume_int32()?;
                        // `findValueByNumberCreatingIfUnknown` for an open enum.
                        let known = enum_type.find_value_by_number(number).is_some();
                        if enum_type.is_closed() && !known {
                            let unknown_value_msg = format!(
                                "Enum type \"{}\" has no value with number {}.",
                                enum_type.full_name, number
                            );
                            if self.allow_unknown_enum_values {
                                // logger.warning(unknownValueMsg)
                                return Ok(());
                            } else {
                                return Err(
                                    tokenizer.parse_exception_previous_token(&unknown_value_msg)
                                );
                            }
                        }
                        Value::Enum(number)
                    } else {
                        let id = tokenizer.consume_identifier()?;
                        match enum_type.find_value_by_name(&id) {
                            Some(v) => Value::Enum(v.number),
                            None => {
                                let unknown_value_msg = format!(
                                    "Enum type \"{}\" has no value named \"{}\".",
                                    enum_type.full_name, id
                                );
                                if self.allow_unknown_enum_values {
                                    // logger.warning(unknownValueMsg)
                                    return Ok(());
                                } else {
                                    return Err(tokenizer
                                        .parse_exception_previous_token(&unknown_value_msg));
                                }
                            }
                        }
                    }
                }
                Type::Message | Type::Group => panic!("RuntimeException: Can't get here."),
            }
        };

        if field.is_repeated() {
            // TODO: If field.isMapField() and FORBID_SINGULAR_OVERWRITES mode,
            //     check for duplicate map keys here.
            target.add_repeated_field(field, value);
        } else {
            target.set_field(field, value);
        }
        Ok(())
    }

    // Java: com/google/protobuf/TextFormat.java:2548 TextFormat.Parser.mergeAnyFieldValue
    fn merge_any_field_value(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        target: &mut DynamicMessage,
        unknown_fields: &mut Vec<UnknownField>,
        any_descriptor: &'static Descriptor,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        // Try to parse human readable format of Any in the form: [type_url]: { ... }
        let mut type_url_builder = String::new();
        // Parse the type_url inside [].
        loop {
            type_url_builder.push_str(&tokenizer.consume_identifier()?);
            if tokenizer.try_consume("]") {
                break;
            }
            if tokenizer.try_consume("/") {
                type_url_builder.push('/');
            } else if tokenizer.try_consume(".") {
                type_url_builder.push('.');
            } else {
                return Err(tokenizer.parse_exception_previous_token("Expected a valid type URL."));
            }
        }
        tokenizer.try_consume(":");
        let any_end_token = if tokenizer.try_consume("<") {
            ">"
        } else {
            tokenizer.consume("{")?;
            "}"
        };
        let type_url = type_url_builder;
        let content_type = match self.type_registry.get_descriptor_for_type_url(&type_url) {
            Ok(c) => c,
            Err(_) => {
                return Err(tokenizer
                    .parse_exception(&format!("Invalid valid type URL. Found: {type_url}")));
            }
        };
        let Some(content_type) = content_type else {
            return Err(tokenizer.parse_exception(&format!(
                "Unable to parse Any of type: {type_url}. Please make sure that the TypeRegistry \
                 contains the descriptors for the given types."
            )));
        };
        let mut content_builder = DynamicMessage::new(content_type);
        while !tokenizer.try_consume(any_end_token) {
            self.merge_field(
                tokenizer,
                &mut content_builder,
                unknown_fields,
                recursion_limit,
            )?;
        }

        let type_url_field = any_descriptor
            .find_field_by_name("type_url")
            .expect("Any.type_url");
        let value_field = any_descriptor
            .find_field_by_name("value")
            .expect("Any.value");
        target.set_field(type_url_field, Value::String(type_url.into_bytes()));
        target.set_field(value_field, Value::Bytes(content_builder.to_byte_array()));
        Ok(())
    }

    /// Skips the next field including the field's name and value.
    // Java: com/google/protobuf/TextFormat.java:2618 TextFormat.Parser.skipField
    fn skip_field(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        type_: &'static Descriptor,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        let _name = consume_full_type_name(tokenizer)?;
        self.guess_field_type_and_skip(tokenizer, type_, recursion_limit)?;

        // For historical reasons, fields may optionally be separated by commas or
        // semicolons.
        if !tokenizer.try_consume(";") {
            tokenizer.try_consume(",");
        }
        Ok(())
    }

    /// Skips the whole body of a message including the beginning delimiter and the ending
    /// delimiter.
    // Java: com/google/protobuf/TextFormat.java:2634 TextFormat.Parser.skipFieldMessage
    fn skip_field_message(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        type_: &'static Descriptor,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        let delimiter = if tokenizer.try_consume("<") {
            ">"
        } else {
            tokenizer.consume("{")?;
            "}"
        };
        while !tokenizer.looking_at(">") && !tokenizer.looking_at("}") {
            self.skip_field(tokenizer, type_, recursion_limit)?;
        }
        tokenizer.consume(delimiter)
    }

    /// Tries to guess the type of this field and skip it.
    ///
    /// If this field is not a message, there should be a ":" between the field name and the
    /// field value and also the field value should not start with "{" or "<" which indicates
    /// the beginning of a message body. If there is no ":" or there is a "{" or "<" after ":",
    /// this field has to be a message or the input is ill-formed. For short-formed repeated
    /// fields (i.e. with "[]"), if it is repeated scalar, there must be a ":" between the field
    /// name and the starting "[" .
    // Java: com/google/protobuf/TextFormat.java:2670 TextFormat.Parser.guessFieldTypeAndSkip
    fn guess_field_type_and_skip(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        type_: &'static Descriptor,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        let semicolon_consumed = tokenizer.try_consume(":");
        if tokenizer.looking_at("[") {
            // Short repeated field form. If a semicolon was consumed, it could be repeated
            // scalar or repeated message. If not, it must be repeated message.
            self.skip_field_short_formed_repeated(
                tokenizer,
                semicolon_consumed,
                type_,
                recursion_limit,
            )
        } else if semicolon_consumed && !tokenizer.looking_at("{") && !tokenizer.looking_at("<") {
            skip_field_value(tokenizer)
        } else {
            if recursion_limit < 1 {
                return Err(tokenizer.parse_exception("Message is nested too deep"));
            }
            self.skip_field_message(tokenizer, type_, recursion_limit - 1)
        }
    }

    /// Skips a short-formed repeated field value.
    ///
    /// Reports an error if scalar type is not allowed but showing up inside "[]".
    // Java: com/google/protobuf/TextFormat.java:2692 TextFormat.Parser.skipFieldShortFormedRepeated
    fn skip_field_short_formed_repeated(
        &self,
        tokenizer: &mut Tokenizer<'_>,
        scalar_allowed: bool,
        type_: &'static Descriptor,
        recursion_limit: i32,
    ) -> Result<(), ParseException> {
        if !tokenizer.try_consume("[") || tokenizer.try_consume("]") {
            // Try skipping "[]".
            return Ok(());
        }

        loop {
            if tokenizer.looking_at("{") || tokenizer.looking_at("<") {
                // Try skipping message field inside "[]"
                if recursion_limit < 1 {
                    return Err(tokenizer.parse_exception("Message is nested too deep"));
                }
                self.skip_field_message(tokenizer, type_, recursion_limit - 1)?;
            } else if scalar_allowed {
                // Try skipping scalar field inside "[]".
                skip_field_value(tokenizer)?;
            } else {
                return Err(tokenizer.parse_exception(
                    "Invalid repeated scalar field: missing \":\" before \"[\".",
                ));
            }
            if tokenizer.try_consume("]") {
                break;
            }
            tokenizer.consume(",")?;
        }
        Ok(())
    }
}

// Java: com/google/protobuf/TextFormat.java:2310 TextFormat.Parser.consumeFullTypeName
fn consume_full_type_name(tokenizer: &mut Tokenizer<'_>) -> Result<String, ParseException> {
    // If there is not a leading `[`, this is just a type name.
    if !tokenizer.try_consume("[") {
        return tokenizer.consume_identifier();
    }

    // Otherwise, this is an extension or google.protobuf.Any type URL: we consume proto path
    // elements until we've addressed the type.
    let mut name = tokenizer.consume_identifier()?;
    while tokenizer.try_consume(".") {
        name = format!("{name}.{}", tokenizer.consume_identifier()?);
    }
    if tokenizer.try_consume("/") {
        name = format!("{name}/{}", tokenizer.consume_identifier()?);
        while tokenizer.try_consume(".") {
            name = format!("{name}.{}", tokenizer.consume_identifier()?);
        }
    }
    tokenizer.consume("]")?;
    Ok(name)
}

/// Skips a field value.
// Java: com/google/protobuf/TextFormat.java:2650 TextFormat.Parser.skipFieldValue
fn skip_field_value(tokenizer: &mut Tokenizer<'_>) -> Result<(), ParseException> {
    if !tokenizer.try_consume_byte_string()
        && !tokenizer.try_consume_identifier() // includes enum & boolean
        && !tokenizer.try_consume_int64() // includes int32
        && !tokenizer.try_consume_uint64() // includes uint32
        && !tokenizer.try_consume_double()
        && !tokenizer.try_consume_float()
    {
        return Err(tokenizer.parse_exception(&format!(
            "Invalid field value: {}",
            tokenizer.current_token_string()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Utility functions.

/// Un-escape a byte sequence as escaped using `escapeBytes`. Two-digit hex escapes (starting
/// with "\x") are also recognized.
// Java: com/google/protobuf/TextFormat.java:2747 TextFormat.unescapeBytes
#[allow(clippy::needless_range_loop)] // Java's loops, kept as written.
pub fn unescape_bytes(char_string: &str) -> Result<Vec<u8>, InvalidEscapeSequenceException> {
    // First convert the Java character sequence to UTF-8 bytes.
    let input = char_string.as_bytes();
    // Then unescape certain byte sequences introduced by ASCII '\\'.  The valid
    // escapes can all be expressed with ASCII characters, so it is safe to
    // operate on bytes here.
    //
    // Unescaping the input byte array will result in a byte sequence that's no
    // longer than the input.  That's because each escape sequence is between
    // two and four bytes long and stands for a single byte.
    let mut result: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let mut c = input[i];
        if c == b'\\' {
            if i + 1 < input.len() {
                i += 1;
                c = input[i];
                if is_octal(c) {
                    // Octal escape.
                    let mut code = digit_value(c);
                    if i + 1 < input.len() && is_octal(input[i + 1]) {
                        i += 1;
                        code = code * 8 + digit_value(input[i]);
                    }
                    if i + 1 < input.len() && is_octal(input[i + 1]) {
                        i += 1;
                        code = code * 8 + digit_value(input[i]);
                    }
                    // TODO: Check that 0 <= code && code <= 0xFF.
                    result.push(code as u8);
                } else {
                    match c {
                        b'a' => result.push(0x07),
                        b'b' => result.push(0x08),
                        b'f' => result.push(0x0C),
                        b'n' => result.push(b'\n'),
                        b'r' => result.push(b'\r'),
                        b't' => result.push(b'\t'),
                        b'v' => result.push(0x0b),
                        b'\\' => result.push(b'\\'),
                        b'\'' => result.push(b'\''),
                        b'"' => result.push(b'"'),
                        b'?' => result.push(b'?'),

                        b'x' => {
                            // hex escape
                            let mut code;
                            if i + 1 < input.len() && is_hex(input[i + 1]) {
                                i += 1;
                                code = digit_value(input[i]);
                            } else {
                                return Err(InvalidEscapeSequenceException(
                                    "Invalid escape sequence: '\\x' with no digits".to_string(),
                                ));
                            }
                            if i + 1 < input.len() && is_hex(input[i + 1]) {
                                i += 1;
                                code = code * 16 + digit_value(input[i]);
                            }
                            result.push(code as u8);
                        }

                        b'u' => {
                            // Unicode escape
                            i += 1;
                            if i + 3 < input.len()
                                && is_hex(input[i])
                                && is_hex(input[i + 1])
                                && is_hex(input[i + 2])
                                && is_hex(input[i + 3])
                            {
                                let ch = (digit_value(input[i]) << 12)
                                    | (digit_value(input[i + 1]) << 8)
                                    | (digit_value(input[i + 2]) << 4)
                                    | digit_value(input[i + 3]);

                                if (0xD800..=0xDFFF).contains(&ch) {
                                    return Err(InvalidEscapeSequenceException(
                                        "Invalid escape sequence: '\\u' refers to a surrogate"
                                            .to_string(),
                                    ));
                                }
                                let ch = char::from_u32(ch).expect("not a surrogate");
                                let mut buf = [0u8; 4];
                                result.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                i += 3;
                            } else {
                                return Err(InvalidEscapeSequenceException(
                                    "Invalid escape sequence: '\\u' with too few hex chars"
                                        .to_string(),
                                ));
                            }
                        }

                        b'U' => {
                            // Unicode escape
                            i += 1;
                            if i + 7 >= input.len() {
                                return Err(InvalidEscapeSequenceException(
                                    "Invalid escape sequence: '\\U' with too few hex chars"
                                        .to_string(),
                                ));
                            }
                            // Java: an `int`; eight hex digits fill it exactly.
                            let mut codepoint: u32 = 0;
                            for offset in i..i + 8 {
                                let b = input[offset];
                                if !is_hex(b) {
                                    return Err(InvalidEscapeSequenceException(
                                        "Invalid escape sequence: '\\U' with too few hex chars"
                                            .to_string(),
                                    ));
                                }
                                codepoint = (codepoint << 4) | digit_value(b);
                            }
                            let digits = String::from_utf8_lossy(&input[i..i + 8]);
                            // `Character.isValidCodePoint` (a negative int is not one).
                            if codepoint > 0x10FFFF {
                                return Err(InvalidEscapeSequenceException(format!(
                                    "Invalid escape sequence: '\\U{digits}' is not a valid code \
                                     point value"
                                )));
                            }
                            // `Character.UnicodeBlock` HIGH_SURROGATES, HIGH_PRIVATE_USE_SURROGATES
                            // and LOW_SURROGATES together are U+D800..U+DFFF.
                            if (0xD800..=0xDFFF).contains(&codepoint) {
                                return Err(InvalidEscapeSequenceException(format!(
                                    "Invalid escape sequence: '\\U{digits}' refers to a surrogate \
                                     code unit"
                                )));
                            }
                            let ch = char::from_u32(codepoint).expect("valid code point");
                            let mut buf = [0u8; 4];
                            result.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            i += 7;
                        }

                        _ => {
                            // Java: `(char) c` of a negative byte is U+FF80..U+FFFF.
                            let ch = if c < 0x80 {
                                c as char
                            } else {
                                char::from_u32(0xFF00 | c as u32).expect("BMP char")
                            };
                            return Err(InvalidEscapeSequenceException(format!(
                                "Invalid escape sequence: '\\{ch}'"
                            )));
                        }
                    }
                }
            } else {
                return Err(InvalidEscapeSequenceException(
                    "Invalid escape sequence: '\\' at end of string.".to_string(),
                ));
            }
        } else {
            result.push(c);
        }
        i += 1;
    }

    Ok(result)
}

/// Like [`unescape_bytes`], but for text: the result as UTF-8 (`ByteString.toStringUtf8`).
// Java: com/google/protobuf/TextFormat.java:2951 TextFormat.unescapeText
pub fn unescape_text(input: &str) -> Result<String, InvalidEscapeSequenceException> {
    Ok(String::from_utf8_lossy(&unescape_bytes(input)?).into_owned())
}

/// Is this an octal digit?
// Java: com/google/protobuf/TextFormat.java:2956 TextFormat.isOctal
fn is_octal(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}

/// Is this a hex digit?
// Java: com/google/protobuf/TextFormat.java:2961 TextFormat.isHex
fn is_hex(c: u8) -> bool {
    c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c)
}

/// Interpret a character as a digit (in any base up to 36) and return the numeric value.
// Java: com/google/protobuf/TextFormat.java:2969 TextFormat.digitValue
fn digit_value(c: u8) -> u32 {
    if c.is_ascii_digit() {
        (c - b'0') as u32
    } else if c.is_ascii_lowercase() {
        (c - b'a' + 10) as u32
    } else {
        (c - b'A' + 10) as u32
    }
}

/// Parse a 32-bit signed integer from the text. Unlike the Java standard `Integer.parseInt()`,
/// this function recognizes the prefixes "0x" and "0" to signify hexadecimal and octal numbers,
/// respectively.
// Java: com/google/protobuf/TextFormat.java:2984 TextFormat.parseInt32
pub fn parse_int32(text: &str) -> Result<i32, NumberFormatException> {
    Ok(parse_integer(text, true, false)? as i32)
}

/// Parse a 32-bit unsigned integer from the text. The result is coerced to a (signed) `int`
/// when returned since Java has no unsigned integer type.
// Java: com/google/protobuf/TextFormat.java:2994 TextFormat.parseUInt32
pub fn parse_uint32(text: &str) -> Result<i32, NumberFormatException> {
    Ok(parse_integer(text, false, false)? as i32)
}

/// Parse a 64-bit signed integer from the text.
// Java: com/google/protobuf/TextFormat.java:3003 TextFormat.parseInt64
pub fn parse_int64(text: &str) -> Result<i64, NumberFormatException> {
    parse_integer(text, true, true)
}

/// Parse a 64-bit unsigned integer from the text. The result is coerced to a (signed) `long`
/// when returned since Java has no unsigned long type.
// Java: com/google/protobuf/TextFormat.java:3013 TextFormat.parseUInt64
pub fn parse_uint64(text: &str) -> Result<i64, NumberFormatException> {
    parse_integer(text, false, true)
}

// Java: com/google/protobuf/TextFormat.java:3017 TextFormat.parseInteger
#[allow(clippy::manual_range_contains)] // Java's comparisons, kept as written.
fn parse_integer(text: &str, is_signed: bool, is_long: bool) -> Result<i64, NumberFormatException> {
    let mut pos = 0;

    let mut negative = false;
    if text[pos..].starts_with('-') {
        if !is_signed {
            return Err(NumberFormatException(format!(
                "Number must be positive: {text}"
            )));
        }
        pos += 1;
        negative = true;
    }

    let mut radix = 10;
    if text[pos..].starts_with("0x") {
        pos += 2;
        radix = 16;
    } else if text[pos..].starts_with('0') {
        radix = 8;
    }

    let number_text = &text[pos..];

    let result: i64;
    // `numberText.length()` counts UTF-16 units; a token here is ASCII.
    if number_text.encode_utf16().count() < 16 {
        // Can safely assume no overflow.
        let mut r = java_parse_long(number_text, radix)?;
        if negative {
            r = r.wrapping_neg();
        }

        // Check bounds.
        // No need to check for 64-bit numbers since they'd have to be 16 chars
        // or longer to overflow.
        if !is_long {
            if is_signed {
                if r > i32::MAX as i64 || r < i32::MIN as i64 {
                    return Err(NumberFormatException(format!(
                        "Number out of range for 32-bit signed integer: {text}"
                    )));
                }
            } else if r >= (1i64 << 32) || r < 0 {
                return Err(NumberFormatException(format!(
                    "Number out of range for 32-bit unsigned integer: {text}"
                )));
            }
        }
        result = r;
    } else {
        let (big_negative, magnitude) = java_parse_big_integer(number_text, radix)?;
        let big_negative = big_negative != negative && magnitude != Some(0);
        let bit_length = big_integer_bit_length(big_negative, magnitude);

        // Check bounds.
        if !is_long {
            if is_signed {
                if bit_length > 31 {
                    return Err(NumberFormatException(format!(
                        "Number out of range for 32-bit signed integer: {text}"
                    )));
                }
            } else if bit_length > 32 {
                return Err(NumberFormatException(format!(
                    "Number out of range for 32-bit unsigned integer: {text}"
                )));
            }
        } else if is_signed {
            if bit_length > 63 {
                return Err(NumberFormatException(format!(
                    "Number out of range for 64-bit signed integer: {text}"
                )));
            }
        } else if bit_length > 64 {
            return Err(NumberFormatException(format!(
                "Number out of range for 64-bit unsigned integer: {text}"
            )));
        }

        // `bigValue.longValue()`: the low 64 bits; bit_length <= 64 here, so the magnitude fits.
        let m = magnitude.expect("bit length <= 64") as i128;
        result = (if big_negative { -m } else { m }) as i64;
    }

    Ok(result)
}

/// `NumberFormatException.forInputString(s, radix)` (JDK 21).
fn for_input_string(s: &str, radix: u32) -> NumberFormatException {
    NumberFormatException(if radix == 10 {
        format!("For input string: \"{s}\"")
    } else {
        format!("For input string: \"{s}\" under radix {radix}")
    })
}

/// `Long.parseLong(s, radix)` for a string shorter than 16 characters (which cannot overflow).
///
/// PORT: `Character.digit` also accepts non-ASCII digits; a number token holds only
/// `[0-9A-Za-z_.+-]`, so ASCII digits are all that can reach this.
fn java_parse_long(s: &str, radix: u32) -> Result<i64, NumberFormatException> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(for_input_string(s, radix));
    }
    let mut i = 0;
    let mut negative = false;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        negative = bytes[0] == b'-';
        if bytes.len() == 1 {
            return Err(for_input_string(s, radix));
        }
        i = 1;
    }
    let mut result: i64 = 0;
    while i < bytes.len() {
        let Some(d) = (bytes[i] as char).to_digit(radix) else {
            return Err(for_input_string(s, radix));
        };
        result = result * radix as i64 + d as i64;
        i += 1;
    }
    Ok(if negative { -result } else { result })
}

/// `new BigInteger(s, radix)` (JDK 21): (negative, magnitude), the magnitude `None` when it does
/// not fit in 128 bits (its bit length is then above 64 in any case). The digits are read in
/// groups of `digitsPerInt[radix]` with `Integer.parseInt`, whose message names the group.
fn java_parse_big_integer(
    val: &str,
    radix: u32,
) -> Result<(bool, Option<u128>), NumberFormatException> {
    // A number token is ASCII: chars are bytes.
    let bytes = val.as_bytes();
    let len = bytes.len();
    let mut cursor = 0;
    if len == 0 {
        return Err(NumberFormatException("Zero length BigInteger".to_string()));
    }
    let mut negative = false;
    let index1 = val.rfind('-');
    let index2 = val.rfind('+');
    if let Some(index1) = index1 {
        if index1 != 0 || index2.is_some() {
            return Err(NumberFormatException(
                "Illegal embedded sign character".to_string(),
            ));
        }
        negative = true;
        cursor = 1;
    } else if let Some(index2) = index2 {
        if index2 != 0 {
            return Err(NumberFormatException(
                "Illegal embedded sign character".to_string(),
            ));
        }
        cursor = 1;
    }
    if cursor == len {
        return Err(NumberFormatException("Zero length BigInteger".to_string()));
    }
    // Skip leading zeros
    while cursor < len && bytes[cursor] == b'0' {
        cursor += 1;
    }
    if cursor == len {
        return Ok((false, Some(0)));
    }
    let num_digits = len - cursor;
    let digits_per_int = match radix {
        8 => 10,
        10 => 9,
        16 => 7,
        _ => unreachable!("parseInteger uses radix 8, 10 or 16"),
    };
    // Process first (potentially short) digit group
    let mut first_group_len = num_digits % digits_per_int;
    if first_group_len == 0 {
        first_group_len = digits_per_int;
    }
    let mut magnitude: Option<u128> = Some(0);
    let mut group_len = first_group_len;
    while cursor < len {
        let group = &val[cursor..cursor + group_len];
        cursor += group_len;
        group_len = digits_per_int;
        // `Integer.parseInt(group, radix)`.
        let mut group_val: u128 = 0;
        for b in group.bytes() {
            let Some(d) = (b as char).to_digit(radix) else {
                return Err(for_input_string(group, radix));
            };
            group_val = group_val * radix as u128 + d as u128;
        }
        let super_radix = (radix as u128).pow(group.len() as u32);
        magnitude = magnitude
            .and_then(|m| m.checked_mul(super_radix))
            .and_then(|m| m.checked_add(group_val));
    }
    Ok((negative, magnitude))
}

/// `BigInteger.bitLength()`: the minimal two's-complement width without the sign bit.
fn big_integer_bit_length(negative: bool, magnitude: Option<u128>) -> u32 {
    let Some(m) = magnitude else {
        return u32::MAX;
    };
    let bits = |x: u128| 128 - x.leading_zeros();
    if negative { bits(m - 1) } else { bits(m) }
}

/// A literal `Double.parseDouble(s)` / `Float.parseFloat(s)` accept
/// (`jdk.internal.math.FloatingDecimal.readJavaFormatString`; the two share the grammar).
enum JavaFloatingLiteral {
    NaN,
    Infinity {
        negative: bool,
    },
    /// A decimal literal without its type suffix, in a form Rust's parsers read (both round
    /// correctly to nearest, as Java does).
    Decimal(String),
    /// A hexadecimal literal.
    Hex,
}

/// `Double.parseDouble(s)`.
fn java_parse_double(s: &str) -> Result<f64, NumberFormatException> {
    Ok(match java_parse_floating(s)? {
        JavaFloatingLiteral::NaN => f64::NAN,
        JavaFloatingLiteral::Infinity { negative } => {
            if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
        JavaFloatingLiteral::Decimal(d) => d.parse::<f64>().expect("validated decimal"),
        JavaFloatingLiteral::Hex => unimplemented!("Java: FloatingDecimal.parseHexString"),
    })
}

/// `Float.parseFloat(s)`.
fn java_parse_float(s: &str) -> Result<f32, NumberFormatException> {
    Ok(match java_parse_floating(s)? {
        JavaFloatingLiteral::NaN => f32::NAN,
        JavaFloatingLiteral::Infinity { negative } => {
            if negative {
                f32::NEG_INFINITY
            } else {
                f32::INFINITY
            }
        }
        JavaFloatingLiteral::Decimal(d) => d.parse::<f32>().expect("validated decimal"),
        JavaFloatingLiteral::Hex => unimplemented!("Java: FloatingDecimal.parseHexString"),
    })
}

/// Whether `Double.parseDouble(s)` / `Float.parseFloat(s)` accept `s`
/// (`jdk.internal.math.FloatingDecimal.readJavaFormatString`), and what it is.
fn java_parse_floating(s: &str) -> Result<JavaFloatingLiteral, NumberFormatException> {
    let err = || NumberFormatException(format!("For input string: \"{s}\""));
    // `in.trim()`
    let t = s.trim_matches(|c: char| c <= ' ');
    let b = t.as_bytes();
    if b.is_empty() {
        return Err(NumberFormatException("empty String".to_string()));
    }
    let mut i = 0;
    if b[0] == b'-' || b[0] == b'+' {
        i += 1;
    }
    let Some(&c) = b.get(i) else {
        return Err(err());
    };
    if c == b'N' {
        return if &b[i..] == b"NaN" {
            Ok(JavaFloatingLiteral::NaN)
        } else {
            Err(err())
        };
    } else if c == b'I' {
        return if &b[i..] == b"Infinity" {
            Ok(JavaFloatingLiteral::Infinity {
                negative: b[0] == b'-',
            })
        } else {
            Err(err())
        };
    } else if c == b'0' && b.len() > i + 1 && (b[i + 1] == b'x' || b[i + 1] == b'X') {
        return if is_java_hex_float(&b[i + 2..]) {
            Ok(JavaFloatingLiteral::Hex)
        } else {
            Err(err())
        };
    }
    // Decimal: digits with at most one '.', at least one digit.
    let mut digits = 0;
    let mut dec_seen = false;
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => digits += 1,
            b'.' => {
                if dec_seen {
                    return Err(NumberFormatException("multiple points".to_string()));
                }
                dec_seen = true;
            }
            _ => break,
        }
        i += 1;
    }
    if digits == 0 {
        return Err(err());
    }
    // Exponent.
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        match b.get(i) {
            None => return Err(err()),
            Some(b'-') | Some(b'+') => i += 1,
            _ => {}
        }
        let exp_at = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_at {
            return Err(err());
        }
    }
    // Optional type suffix, only as the last character.
    if i < b.len() && (i != b.len() - 1 || !matches!(b[i], b'f' | b'F' | b'd' | b'D')) {
        return Err(err());
    }
    // The literal without the type suffix.
    let end = if i < b.len() { i } else { b.len() };
    Ok(JavaFloatingLiteral::Decimal(t[..end].to_string()))
}

/// The part after `0x` of `FloatingDecimal.HEX_FLOAT_PATTERN`:
/// `((\p{XDigit}+)\.?)|((\p{XDigit}*)\.(\p{XDigit}+)))[pP]([-+])?(\p{Digit}+)[fFdD]?`.
fn is_java_hex_float(b: &[u8]) -> bool {
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_hexdigit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            i += 1;
        }
        frac_digits = i - frac_start;
    }
    if int_digits == 0 && frac_digits == 0 {
        return false;
    }
    if i >= b.len() || (b[i] != b'p' && b[i] != b'P') {
        return false;
    }
    i += 1;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let exp_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == exp_start {
        return false;
    }
    if i < b.len() && matches!(b[i], b'f' | b'F' | b'd' | b'D') {
        i += 1;
    }
    i == b.len()
}
