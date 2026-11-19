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
/// `PrintStream` (cquery and aquery `--output=jsonproto`) encode the printer's text this way: an
/// internal string comes out as the bytes it stands for, a Unicode string as Latin-1.
///
/// The same function as `blaze_util::string_encoding::latin1_encode_lossy`, which code above
/// this crate calls; blaze_protos is below blaze_util and keeps its own copy.
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

/// `new String(bytes, ISO_8859_1)`: one char per byte. The result is an internal string.
///
/// The same function as `blaze_util::string_encoding::latin1_decode`, which code above this
/// crate calls; blaze_protos is below blaze_util and keeps its own copy.
pub fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// `new String(bytes, UTF_8)`: the JDK's replacing UTF-8 decoder, which protobuf-java uses for
/// `string` values (`ByteString.toStringUtf8`, `CodedInputStream.readString`).
///
/// The same function as `blaze_util::string_encoding::java_new_string_utf8`, which code above this
/// crate calls; blaze_protos is below blaze_util and keeps its own copy. Test vectors from the
/// JDK: `tests/protobuf_utf8_test.rs`.
///
/// One U+FFFD per malformed sequence, with the JDK's own notion of how long a malformed sequence
/// is (it differs from `String::from_utf8_lossy`, for example for an encoded surrogate, which is
/// one U+FFFD in Java and three in Rust).
///
/// Java (JDK): java.base/java/lang/String.java String.decodeUTF8_UTF16 with `doReplace = true`.
pub fn new_string_utf8(src: &[u8]) -> String {
    const REPL: char = '\u{fffd}';
    // Java works on signed bytes; the predicates below are written on `u8`.
    fn is_not_continuation(b: u8) -> bool {
        b & 0xc0 != 0x80
    }
    fn is_malformed3(b1: u8, b2: u8, b3: u8) -> bool {
        (b1 == 0xe0 && b2 & 0xe0 == 0x80) || b2 & 0xc0 != 0x80 || b3 & 0xc0 != 0x80
    }
    fn is_malformed3_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xe0 && b2 & 0xe0 == 0x80) || b2 & 0xc0 != 0x80
    }
    fn is_malformed4(b2: u8, b3: u8, b4: u8) -> bool {
        b2 & 0xc0 != 0x80 || b3 & 0xc0 != 0x80 || b4 & 0xc0 != 0x80
    }
    fn is_malformed4_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
            || (b1 == 0xf4 && b2 & 0xf0 != 0x80)
            || b2 & 0xc0 != 0x80
    }
    fn is_malformed4_3(b3: u8) -> bool {
        b3 & 0xc0 != 0x80
    }
    fn malformed3(src: &[u8], sp: usize) -> usize {
        let b1 = src[sp];
        let b2 = src[sp + 1]; // no need to lookup b3
        if (b1 == 0xe0 && b2 & 0xe0 == 0x80) || is_not_continuation(b2) {
            1
        } else {
            2
        }
    }
    fn malformed4(src: &[u8], sp: usize) -> usize {
        // we don't care the speed here
        let b1 = src[sp];
        let b2 = src[sp + 1];
        if b1 > 0xf4
            || (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
            || (b1 == 0xf4 && b2 & 0xf0 != 0x80)
            || is_not_continuation(b2)
        {
            return 1;
        }
        if is_not_continuation(src[sp + 2]) {
            return 2;
        }
        3
    }

    let sl = src.len();
    let mut sp = 0;
    let mut dst = String::with_capacity(sl);
    while sp < sl {
        let b1 = src[sp];
        sp += 1;
        if b1 < 0x80 {
            dst.push(b1 as char);
        } else if b1 >> 5 == 0b110 && b1 & 0x1e != 0 {
            if sp < sl {
                let b2 = src[sp];
                sp += 1;
                if is_not_continuation(b2) {
                    dst.push(REPL);
                    sp -= 1;
                } else {
                    let c = (((b1 & 0x1f) as u32) << 6) | (b2 & 0x3f) as u32;
                    dst.push(char::from_u32(c).expect("two-byte sequence"));
                }
                continue;
            }
            // underflow()
            dst.push(REPL);
            break;
        } else if b1 >> 4 == 0b1110 {
            if sp + 1 < sl {
                let b2 = src[sp];
                let b3 = src[sp + 1];
                sp += 2;
                if is_malformed3(b1, b2, b3) {
                    dst.push(REPL);
                    sp -= 3;
                    sp += malformed3(src, sp);
                } else {
                    let c = (((b1 & 0x0f) as u32) << 12)
                        | (((b2 & 0x3f) as u32) << 6)
                        | (b3 & 0x3f) as u32;
                    // Character.isSurrogate(c)
                    match char::from_u32(c) {
                        None => dst.push(REPL),
                        Some(c) => dst.push(c),
                    }
                }
                continue;
            }
            if sp < sl && is_malformed3_2(b1, src[sp]) {
                dst.push(REPL);
                continue;
            }
            dst.push(REPL);
            break;
        } else if b1 >> 3 == 0b11110 {
            if sp + 2 < sl {
                let b2 = src[sp];
                let b3 = src[sp + 1];
                let b4 = src[sp + 2];
                sp += 3;
                let uc = (((b1 & 0x07) as u32) << 18)
                    | (((b2 & 0x3f) as u32) << 12)
                    | (((b3 & 0x3f) as u32) << 6)
                    | (b4 & 0x3f) as u32;
                // shortest form check: Character.isSupplementaryCodePoint(uc)
                if is_malformed4(b2, b3, b4) || !(0x1_0000..=0x10_ffff).contains(&uc) {
                    dst.push(REPL);
                    sp -= 4;
                    sp += malformed4(src, sp);
                } else {
                    dst.push(char::from_u32(uc).expect("supplementary code point"));
                }
                continue;
            }
            if b1 > 0xf4 || (sp < sl && is_malformed4_2(b1, src[sp])) {
                dst.push(REPL);
                continue;
            }
            sp += 1;
            dst.push(REPL);
            if sp < sl && is_malformed4_3(src[sp]) {
                continue;
            }
            break;
        } else {
            dst.push(REPL);
        }
    }
    dst
}
