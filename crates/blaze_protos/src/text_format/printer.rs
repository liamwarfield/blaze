//! Port of the printing half of `com.google.protobuf.TextFormat` (protobuf-java 33.4):
//! `TextFormat.Printer`, `TextGenerator`, `TextFormatEscaper`, `unsignedToString`.
//!
//! Descriptor-driven: a generated message is printed through its [`DynamicMessage`]
//! ([`DynamicMessage::from_message`], the message parsed back from its Java encoding against the
//! runtime descriptors of [`crate::descriptors`]). No per-message code.
//!
//! PORT: the safe debug format (`enablingSafeDebugFormat`, `debugFormatPrinter()`: redaction of
//! `debug_redact` fields) and the field reporter levels (a no-op by default) are not ported;
//! `debug_format_printer` fails loudly. Extensions are never set on a Blaze message (they would
//! be unknown fields of the parsed message, as in Java with an empty extension registry).

use crate::JavaMessage;
use crate::descriptors::{Descriptor, FieldDescriptor, JavaType, Type, pool};
use crate::dynamic_message::{DynamicMessage, UnknownFieldSet, Value};
use crate::json::{java_double_to_string, java_float_to_string};

use super::TypeRegistry;

const DEBUG_STRING_SILENT_MARKER: &str = " \t ";

/// `ENABLE_INSERT_SILENT_MARKER`: `System.getenv().getOrDefault(
/// "SILENT_MARKER_INSERTION_ENABLED", "false").equals("true")` (the server's environment).
// Java: com/google/protobuf/TextFormat.java:42 TextFormat.ENABLE_INSERT_SILENT_MARKER
fn enable_insert_silent_marker() -> bool {
    std::env::var_os("SILENT_MARKER_INSERTION_ENABLED").is_some_and(|v| v == "true")
}

/// Helper class for converting protobufs to text.
// Java: com/google/protobuf/TextFormat.java:124 TextFormat.Printer
#[derive(Clone, Debug)]
pub struct Printer {
    /// Whether to escape non ASCII characters with backslash and octal.
    escape_non_ascii: bool,
    /// Whether to print repeated primitive fields using short square bracket notation.
    use_short_repeated_primitives: bool,
    type_registry: TypeRegistry,
    single_line: bool,
    insert_silent_marker: bool,
}

/// `TextFormat.printer()`: printer instance which escapes non-ASCII characters.
// Java: com/google/protobuf/TextFormat.java:116 TextFormat.printer
pub fn printer() -> Printer {
    Printer {
        escape_non_ascii: true,
        use_short_repeated_primitives: false,
        type_registry: TypeRegistry::get_empty_type_registry(),
        single_line: false,
        insert_silent_marker: false,
    }
}

/// `TextFormat.defaultFormatPrinter()`: [`printer`] that inserts the silent marker when the
/// server's environment says so. `Message.toString()` uses it.
// Java: com/google/protobuf/TextFormat.java:126 TextFormat.defaultFormatPrinter
pub fn default_format_printer() -> Printer {
    Printer {
        insert_silent_marker: enable_insert_silent_marker(),
        ..printer()
    }
}

/// `TextFormat.debugFormatPrinter()`.
pub fn debug_format_printer() -> Printer {
    unimplemented!("Java: TextFormat.debugFormatPrinter (safe debug format)")
}

/// `AbstractMessage.toString()`: `TextFormat.Printer.getOutputModePrinter().printToString(this)`
/// with the default output mode (`ProtobufToStringOutput.isDefaultFormat()`).
// Java: com/google/protobuf/AbstractMessage.java:93 AbstractMessage.toString
pub fn message_to_string<M: JavaMessage>(message: &M) -> String {
    default_format_printer().print_to_string(message)
}

impl Printer {
    /// `escapingNonAscii(escapeNonAscii)`.
    pub fn escaping_non_ascii(&self, escape_non_ascii: bool) -> Printer {
        Printer {
            escape_non_ascii,
            insert_silent_marker: false,
            ..self.clone()
        }
    }

    /// `usingTypeRegistry(typeRegistry)`.
    pub fn using_type_registry(&self, type_registry: TypeRegistry) -> Printer {
        if !self.type_registry.is_empty() {
            panic!("IllegalArgumentException: Only one typeRegistry is allowed.");
        }
        Printer {
            type_registry,
            insert_silent_marker: false,
            ..self.clone()
        }
    }

    /// `usingShortRepeatedPrimitives(useShortRepeatedPrimitives)`.
    pub fn using_short_repeated_primitives(&self, use_short_repeated_primitives: bool) -> Printer {
        Printer {
            use_short_repeated_primitives,
            insert_silent_marker: false,
            ..self.clone()
        }
    }

    /// `emittingSingleLine(singleLine)`.
    pub fn emitting_single_line(&self, single_line: bool) -> Printer {
        Printer {
            single_line,
            insert_silent_marker: false,
            ..self.clone()
        }
    }

    /// `print(message, output)`: appends the text of `message` to `output`.
    // Java: com/google/protobuf/TextFormat.java:238 TextFormat.Printer.print
    pub fn print<M: JavaMessage>(&self, message: &M, output: &mut String) {
        self.print_dynamic(&DynamicMessage::from_message(message), output);
    }

    /// `print(message, output)` of a [`DynamicMessage`].
    pub fn print_dynamic(&self, message: &DynamicMessage, output: &mut String) {
        let mut generator = TextGenerator::new(output, self.single_line, self.insert_silent_marker);
        self.print_message_or_any(message, &mut generator);
    }

    /// `printToString(message)`.
    // Java: com/google/protobuf/TextFormat.java:538 TextFormat.Printer.printToString
    pub fn print_to_string<M: JavaMessage>(&self, message: &M) -> String {
        let mut text = String::new();
        self.print(message, &mut text);
        text
    }

    /// `printToString(message)` of a [`DynamicMessage`].
    pub fn print_dynamic_to_string(&self, message: &DynamicMessage) -> String {
        let mut text = String::new();
        self.print_dynamic(message, &mut text);
        text
    }

    /// `printToString(UnknownFieldSet)`.
    pub fn print_unknown_fields_to_string(&self, fields: &UnknownFieldSet) -> String {
        let mut text = String::new();
        let mut generator = TextGenerator::new(&mut text, self.single_line, false);
        print_unknown_fields(fields, &mut generator);
        text
    }

    /// `shortDebugString(message)`: `emittingSingleLine(true).printToString(message)`.
    pub fn short_debug_string<M: JavaMessage>(&self, message: &M) -> String {
        self.emitting_single_line(true).print_to_string(message)
    }

    /// `printFieldValue(field, value, output)`.
    pub fn print_field_value_to(
        &self,
        field: &FieldDescriptor,
        value: &Value,
        output: &mut String,
    ) {
        let mut generator = TextGenerator::new(output, self.single_line, false);
        self.print_field_value(field, value, &mut generator);
    }

    /// `printFieldToString(field, value)` of a singular field value.
    pub fn print_field_to_string(&self, field: &FieldDescriptor, values: &[Value]) -> String {
        let mut text = String::new();
        let mut generator = TextGenerator::new(&mut text, self.single_line, false);
        self.print_field(field, values, &mut generator);
        text
    }

    // Java: com/google/protobuf/TextFormat.java:256 TextFormat.Printer.print
    fn print_message_or_any(&self, message: &DynamicMessage, generator: &mut TextGenerator<'_>) {
        if message.get_descriptor_for_type().full_name == "google.protobuf.Any"
            && self.print_any(message, generator)
        {
            return;
        }
        self.print_message(message, generator);
    }

    /// Attempt to print the 'google.protobuf.Any' message in a human-friendly format. Returns
    /// false if the message isn't a valid 'google.protobuf.Any' message (in which case the
    /// message should be rendered just like a regular message to help debugging).
    // Java: com/google/protobuf/TextFormat.java:278 TextFormat.Printer.printAny
    fn print_any(&self, message: &DynamicMessage, generator: &mut TextGenerator<'_>) -> bool {
        let message_type = message.get_descriptor_for_type();
        let type_url_field = message_type.find_field_by_number(1);
        let value_field = message_type.find_field_by_number(2);
        let (Some(type_url_field), Some(value_field)) = (type_url_field, value_field) else {
            return false;
        };
        if type_url_field.type_ != Type::String || value_field.type_ != Type::Bytes {
            // The message may look like an Any but isn't actually an Any message (might happen
            // if the user tries to use DynamicMessage to construct an Any from incomplete
            // Descriptor).
            return false;
        }
        let type_url = match message.get_field(type_url_field) {
            Some(Value::String(s)) => String::from_utf8_lossy(s).into_owned(),
            _ => String::new(),
        };
        // If type_url is not set, we will not be able to decode the content of the value, so
        // just print out the Any like a regular message.
        if type_url.is_empty() {
            return false;
        }
        let value: &[u8] = match message.get_field(value_field) {
            Some(Value::Bytes(b)) => b,
            _ => &[],
        };

        let content = match self.type_registry.get_descriptor_for_type_url(&type_url) {
            Err(_) => return false,
            Ok(None) => return false,
            // The value of Any is malformed. We cannot print it out nicely, so fallback to
            // printing out the type_url and value as bytes. Note that we fail open here to be
            // consistent with text_format.cc, and also to allow a way for users to inspect the
            // content of the broken message.
            Ok(Some(content_type)) => match DynamicMessage::parse_from(content_type, value) {
                Ok(m) => m,
                Err(_) => return false,
            },
        };
        generator.print("[");
        generator.print(&type_url);
        generator.print("]");
        generator.maybe_print_silent_marker();
        generator.print("{");
        generator.eol();
        generator.indent();
        self.print_message_or_any(&content, generator);
        generator.outdent();
        generator.print("}");
        generator.eol();
        true
    }

    // Java: com/google/protobuf/TextFormat.java:346 TextFormat.Printer.printField
    fn print_field(
        &self,
        field: &FieldDescriptor,
        values: &[Value],
        generator: &mut TextGenerator<'_>,
    ) {
        // Sort map field entries by key
        if field.is_map_field() {
            let mut adapters: Vec<MapEntryAdapter<'_>> = values
                .iter()
                .map(|entry| MapEntryAdapter::new(entry, field))
                .collect();
            // `Collections.sort` is stable.
            adapters.sort_by(|a, b| a.compare_to(b));
            for adapter in adapters {
                self.print_single_field(field, adapter.entry, generator);
            }
        } else if field.is_repeated() {
            if self.use_short_repeated_primitives && field.get_java_type() != JavaType::Message {
                self.print_short_repeated_field(field, values, generator);
            } else {
                for element in values {
                    self.print_single_field(field, element, generator);
                }
            }
        } else {
            self.print_single_field(field, &values[0], generator);
        }
    }

    /// Outputs a textual representation of the value of given field value.
    // Java: com/google/protobuf/TextFormat.java:443 TextFormat.Printer.printFieldValue
    fn print_field_value(
        &self,
        field: &FieldDescriptor,
        value: &Value,
        generator: &mut TextGenerator<'_>,
    ) {
        match (field.type_, value) {
            (Type::Int32 | Type::Sint32 | Type::Sfixed32, Value::Int(v)) => {
                generator.print(&v.to_string())
            }
            (Type::Int64 | Type::Sint64 | Type::Sfixed64, Value::Long(v)) => {
                generator.print(&v.to_string())
            }
            (Type::Bool, Value::Boolean(v)) => generator.print(&v.to_string()),
            (Type::Float, Value::Float(v)) => generator.print(&java_float_to_string(*v)),
            (Type::Double, Value::Double(v)) => generator.print(&java_double_to_string(*v)),
            (Type::Uint32 | Type::Fixed32, Value::Int(v)) => {
                generator.print(&unsigned_to_string_32(*v))
            }
            (Type::Uint64 | Type::Fixed64, Value::Long(v)) => {
                generator.print(&unsigned_to_string_64(*v))
            }
            (Type::String, Value::String(v)) => {
                generator.print("\"");
                if self.escape_non_ascii {
                    generator.print(&escape_bytes(v));
                } else {
                    // `escapeDoubleQuotesAndBackslashes(value).replace("\n", "\\n")` of the Java
                    // string.
                    let s = String::from_utf8_lossy(v);
                    generator.print(&escape_double_quotes_and_backslashes(&s).replace('\n', "\\n"));
                }
                generator.print("\"");
            }
            (Type::Bytes, Value::Bytes(v)) => {
                generator.print("\"");
                generator.print(&escape_bytes(v));
                generator.print("\"");
            }
            (Type::Enum, Value::Enum(number)) => {
                match field.get_enum_type().find_value_by_number(*number) {
                    // Unknown enum value, print the number instead of the name.
                    None => generator.print(&number.to_string()),
                    Some(v) => generator.print(&v.name),
                }
            }
            (Type::Message | Type::Group, Value::Message(m)) => {
                self.print_message_or_any(m, generator)
            }
            (t, v) => panic!("ClassCastException: {v:?} for a field of type {t:?}"),
        }
    }

    // Java: com/google/protobuf/TextFormat.java:653 TextFormat.Printer.printMessage
    fn print_message(&self, message: &DynamicMessage, generator: &mut TextGenerator<'_>) {
        for (field, values) in message.get_all_fields() {
            self.print_field(field, values, generator);
        }
        print_unknown_fields(message.get_unknown_fields(), generator);
    }

    // Java: com/google/protobuf/TextFormat.java:661 TextFormat.Printer.printShortRepeatedField
    fn print_short_repeated_field(
        &self,
        field: &FieldDescriptor,
        values: &[Value],
        generator: &mut TextGenerator<'_>,
    ) {
        generator.print(&field.name);
        generator.print(": ");
        generator.print("[");
        let mut separator = "";
        for element in values {
            generator.print(separator);
            self.print_field_value(field, element, generator);
            separator = ", ";
        }
        generator.print("]");
        generator.eol();
    }

    // Java: com/google/protobuf/TextFormat.java:677 TextFormat.Printer.printSingleField
    fn print_single_field(
        &self,
        field: &FieldDescriptor,
        value: &Value,
        generator: &mut TextGenerator<'_>,
    ) {
        if field.is_extension() {
            unreachable!("no extension is ever a field of a pool descriptor");
        } else if field.is_group_like() {
            // Groups must be serialized with their original capitalization.
            generator.print(&field.get_message_type().name);
        } else {
            generator.print(&field.name);
        }

        if field.get_java_type() == JavaType::Message {
            generator.maybe_print_silent_marker();
            generator.print("{");
            generator.eol();
            generator.indent();
        } else {
            generator.print(":");
            generator.maybe_print_silent_marker();
        }

        self.print_field_value(field, value, generator);

        if field.get_java_type() == JavaType::Message {
            generator.outdent();
            generator.print("}");
        }
        generator.eol();
    }
}

/// An adapter class that can take a `MapEntry` and returns its key and entry.
// Java: com/google/protobuf/TextFormat.java:373 TextFormat.Printer.MapEntryAdapter
struct MapEntryAdapter<'a> {
    entry: &'a Value,
    key_field: Option<&'static FieldDescriptor>,
}

impl<'a> MapEntryAdapter<'a> {
    fn new(entry: &'a Value, field_descriptor: &FieldDescriptor) -> MapEntryAdapter<'a> {
        MapEntryAdapter {
            entry,
            key_field: field_descriptor
                .get_message_type()
                .find_field_by_name("key"),
        }
    }

    /// `getKey()`: the entry's key (`MapEntry.getField(keyField)`: the default when unset).
    fn get_key(&self) -> Option<Value> {
        match (self.entry, self.key_field) {
            (Value::Message(m), Some(key_field)) => Some(
                m.get_field(key_field)
                    .cloned()
                    .unwrap_or_else(|| default_scalar(key_field)),
            ),
            _ => None,
        }
    }

    // Java: com/google/protobuf/TextFormat.java:402 MapEntryAdapter.compareTo
    fn compare_to(&self, b: &MapEntryAdapter<'_>) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let a_key = self.get_key();
        let b_key = b.get_key();
        match (a_key, b_key) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(a), Some(b)) => match (a, b) {
                (Value::Boolean(a), Value::Boolean(b)) => a.cmp(&b),
                (Value::Long(a), Value::Long(b)) => a.cmp(&b),
                (Value::Int(a), Value::Int(b)) => a.cmp(&b),
                // `String.compareTo`: UTF-16 code units.
                (Value::String(a), Value::String(b)) => {
                    let a = String::from_utf8_lossy(&a)
                        .encode_utf16()
                        .collect::<Vec<_>>();
                    let b = String::from_utf8_lossy(&b)
                        .encode_utf16()
                        .collect::<Vec<_>>();
                    a.cmp(&b)
                }
                _ => Ordering::Equal,
            },
        }
    }
}

fn default_scalar(field: &FieldDescriptor) -> Value {
    match field.get_java_type() {
        JavaType::Int => Value::Int(0),
        JavaType::Long => Value::Long(0),
        JavaType::Boolean => Value::Boolean(false),
        JavaType::String => Value::String(Vec::new()),
        JavaType::Float => Value::Float(0.0),
        JavaType::Double => Value::Double(0.0),
        JavaType::ByteString => Value::Bytes(Vec::new()),
        JavaType::Enum => Value::Enum(field.get_enum_type().values[0].number),
        JavaType::Message => Value::Message(DynamicMessage::new(field.get_message_type())),
    }
}

// Java: com/google/protobuf/TextFormat.java:721 TextFormat.Printer.printUnknownFields
fn print_unknown_fields(unknown_fields: &UnknownFieldSet, generator: &mut TextGenerator<'_>) {
    if unknown_fields.is_empty() {
        return;
    }
    for (&number, field) in &unknown_fields.fields {
        for &v in &field.varint {
            print_unknown_field_prefix(number, generator);
            generator.print(&unsigned_to_string_64(v));
            generator.eol();
        }
        for &v in &field.fixed32 {
            print_unknown_field_prefix(number, generator);
            generator.print(&format!("0x{:08x}", v as u32));
            generator.eol();
        }
        for &v in &field.fixed64 {
            print_unknown_field_prefix(number, generator);
            generator.print(&format!("0x{:016x}", v as u64));
            generator.eol();
        }
        for v in &field.length_delimited {
            print_unknown_field_prefix(number, generator);
            match UnknownFieldSet::parse_from(v) {
                Ok(message) => {
                    // Try to parse and print the field as an embedded message
                    generator.print("{");
                    generator.eol();
                    generator.indent();
                    print_unknown_fields(&message, generator);
                    generator.outdent();
                    generator.print("}");
                }
                Err(_) => {
                    // If not parseable as a message, print as a String
                    generator.print("\"");
                    generator.print(&escape_bytes(v));
                    generator.print("\"");
                }
            }
            generator.eol();
        }
        for value in &field.group {
            generator.print(&number.to_string());
            generator.maybe_print_silent_marker();
            generator.print("{");
            generator.eol();
            generator.indent();
            print_unknown_fields(value, generator);
            generator.outdent();
            generator.print("}");
            generator.eol();
        }
    }
}

/// `printUnknownField`'s `number` `:` and marker before each value.
// Java: com/google/protobuf/TextFormat.java:756 TextFormat.Printer.printUnknownField
fn print_unknown_field_prefix(number: i32, generator: &mut TextGenerator<'_>) {
    generator.print(&number.to_string());
    generator.print(":");
    generator.maybe_print_silent_marker();
}

/// Convert an unsigned 32-bit integer to a string.
// Java: com/google/protobuf/TextFormat.java:845 TextFormat.unsignedToString
pub fn unsigned_to_string_32(value: i32) -> String {
    (value as u32).to_string()
}

/// Convert an unsigned 64-bit integer to a string.
// Java: com/google/protobuf/TextFormat.java:854 TextFormat.unsignedToString
pub fn unsigned_to_string_64(value: i64) -> String {
    (value as u64).to_string()
}

/// An inner class for writing text to the output stream.
// Java: com/google/protobuf/TextFormat.java:880 TextFormat.TextGenerator
struct TextGenerator<'a> {
    output: &'a mut String,
    indent: String,
    single_line_mode: bool,
    should_emit_silent_marker: bool,
    // While technically we are "at the start of a line" at the very beginning of the output, all
    // we would do in response to this is emit the (zero length) indentation, so it has no
    // effect. Setting it false here does however suppress an unwanted leading space in
    // single-line mode.
    at_start_of_line: bool,
}

impl<'a> TextGenerator<'a> {
    fn new(
        output: &'a mut String,
        single_line_mode: bool,
        should_emit_silent_marker: bool,
    ) -> TextGenerator<'a> {
        TextGenerator {
            output,
            indent: String::new(),
            single_line_mode,
            should_emit_silent_marker,
            at_start_of_line: false,
        }
    }

    /// Indent text by two spaces.
    fn indent(&mut self) {
        self.indent.push_str("  ");
    }

    /// Reduces the current indent level by two spaces, or crashes if the indent level is zero.
    fn outdent(&mut self) {
        let length = self.indent.len();
        if length == 0 {
            panic!("IllegalArgumentException:  Outdent() without matching Indent().");
        }
        self.indent.truncate(length - 2);
    }

    /// Print text to the output stream.
    fn print(&mut self, text: &str) {
        if self.at_start_of_line {
            self.at_start_of_line = false;
            if self.single_line_mode {
                self.output.push(' ');
            } else {
                self.output.push_str(&self.indent);
            }
        }
        self.output.push_str(text);
    }

    /// Signifies reaching the "end of the current line" in the output.
    fn eol(&mut self) {
        if !self.single_line_mode {
            self.output.push('\n');
        }
        self.at_start_of_line = true;
    }

    fn maybe_print_silent_marker(&mut self) {
        if self.should_emit_silent_marker {
            self.output.push_str(DEBUG_STRING_SILENT_MARKER);
            self.should_emit_silent_marker = false;
        } else {
            self.output.push(' ');
        }
    }
}

/// Escapes bytes in the format used in protocol buffer text format, which is the same as the
/// format used for C string literals. All bytes that are not printable 7-bit ASCII characters
/// are escaped, as well as backslash, single-quote, and double-quote characters. Characters for
/// which no defined short-hand escape sequence is defined will be escaped using 3-digit octal
/// sequences.
// Java: com/google/protobuf/TextFormatEscaper.java:40 TextFormatEscaper.escapeBytes
pub fn escape_bytes(input: &[u8]) -> String {
    let mut builder = String::with_capacity(input.len());
    for &b in input {
        match b {
            0x07 => builder.push_str("\\a"),
            b'\x08' => builder.push_str("\\b"),
            b'\x0c' => builder.push_str("\\f"),
            b'\n' => builder.push_str("\\n"),
            b'\r' => builder.push_str("\\r"),
            b'\t' => builder.push_str("\\t"),
            0x0b => builder.push_str("\\v"),
            b'\\' => builder.push_str("\\\\"),
            b'\'' => builder.push_str("\\'"),
            b'"' => builder.push_str("\\\""),
            _ => {
                // Only ASCII characters between 0x20 (space) and 0x7e (tilde) are
                // printable.  Other byte values must be escaped.
                if (0x20..=0x7e).contains(&b) {
                    builder.push(b as char);
                } else {
                    builder.push('\\');
                    builder.push((b'0' + ((b >> 6) & 3)) as char);
                    builder.push((b'0' + ((b >> 3) & 7)) as char);
                    builder.push((b'0' + (b & 7)) as char);
                }
            }
        }
    }
    builder
}

/// Like [`escape_bytes`], but escapes a text string (its UTF-8 bytes).
// Java: com/google/protobuf/TextFormatEscaper.java:129 TextFormatEscaper.escapeText
pub fn escape_text(input: &str) -> String {
    escape_bytes(input.as_bytes())
}

/// Escape double quotes and backslashes in a String for unicode output of a message.
// Java: com/google/protobuf/TextFormatEscaper.java:134 TextFormatEscaper.escapeDoubleQuotesAndBackslashes
pub fn escape_double_quotes_and_backslashes(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `TypeRegistry.getDescriptorForTypeUrl`'s lookup by full name in the pool.
pub(super) fn find_message_type(name: &str) -> Option<&'static Descriptor> {
    pool().find_message_type_by_name(name)
}
