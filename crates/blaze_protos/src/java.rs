//! Java-compatible binary encoding (`com.google.protobuf.CodedOutputStream` as driven by
//! protoc's Java `writeTo`).
//!
//! Java's generated `writeTo` writes fields sorted by field number; a set oneof member is written
//! at its own number; proto3 implicit-presence scalars are skipped when they equal the default
//! (floats compared by raw bits, so `-0.0` is written); proto2 and explicit-presence fields are
//! written when set; packed repeated fields are skipped when empty; map entries always write
//! key and value. Unknown fields are never produced by Blaze and are not kept.

use prost::encoding::{WireType, encode_key, encode_varint, encoded_len_varint, key_len};

use crate::json::{InvalidProtocolBufferException, JsonParser, JsonPrinter};

/// A generated message: Java binary encoding and `JsonFormat` printing. Implemented by
/// `build.rs` for every message type in the crate.
pub trait JavaMessage: prost::Message + Default + Sized {
    /// `Descriptor.getFullName()`.
    const FULL_NAME: &'static str;
    /// `FileDescriptor.getName()` of the declaring file.
    const FILE: &'static str;
    /// Whether `JsonFormat` prints this type with a well-known-type printer.
    const JSON_WELL_KNOWN: bool;

    /// `getSerializedSize()`.
    fn java_encoded_len(&self) -> usize;

    /// `writeTo(CodedOutputStream)`: the fields, without a length prefix.
    fn java_encode_raw(&self, buf: &mut Vec<u8>);

    /// The fields of this message in `JsonFormat.PrinterImpl.print(message, typeUrl)` order,
    /// without the enclosing braces.
    fn print_json_fields(
        &self,
        p: &mut JsonPrinter<'_>,
    ) -> Result<(), InvalidProtocolBufferException>;

    /// `JsonFormat.PrinterImpl.print(MessageOrBuilder)`: the message as a JSON value.
    fn print_json(&self, p: &mut JsonPrinter<'_>) -> Result<(), InvalidProtocolBufferException> {
        p.begin_message(None);
        self.print_json_fields(p)?;
        p.end_message();
        Ok(())
    }

    /// Merges the members of a JSON object into this message (`ParserImpl.mergeMessage`).
    fn merge_json_fields(
        &mut self,
        obj: &serde_json::Map<String, serde_json::Value>,
        p: &JsonParser<'_>,
    ) -> Result<(), InvalidProtocolBufferException>;

    /// `ParserImpl.merge(JsonElement, Message.Builder)`: well-known types override this.
    fn merge_json(
        &mut self,
        v: &serde_json::Value,
        p: &JsonParser<'_>,
    ) -> Result<(), InvalidProtocolBufferException> {
        match v.as_object() {
            Some(obj) => self.merge_json_fields(obj, p),
            None => Err(InvalidProtocolBufferException::new(format!(
                "Expect message object but got: {v}"
            ))),
        }
    }

    /// `CodedOutputStream.writeMessage(fieldNumber, value)`.
    fn java_encode_field(&self, field_number: u32, buf: &mut Vec<u8>) {
        encode_key(field_number, WireType::LengthDelimited, buf);
        encode_varint(self.java_encoded_len() as u64, buf);
        self.java_encode_raw(buf);
    }

    /// `CodedOutputStream.computeMessageSize(fieldNumber, value)`.
    fn java_encoded_field_len(&self, field_number: u32) -> usize {
        let len = self.java_encoded_len();
        key_len(field_number) + encoded_len_varint(len as u64) + len
    }

    /// `MessageLite.getSerializedSize()`.
    fn get_serialized_size(&self) -> usize {
        self.java_encoded_len()
    }

    /// `MessageLite.toByteArray()`.
    fn to_byte_array(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.java_encoded_len());
        self.java_encode_raw(&mut buf);
        buf
    }

    /// `MessageLite.writeTo(OutputStream)`, appending to `buf`.
    fn write_to(&self, buf: &mut Vec<u8>) {
        self.java_encode_raw(buf);
    }

    /// `MessageLite.writeDelimitedTo(OutputStream)`: varint length, then the message.
    fn write_delimited_to(&self, buf: &mut Vec<u8>) {
        encode_varint(self.java_encoded_len() as u64, buf);
        self.java_encode_raw(buf);
    }
}

/// `CodedOutputStream.writeMessage(fieldNumber, value)`: one length-delimited field holding
/// `msg`. Streamed query and aquery `--output=proto` write each element this way, as if it were
/// part of an enclosing `QueryResult` / `ActionGraphContainer`.
pub fn write_message_field<M: JavaMessage>(field_number: u32, msg: &M, buf: &mut Vec<u8>) {
    msg.java_encode_field(field_number, buf);
}

/// Reads one `writeDelimitedTo` record from `buf`, advancing it. `Ok(None)` at end of input.
/// (`MessageLite.parseDelimitedFrom`.)
pub fn parse_delimited_from<M: JavaMessage>(
    buf: &mut &[u8],
) -> Result<Option<M>, prost::DecodeError> {
    if buf.is_empty() {
        return Ok(None);
    }
    M::decode_length_delimited(buf).map(Some)
}

/// `String.getBytes(ISO_8859_1)` / a Latin-1 `PrintStream` (Bazel's default `file.encoding`):
/// chars up to U+00FF become one byte, others `'?'`. Surfaces that print JSON through a
/// `PrintStream` (cquery and aquery `--output=jsonproto`) encode the printer's text this way.
pub fn encode_latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            if (c as u32) <= 0xFF {
                c as u32 as u8
            } else {
                b'?'
            }
        })
        .collect()
}

/// `new String(bytes, ISO_8859_1)`: one char per byte.
pub fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// `StringEncoding.unicodeToInternal`: the Java string whose chars are the UTF-8 bytes of `s`,
/// as a Rust string. Proto string fields that Java fills with internal strings (aquery, the
/// execution log) hold this form, so their UTF-8 bytes on the wire are "double encoded".
// Java: com/google/devtools/build/lib/util/StringEncoding.java StringEncoding.unicodeToInternal
pub fn unicode_to_internal(s: &str) -> String {
    decode_latin1(s.as_bytes())
}
