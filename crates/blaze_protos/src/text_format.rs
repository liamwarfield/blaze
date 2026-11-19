//! Port of the reading half of `com.google.protobuf.TextFormat` (protobuf-java 33.4, the version
//! Bazel 9.3.2 builds with: `MODULE.bazel` `single_version_override(module_name = "protobuf",
//! version = "33.4")`; Bazel's `third_party/protobuf.patch` does not touch `TextFormat.java`).
//! Paths in the `// Java:` markers are relative to `java/core/src/main/java` of that release.
//!
//! What is ported: `Tokenizer`, `Parser.merge(CharSequence, Message.Builder)` with the default
//! parser (`TextFormat.getParser()`: unknown fields and extensions are errors, singular fields may
//! be overwritten, recursion limit 100, empty extension and type registries), the skipping of
//! unknown fields, `unescapeBytes` and `parseInteger`.
//!
//! PORT: protobuf-java drives the parser with the message's `Descriptor`. Blaze has no runtime
//! descriptors, so the descriptors of the one message Blaze parses from text,
//! `build.bazel.remote.execution.v2.Platform` (read by `PlatformUtils.getPlatformProto`), are
//! written out here ([`PLATFORM`], [`PROPERTY`]) and the parse goes into a [`DynamicMessage`]
//! (Java: `MessageReflection.BuilderAdapter` over the builder). The field types that schema does
//! not use (numbers, bools, enums, bytes, groups, maps) have no [`FieldType`] variant, so their
//! branches of `consumeFieldValue` cannot be reached and are not ported; values of those shapes
//! are still tokenized and skipped as Java does when they belong to unknown fields.
//!
//! PORT: Java's `CharSequence` is UTF-16; the tokenizer works on the UTF-16 code units of the
//! input, so line and column numbers in errors count what Java counts.

use std::fmt;

use crate::build::bazel::remote::execution::v2::{Platform, platform};

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
/// PORT: the messages of `Long.parseLong` and `new BigInteger(String, int)` are JDK text that
/// only reaches a user through `Couldn't parse integer: ...` for an integer field; the Platform
/// schema has none (numbers are only skipped inside unknown fields, where the message is
/// dropped), so those two use `NumberFormatException.forInputString`'s format. The messages
/// `parseInteger` builds itself are Java's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberFormatException(pub String);

impl fmt::Display for NumberFormatException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Descriptors (the schema the parser is driven by).

/// The `FieldDescriptor.Type`s of the fields Blaze parses from text.
#[derive(Debug)]
pub enum FieldType {
    /// `TYPE_STRING` (`JavaType.STRING`).
    String,
    /// `TYPE_MESSAGE` (`JavaType.MESSAGE`) with its message type.
    Message(&'static Descriptor),
}

/// The parts of `Descriptors.FieldDescriptor` the parser reads.
#[derive(Debug)]
pub struct FieldDescriptor {
    /// `getName()`.
    pub name: &'static str,
    /// `getFullName()`.
    pub full_name: &'static str,
    /// `isRepeated()`.
    pub repeated: bool,
    /// `getType()`.
    pub field_type: FieldType,
}

impl FieldDescriptor {
    /// `isGroupLike()`: only `TYPE_GROUP` fields can be; no field here is a group.
    fn is_group_like(&self) -> bool {
        false
    }

    /// `getMessageType()`.
    fn get_message_type(&self) -> &'static Descriptor {
        match self.field_type {
            FieldType::Message(d) => d,
            FieldType::String => {
                panic!("UnsupportedOperationException: This field is not of message type.")
            }
        }
    }

    /// `getJavaType() == JavaType.MESSAGE`.
    fn is_message(&self) -> bool {
        matches!(self.field_type, FieldType::Message(_))
    }
}

/// The parts of `Descriptors.Descriptor` the parser reads.
#[derive(Debug)]
pub struct Descriptor {
    /// `getName()`.
    pub name: &'static str,
    /// `getFullName()`.
    pub full_name: &'static str,
    /// `getFields()`, in declaration order.
    pub fields: &'static [FieldDescriptor],
}

/// Descriptors are singletons: equal iff the same descriptor.
impl PartialEq for Descriptor {
    fn eq(&self, other: &Descriptor) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Descriptor {}

impl Descriptor {
    /// `findFieldByName(name)`.
    fn find_field_by_name(&self, name: &str) -> Option<&'static FieldDescriptor> {
        // `fields` is 'static; reborrow through the static slice.
        let fields: &'static [FieldDescriptor] = self.fields;
        fields.iter().find(|f| f.name == name)
    }

    fn field_index(&self, field: &FieldDescriptor) -> usize {
        self.fields
            .iter()
            .position(|f| std::ptr::eq(f, field))
            .expect("field of this message type")
    }
}

/// `build.bazel.remote.execution.v2.Platform.Property` (`remote_execution.proto`:
/// `string name = 1; string value = 2;`).
pub static PROPERTY: Descriptor = Descriptor {
    name: "Property",
    full_name: "build.bazel.remote.execution.v2.Platform.Property",
    fields: &[
        FieldDescriptor {
            name: "name",
            full_name: "build.bazel.remote.execution.v2.Platform.Property.name",
            repeated: false,
            field_type: FieldType::String,
        },
        FieldDescriptor {
            name: "value",
            full_name: "build.bazel.remote.execution.v2.Platform.Property.value",
            repeated: false,
            field_type: FieldType::String,
        },
    ],
};

/// `build.bazel.remote.execution.v2.Platform` (`repeated Property properties = 1;`).
pub static PLATFORM: Descriptor = Descriptor {
    name: "Platform",
    full_name: "build.bazel.remote.execution.v2.Platform",
    fields: &[FieldDescriptor {
        name: "properties",
        full_name: "build.bazel.remote.execution.v2.Platform.properties",
        repeated: true,
        field_type: FieldType::Message(&PROPERTY),
    }],
};

/// A value of one field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    String(String),
    Message(DynamicMessage),
}

/// A message under construction (`MessageReflection.BuilderAdapter` over its builder): per field
/// of the descriptor, its values (at most one for a singular field; present iff `hasField`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicMessage {
    pub descriptor: &'static Descriptor,
    pub values: Vec<Vec<FieldValue>>,
}

impl DynamicMessage {
    pub fn new(descriptor: &'static Descriptor) -> DynamicMessage {
        DynamicMessage {
            descriptor,
            values: descriptor.fields.iter().map(|_| Vec::new()).collect(),
        }
    }

    /// The values of the field with this name.
    pub fn get(&self, name: &str) -> &[FieldValue] {
        match self.descriptor.find_field_by_name(name) {
            Some(f) => &self.values[self.descriptor.field_index(f)],
            None => &[],
        }
    }

    // Java: com/google/protobuf/MessageReflection.java BuilderAdapter.hasField
    fn has_field(&self, field: &FieldDescriptor) -> bool {
        !self.values[self.descriptor.field_index(field)].is_empty()
    }

    // Java: com/google/protobuf/MessageReflection.java BuilderAdapter.setField
    fn set_field(&mut self, field: &FieldDescriptor, value: FieldValue) {
        let i = self.descriptor.field_index(field);
        self.values[i] = vec![value];
    }

    // Java: com/google/protobuf/MessageReflection.java BuilderAdapter.addRepeatedField
    fn add_repeated_field(&mut self, field: &FieldDescriptor, value: FieldValue) {
        let i = self.descriptor.field_index(field);
        self.values[i].push(value);
    }

    /// `newMergeTargetForField(field, null)`: a singular message field that is set is merged
    /// into; otherwise a new builder of the field's type.
    // Java: com/google/protobuf/MessageReflection.java:603 BuilderAdapter.newMergeTargetForField
    fn new_merge_target_for_field(&mut self, field: &FieldDescriptor) -> DynamicMessage {
        if !field.repeated && self.has_field(field) {
            let i = self.descriptor.field_index(field);
            if let Some(FieldValue::Message(m)) = self.values[i].first() {
                return m.clone();
            }
        }
        DynamicMessage::new(field.get_message_type())
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

    /// If the next token is a double, consume it. Otherwise, throw a [`ParseException`].
    ///
    /// PORT: returns no value; only `skipFieldValue` calls it here, and it drops the value.
    // Java: com/google/protobuf/TextFormat.java:1498 TextFormat.Tokenizer.consumeDouble
    fn consume_double(&mut self) -> Result<(), ParseException> {
        // We need to parse infinity and nan separately because
        // Double.parseDouble() does not accept "inf", "infinity", or "nan".
        match self.current_token_string().to_ascii_lowercase().as_str() {
            "-inf" | "-infinity" | "inf" | "infinity" | "nan" => {
                self.next_token();
                return Ok(());
            }
            _ => {
                // fall through
            }
        }

        match java_parse_floating(&self.current_token_string()) {
            Ok(()) => {
                self.next_token();
                Ok(())
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

    /// If the next token is a float, consume it. Otherwise, throw a [`ParseException`].
    ///
    /// PORT: returns no value, as [`Self::consume_double`].
    // Java: com/google/protobuf/TextFormat.java:1543 TextFormat.Tokenizer.consumeFloat
    fn consume_float(&mut self) -> Result<(), ParseException> {
        // We need to parse infinity and nan separately because
        // Float.parseFloat() does not accept "inf", "infinity", or "nan".
        match self.current_token_string().to_ascii_lowercase().as_str() {
            "-inf" | "-inff" | "-infinity" | "-infinityf" | "inf" | "inff" | "infinity"
            | "infinityf" | "nan" | "nanf" => {
                self.next_token();
                return Ok(());
            }
            _ => {
                // fall through
            }
        }

        match java_parse_floating(&self.current_token_string()) {
            Ok(()) => {
                self.next_token();
                Ok(())
            }
            Err(e) => Err(self.float_parse_exception(&e)),
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
    #[allow(dead_code)]
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

/// `Parser.SingularOverwritePolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum SingularOverwritePolicy {
    /// Later values are merged with earlier values. For primitive fields or conflicting oneofs,
    /// the last value is retained.
    AllowSingularOverwrites,
    /// An error is issued.
    ForbidSingularOverwrites,
}

/// The settings of `TextFormat.getParser()` (`Parser.newBuilder().build()`).
// Java: com/google/protobuf/TextFormat.java:1933 TextFormat.Parser.Builder
const ALLOW_UNKNOWN_FIELDS: bool = false;
const ALLOW_UNKNOWN_EXTENSIONS: bool = false;
const SINGULAR_OVERWRITE_POLICY: SingularOverwritePolicy =
    SingularOverwritePolicy::AllowSingularOverwrites;

/// `TextFormat.getParser().merge(input, platformBuilder)` for a
/// `build.bazel.remote.execution.v2.Platform.Builder`: parses a text-format `Platform` from
/// `input` and merges it into `builder` (repeated `properties` are appended).
///
/// PORT: the parse goes into a fresh [`DynamicMessage`] and is merged into `builder` at the end;
/// Java writes into the builder as it goes, so on a `ParseException` Java's builder holds the
/// fields parsed so far and this one is unchanged. `PlatformUtils.getPlatformProto` discards
/// the builder on a `ParseException`.
// Java: com/google/protobuf/TextFormat.java:2019 TextFormat.Parser.merge
pub fn merge_platform(input: &str, builder: &mut Platform) -> Result<(), ParseException> {
    let message = merge(input, &PLATFORM)?;
    for value in message.get("properties") {
        let FieldValue::Message(property) = value else {
            unreachable!("properties is a message field")
        };
        let string = |name: &str| match property.get(name).first() {
            Some(FieldValue::String(s)) => s.clone(),
            Some(FieldValue::Message(_)) => unreachable!("{name} is a string field"),
            None => String::new(),
        };
        builder.properties.push(platform::Property {
            name: string("name"),
            value: string("value"),
        });
    }
    Ok(())
}

/// Parse a text-format message of type `descriptor` from `input`.
///
/// Extensions are never recognized (`ExtensionRegistry.getEmptyRegistry()`).
// Java: com/google/protobuf/TextFormat.java:2119 TextFormat.Parser.merge
pub fn merge(
    input: &str,
    descriptor: &'static Descriptor,
) -> Result<DynamicMessage, ParseException> {
    let text: Vec<u16> = input.encode_utf16().collect();
    let mut tokenizer = Tokenizer::new(&text);
    let mut target = DynamicMessage::new(descriptor);
    let mut unknown_fields: Vec<UnknownField> = Vec::new();

    while !tokenizer.at_end() {
        merge_field(
            &mut tokenizer,
            &mut target,
            &mut unknown_fields,
            RECURSION_LIMIT,
        )?;
    }
    check_unknown_fields(&unknown_fields)?;
    Ok(target)
}

/// Check both unknown fields and unknown extensions and log warning messages or throw
/// exceptions according to the flag.
// Java: com/google/protobuf/TextFormat.java:2079 TextFormat.Parser.checkUnknownFields
fn check_unknown_fields(unknown_fields: &[UnknownField]) -> Result<(), ParseException> {
    if unknown_fields.is_empty() {
        return Ok(());
    }

    let mut msg = String::from("Input contains unknown fields and/or extensions:");
    for field in unknown_fields {
        msg.push('\n');
        msg.push_str(&field.message);
    }

    if ALLOW_UNKNOWN_FIELDS {
        unreachable!("TextFormat.getParser() does not allow unknown fields");
    }

    let mut first_error_index = 0;
    if ALLOW_UNKNOWN_EXTENSIONS {
        let mut all_unknown_extensions = true;
        for field in unknown_fields {
            if field.type_ == UnknownFieldType::Field {
                all_unknown_extensions = false;
                break;
            }
            first_error_index += 1;
        }
        if all_unknown_extensions {
            unreachable!("TextFormat.getParser() does not allow unknown extensions");
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
///
/// PORT: `parseTreeBuilder` is null for `TextFormat.getParser()`, and `detectSilentMarker` is
/// empty in this version; both are left out.
// Java: com/google/protobuf/TextFormat.java:2152 TextFormat.Parser.mergeField
fn merge_field(
    tokenizer: &mut Tokenizer<'_>,
    target: &mut DynamicMessage,
    unknown_fields: &mut Vec<UnknownField>,
    recursion_limit: i32,
) -> Result<(), ParseException> {
    let mut field: Option<&'static FieldDescriptor> = None;
    let name: String;
    let type_ = target.descriptor;

    if type_.full_name == "google.protobuf.Any" && tokenizer.try_consume("[") {
        // No Any message is parsed from text in Blaze.
        // PORT: blaze_protos has no blaze_util; `unimplemented!` per PORTING.md.
        unimplemented!("Java: TextFormat.Parser.mergeAnyFieldValue (google.protobuf.Any)");
    }

    if tokenizer.try_consume("[") {
        // An extension.
        let mut name_builder = tokenizer.consume_identifier()?;
        while tokenizer.try_consume(".") {
            name_builder.push('.');
            name_builder.push_str(&tokenizer.consume_identifier()?);
        }
        name = name_builder;

        // `target.findExtensionByName(extensionRegistry, name)`: the registry is
        // `ExtensionRegistry.getEmptyRegistry()`, so there is no extension.
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
            // PORT: an identifier is ASCII ([A-Za-z0-9_.]), where `toLowerCase(Locale.US)` is
            // ASCII lowercasing.
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
        guess_field_type_and_skip(tokenizer, type_, recursion_limit)?;
        return Ok(());
    };

    // Handle potential ':'.
    if field.is_message() {
        tokenizer.try_consume(":"); // optional
        consume_field_values(tokenizer, target, field, unknown_fields, recursion_limit)?;
    } else {
        tokenizer.consume(":")?; // required
        consume_field_values(tokenizer, target, field, unknown_fields, recursion_limit)?;
    }

    // For historical reasons, fields may optionally be separated by commas or
    // semicolons.
    if !tokenizer.try_consume(";") {
        tokenizer.try_consume(",");
    }
    Ok(())
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

/// Parse a one or more field values from `tokenizer` and merge it into `target`.
// Java: com/google/protobuf/TextFormat.java:2335 TextFormat.Parser.consumeFieldValues
fn consume_field_values(
    tokenizer: &mut Tokenizer<'_>,
    target: &mut DynamicMessage,
    field: &'static FieldDescriptor,
    unknown_fields: &mut Vec<UnknownField>,
    recursion_limit: i32,
) -> Result<(), ParseException> {
    // Support specifying repeated field values as a comma-separated list.
    // Ex."foo: [1, 2, 3]"
    if field.repeated && tokenizer.try_consume("[") {
        if !tokenizer.try_consume("]") {
            // Allow "foo: []" to be treated as empty.
            loop {
                consume_field_value(tokenizer, target, field, unknown_fields, recursion_limit)?;
                if tokenizer.try_consume("]") {
                    // End of list.
                    break;
                }
                tokenizer.consume(",")?;
            }
        }
    } else {
        consume_field_value(tokenizer, target, field, unknown_fields, recursion_limit)?;
    }
    Ok(())
}

/// Parse a single field value from `tokenizer` and merge it into `target`.
// Java: com/google/protobuf/TextFormat.java:2380 TextFormat.Parser.consumeFieldValue
fn consume_field_value(
    tokenizer: &mut Tokenizer<'_>,
    target: &mut DynamicMessage,
    field: &'static FieldDescriptor,
    unknown_fields: &mut Vec<UnknownField>,
    recursion_limit: i32,
) -> Result<(), ParseException> {
    if SINGULAR_OVERWRITE_POLICY == SingularOverwritePolicy::ForbidSingularOverwrites
        && !field.repeated
    {
        // PORT: the oneof half of this check is left out: the schema has no oneof.
        if target.has_field(field) {
            return Err(tokenizer.parse_exception_previous_token(&format!(
                "Non-repeated field \"{}\" cannot be overwritten.",
                field.full_name
            )));
        }
    }

    let value = match field.field_type {
        FieldType::Message(_) => {
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
                merge_field(
                    tokenizer,
                    &mut sub_field,
                    unknown_fields,
                    recursion_limit - 1,
                )?;
            }

            FieldValue::Message(sub_field)
        }
        FieldType::String => FieldValue::String(tokenizer.consume_string()?),
    };

    if field.repeated {
        target.add_repeated_field(field, value);
    } else {
        target.set_field(field, value);
    }
    Ok(())
}

/// Skips the next field including the field's name and value.
// Java: com/google/protobuf/TextFormat.java:2618 TextFormat.Parser.skipField
fn skip_field(
    tokenizer: &mut Tokenizer<'_>,
    type_: &'static Descriptor,
    recursion_limit: i32,
) -> Result<(), ParseException> {
    let _name = consume_full_type_name(tokenizer)?;
    guess_field_type_and_skip(tokenizer, type_, recursion_limit)?;

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
        skip_field(tokenizer, type_, recursion_limit)?;
    }
    tokenizer.consume(delimiter)
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

/// Tries to guess the type of this field and skip it.
///
/// If this field is not a message, there should be a ":" between the field name and the field
/// value and also the field value should not start with "{" or "<" which indicates the beginning
/// of a message body. If there is no ":" or there is a "{" or "<" after ":", this field has to
/// be a message or the input is ill-formed. For short-formed repeated fields (i.e. with "[]"),
/// if it is repeated scalar, there must be a ":" between the field name and the starting "[" .
// Java: com/google/protobuf/TextFormat.java:2670 TextFormat.Parser.guessFieldTypeAndSkip
fn guess_field_type_and_skip(
    tokenizer: &mut Tokenizer<'_>,
    type_: &'static Descriptor,
    recursion_limit: i32,
) -> Result<(), ParseException> {
    let semicolon_consumed = tokenizer.try_consume(":");
    if tokenizer.looking_at("[") {
        // Short repeated field form. If a semicolon was consumed, it could be repeated scalar or
        // repeated message. If not, it must be repeated message.
        skip_field_short_formed_repeated(tokenizer, semicolon_consumed, type_, recursion_limit)
    } else if semicolon_consumed && !tokenizer.looking_at("{") && !tokenizer.looking_at("<") {
        skip_field_value(tokenizer)
    } else {
        if recursion_limit < 1 {
            return Err(tokenizer.parse_exception("Message is nested too deep"));
        }
        skip_field_message(tokenizer, type_, recursion_limit - 1)
    }
}

/// Skips a short-formed repeated field value.
///
/// Reports an error if scalar type is not allowed but showing up inside "[]".
// Java: com/google/protobuf/TextFormat.java:2692 TextFormat.Parser.skipFieldShortFormedRepeated
fn skip_field_short_formed_repeated(
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
            skip_field_message(tokenizer, type_, recursion_limit - 1)?;
        } else if scalar_allowed {
            // Try skipping scalar field inside "[]".
            skip_field_value(tokenizer)?;
        } else {
            return Err(tokenizer
                .parse_exception("Invalid repeated scalar field: missing \":\" before \"[\"."));
        }
        if tokenizer.try_consume("]") {
            break;
        }
        tokenizer.consume(",")?;
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

/// `new BigInteger(s, radix)`: (negative, magnitude), the magnitude `None` when it does not fit
/// in 128 bits (its bit length is then above 64 in any case).
fn java_parse_big_integer(
    s: &str,
    radix: u32,
) -> Result<(bool, Option<u128>), NumberFormatException> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut negative = false;
    if let Some(&b) = bytes.first()
        && (b == b'-' || b == b'+')
    {
        negative = b == b'-';
        i = 1;
    }
    if i == bytes.len() {
        return Err(for_input_string(s, radix));
    }
    let mut magnitude: Option<u128> = Some(0);
    while i < bytes.len() {
        let Some(d) = (bytes[i] as char).to_digit(radix) else {
            return Err(for_input_string(s, radix));
        };
        magnitude = magnitude
            .and_then(|m| m.checked_mul(radix as u128))
            .and_then(|m| m.checked_add(d as u128));
        i += 1;
    }
    Ok((negative && magnitude != Some(0), magnitude))
}

/// `BigInteger.bitLength()`: the minimal two's-complement width without the sign bit.
fn big_integer_bit_length(negative: bool, magnitude: Option<u128>) -> u32 {
    let Some(m) = magnitude else {
        return u32::MAX;
    };
    let bits = |x: u128| 128 - x.leading_zeros();
    if negative { bits(m - 1) } else { bits(m) }
}

/// Whether `Double.parseDouble(s)` / `Float.parseFloat(s)` accept `s`
/// (`jdk.internal.math.FloatingDecimal.readJavaFormatString`). The two share the grammar.
///
/// PORT: the value is not computed; the callers only skip the token.
fn java_parse_floating(s: &str) -> Result<(), NumberFormatException> {
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
            Ok(())
        } else {
            Err(err())
        };
    } else if c == b'I' {
        return if &b[i..] == b"Infinity" {
            Ok(())
        } else {
            Err(err())
        };
    } else if c == b'0' && b.len() > i + 1 && (b[i + 1] == b'x' || b[i + 1] == b'X') {
        return if is_java_hex_float(&b[i + 2..]) {
            Ok(())
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
    Ok(())
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
