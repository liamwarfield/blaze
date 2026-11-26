//! A message of any type of the pool ([`crate::descriptors`]): the parts of
//! `com.google.protobuf.DynamicMessage`, `UnknownFieldSet`, `MessageReflection` (binary parsing)
//! and `CodedInputStream` that the descriptor-driven `TextFormat` printer and parser need
//! (protobuf-java 33.4; paths in `// Java:` markers are relative to `java/core/src/main/java`).
//!
//! A generated (prost) message reaches the printer as a [`DynamicMessage`] parsed from its Java
//! encoding ([`DynamicMessage::from_message`]): `JavaMessage` writes exactly the fields Java's
//! `getAllFields()` holds, so the parsed message has the same fields, in the same order, with
//! the same values. The parser builds a [`DynamicMessage`] and hands it back through its binary
//! form ([`DynamicMessage::into_message`]).

use std::collections::BTreeMap;

use crate::JavaMessage;
use crate::descriptors::{Descriptor, FieldDescriptor, JavaType, Type, pool};
use crate::json::InvalidProtocolBufferException;

/// `CodedInputStream.DEFAULT_RECURSION_LIMIT`.
const DEFAULT_RECURSION_LIMIT: i32 = 100;

const WIRETYPE_VARINT: u32 = 0;
const WIRETYPE_FIXED64: u32 = 1;
const WIRETYPE_LENGTH_DELIMITED: u32 = 2;
const WIRETYPE_START_GROUP: u32 = 3;
const WIRETYPE_END_GROUP: u32 = 4;
const WIRETYPE_FIXED32: u32 = 5;

/// The value of one field element, as Java's reflection API returns it.
///
/// Unsigned 32- and 64-bit types are held as Java's `int` / `long` (two's complement).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `JavaType.INT`.
    Int(i32),
    /// `JavaType.LONG`.
    Long(i64),
    /// `JavaType.FLOAT`.
    Float(f32),
    /// `JavaType.DOUBLE`.
    Double(f64),
    /// `JavaType.BOOLEAN`.
    Boolean(bool),
    /// `JavaType.STRING`: the UTF-8 bytes of the Java `String` (`getBytes(UTF_8)`, which is
    /// how it is written and escaped).
    String(Vec<u8>),
    /// `JavaType.BYTE_STRING`.
    Bytes(Vec<u8>),
    /// `JavaType.ENUM`: the number (an `EnumValueDescriptor`, possibly an unknown one of an open
    /// enum, whose `getIndex()` is -1).
    Enum(i32),
    /// `JavaType.MESSAGE`.
    Message(DynamicMessage),
}

impl Value {
    /// `field.getDefaultValue()` of a singular non-message field.
    ///
    /// PORT: explicit `[default = ...]` values only matter for implicit-presence fields, which
    /// only proto3 has, and proto3 has no explicit defaults.
    fn default_for(field: &FieldDescriptor) -> Value {
        match field.type_ {
            Type::Int32 | Type::Sint32 | Type::Sfixed32 | Type::Uint32 | Type::Fixed32 => {
                Value::Int(0)
            }
            Type::Int64 | Type::Sint64 | Type::Sfixed64 | Type::Uint64 | Type::Fixed64 => {
                Value::Long(0)
            }
            Type::Float => Value::Float(0.0),
            Type::Double => Value::Double(0.0),
            Type::Bool => Value::Boolean(false),
            Type::String => Value::String(Vec::new()),
            Type::Bytes => Value::Bytes(Vec::new()),
            Type::Enum => Value::Enum(field.get_enum_type().values[0].number),
            Type::Message | Type::Group => {
                Value::Message(DynamicMessage::new(field.get_message_type()))
            }
        }
    }

    /// `Object.equals`: `Float.equals` / `Double.equals` compare `floatToIntBits` (NaNs equal,
    /// `-0.0 != 0.0`).
    fn java_equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => java_float_bits(*a) == java_float_bits(*b),
            (Value::Double(a), Value::Double(b)) => java_double_bits(*a) == java_double_bits(*b),
            _ => self == other,
        }
    }
}

fn java_float_bits(f: f32) -> u32 {
    if f.is_nan() { 0x7fc0_0000 } else { f.to_bits() }
}

fn java_double_bits(d: f64) -> u64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits()
    }
}

/// One field of an [`UnknownFieldSet`].
// Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet.Field
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnknownField {
    /// `getVarintList()`.
    pub varint: Vec<i64>,
    /// `getFixed32List()`.
    pub fixed32: Vec<i32>,
    /// `getFixed64List()`.
    pub fixed64: Vec<i64>,
    /// `getLengthDelimitedList()`.
    pub length_delimited: Vec<Vec<u8>>,
    /// `getGroupList()`.
    pub group: Vec<UnknownFieldSet>,
}

/// `com.google.protobuf.UnknownFieldSet`: fields by number (`asMap()` is a `TreeMap`).
// Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnknownFieldSet {
    pub fields: BTreeMap<i32, UnknownField>,
}

impl UnknownFieldSet {
    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// `parseFrom(ByteString)`: `newBuilder().mergeFrom(data)` then `checkLastTagWas(0)`.
    // Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet.parseFrom
    pub fn parse_from(data: &[u8]) -> Result<UnknownFieldSet, InvalidProtocolBufferException> {
        let mut input = CodedInputStream::new(data);
        let mut set = UnknownFieldSet::default();
        set.merge_from(&mut input)?;
        input.check_last_tag_was(0)?;
        Ok(set)
    }

    fn field(&mut self, number: i32) -> &mut UnknownField {
        self.fields.entry(number).or_default()
    }

    /// `Builder.mergeFrom(CodedInputStream)`.
    // Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet.Builder.mergeFrom
    fn merge_from(
        &mut self,
        input: &mut CodedInputStream<'_>,
    ) -> Result<(), InvalidProtocolBufferException> {
        loop {
            let tag = input.read_tag()?;
            if tag == 0 || !self.merge_field_from(tag, input)? {
                break;
            }
        }
        Ok(())
    }

    /// `Builder.mergeFieldFrom(tag, input)`: `false` at an end-group tag.
    // Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet.Builder.mergeFieldFrom
    fn merge_field_from(
        &mut self,
        tag: u32,
        input: &mut CodedInputStream<'_>,
    ) -> Result<bool, InvalidProtocolBufferException> {
        let number = (tag >> 3) as i32;
        match tag & 7 {
            WIRETYPE_VARINT => {
                let v = input.read_raw_varint64()? as i64;
                self.field(number).varint.push(v);
                Ok(true)
            }
            WIRETYPE_FIXED64 => {
                let v = input.read_raw_little_endian64()? as i64;
                self.field(number).fixed64.push(v);
                Ok(true)
            }
            WIRETYPE_LENGTH_DELIMITED => {
                let v = input.read_bytes()?;
                self.field(number).length_delimited.push(v);
                Ok(true)
            }
            WIRETYPE_START_GROUP => {
                let mut sub = UnknownFieldSet::default();
                input.read_group(number, |input| sub.merge_from(input))?;
                self.field(number).group.push(sub);
                Ok(true)
            }
            WIRETYPE_END_GROUP => Ok(false),
            WIRETYPE_FIXED32 => {
                let v = input.read_raw_little_endian32()? as i32;
                self.field(number).fixed32.push(v);
                Ok(true)
            }
            _ => Err(InvalidProtocolBufferException::new(
                "Protocol message tag had invalid wire type.",
            )),
        }
    }

    /// `toByteArray()`.
    pub fn to_byte_array(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_to(&mut buf);
        buf
    }

    /// `mergeVarintField(number, value)`.
    fn merge_varint_field(&mut self, number: i32, value: i32) {
        self.field(number).varint.push(value as i64);
    }

    /// `writeTo(CodedOutputStream)`: per field number, varints, fixed32s, fixed64s,
    /// length-delimited values, groups.
    // Java: com/google/protobuf/UnknownFieldSet.java UnknownFieldSet.Field.writeTo
    fn write_to(&self, buf: &mut Vec<u8>) {
        for (&number, field) in &self.fields {
            let number = number as u32;
            for &v in &field.varint {
                write_tag(number, WIRETYPE_VARINT, buf);
                write_varint(v as u64, buf);
            }
            for &v in &field.fixed32 {
                write_tag(number, WIRETYPE_FIXED32, buf);
                buf.extend_from_slice(&(v as u32).to_le_bytes());
            }
            for &v in &field.fixed64 {
                write_tag(number, WIRETYPE_FIXED64, buf);
                buf.extend_from_slice(&(v as u64).to_le_bytes());
            }
            for v in &field.length_delimited {
                write_tag(number, WIRETYPE_LENGTH_DELIMITED, buf);
                write_varint(v.len() as u64, buf);
                buf.extend_from_slice(v);
            }
            for g in &field.group {
                write_tag(number, WIRETYPE_START_GROUP, buf);
                g.write_to(buf);
                write_tag(number, WIRETYPE_END_GROUP, buf);
            }
        }
    }
}

/// `com.google.protobuf.DynamicMessage` (and its builder: the parser mutates it in place).
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicMessage {
    descriptor: &'static Descriptor,
    /// Per field of the descriptor (declaration order), then per extension of it
    /// (`Descriptor::extensions`): its elements. A singular field holds at most one; it is set
    /// iff non-empty.
    fields: Vec<Vec<Value>>,
    unknown_fields: UnknownFieldSet,
}

impl DynamicMessage {
    /// `DynamicMessage.getDefaultInstance(type).newBuilderForType()`.
    pub fn new(descriptor: &'static Descriptor) -> DynamicMessage {
        DynamicMessage {
            descriptor,
            fields: (0..descriptor.fields.len() + descriptor.extensions.len())
                .map(|_| Vec::new())
                .collect(),
            unknown_fields: UnknownFieldSet::default(),
        }
    }

    /// An empty message of the type with this full name (`None` if the crate has no such type).
    pub fn for_type_name(full_name: &str) -> Option<DynamicMessage> {
        pool()
            .find_message_type_by_name(full_name)
            .map(DynamicMessage::new)
    }

    /// `getDescriptorForType()`.
    pub fn get_descriptor_for_type(&self) -> &'static Descriptor {
        self.descriptor
    }

    /// `getUnknownFields()`.
    pub fn get_unknown_fields(&self) -> &UnknownFieldSet {
        &self.unknown_fields
    }

    /// The message `m` as Java holds it: parsed from its Java encoding, with the compiled
    /// extensions registered (a generated Java message holds its set extensions as such).
    pub fn from_message<M: JavaMessage>(m: &M) -> DynamicMessage {
        let descriptor = pool()
            .find_message_type_by_name(M::FULL_NAME)
            .unwrap_or_else(|| panic!("no descriptor for {}", M::FULL_NAME));
        let data = m.to_byte_array();
        let mut input = CodedInputStream::new(&data);
        let mut message = DynamicMessage::new(descriptor);
        message
            .merge_from(&mut input, /* extensions= */ true)
            .and_then(|()| input.check_last_tag_was(0))
            .unwrap_or_else(|e| panic!("re-parsing {}: {e}", M::FULL_NAME));
        message
    }

    /// `build()` into the generated type: decoded from [`Self::to_byte_array`].
    pub fn into_message<M: JavaMessage>(&self) -> M {
        assert_eq!(
            self.descriptor.full_name,
            M::FULL_NAME,
            "DynamicMessage of another type"
        );
        M::decode(self.to_byte_array().as_slice())
            .unwrap_or_else(|e| panic!("decoding {}: {e}", M::FULL_NAME))
    }

    /// `DynamicMessage.parseFrom(type, data)` (empty extension registry).
    // Java: com/google/protobuf/DynamicMessage.java DynamicMessage.parseFrom
    pub fn parse_from(
        descriptor: &'static Descriptor,
        data: &[u8],
    ) -> Result<DynamicMessage, InvalidProtocolBufferException> {
        let mut input = CodedInputStream::new(data);
        let mut message = DynamicMessage::new(descriptor);
        message.merge_from(&mut input, /* extensions= */ false)?;
        input.check_last_tag_was(0)?;
        Ok(message)
    }

    /// `AbstractMessage.Builder.mergeFrom(CodedInputStream, extensionRegistry)` through
    /// `MessageReflection.mergeFieldFrom`. `extensions`: whether the registry holds the
    /// compiled extensions (otherwise it is empty).
    fn merge_from(
        &mut self,
        input: &mut CodedInputStream<'_>,
        extensions: bool,
    ) -> Result<(), InvalidProtocolBufferException> {
        loop {
            let tag = input.read_tag()?;
            if tag == 0 {
                break;
            }
            if !self.merge_field_from(input, tag, extensions)? {
                // end group tag
                break;
            }
        }
        Ok(())
    }

    /// Parses one field of the wire format into this builder; `false` at an end-group tag.
    // Java: com/google/protobuf/MessageReflection.java MessageReflection.mergeFieldFrom
    fn merge_field_from(
        &mut self,
        input: &mut CodedInputStream<'_>,
        tag: u32,
        extensions: bool,
    ) -> Result<bool, InvalidProtocolBufferException> {
        let descriptor = self.descriptor;
        let wire_type = tag & 7;
        let field_number = (tag >> 3) as i32;

        // `descriptor.isExtensionNumber(fieldNumber)`: the registry's extension, else unknown.
        let field = match descriptor.find_field_by_number(field_number) {
            Some(f) => Some(f),
            None if extensions => descriptor.find_extension_by_number(field_number),
            None => None,
        };

        let mut unknown = false;
        let mut packed = false;
        match field {
            None => unknown = true, // Unknown field.
            Some(f) => {
                if wire_type == f.type_.wire_type() {
                    packed = false;
                } else if f.is_packable() && wire_type == WIRETYPE_LENGTH_DELIMITED {
                    packed = true;
                } else {
                    unknown = true; // Unknown wire type.
                }
            }
        }

        if unknown {
            // Unknown field or wrong wire type.  Skip.
            return self.unknown_fields.merge_field_from(tag, input);
        }
        let field = field.expect("known field");

        if packed {
            let length = input.read_raw_varint32()?;
            let limit = input.push_limit(length)?;
            if field.type_ == Type::Enum {
                while input.get_bytes_until_limit() > 0 {
                    let raw_value = input.read_raw_varint32()? as i32;
                    if field.enum_is_closed() {
                        if field
                            .get_enum_type()
                            .find_value_by_number(raw_value)
                            .is_none()
                        {
                            // If the number isn't recognized as a valid value for this
                            // enum, add it to the unknown fields.
                            self.unknown_fields
                                .merge_varint_field(field_number, raw_value);
                        } else {
                            self.add_repeated_field(field, Value::Enum(raw_value));
                        }
                    } else {
                        self.add_repeated_field(field, Value::Enum(raw_value));
                    }
                }
            } else {
                while input.get_bytes_until_limit() > 0 {
                    let value = read_primitive_field(input, field)?;
                    self.add_repeated_field(field, value);
                }
            }
            input.pop_limit(limit);
        } else {
            let value = match field.type_ {
                Type::Group => {
                    let mut sub = self.new_merge_target_for_field(field);
                    input.read_group(field_number, |input| sub.merge_from(input, extensions))?;
                    Value::Message(sub)
                }
                Type::Message => {
                    let mut sub = self.new_merge_target_for_field(field);
                    input.read_message(|input| sub.merge_from(input, extensions))?;
                    Value::Message(sub)
                }
                Type::Enum => {
                    let raw_value = input.read_raw_varint32()? as i32;
                    if field.enum_is_closed()
                        && field
                            .get_enum_type()
                            .find_value_by_number(raw_value)
                            .is_none()
                    {
                        // If the number isn't recognized as a valid value for this enum,
                        // add it to the unknown fields.
                        self.unknown_fields
                            .merge_varint_field(field_number, raw_value);
                        return Ok(true);
                    }
                    Value::Enum(raw_value)
                }
                _ => read_primitive_field(input, field)?,
            };

            if field.is_repeated() {
                self.add_repeated_field(field, value);
            } else {
                self.set_field(field, value);
            }
        }

        Ok(true)
    }

    fn slot(&self, field: &FieldDescriptor) -> &Vec<Value> {
        assert!(
            field.get_containing_type().same(self.descriptor),
            "FieldDescriptor does not match message type."
        );
        &self.fields[field.index]
    }

    fn slot_mut(&mut self, field: &FieldDescriptor) -> &mut Vec<Value> {
        assert!(
            field.get_containing_type().same(self.descriptor),
            "FieldDescriptor does not match message type."
        );
        &mut self.fields[field.index]
    }

    /// `hasField(field)` of a singular field.
    // Java: com/google/protobuf/DynamicMessage.java DynamicMessage.hasField
    pub fn has_field(&self, field: &FieldDescriptor) -> bool {
        let slot = self.slot(field);
        match slot.first() {
            None => false,
            Some(v) => {
                if field.has_presence() {
                    true
                } else {
                    !v.java_equals(&Value::default_for(field))
                }
            }
        }
    }

    /// `getField(field)` of a singular field (`None` when unset: the default value).
    pub fn get_field(&self, field: &FieldDescriptor) -> Option<&Value> {
        self.slot(field).first()
    }

    /// `getRepeatedField` as a list (for a singular field: the set value, if any).
    pub fn get_values(&self, field: &FieldDescriptor) -> &[Value] {
        self.slot(field)
    }

    /// The values of the field with this name (empty when unset or no such field).
    pub fn get_by_name(&self, name: &str) -> &[Value] {
        match self.descriptor.find_field_by_name(name) {
            Some(f) => &self.fields[f.index],
            None => &[],
        }
    }

    /// `getOneofFieldDescriptor(oneof)`: the set member of a oneof.
    pub fn get_oneof_field_descriptor(
        &self,
        oneof_index: usize,
    ) -> Option<&'static FieldDescriptor> {
        let descriptor = self.descriptor;
        descriptor.fields.iter().find(|f| {
            f.get_containing_oneof().map(|o| o.index) == Some(oneof_index)
                && !self.fields[f.index].is_empty()
        })
    }

    /// `hasOneof(oneof)`.
    pub fn has_oneof(&self, oneof_index: usize) -> bool {
        self.get_oneof_field_descriptor(oneof_index).is_some()
    }

    /// `Builder.setField(field, value)`: a member of a oneof clears the other members.
    // Java: com/google/protobuf/DynamicMessage.java DynamicMessage.Builder.setField
    pub fn set_field(&mut self, field: &FieldDescriptor, value: Value) {
        if let Some(oneof) = field.get_containing_oneof() {
            let descriptor = self.descriptor;
            for other in &descriptor.fields {
                if other.index != field.index
                    && other.get_containing_oneof().map(|o| o.index) == Some(oneof.index)
                {
                    self.fields[other.index].clear();
                }
            }
        } else if !field.has_presence()
            && !field.is_repeated()
            && value.java_equals(&Value::default_for(field))
        {
            // Setting a field without presence to its default value is equivalent to clearing
            // the field.
            self.slot_mut(field).clear();
            return;
        }
        *self.slot_mut(field) = vec![value];
    }

    /// `Builder.addRepeatedField(field, value)`.
    pub fn add_repeated_field(&mut self, field: &FieldDescriptor, value: Value) {
        self.slot_mut(field).push(value);
    }

    /// `MessageReflection.BuilderAdapter.newMergeTargetForField(field, null)`: a singular
    /// message field that is set is merged into; otherwise a new builder of the field's type.
    // Java: com/google/protobuf/MessageReflection.java BuilderAdapter.newMergeTargetForField
    pub fn new_merge_target_for_field(&self, field: &FieldDescriptor) -> DynamicMessage {
        if !field.is_repeated()
            && let Some(Value::Message(m)) = self.slot(field).first()
        {
            return m.clone();
        }
        DynamicMessage::new(field.get_message_type())
    }

    /// `getAllFields()`: the set fields in field-number order (a `TreeMap` keyed by
    /// `FieldDescriptor.compareTo`, which compares numbers). Repeated fields are included when
    /// non-empty; a map field's entries in insertion order.
    // Java: com/google/protobuf/DynamicMessage.java DynamicMessage.getAllFields
    pub fn get_all_fields(&self) -> Vec<(&'static FieldDescriptor, &[Value])> {
        let descriptor = self.descriptor;
        let mut result: Vec<(&'static FieldDescriptor, &[Value])> = Vec::new();
        for field in descriptor.fields.iter().chain(descriptor.extensions.iter()) {
            let slot = &self.fields[field.index];
            if field.is_repeated() {
                if !slot.is_empty() {
                    result.push((field, slot.as_slice()));
                }
            } else if self.has_field(field) {
                result.push((field, &slot[..1]));
            }
        }
        result.sort_by_key(|(f, _)| f.number);
        result
    }

    /// `AbstractMessage.equals(other)`: same type, same fields (`compareFields`: a map field is
    /// compared as a map, so the order of its entries does not matter; bytes by content), same
    /// unknown fields.
    // Java: com/google/protobuf/AbstractMessage.java AbstractMessage.equals
    pub fn java_equals(&self, other: &DynamicMessage) -> bool {
        if !self.descriptor.same(other.descriptor) {
            return false;
        }
        let a = self.get_all_fields();
        let b = other.get_all_fields();
        if a.len() != b.len() {
            return false;
        }
        for ((fa, va), (fb, vb)) in a.iter().zip(b.iter()) {
            if fa.index != fb.index {
                return false;
            }
            if fa.is_map_field() {
                // `compareMapField`: both as maps, last entry per key wins.
                let (Some(ma), Some(mb)) = (map_field_as_map(va), map_field_as_map(vb)) else {
                    return false;
                };
                if ma.len() != mb.len() {
                    return false;
                }
                for (key, ea) in &ma {
                    let Some((_, eb)) = mb.iter().find(|(k, _)| k == key) else {
                        return false;
                    };
                    if !value_java_equals(ea, eb) {
                        return false;
                    }
                }
            } else {
                if va.len() != vb.len() {
                    return false;
                }
                for (x, y) in va.iter().zip(vb.iter()) {
                    if !value_java_equals(x, y) {
                        return false;
                    }
                }
            }
        }
        self.unknown_fields == other.unknown_fields
    }

    /// `MessageReflection.findMissingFields(message)`: the paths of the required fields that are
    /// not set (`a`, `b.c`, `d[0].e`).
    // Java: com/google/protobuf/MessageReflection.java MessageReflection.findMissingFields
    pub fn find_missing_fields(&self) -> Vec<String> {
        let mut results = Vec::new();
        self.find_missing_fields_into("", &mut results);
        results
    }

    fn find_missing_fields_into(&self, prefix: &str, results: &mut Vec<String>) {
        let descriptor = self.descriptor;
        for field in &descriptor.fields {
            if field.is_required() && !self.has_field(field) {
                results.push(format!("{prefix}{}", field.name));
            }
        }

        for (field, values) in self.get_all_fields() {
            if field.get_java_type() == JavaType::Message {
                if field.is_repeated() {
                    for (i, element) in values.iter().enumerate() {
                        if let Value::Message(m) = element {
                            m.find_missing_fields_into(
                                &format!("{prefix}{}[{i}].", field.name),
                                results,
                            );
                        }
                    }
                } else if let Value::Message(m) = &values[0] {
                    m.find_missing_fields_into(&format!("{prefix}{}.", field.name), results);
                }
            }
        }
    }

    /// `toByteArray()`: the fields of `getAllFields()` in number order (packed where
    /// `isPacked()`), then the unknown fields.
    // Java: com/google/protobuf/MessageReflection.java MessageReflection.writeMessageTo
    pub fn to_byte_array(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_to(&mut buf);
        buf
    }

    fn write_to(&self, buf: &mut Vec<u8>) {
        for (field, values) in self.get_all_fields() {
            let number = field.number as u32;
            if field.is_repeated() && field.is_packed() {
                let mut data = Vec::new();
                for v in values {
                    write_primitive_no_tag(field, v, &mut data);
                }
                write_tag(number, WIRETYPE_LENGTH_DELIMITED, buf);
                write_varint(data.len() as u64, buf);
                buf.extend_from_slice(&data);
            } else {
                for v in values {
                    write_element(field, v, buf);
                }
            }
        }
        self.unknown_fields.write_to(buf);
    }
}

/// The entries of a map field as a map (`MapField.getMap()`): the encoded key, the last entry
/// with that key, in first-insertion order. `None` if an element is not an entry.
fn map_field_as_map(values: &[Value]) -> Option<Vec<(Vec<u8>, &Value)>> {
    let mut map: Vec<(Vec<u8>, &Value)> = Vec::new();
    for v in values {
        let Value::Message(entry) = v else {
            return None;
        };
        let key_field = entry.descriptor.find_field_by_number(1)?;
        let key_value = entry
            .get_field(key_field)
            .cloned()
            .unwrap_or_else(|| Value::default_for(key_field));
        let mut key = Vec::new();
        write_primitive_no_tag(key_field, &key_value, &mut key);
        match map.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = v,
            None => map.push((key, v)),
        }
    }
    Some(map)
}

/// `Object.equals` of two field values (messages by [`DynamicMessage::java_equals`]).
fn value_java_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Message(x), Value::Message(y)) => x.java_equals(y),
        _ => a.java_equals(b),
    }
}

fn write_element(field: &FieldDescriptor, value: &Value, buf: &mut Vec<u8>) {
    let number = field.number as u32;
    match (field.type_, value) {
        (Type::Message, Value::Message(m)) => {
            let data = m.to_byte_array();
            write_tag(number, WIRETYPE_LENGTH_DELIMITED, buf);
            write_varint(data.len() as u64, buf);
            buf.extend_from_slice(&data);
        }
        (Type::Group, Value::Message(m)) => {
            write_tag(number, WIRETYPE_START_GROUP, buf);
            m.write_to(buf);
            write_tag(number, WIRETYPE_END_GROUP, buf);
        }
        _ => {
            write_tag(number, field.type_.wire_type(), buf);
            write_primitive_no_tag(field, value, buf);
        }
    }
}

fn write_primitive_no_tag(field: &FieldDescriptor, value: &Value, buf: &mut Vec<u8>) {
    match (field.type_, value) {
        (Type::Int32, Value::Int(v)) => write_varint(*v as i64 as u64, buf),
        (Type::Uint32, Value::Int(v)) => write_varint(*v as u32 as u64, buf),
        (Type::Sint32, Value::Int(v)) => write_varint(((v << 1) ^ (v >> 31)) as u32 as u64, buf),
        (Type::Fixed32 | Type::Sfixed32, Value::Int(v)) => buf.extend_from_slice(&v.to_le_bytes()),
        (Type::Int64 | Type::Uint64, Value::Long(v)) => write_varint(*v as u64, buf),
        (Type::Sint64, Value::Long(v)) => write_varint(((v << 1) ^ (v >> 63)) as u64, buf),
        (Type::Fixed64 | Type::Sfixed64, Value::Long(v)) => buf.extend_from_slice(&v.to_le_bytes()),
        (Type::Float, Value::Float(v)) => buf.extend_from_slice(&v.to_bits().to_le_bytes()),
        (Type::Double, Value::Double(v)) => buf.extend_from_slice(&v.to_bits().to_le_bytes()),
        (Type::Bool, Value::Boolean(v)) => write_varint(*v as u64, buf),
        (Type::Enum, Value::Enum(v)) => write_varint(*v as i64 as u64, buf),
        (Type::String, Value::String(v)) | (Type::Bytes, Value::Bytes(v)) => {
            write_varint(v.len() as u64, buf);
            buf.extend_from_slice(v);
        }
        (t, v) => panic!("ClassCastException: {v:?} for a field of type {t:?}"),
    }
}

/// `FieldSet.readPrimitiveField(input, type, checkUtf8)`.
// Java: com/google/protobuf/FieldSet.java FieldSet.readPrimitiveField
fn read_primitive_field(
    input: &mut CodedInputStream<'_>,
    field: &FieldDescriptor,
) -> Result<Value, InvalidProtocolBufferException> {
    Ok(match field.type_ {
        Type::Double => Value::Double(f64::from_bits(input.read_raw_little_endian64()?)),
        Type::Float => Value::Float(f32::from_bits(input.read_raw_little_endian32()?)),
        Type::Int64 | Type::Uint64 => Value::Long(input.read_raw_varint64()? as i64),
        Type::Int32 | Type::Uint32 => Value::Int(input.read_raw_varint32()? as i32),
        Type::Fixed64 | Type::Sfixed64 => Value::Long(input.read_raw_little_endian64()? as i64),
        Type::Fixed32 | Type::Sfixed32 => Value::Int(input.read_raw_little_endian32()? as i32),
        Type::Bool => Value::Boolean(input.read_raw_varint64()? != 0),
        Type::Bytes => Value::Bytes(input.read_bytes()?),
        Type::Sint32 => {
            let n = input.read_raw_varint32()?;
            Value::Int(((n >> 1) as i32) ^ -((n & 1) as i32))
        }
        Type::Sint64 => {
            let n = input.read_raw_varint64()?;
            Value::Long(((n >> 1) as i64) ^ -((n & 1) as i64))
        }
        Type::String => {
            let bytes = input.read_bytes()?;
            // `readStringRequireUtf8` where the field requires UTF-8 validation (proto3), else
            // `readString` (`new String(bytes, UTF_8)`, malformed input replaced).
            match String::from_utf8(bytes) {
                Ok(s) => Value::String(s.into_bytes()),
                Err(e) => {
                    if field.get_containing_type().get_file().syntax
                        == crate::descriptors::Syntax::Proto3
                    {
                        return Err(InvalidProtocolBufferException::new(
                            "Protocol message had invalid UTF-8.",
                        ));
                    }
                    Value::String(crate::java::new_string_utf8(e.as_bytes()).into_bytes())
                }
            }
        }
        Type::Group | Type::Message | Type::Enum => {
            panic!("readPrimitiveField() cannot handle nested or enum types.")
        }
    })
}

/// Whether a field's Java type is a message.
pub fn is_message_field(field: &FieldDescriptor) -> bool {
    field.get_java_type() == JavaType::Message
}

fn write_tag(number: u32, wire_type: u32, buf: &mut Vec<u8>) {
    write_varint(((number << 3) | wire_type) as u64, buf);
}

fn write_varint(mut v: u64, buf: &mut Vec<u8>) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// The parts of `com.google.protobuf.CodedInputStream` (array-backed) the parsers use.
struct CodedInputStream<'a> {
    buffer: &'a [u8],
    pos: usize,
    /// `currentLimit` (absolute).
    current_limit: usize,
    last_tag: u32,
    recursion_depth: i32,
}

fn truncated_message() -> InvalidProtocolBufferException {
    InvalidProtocolBufferException::new(
        "While parsing a protocol message, the input ended unexpectedly in the middle of a \
         field.  This could mean either that the input has been truncated or that an embedded \
         message misreported its own length.",
    )
}

fn negative_size() -> InvalidProtocolBufferException {
    InvalidProtocolBufferException::new(
        "CodedInputStream encountered an embedded string or message which claimed to have \
         negative size.",
    )
}

impl<'a> CodedInputStream<'a> {
    fn new(buffer: &'a [u8]) -> CodedInputStream<'a> {
        CodedInputStream {
            buffer,
            pos: 0,
            current_limit: buffer.len(),
            last_tag: 0,
            recursion_depth: 0,
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos == self.current_limit
    }

    // Java: com/google/protobuf/CodedInputStream.java ArrayDecoder.readTag
    fn read_tag(&mut self) -> Result<u32, InvalidProtocolBufferException> {
        if self.is_at_end() {
            self.last_tag = 0;
            return Ok(0);
        }

        self.last_tag = self.read_raw_varint32()?;
        if (self.last_tag >> 3) == 0 {
            // If we actually read zero (or any tag number corresponding to field
            // number zero), that's not a valid tag.
            return Err(InvalidProtocolBufferException::new(
                "Protocol message contained an invalid tag (zero).",
            ));
        }
        Ok(self.last_tag)
    }

    fn check_last_tag_was(&self, value: u32) -> Result<(), InvalidProtocolBufferException> {
        if self.last_tag != value {
            return Err(InvalidProtocolBufferException::new(
                "Protocol message end-group tag did not match expected tag.",
            ));
        }
        Ok(())
    }

    fn read_raw_byte(&mut self) -> Result<u8, InvalidProtocolBufferException> {
        if self.pos == self.current_limit {
            return Err(truncated_message());
        }
        let b = self.buffer[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_raw_varint64(&mut self) -> Result<u64, InvalidProtocolBufferException> {
        let mut result: u64 = 0;
        let mut shift = 0;
        while shift < 64 {
            let b = self.read_raw_byte()?;
            result |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
        Err(InvalidProtocolBufferException::new(
            "CodedInputStream encountered a malformed varint.",
        ))
    }

    fn read_raw_varint32(&mut self) -> Result<u32, InvalidProtocolBufferException> {
        Ok(self.read_raw_varint64()? as u32)
    }

    fn read_raw_little_endian32(&mut self) -> Result<u32, InvalidProtocolBufferException> {
        if self.current_limit - self.pos < 4 {
            return Err(truncated_message());
        }
        let v = u32::from_le_bytes(self.buffer[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    fn read_raw_little_endian64(&mut self) -> Result<u64, InvalidProtocolBufferException> {
        if self.current_limit - self.pos < 8 {
            return Err(truncated_message());
        }
        let v = u64::from_le_bytes(self.buffer[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    fn read_bytes(&mut self) -> Result<Vec<u8>, InvalidProtocolBufferException> {
        let size = self.read_raw_varint32()? as i32;
        if size < 0 {
            return Err(negative_size());
        }
        let size = size as usize;
        if size > self.current_limit - self.pos {
            return Err(truncated_message());
        }
        let v = self.buffer[self.pos..self.pos + size].to_vec();
        self.pos += size;
        Ok(v)
    }

    /// `pushLimit(byteLimit)`: returns the old limit.
    fn push_limit(&mut self, byte_limit: u32) -> Result<usize, InvalidProtocolBufferException> {
        let byte_limit = byte_limit as i32;
        if byte_limit < 0 {
            return Err(negative_size());
        }
        let new_limit = self.pos + byte_limit as usize;
        if new_limit > self.current_limit {
            return Err(truncated_message());
        }
        let old = self.current_limit;
        self.current_limit = new_limit;
        Ok(old)
    }

    fn pop_limit(&mut self, old_limit: usize) {
        self.current_limit = old_limit;
    }

    fn get_bytes_until_limit(&self) -> usize {
        self.current_limit - self.pos
    }

    fn check_recursion_limit(&self) -> Result<(), InvalidProtocolBufferException> {
        if self.recursion_depth >= DEFAULT_RECURSION_LIMIT {
            return Err(InvalidProtocolBufferException::new(
                "Protocol message had too many levels of nesting.  May be malicious.  Use \
                 setRecursionLimit() to increase the recursion depth limit.",
            ));
        }
        Ok(())
    }

    // Java: com/google/protobuf/CodedInputStream.java ArrayDecoder.readGroup
    fn read_group(
        &mut self,
        field_number: i32,
        merge: impl FnOnce(&mut Self) -> Result<(), InvalidProtocolBufferException>,
    ) -> Result<(), InvalidProtocolBufferException> {
        self.check_recursion_limit()?;
        self.recursion_depth += 1;
        merge(self)?;
        self.check_last_tag_was(((field_number as u32) << 3) | WIRETYPE_END_GROUP)?;
        self.recursion_depth -= 1;
        Ok(())
    }

    // Java: com/google/protobuf/CodedInputStream.java ArrayDecoder.readMessage
    fn read_message(
        &mut self,
        merge: impl FnOnce(&mut Self) -> Result<(), InvalidProtocolBufferException>,
    ) -> Result<(), InvalidProtocolBufferException> {
        let length = self.read_raw_varint32()?;
        self.check_recursion_limit()?;
        let old_limit = self.push_limit(length)?;
        self.recursion_depth += 1;
        merge(self)?;
        self.check_last_tag_was(0)?;
        self.recursion_depth -= 1;
        if self.get_bytes_until_limit() != 0 {
            return Err(truncated_message());
        }
        self.pop_limit(old_limit);
        Ok(())
    }
}
