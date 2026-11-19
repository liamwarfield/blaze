//! Port of `com.google.protobuf.util.JsonFormat` (protobuf-java-util as bundled in the Bazel
//! 9.3.2 server jar), printing side only.
//!
//! Bazel's printer configurations:
//! - query `--output=streamed_jsonproto` (`StreamedJSONProtoOutputFormatter`):
//!   `printer().omittingInsignificantWhitespace()`, one `Target` per line, `'\n'` after each.
//! - cquery `--output=jsonproto` (`cquery/ProtoOutputFormatterCallback`): `printer()` (pretty),
//!   `appendTo`, then `'\n'`.
//! - aquery `--output=jsonproto` (`actiongraph/v2/MonolithicOutputHandler`): `printer()`
//!   (pretty), `appendTo(ActionGraphContainer)`, then `println()`.
//! - `--execution_log_json_file` (`MessageOutputStreamWrapper.JsonOutputStreamWrapper`):
//!   `printer().alwaysPrintFieldsWithNoPresence()` (pretty), messages concatenated with no
//!   separator.
//! - `--build_event_json_file` (`JsonFormatFileTransport`):
//!   `printer().usingTypeRegistry(registry).omittingInsignificantWhitespace()`, `'\n'` after
//!   each event; the registry is `TypeRegistry.newBuilder().add(SpawnExec.getDescriptor())`.
//! - JSON worker protocol (`JsonWorkerProtocol`): `omittingInsignificantWhitespace()`;
//!   (`JsonWorkerMessageProcessor`): `omittingInsignificantWhitespace()
//!   .alwaysPrintFieldsWithNoPresence()`.
//! - `mod show_repo --output=streamed_jsonproto` (`RepoOutputFormatter`):
//!   `omittingInsignificantWhitespace()`.
//!
//! The printer returns a Rust `String`; the bytes a surface writes depend on which kind of
//! string the messages hold (internal or Unicode, `blaze_util::string_encoding`) and on the output
//! stream (see `java::encode_latin1`), as the oracle goldens show: query `streamed_jsonproto`
//! writes `getBytes(UTF_8)` of Unicode strings; cquery `jsonproto` prints Unicode strings through
//! a Latin-1 `PrintStream` ('é' -> 0xE9); aquery `jsonproto` prints internal strings through a
//! Latin-1 `PrintStream` (each comes out as the bytes it stands for); the execution log writes
//! `getBytes(UTF_8)` of internal strings (double-encoded).
//!
//! Verified against the protobuf-java-util bytecode in the oracle's `A-server.jar`
//! (`JsonFormat$PrinterImpl`, `JsonFormat$ParserImpl`, `Timestamps`, `Durations`) and against
//! Java output (`tests/golden.rs`, `tests/synthetic.rs`). `TextFormat` is not ported yet.

use std::collections::BTreeSet;
use std::fmt;

use base64::Engine as _;

use crate::JavaMessage;

/// `com.google.protobuf.InvalidProtocolBufferException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidProtocolBufferException {
    message: String,
}

impl InvalidProtocolBufferException {
    pub fn new(message: impl Into<String>) -> Self {
        InvalidProtocolBufferException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for InvalidProtocolBufferException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidProtocolBufferException {}

/// Prints the content of an `Any` whose type was found in the registry.
pub(crate) type AnyContentPrinter =
    fn(&[u8], &mut JsonPrinter<'_>, &str) -> Result<(), InvalidProtocolBufferException>;

/// Parses the JSON of an `Any` whose type was found in the registry into the content bytes.
pub(crate) type AnyContentParser = fn(
    &serde_json::Map<String, serde_json::Value>,
    &JsonParser<'_>,
) -> Result<Vec<u8>, InvalidProtocolBufferException>;

/// A message type that an `Any` may hold (generated table).
pub(crate) struct AnyEntry {
    pub(crate) file: &'static str,
    pub(crate) print: AnyContentPrinter,
    pub(crate) parse: AnyContentParser,
}

/// `JsonFormat.TypeRegistry`: the message types `Any` fields may be printed as.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    files: BTreeSet<&'static str>,
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
        }
    }

    /// `TypeRegistry.find(name)`.
    fn find(&self, name: &str) -> Option<AnyEntry> {
        let entry = crate::any_entry(name)?;
        self.files.contains(entry.file).then_some(entry)
    }

    /// `TypeRegistry.getDescriptorForTypeUrl(typeUrl)`.
    fn get_descriptor_for_type_url(
        &self,
        type_url: &str,
    ) -> Result<Option<AnyEntry>, InvalidProtocolBufferException> {
        Ok(self.find(get_type_name(type_url)?))
    }
}

/// `JsonFormat.TypeRegistry.Builder`.
#[derive(Clone, Debug)]
pub struct TypeRegistryBuilder {
    files: BTreeSet<&'static str>,
}

impl TypeRegistryBuilder {
    /// `add(Descriptor)`: adds every message of the type's file and of its dependencies.
    pub fn add<M: JavaMessage>(mut self) -> Self {
        self.add_file(M::FILE);
        self
    }

    fn add_file(&mut self, file: &'static str) {
        if !self.files.insert(file) {
            return;
        }
        for dependency in crate::file_dependencies(file) {
            self.add_file(dependency);
        }
    }

    pub fn build(self) -> TypeRegistry {
        TypeRegistry { files: self.files }
    }
}

/// `JsonFormat.getTypeName(typeUrl)`.
fn get_type_name(type_url: &str) -> Result<&str, InvalidProtocolBufferException> {
    // Java String.split drops trailing empty strings.
    let mut parts: Vec<&str> = type_url.split('/').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() == 1 {
        return Err(InvalidProtocolBufferException::new(format!(
            "Invalid type url found: {type_url}"
        )));
    }
    // PORT: Java throws ArrayIndexOutOfBoundsException when every part is empty.
    Ok(parts
        .last()
        .copied()
        .expect("ArrayIndexOutOfBoundsException in JsonFormat.getTypeName"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ShouldPrintDefaults {
    #[default]
    OnlyIfPresent,
    AlwaysPrintWithoutPresenceFields,
}

/// `com.google.protobuf.util.JsonFormat`.
pub struct JsonFormat;

impl JsonFormat {
    /// `JsonFormat.printer()`.
    pub fn printer() -> Printer {
        Printer::default()
    }

    /// `JsonFormat.parser()`.
    pub fn parser() -> Parser {
        Parser::default()
    }
}

/// `JsonFormat.Printer`. Immutable; each option returns a new printer.
#[derive(Clone, Debug, Default)]
pub struct Printer {
    registry: TypeRegistry,
    registry_set: bool,
    should_print_defaults: ShouldPrintDefaults,
    preserving_proto_field_names: bool,
    omitting_insignificant_whitespace: bool,
}

impl Printer {
    /// `usingTypeRegistry(JsonFormat.TypeRegistry)`.
    pub fn using_type_registry(&self, registry: TypeRegistry) -> Printer {
        if self.registry_set {
            panic!("Only one registry is allowed.");
        }
        Printer {
            registry,
            registry_set: true,
            ..self.clone()
        }
    }

    /// `alwaysPrintFieldsWithNoPresence()`.
    pub fn always_print_fields_with_no_presence(&self) -> Printer {
        if self.should_print_defaults != ShouldPrintDefaults::OnlyIfPresent {
            panic!("Only one of the JsonFormat defaults options can be set.");
        }
        Printer {
            should_print_defaults: ShouldPrintDefaults::AlwaysPrintWithoutPresenceFields,
            ..self.clone()
        }
    }

    /// `preservingProtoFieldNames()`.
    pub fn preserving_proto_field_names(&self) -> Printer {
        Printer {
            preserving_proto_field_names: true,
            ..self.clone()
        }
    }

    /// `omittingInsignificantWhitespace()`.
    pub fn omitting_insignificant_whitespace(&self) -> Printer {
        Printer {
            omitting_insignificant_whitespace: true,
            ..self.clone()
        }
    }

    /// `appendTo(MessageOrBuilder, Appendable)`.
    pub fn append_to<M: JavaMessage>(
        &self,
        message: &M,
        out: &mut String,
    ) -> Result<(), InvalidProtocolBufferException> {
        let mut p = JsonPrinter::new(self, out);
        message.print_json(&mut p)
    }

    /// `print(MessageOrBuilder)`.
    pub fn print<M: JavaMessage>(
        &self,
        message: &M,
    ) -> Result<String, InvalidProtocolBufferException> {
        let mut out = String::new();
        self.append_to(message, &mut out)?;
        Ok(out)
    }
}

/// `JsonFormat.PrinterImpl` with its `TextGenerator` (`PrettyTextGenerator` or
/// `CompactTextGenerator`). Generated `JavaMessage::print_json_fields` code drives it.
pub struct JsonPrinter<'a> {
    out: &'a mut String,
    registry: &'a TypeRegistry,
    always_print_no_presence: bool,
    preserving_proto_field_names: bool,
    pretty: bool,
    indent: String,
    at_start_of_line: bool,
    /// One entry per open object, array or map: whether an element was printed.
    printed: Vec<bool>,
}

impl<'a> JsonPrinter<'a> {
    fn new(config: &'a Printer, out: &'a mut String) -> Self {
        JsonPrinter {
            out,
            registry: &config.registry,
            always_print_no_presence: config.should_print_defaults
                == ShouldPrintDefaults::AlwaysPrintWithoutPresenceFields,
            preserving_proto_field_names: config.preserving_proto_field_names,
            pretty: !config.omitting_insignificant_whitespace,
            indent: String::new(),
            at_start_of_line: true,
            printed: Vec::new(),
        }
    }

    fn blank_or_space(&self) -> &'static str {
        if self.pretty { " " } else { "" }
    }

    fn blank_or_new_line(&self) -> &'static str {
        if self.pretty { "\n" } else { "" }
    }

    /// `TextGenerator.print(CharSequence)`.
    fn print(&mut self, text: &str) {
        if !self.pretty {
            self.out.push_str(text);
            return;
        }
        let mut pos = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                self.write(&text[pos..=i]);
                pos = i + 1;
                self.at_start_of_line = true;
            }
        }
        self.write(&text[pos..]);
    }

    fn write(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        if self.at_start_of_line {
            self.at_start_of_line = false;
            self.out.push_str(&self.indent);
        }
        self.out.push_str(data);
    }

    fn indent(&mut self) {
        if self.pretty {
            self.indent.push_str("  ");
        }
    }

    fn outdent(&mut self) {
        if self.pretty {
            if self.indent.is_empty() {
                panic!(" Outdent() without matching Indent().");
            }
            self.indent.truncate(self.indent.len() - 2);
        }
    }

    /// Whether `alwaysPrintFieldsWithNoPresence()` is set.
    pub fn always_print_fields_with_no_presence(&self) -> bool {
        self.always_print_no_presence
    }

    /// Opening of `print(message, typeUrl)`.
    pub fn begin_message(&mut self, type_url: Option<&str>) {
        let nl = self.blank_or_new_line();
        self.print(&format!("{{{nl}"));
        self.indent();
        let mut printed = false;
        if let Some(url) = type_url {
            let s = format!("\"@type\":{}{}", self.blank_or_space(), gson_to_json(url));
            self.print(&s);
            printed = true;
        }
        self.printed.push(printed);
    }

    /// Separator and `"name": ` before a field (`printField`).
    pub fn field(&mut self, json_name: &str, proto_name: &str) {
        let nl = self.blank_or_new_line();
        if self.mark_printed() {
            self.print(&format!(",{nl}"));
        }
        let name = if self.preserving_proto_field_names {
            proto_name
        } else {
            json_name
        };
        let s = format!("\"{name}\":{}", self.blank_or_space());
        self.print(&s);
    }

    /// Closing of `print(message, typeUrl)`.
    pub fn end_message(&mut self) {
        let printed = self.printed.pop().expect("unbalanced end_message");
        if printed {
            let nl = self.blank_or_new_line();
            self.print(nl);
        }
        self.outdent();
        self.print("}");
    }

    /// Opening of `printRepeatedFieldValue`.
    pub fn begin_array(&mut self) {
        self.print("[");
        self.printed.push(false);
    }

    /// Separator before a repeated element.
    pub fn array_element(&mut self) {
        if self.mark_printed() {
            let s = format!(",{}", self.blank_or_space());
            self.print(&s);
        }
    }

    pub fn end_array(&mut self) {
        self.printed.pop().expect("unbalanced end_array");
        self.print("]");
    }

    /// Opening of `printMapFieldValue`.
    pub fn begin_map(&mut self) {
        let nl = self.blank_or_new_line();
        self.print(&format!("{{{nl}"));
        self.indent();
        self.printed.push(false);
    }

    /// Separator before a map key.
    pub fn map_key(&mut self) {
        if self.mark_printed() {
            let s = format!(",{}", self.blank_or_new_line());
            self.print(&s);
        }
    }

    /// Between a map key and its value.
    pub fn map_value(&mut self) {
        let s = format!(":{}", self.blank_or_space());
        self.print(&s);
    }

    pub fn end_map(&mut self) {
        let printed = self.printed.pop().expect("unbalanced end_map");
        if printed {
            let nl = self.blank_or_new_line();
            self.print(nl);
        }
        self.outdent();
        self.print("}");
    }

    /// Returns whether an element was already printed at this level, and marks one printed.
    fn mark_printed(&mut self) -> bool {
        let top = self.printed.last_mut().expect("no open JSON container");
        std::mem::replace(top, true)
    }

    fn quoted(&mut self, always_with_quotes: bool, s: &str) {
        if always_with_quotes {
            self.print("\"");
        }
        self.print(s);
        if always_with_quotes {
            self.print("\"");
        }
    }

    // printSingleFieldValue, per field type.

    /// INT32, SINT32, SFIXED32.
    pub fn print_i32(&mut self, v: i32, always_with_quotes: bool) {
        self.quoted(always_with_quotes, &v.to_string());
    }

    /// INT64, SINT64, SFIXED64: always quoted.
    pub fn print_i64(&mut self, v: i64) {
        self.print(&format!("\"{v}\""));
    }

    /// UINT32, FIXED32.
    pub fn print_u32(&mut self, v: u32, always_with_quotes: bool) {
        self.quoted(always_with_quotes, &v.to_string());
    }

    /// UINT64, FIXED64: always quoted.
    pub fn print_u64(&mut self, v: u64) {
        self.print(&format!("\"{v}\""));
    }

    /// BOOL.
    pub fn print_bool(&mut self, v: bool, always_with_quotes: bool) {
        self.quoted(always_with_quotes, if v { "true" } else { "false" });
    }

    /// FLOAT.
    pub fn print_f32(&mut self, v: f32, always_with_quotes: bool) {
        if v.is_nan() {
            self.print("\"NaN\"");
        } else if v.is_infinite() {
            self.print(if v < 0.0 {
                "\"-Infinity\""
            } else {
                "\"Infinity\""
            });
        } else {
            self.quoted(always_with_quotes, &java_float_to_string(v));
        }
    }

    /// DOUBLE.
    pub fn print_f64(&mut self, v: f64, always_with_quotes: bool) {
        if v.is_nan() {
            self.print("\"NaN\"");
        } else if v.is_infinite() {
            self.print(if v < 0.0 {
                "\"-Infinity\""
            } else {
                "\"Infinity\""
            });
        } else {
            self.quoted(always_with_quotes, &java_double_to_string(v));
        }
    }

    /// STRING: `gson.toJson(value)`.
    pub fn print_string(&mut self, v: &str) {
        self.print(&gson_to_json(v));
    }

    /// BYTES: `Base64.getEncoder()`.
    pub fn print_bytes(&mut self, v: &[u8]) {
        let s = format!(
            "\"{}\"",
            base64::engine::general_purpose::STANDARD.encode(v)
        );
        self.print(&s);
    }

    /// ENUM: the value name, or the number when the value is unknown.
    pub fn print_enum(
        &mut self,
        v: i32,
        name: fn(i32) -> Option<&'static str>,
        is_null_value: bool,
        always_with_quotes: bool,
    ) {
        if is_null_value {
            self.quoted(always_with_quotes, "null");
        } else if let Some(name) = name(v) {
            self.print(&format!("\"{name}\""));
        } else {
            self.print(&v.to_string());
        }
    }

    /// `printTimestamp`.
    pub fn print_timestamp(
        &mut self,
        seconds: i64,
        nanos: i32,
    ) -> Result<(), InvalidProtocolBufferException> {
        let s = format!("\"{}\"", timestamps_to_string(seconds, nanos));
        self.print(&s);
        Ok(())
    }

    /// `printDuration`.
    pub fn print_duration(
        &mut self,
        seconds: i64,
        nanos: i32,
    ) -> Result<(), InvalidProtocolBufferException> {
        let s = format!("\"{}\"", durations_to_string(seconds, nanos));
        self.print(&s);
        Ok(())
    }

    /// `printFieldMask`: `FieldMaskUtil.toJsonString`, unescaped.
    pub fn print_field_mask(
        &mut self,
        paths: &[String],
    ) -> Result<(), InvalidProtocolBufferException> {
        let joined: Vec<String> = paths
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| lower_underscore_to_lower_camel(p))
            .collect();
        let s = format!("\"{}\"", joined.join(","));
        self.print(&s);
        Ok(())
    }
}

/// `PrinterImpl.printAny`.
pub(crate) fn print_any(
    type_url: &str,
    value: &[u8],
    p: &mut JsonPrinter<'_>,
) -> Result<(), InvalidProtocolBufferException> {
    if type_url.is_empty() && value.is_empty() {
        p.print("{}");
        return Ok(());
    }
    // PORT: Java consults `registry` (com.google.protobuf.TypeRegistry, never set by Bazel, so
    // empty) and then `oldRegistry` (JsonFormat.TypeRegistry); both validate the url the same way.
    let Some(entry) = p.registry.get_descriptor_for_type_url(type_url)? else {
        return Err(InvalidProtocolBufferException::new(format!(
            "Cannot find type for url: {type_url}"
        )));
    };
    (entry.print)(value, p, type_url)
}

/// Second half of `PrinterImpl.printAny`: the content message parsed and printed.
pub(crate) fn print_any_content<M: JavaMessage>(
    bytes: &[u8],
    p: &mut JsonPrinter<'_>,
    type_url: &str,
) -> Result<(), InvalidProtocolBufferException> {
    // PORT: the message of a parse failure is prost's, not Java's.
    let content =
        M::decode(bytes).map_err(|e| InvalidProtocolBufferException::new(e.to_string()))?;
    if M::JSON_WELL_KNOWN {
        let nl = p.blank_or_new_line();
        let bs = p.blank_or_space();
        p.print(&format!("{{{nl}"));
        p.indent();
        p.print(&format!("\"@type\":{bs}{},{nl}", gson_to_json(type_url)));
        p.print(&format!("\"value\":{bs}"));
        content.print_json(p)?;
        p.print(nl);
        p.outdent();
        p.print("}");
    } else {
        p.begin_message(Some(type_url));
        content.print_json_fields(p)?;
        p.end_message();
    }
    Ok(())
}

/// `Gson.toJson(String)` with Gson's default (HTML-safe) escaping, as `JsonFormat` uses it.
pub fn gson_to_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '=' => out.push_str("\\u003d"),
            '\'' => out.push_str("\\u0027"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Splits Rust's shortest round-trip `{:e}` rendering into (digits, decimal exponent of the
/// first digit).
fn split_exp(s: &str) -> (String, i32) {
    let (mantissa, exp) = s.split_once('e').expect("exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    (digits, exp.parse().expect("exponent"))
}

/// Java (JDK 19+) `Double.toString` / `Float.toString` layout of a decimal `0.d1...dn x 10^(e+1)`.
fn java_layout(negative: bool, shortest: &str, two_digits: &str) -> String {
    let (mut digits, mut exp) = split_exp(shortest);
    if digits.len() == 1 {
        // Java picks the closest decimal of length 1 or 2 when the shortest has length 1.
        let (d2, e2) = split_exp(two_digits);
        digits = d2.trim_end_matches('0').to_string();
        if digits.is_empty() {
            digits.push('0');
        }
        exp = e2;
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    let n = digits.len() as i32;
    if (-3..7).contains(&exp) {
        if exp >= 0 {
            let int_len = exp + 1;
            if n <= int_len {
                out.push_str(&digits);
                for _ in 0..(int_len - n) {
                    out.push('0');
                }
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len as usize]);
                out.push('.');
                out.push_str(&digits[int_len as usize..]);
            }
        } else {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if n > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}

/// `Double.toString(double)` (JDK 19+ shortest-decimal algorithm).
pub fn java_double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = v.abs();
    java_layout(v < 0.0, &format!("{a:e}"), &format!("{a:.1e}"))
}

/// `Float.toString(float)` (JDK 19+ shortest-decimal algorithm).
pub fn java_float_to_string(v: f32) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let a = v.abs();
    java_layout(v < 0.0, &format!("{a:e}"), &format!("{a:.1e}"))
}

/// `Timestamps.formatNanos`.
fn format_nanos(nanos: i32) -> String {
    if nanos % 1_000_000 == 0 {
        format!("{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!("{:06}", nanos / 1_000)
    } else {
        format!("{nanos:09}")
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `Timestamps.toString(Timestamp)`. Invalid values panic like Java's unchecked
/// `IllegalArgumentException`.
pub fn timestamps_to_string(seconds: i64, nanos: i32) -> String {
    const MIN: i64 = -62_135_596_800;
    const MAX: i64 = 253_402_300_799;
    if !(MIN..=MAX).contains(&seconds) || !(0..=999_999_999).contains(&nanos) {
        panic!(
            "Timestamp is not valid. See proto definition for valid values. Seconds ({seconds}) must be in range [-62,135,596,800, +253,402,300,799]. Nanos ({nanos}) must be in range [0, +999,999,999]."
        );
    }
    let days = seconds.div_euclid(86_400);
    let secs = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let mut out = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    );
    if nanos != 0 {
        out.push('.');
        out.push_str(&format_nanos(nanos));
    }
    out.push('Z');
    out
}

/// `Durations.toString(Duration)`. Invalid values panic like Java's unchecked
/// `IllegalArgumentException`.
pub fn durations_to_string(seconds: i64, nanos: i32) -> String {
    const MAX: i64 = 315_576_000_000;
    let valid = (-MAX..=MAX).contains(&seconds)
        && (-999_999_999..=999_999_999).contains(&nanos)
        && !((seconds < 0 && nanos > 0) || (seconds > 0 && nanos < 0));
    if !valid {
        panic!(
            "Duration is not valid. See proto definition for valid values. Seconds ({seconds}) must be in range [-315,576,000,000, +315,576,000,000]. Nanos ({nanos}) must be in range [-999,999,999, +999,999,999]. Nanos must have the same sign as seconds"
        );
    }
    let mut out = String::new();
    let (mut seconds, mut nanos) = (seconds, nanos);
    if seconds < 0 || nanos < 0 {
        out.push('-');
        seconds = -seconds;
        nanos = -nanos;
    }
    out.push_str(&seconds.to_string());
    if nanos != 0 {
        out.push('.');
        out.push_str(&format_nanos(nanos));
    }
    out.push('s');
    out
}

/// Guava `CaseFormat.LOWER_UNDERSCORE.to(LOWER_CAMEL, s)` for ASCII field paths.
fn lower_underscore_to_lower_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, word) in s.split('_').enumerate() {
        if i == 0 {
            out.push_str(&word.to_ascii_lowercase());
        } else {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(&chars.as_str().to_ascii_lowercase());
            }
        }
    }
    out
}

/// `JsonFormat.Parser`. Immutable; each option returns a new parser.
#[derive(Clone, Debug, Default)]
pub struct Parser {
    registry: TypeRegistry,
    registry_set: bool,
    ignoring_unknown_fields: bool,
}

impl Parser {
    /// `usingTypeRegistry(JsonFormat.TypeRegistry)`.
    pub fn using_type_registry(&self, registry: TypeRegistry) -> Parser {
        if self.registry_set {
            panic!("Only one registry is allowed.");
        }
        Parser {
            registry,
            registry_set: true,
            ..self.clone()
        }
    }

    /// `ignoringUnknownFields()`.
    pub fn ignoring_unknown_fields(&self) -> Parser {
        Parser {
            ignoring_unknown_fields: true,
            ..self.clone()
        }
    }

    /// `merge(String json, Message.Builder builder)`.
    // PORT: syntax errors carry serde_json's message, not Gson's.
    pub fn merge<M: JavaMessage>(
        &self,
        json: &str,
        builder: &mut M,
    ) -> Result<(), InvalidProtocolBufferException> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| InvalidProtocolBufferException::new(e.to_string()))?;
        self.merge_value(&value, builder)
    }

    /// `ParserImpl.merge(JsonElement, Message.Builder)` on an already parsed JSON value.
    pub fn merge_value<M: JavaMessage>(
        &self,
        json: &serde_json::Value,
        builder: &mut M,
    ) -> Result<(), InvalidProtocolBufferException> {
        let p = JsonParser {
            registry: &self.registry,
            ignoring_unknown_fields: self.ignoring_unknown_fields,
        };
        builder.merge_json(json, &p)
    }
}

/// `JsonFormat.ParserImpl`. Generated `JavaMessage::merge_json_fields` code drives it.
pub struct JsonParser<'a> {
    registry: &'a TypeRegistry,
    ignoring_unknown_fields: bool,
}

/// `JsonElement.getAsString()` for primitives.
fn as_string(json: &serde_json::Value) -> Option<String> {
    match json {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `new BigDecimal(text)` reduced to an exact integer: `Err` when `text` is not a decimal
/// number, `Ok(None)` when it has a fractional part or does not fit.
fn decimal_to_integer(text: &str) -> Result<Option<i128>, ()> {
    let (mantissa, exp) = match text.find(['e', 'E']) {
        Some(i) => (&text[..i], text[i + 1..].parse::<i32>().map_err(|_| ())?),
        None => (text, 0),
    };
    let (negative, mantissa) = match mantissa.as_bytes().first() {
        Some(b'-') => (true, &mantissa[1..]),
        Some(b'+') => (false, &mantissa[1..]),
        _ => (false, mantissa),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int_part.is_empty() && frac_part.is_empty()
        || !int_part
            .bytes()
            .chain(frac_part.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(());
    }
    let mut digits: String = format!("{int_part}{frac_part}");
    let mut scale = frac_part.len() as i64 - exp as i64;
    while scale > 0 && digits.ends_with('0') {
        digits.pop();
        scale -= 1;
    }
    if scale > 0 {
        return Ok(None);
    }
    let digits = digits.trim_start_matches('0');
    if digits.len() as i64 - scale > 38 {
        return Ok(None);
    }
    let mut value: i128 = if digits.is_empty() {
        0
    } else {
        digits.parse().map_err(|_| ())?
    };
    for _ in 0..(-scale) {
        if value != 0 {
            value = match value.checked_mul(10) {
                Some(v) => v,
                None => return Ok(None),
            };
        }
    }
    Ok(Some(if negative { -value } else { value }))
}

impl JsonParser<'_> {
    /// Whether `ignoringUnknownFields()` is set.
    pub fn ignoring_unknown_fields(&self) -> bool {
        self.ignoring_unknown_fields
    }

    /// An object member that names no field of `message` (`mergeMessage`).
    pub fn unknown_field(
        &self,
        name: &str,
        message: &str,
    ) -> Result<(), InvalidProtocolBufferException> {
        if self.ignoring_unknown_fields {
            return Ok(());
        }
        Err(InvalidProtocolBufferException::new(format!(
            "Cannot find field: {name} in message {message}"
        )))
    }

    /// `mergeRepeatedField`: the value must be an array.
    pub fn expect_array<'v>(
        &self,
        json: &'v serde_json::Value,
        field: &str,
    ) -> Result<&'v Vec<serde_json::Value>, InvalidProtocolBufferException> {
        json.as_array().ok_or_else(|| {
            InvalidProtocolBufferException::new(format!(
                "Expected an array for {field} but found {json}"
            ))
        })
    }

    /// `mergeMapField`: the value must be an object.
    pub fn expect_map<'v>(
        &self,
        json: &'v serde_json::Value,
    ) -> Result<&'v serde_json::Map<String, serde_json::Value>, InvalidProtocolBufferException>
    {
        json.as_object().ok_or_else(|| {
            InvalidProtocolBufferException::new(format!("Expect a map object but found: {json}"))
        })
    }

    /// A `null` element of a repeated field.
    pub fn null_element(&self, field: &str) -> InvalidProtocolBufferException {
        InvalidProtocolBufferException::new(format!(
            "Repeated field elements cannot be null in field: {field}"
        ))
    }

    /// `parseInt32`.
    pub fn parse_i32(
        &self,
        json: &serde_json::Value,
    ) -> Result<i32, InvalidProtocolBufferException> {
        let err = || InvalidProtocolBufferException::new(format!("Not an int32 value: {json}"));
        let text = as_string(json).ok_or_else(err)?;
        if let Ok(v) = text.parse::<i32>() {
            return Ok(v);
        }
        match decimal_to_integer(&text) {
            Ok(Some(v)) => i32::try_from(v).map_err(|_| err()),
            _ => Err(err()),
        }
    }

    /// `parseInt64`.
    pub fn parse_i64(
        &self,
        json: &serde_json::Value,
    ) -> Result<i64, InvalidProtocolBufferException> {
        let err = || InvalidProtocolBufferException::new(format!("Not an int64 value: {json}"));
        let text = as_string(json).ok_or_else(err)?;
        if let Ok(v) = text.parse::<i64>() {
            return Ok(v);
        }
        match decimal_to_integer(&text) {
            Ok(Some(v)) => i64::try_from(v).map_err(|_| err()),
            _ => Err(err()),
        }
    }

    /// `parseUint32`.
    pub fn parse_u32(
        &self,
        json: &serde_json::Value,
    ) -> Result<u32, InvalidProtocolBufferException> {
        let not = || InvalidProtocolBufferException::new(format!("Not an uint32 value: {json}"));
        let range =
            || InvalidProtocolBufferException::new(format!("Out of range uint32 value: {json}"));
        let text = as_string(json).ok_or_else(not)?;
        if let Ok(v) = text.parse::<i64>() {
            return u32::try_from(v).map_err(|_| range());
        }
        match decimal_to_integer(&text) {
            Ok(Some(v)) => u32::try_from(v).map_err(|_| range()),
            // PORT: Java reports values too large for BigDecimal arithmetic here as out of range.
            Ok(None) => Err(not()),
            Err(()) => Err(not()),
        }
    }

    /// `parseUint64`.
    pub fn parse_u64(
        &self,
        json: &serde_json::Value,
    ) -> Result<u64, InvalidProtocolBufferException> {
        let not = || InvalidProtocolBufferException::new(format!("Not an uint64 value: {json}"));
        let range =
            || InvalidProtocolBufferException::new(format!("Out of range uint64 value: {json}"));
        let text = as_string(json).ok_or_else(not)?;
        match decimal_to_integer(&text) {
            Ok(Some(v)) => u64::try_from(v).map_err(|_| range()),
            _ => Err(not()),
        }
    }

    /// `parseBool`.
    pub fn parse_bool(
        &self,
        json: &serde_json::Value,
    ) -> Result<bool, InvalidProtocolBufferException> {
        match as_string(json).as_deref() {
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            _ => Err(InvalidProtocolBufferException::new(format!(
                "Invalid bool value: {json}"
            ))),
        }
    }

    /// `parseFloat`.
    pub fn parse_f32(
        &self,
        json: &serde_json::Value,
    ) -> Result<f32, InvalidProtocolBufferException> {
        let text = as_string(json).ok_or_else(|| {
            InvalidProtocolBufferException::new(format!("Not a float value: {json}"))
        })?;
        match text.as_str() {
            "NaN" => return Ok(f32::NAN),
            "Infinity" => return Ok(f32::INFINITY),
            "-Infinity" => return Ok(f32::NEG_INFINITY),
            _ => {}
        }
        let v: f64 = text.parse().map_err(|_| {
            InvalidProtocolBufferException::new(format!("Not a float value: {json}"))
        })?;
        if !(-3.402826869208755E38..=3.402826869208755E38).contains(&v) {
            return Err(InvalidProtocolBufferException::new(format!(
                "Out of range float value: {json}"
            )));
        }
        Ok(v as f32)
    }

    /// `parseDouble`.
    pub fn parse_f64(
        &self,
        json: &serde_json::Value,
    ) -> Result<f64, InvalidProtocolBufferException> {
        let text = as_string(json).ok_or_else(|| {
            InvalidProtocolBufferException::new(format!("Not a double value: {json}"))
        })?;
        match text.as_str() {
            "NaN" => return Ok(f64::NAN),
            "Infinity" => return Ok(f64::INFINITY),
            "-Infinity" => return Ok(f64::NEG_INFINITY),
            _ => {}
        }
        let v: f64 = text.parse().map_err(|_| {
            InvalidProtocolBufferException::new(format!("Not a double value: {json}"))
        })?;
        if v.is_infinite() {
            return Err(InvalidProtocolBufferException::new(format!(
                "Out of range double value: {json}"
            )));
        }
        Ok(v)
    }

    /// STRING: `json.getAsString()`.
    pub fn parse_string(
        &self,
        json: &serde_json::Value,
    ) -> Result<String, InvalidProtocolBufferException> {
        as_string(json).ok_or_else(|| {
            InvalidProtocolBufferException::new(format!("Unexpected json data: {json}"))
        })
    }

    /// `parseBytes`: standard Base64, then URL-safe Base64.
    pub fn parse_bytes(
        &self,
        json: &serde_json::Value,
    ) -> Result<Vec<u8>, InvalidProtocolBufferException> {
        use base64::engine::general_purpose::{STANDARD, URL_SAFE};
        let text = self.parse_string(json)?;
        STANDARD
            .decode(&text)
            .or_else(|_| URL_SAFE.decode(&text))
            .map_err(|e| InvalidProtocolBufferException::new(e.to_string()))
    }

    /// `parseEnum`. `Ok(None)`: unknown value skipped under `ignoringUnknownFields()`.
    pub fn parse_enum(
        &self,
        json: &serde_json::Value,
        by_name: fn(&str) -> Option<i32>,
        by_number: fn(i32) -> Option<&'static str>,
        closed: bool,
        full_name: &str,
    ) -> Result<Option<i32>, InvalidProtocolBufferException> {
        let value = as_string(json).unwrap_or_default();
        if let Some(n) = by_name(&value) {
            return Ok(Some(n));
        }
        let mut result = None;
        if let Ok(n) = self.parse_i32(json)
            && (!closed || by_number(n).is_some())
        {
            result = Some(n);
        }
        if result.is_none() && !self.ignoring_unknown_fields {
            return Err(InvalidProtocolBufferException::new(format!(
                "Invalid enum value: {value} for enum type: {full_name}"
            )));
        }
        Ok(result)
    }

    /// `mergeTimestamp`.
    pub fn parse_timestamp(
        &self,
        json: &serde_json::Value,
    ) -> Result<(i64, i32), InvalidProtocolBufferException> {
        as_string(json)
            .and_then(|s| timestamps_parse(&s))
            .ok_or_else(|| {
                InvalidProtocolBufferException::new(format!("Failed to parse timestamp: {json}"))
            })
    }

    /// `mergeDuration`.
    pub fn parse_duration(
        &self,
        json: &serde_json::Value,
    ) -> Result<(i64, i32), InvalidProtocolBufferException> {
        as_string(json)
            .and_then(|s| durations_parse(&s))
            .ok_or_else(|| {
                InvalidProtocolBufferException::new(format!("Failed to parse duration: {json}"))
            })
    }

    /// `mergeFieldMask`: `FieldMaskUtil.fromJsonString`.
    pub fn parse_field_mask(
        &self,
        json: &serde_json::Value,
    ) -> Result<Vec<String>, InvalidProtocolBufferException> {
        let text = self.parse_string(json)?;
        Ok(text
            .split(',')
            .filter(|p| !p.is_empty())
            .map(lower_camel_to_lower_underscore)
            .collect())
    }
}

/// `ParserImpl.mergeAny`.
pub(crate) fn parse_any(
    type_url_out: &mut String,
    value_out: &mut Vec<u8>,
    json: &serde_json::Value,
    p: &JsonParser<'_>,
) -> Result<(), InvalidProtocolBufferException> {
    let Some(object) = json.as_object() else {
        return Err(InvalidProtocolBufferException::new(format!(
            "Expect message object but got: {json}"
        )));
    };
    if object.is_empty() {
        return Ok(());
    }
    let Some(type_url_element) = object.get("@type") else {
        return Err(InvalidProtocolBufferException::new(format!(
            "Missing type url when parsing: {json}"
        )));
    };
    let type_url = as_string(type_url_element).unwrap_or_default();
    let Some(entry) = p.registry.get_descriptor_for_type_url(&type_url)? else {
        return Err(InvalidProtocolBufferException::new(format!(
            "Cannot resolve type: {type_url}"
        )));
    };
    let bytes = (entry.parse)(object, p)?;
    *type_url_out = type_url;
    *value_out = bytes;
    Ok(())
}

/// Content half of `ParserImpl.mergeAny`.
pub(crate) fn parse_any_content<M: JavaMessage>(
    object: &serde_json::Map<String, serde_json::Value>,
    p: &JsonParser<'_>,
) -> Result<Vec<u8>, InvalidProtocolBufferException> {
    let mut content = M::default();
    if M::JSON_WELL_KNOWN {
        if let Some(value) = object.get("value") {
            content.merge_json(value, p)?;
        }
    } else {
        let mut rest = object.clone();
        rest.remove("@type");
        content.merge_json_fields(&rest, p)?;
    }
    Ok(content.to_byte_array())
}

/// `Timestamps.parseNanos`.
fn parse_nanos(value: &str) -> Option<i32> {
    let mut result: i32 = 0;
    let bytes = value.as_bytes();
    for i in 0..9 {
        result *= 10;
        if i < bytes.len() {
            let b = bytes[i];
            if !b.is_ascii_digit() {
                return None;
            }
            result += (b - b'0') as i32;
        }
    }
    if bytes.len() > 9 {
        return None;
    }
    Some(result)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Timestamps.parse(String)`; `None` on any `ParseException` or invalid result.
// PORT: SimpleDateFormat leniency is not reproduced; only the canonical
// `yyyy-MM-ddTHH:mm:ss` layout is accepted.
fn timestamps_parse(value: &str) -> Option<(i64, i32)> {
    let day_offset = value.find('T')?;
    let tz = value[day_offset..]
        .find('Z')
        .or_else(|| value[day_offset..].find('+'))
        .or_else(|| value[day_offset..].find('-'))?
        + day_offset;
    let time_value = &value[..tz];
    let (second_value, nano_value) = time_value.split_once('.').unwrap_or((time_value, ""));
    let b = second_value.as_bytes();
    if b.len() != 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let s = &second_value[r];
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, mo, d, h, mi, s) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let mut seconds = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    let nanos = if nano_value.is_empty() {
        0
    } else {
        parse_nanos(nano_value)?
    };
    if value.as_bytes()[tz] == b'Z' {
        if value.len() != tz + 1 {
            return None;
        }
    } else {
        let offset_value = &value[tz + 1..];
        let (hh, mm) = offset_value.split_once(':')?;
        let offset = hh.parse::<i64>().ok()? * 3600 + mm.parse::<i64>().ok()? * 60;
        if value.as_bytes()[tz] == b'+' {
            seconds -= offset;
        } else {
            seconds += offset;
        }
    }
    let valid = (-62_135_596_800..=253_402_300_799).contains(&seconds);
    valid.then_some((seconds, nanos))
}

/// `Durations.parse(String)`; `None` on any `ParseException` or invalid result.
fn durations_parse(value: &str) -> Option<(i64, i32)> {
    let mut value = value;
    if value.is_empty() || !value.ends_with('s') {
        return None;
    }
    let mut negative = false;
    if let Some(rest) = value.strip_prefix('-') {
        negative = true;
        value = rest;
    }
    let body = &value[..value.len() - 1];
    let (second_value, nano_value) = body.split_once('.').unwrap_or((body, ""));
    let mut seconds: i64 = second_value.parse().ok()?;
    let mut nanos = if nano_value.is_empty() {
        0
    } else {
        parse_nanos(nano_value)?
    };
    if seconds < 0 {
        return None;
    }
    if negative {
        seconds = -seconds;
        nanos = -nanos;
    }
    let valid = (-315_576_000_000..=315_576_000_000).contains(&seconds);
    valid.then_some((seconds, nanos))
}

/// Guava `CaseFormat.LOWER_CAMEL.to(LOWER_UNDERSCORE, s)` for ASCII field paths.
fn lower_camel_to_lower_underscore(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_to_string_matches_java() {
        // Expected values: OpenJDK 25 Double.toString (same algorithm as Bazel's JDK 21).
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (-1.5, "-1.5"),
            (100.0, "100.0"),
            (1234567.0, "1234567.0"),
            (1.0e7, "1.0E7"),
            (12345678.9, "1.23456789E7"),
            (0.001, "0.001"),
            (0.0001, "1.0E-4"),
            (0.1 + 0.2, "0.30000000000000004"),
            (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
            (5e-324, "4.9E-324"),
            (f64::MAX, "1.7976931348623157E308"),
            (1.0e23, "1.0E23"),
            (2.0e-3, "0.002"),
            (9.0e15, "9.0E15"),
            (2e-323, "2.0E-323"),
            (1e22, "1.0E22"),
            (123456789012345678.0, "1.2345678901234568E17"),
        ];
        for (v, want) in cases {
            assert_eq!(java_double_to_string(*v), *want, "{v:e}");
        }
    }

    #[test]
    fn float_to_string_matches_java() {
        let cases: &[(f32, &str)] = &[
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1.0e10, "1.0E10"),
            (f32::MIN_POSITIVE, "1.1754944E-38"),
            (1.4e-45, "1.4E-45"),
            (f32::MAX, "3.4028235E38"),
            (3.3e-44, "3.4E-44"),
            (1.0e-3, "0.001"),
            (9.999999e6, "9999999.0"),
        ];
        for (v, want) in cases {
            assert_eq!(java_float_to_string(*v), *want, "{v:e}");
        }
    }

    #[test]
    fn gson_escapes_html_characters() {
        assert_eq!(
            gson_to_json("a=b <c> & 'd' \"e\" \\ \u{1} \u{7f} é\u{2028}"),
            "\"a\\u003db \\u003cc\\u003e \\u0026 \\u0027d\\u0027 \\\"e\\\" \\\\ \\u0001 \u{7f} é\\u2028\""
        );
    }

    #[test]
    fn timestamps_and_durations() {
        assert_eq!(timestamps_to_string(0, 0), "1970-01-01T00:00:00Z");
        assert_eq!(
            timestamps_to_string(1_700_000_000, 5_000_000),
            "2023-11-14T22:13:20.005Z"
        );
        assert_eq!(
            timestamps_to_string(-62_135_596_800, 1),
            "0001-01-01T00:00:00.000000001Z"
        );
        assert_eq!(
            timestamps_to_string(253_402_300_799, 1_000),
            "9999-12-31T23:59:59.000001Z"
        );
        assert_eq!(durations_to_string(0, 0), "0s");
        assert_eq!(durations_to_string(1, 500_000_000), "1.500s");
        assert_eq!(durations_to_string(-1, -1), "-1.000000001s");
        assert_eq!(durations_to_string(0, -1000), "-0.000001s");
    }

    #[test]
    fn type_name_from_url() {
        assert_eq!(get_type_name("type.googleapis.com/a.B").unwrap(), "a.B");
        assert_eq!(
            get_type_name("a.B").unwrap_err().get_message(),
            "Invalid type url found: a.B"
        );
        assert!(get_type_name("x/a.B/").is_ok());
    }
}
