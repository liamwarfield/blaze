//! The parts of Gson 2.11 (`new Gson()`, default settings) that the credential helper protocol
//! uses: `Gson.toJson` of the request and `Gson.fromJson(Reader, Class)` of the response.
//!
//! Reading goes through `blaze_bzlmod_model::gson::parse_lenient` (Gson's default lenient
//! `JsonReader`, full-consumption check included) and the type adapters then walk the tree.
//!
//! PORT: Gson's type adapters here read the token stream while parsing; walking a parsed tree
//! instead differs only in which `JsonSyntaxException` is thrown first for a document that is
//! both malformed and of the wrong shape (every caller reports any of them with one message),
//! and in a repeated object member name, which the tree keeps once (the last value, at the first
//! position; see `GetCredentialsResponse::from_json`).

use std::fmt;

pub use blaze_bzlmod_model::gson::JsonValue;

/// `com.google.gson.JsonSyntaxException`, with its message (`Throwable.getMessage()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonSyntaxException(pub String);

impl fmt::Display for JsonSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsonSyntaxException {}

/// The result of `Gson.fromJson(json, Class)` before the type adapter runs: `Ok(None)` is Gson's
/// `null` (an empty document; a top-level JSON `null`, which the `@JsonAdapter`'s `nullSafe()`
/// wrapper turns into `null` too).
// Java: com/google/gson/Gson.java Gson.fromJson
pub(crate) fn from_json(json: &str) -> Result<Option<JsonValue>, JsonSyntaxException> {
    match blaze_bzlmod_model::gson::parse_lenient(json) {
        // MalformedJsonException (an IOException) is wrapped: `new JsonSyntaxException(e)`.
        Err(e) => Err(JsonSyntaxException(format!(
            "com.google.gson.stream.MalformedJsonException: {}",
            e.0
        ))),
        Ok(None) | Ok(Some(JsonValue::Null)) => Ok(None),
        Ok(Some(value)) => Ok(Some(value)),
    }
}

/// `JsonWriter.string(value)` with `new Gson()`'s settings (`htmlSafe` true): the
/// `HTML_SAFE_REPLACEMENT_CHARS` table.
// Java: com/google/gson/stream/JsonWriter.java JsonWriter.string
pub(crate) fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
