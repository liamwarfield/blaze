//! Runtime descriptors: the parts of `com.google.protobuf.Descriptors` that the descriptor-driven
//! `TextFormat` printer and parser read (protobuf-java 33.4; paths in the `// Java:` markers are
//! relative to `java/core/src/main/java` of that release).
//!
//! PORT: Java builds a `FileDescriptor` per generated file from its embedded serialized
//! `FileDescriptorProto`. Here `build.rs` writes the `FileDescriptorSet` that `protox` compiled
//! (every `.proto` the crate generates types for, imports included) and [`pool`] decodes it once
//! into one table of messages and enums. Cross references are indices into that table, so every
//! descriptor is a `&'static` borrow of the pool.
//!
//! Only `proto2` and `proto3` files occur (no editions); the features a file of either syntax
//! implies are computed here as `Descriptors` resolves them (`hasPresence`, `isPacked`,
//! `EnumDescriptor.isClosed`).

use std::collections::HashMap;
use std::sync::OnceLock;

use prost::Message as _;
use prost_types::field_descriptor_proto::{Label as ProtoLabel, Type as ProtoType};
use prost_types::{DescriptorProto, EnumDescriptorProto, FileDescriptorSet};

static DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor_set.bin"));

/// The syntax of a `.proto` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Proto2,
    Proto3,
}

/// `Descriptors.FieldDescriptor.Type`.
// Java: com/google/protobuf/Descriptors.java FieldDescriptor.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Double,
    Float,
    Int64,
    Uint64,
    Int32,
    Fixed64,
    Fixed32,
    Bool,
    String,
    Group,
    Message,
    Bytes,
    Uint32,
    Enum,
    Sfixed32,
    Sfixed64,
    Sint32,
    Sint64,
}

/// `Descriptors.FieldDescriptor.JavaType`.
// Java: com/google/protobuf/Descriptors.java FieldDescriptor.JavaType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaType {
    Int,
    Long,
    Float,
    Double,
    Boolean,
    String,
    ByteString,
    Enum,
    Message,
}

impl Type {
    fn from_proto(t: ProtoType) -> Type {
        match t {
            ProtoType::Double => Type::Double,
            ProtoType::Float => Type::Float,
            ProtoType::Int64 => Type::Int64,
            ProtoType::Uint64 => Type::Uint64,
            ProtoType::Int32 => Type::Int32,
            ProtoType::Fixed64 => Type::Fixed64,
            ProtoType::Fixed32 => Type::Fixed32,
            ProtoType::Bool => Type::Bool,
            ProtoType::String => Type::String,
            ProtoType::Group => Type::Group,
            ProtoType::Message => Type::Message,
            ProtoType::Bytes => Type::Bytes,
            ProtoType::Uint32 => Type::Uint32,
            ProtoType::Enum => Type::Enum,
            ProtoType::Sfixed32 => Type::Sfixed32,
            ProtoType::Sfixed64 => Type::Sfixed64,
            ProtoType::Sint32 => Type::Sint32,
            ProtoType::Sint64 => Type::Sint64,
        }
    }

    /// `Type.getJavaType()`.
    pub fn get_java_type(self) -> JavaType {
        match self {
            Type::Double => JavaType::Double,
            Type::Float => JavaType::Float,
            Type::Int64 | Type::Uint64 | Type::Fixed64 | Type::Sfixed64 | Type::Sint64 => {
                JavaType::Long
            }
            Type::Int32 | Type::Fixed32 | Type::Uint32 | Type::Sfixed32 | Type::Sint32 => {
                JavaType::Int
            }
            Type::Bool => JavaType::Boolean,
            Type::String => JavaType::String,
            Type::Bytes => JavaType::ByteString,
            Type::Enum => JavaType::Enum,
            Type::Group | Type::Message => JavaType::Message,
        }
    }

    /// `WireFormat.FieldType.getWireType()`.
    pub fn wire_type(self) -> u32 {
        match self {
            Type::Double | Type::Fixed64 | Type::Sfixed64 => 1,
            Type::Float | Type::Fixed32 | Type::Sfixed32 => 5,
            Type::String | Type::Message | Type::Bytes => 2,
            Type::Group => 3,
            _ => 0,
        }
    }

    /// `WireFormat.FieldType.isPackable()`.
    pub fn is_packable(self) -> bool {
        !matches!(
            self,
            Type::String | Type::Group | Type::Message | Type::Bytes
        )
    }
}

/// `Descriptors.FileDescriptor`.
#[derive(Debug)]
pub struct FileDescriptor {
    /// `getName()`.
    pub name: String,
    /// `getPackage()`.
    pub package: String,
    pub syntax: Syntax,
    /// `getDependencies()` (names).
    pub dependencies: Vec<String>,
    /// `getMessageTypes()` (pool indices).
    message_types: Vec<usize>,
}

impl FileDescriptor {
    /// `getMessageTypes()`.
    pub fn get_message_types(&self) -> impl Iterator<Item = &'static Descriptor> + '_ {
        self.message_types.iter().map(|&i| &pool().messages[i])
    }
}

/// `Descriptors.OneofDescriptor`.
#[derive(Debug)]
pub struct OneofDescriptor {
    /// `getName()`.
    pub name: String,
    /// `getIndex()`.
    pub index: usize,
    /// `isSynthetic()`: the oneof of a proto3 `optional` field.
    pub synthetic: bool,
}

/// `Descriptors.Descriptor`.
#[derive(Debug)]
pub struct Descriptor {
    /// `getName()`.
    pub name: String,
    /// `getFullName()`.
    pub full_name: String,
    /// `getFile()` (pool index).
    file: usize,
    /// `getContainingType()` (pool index).
    containing_type: Option<usize>,
    /// `getFields()`, in declaration order.
    pub fields: Vec<FieldDescriptor>,
    /// `getOneofs()`.
    pub oneofs: Vec<OneofDescriptor>,
    /// `getNestedTypes()` (pool indices).
    nested_types: Vec<usize>,
    /// `getOptions().getMapEntry()`.
    pub map_entry: bool,
    /// `getOptions().getMessageSetWireFormat()`.
    pub message_set_wire_format: bool,
    /// The index in the pool.
    index: usize,
}

/// Descriptors are singletons: equal iff the same descriptor.
impl PartialEq for Descriptor {
    fn eq(&self, other: &Descriptor) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Descriptor {}

impl Descriptor {
    /// `getFile()`.
    pub fn get_file(&self) -> &'static FileDescriptor {
        &pool().files[self.file]
    }

    /// `getContainingType()`.
    pub fn get_containing_type(&self) -> Option<&'static Descriptor> {
        self.containing_type.map(|i| &pool().messages[i])
    }

    /// `getNestedTypes()`.
    pub fn get_nested_types(&self) -> impl Iterator<Item = &'static Descriptor> + '_ {
        self.nested_types.iter().map(|&i| &pool().messages[i])
    }

    /// `findFieldByName(name)`.
    pub fn find_field_by_name(&self, name: &str) -> Option<&FieldDescriptor> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// `findFieldByNumber(number)`.
    pub fn find_field_by_number(&self, number: i32) -> Option<&FieldDescriptor> {
        self.fields.iter().find(|f| f.number == number)
    }

    /// Whether `other` is this descriptor.
    pub fn same(&self, other: &Descriptor) -> bool {
        self.index == other.index
    }
}

/// `Descriptors.FieldDescriptor`.
#[derive(Debug)]
pub struct FieldDescriptor {
    /// `getName()`.
    pub name: String,
    /// `getFullName()`.
    pub full_name: String,
    /// `getNumber()`.
    pub number: i32,
    /// `getType()`.
    pub type_: Type,
    repeated: bool,
    required: bool,
    /// `getIndex()`: the position in the containing type's `getFields()`.
    pub index: usize,
    containing_type: usize,
    message_type: Option<usize>,
    enum_type: Option<usize>,
    /// `getContainingOneof()` (index into the containing type's oneofs, synthetic included).
    oneof_index: Option<usize>,
    proto3_optional: bool,
    packed_option: Option<bool>,
    syntax: Syntax,
}

impl FieldDescriptor {
    /// `getJavaType()`.
    pub fn get_java_type(&self) -> JavaType {
        self.type_.get_java_type()
    }

    /// `isRepeated()`.
    pub fn is_repeated(&self) -> bool {
        self.repeated
    }

    /// `isRequired()`.
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// `isOptional()`.
    pub fn is_optional(&self) -> bool {
        !self.repeated && !self.required
    }

    /// `isExtension()`: extensions are never in the pool's field lists.
    pub fn is_extension(&self) -> bool {
        false
    }

    /// `getContainingType()`.
    pub fn get_containing_type(&self) -> &'static Descriptor {
        &pool().messages[self.containing_type]
    }

    /// `getMessageType()`.
    // Java: com/google/protobuf/Descriptors.java FieldDescriptor.getMessageType
    pub fn get_message_type(&self) -> &'static Descriptor {
        match self.message_type {
            Some(i) => &pool().messages[i],
            None => panic!("UnsupportedOperationException: This field is not of message type."),
        }
    }

    /// `getEnumType()`.
    // Java: com/google/protobuf/Descriptors.java FieldDescriptor.getEnumType
    pub fn get_enum_type(&self) -> &'static EnumDescriptor {
        match self.enum_type {
            Some(i) => &pool().enums[i],
            None => panic!("UnsupportedOperationException: This field is not of enum type."),
        }
    }

    /// `getContainingOneof()`.
    pub fn get_containing_oneof(&self) -> Option<&'static OneofDescriptor> {
        self.oneof_index
            .map(|i| &self.get_containing_type().oneofs[i])
    }

    /// `getRealContainingOneof()`.
    pub fn get_real_containing_oneof(&self) -> Option<&'static OneofDescriptor> {
        self.get_containing_oneof().filter(|o| !o.synthetic)
    }

    /// `isMapField()`.
    // Java: com/google/protobuf/Descriptors.java FieldDescriptor.isMapField
    pub fn is_map_field(&self) -> bool {
        self.type_ == Type::Message && self.repeated && self.get_message_type().map_entry
    }

    /// `isPacked()`.
    // Java: com/google/protobuf/Descriptors.java FieldDescriptor.isPacked
    pub fn is_packed(&self) -> bool {
        if !self.is_packable() {
            return false;
        }
        match self.syntax {
            Syntax::Proto2 => self.packed_option == Some(true),
            Syntax::Proto3 => self.packed_option != Some(false),
        }
    }

    /// `isPackable()`.
    pub fn is_packable(&self) -> bool {
        self.repeated && self.type_.is_packable()
    }

    /// `hasPresence()`.
    // Java: com/google/protobuf/Descriptors.java FieldDescriptor.hasPresence
    pub fn has_presence(&self) -> bool {
        if self.repeated {
            return false;
        }
        match self.syntax {
            Syntax::Proto2 => true,
            Syntax::Proto3 => {
                self.type_ == Type::Message
                    || self.type_ == Type::Group
                    || self.oneof_index.is_some()
            }
        }
    }

    /// Whether the field is a proto3 `optional` field.
    pub fn has_optional_keyword(&self) -> bool {
        self.proto3_optional
    }

    /// `isGroupLike()`.
    // Java: com/google/protobuf/Descriptors.java:1641 FieldDescriptor.isGroupLike
    pub fn is_group_like(&self) -> bool {
        if self.type_ != Type::Group {
            // Groups are always tag-delimited.
            return false;
        }

        let message_type = self.get_message_type();
        // PORT: `toLowerCase()` of an identifier (ASCII).
        if message_type.name.to_ascii_lowercase() != self.name {
            // Group fields always are always the lowercase type name.
            return false;
        }

        if message_type.file != self.get_containing_type().file {
            // Groups could only be defined in the same file they're used.
            return false;
        }

        // Group messages are always defined in the same scope as the field.
        message_type.containing_type == Some(self.containing_type)
    }

    /// `legacyEnumFieldTreatedAsClosed()`: whether unknown values of this enum field go to the
    /// unknown fields when parsed.
    pub fn enum_is_closed(&self) -> bool {
        self.get_enum_type().is_closed()
    }
}

/// `Descriptors.EnumDescriptor`.
#[derive(Debug)]
pub struct EnumDescriptor {
    /// `getName()`.
    pub name: String,
    /// `getFullName()`.
    pub full_name: String,
    /// `getValues()`, in declaration order.
    pub values: Vec<EnumValueDescriptor>,
    closed: bool,
}

/// `Descriptors.EnumValueDescriptor`.
#[derive(Debug)]
pub struct EnumValueDescriptor {
    /// `getName()`.
    pub name: String,
    /// `getNumber()`.
    pub number: i32,
    /// `getIndex()`.
    pub index: usize,
}

impl EnumDescriptor {
    /// `isClosed()`: enums of proto2 files are closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// `findValueByNumber(number)`: the first value declared with that number.
    pub fn find_value_by_number(&self, number: i32) -> Option<&EnumValueDescriptor> {
        self.values.iter().find(|v| v.number == number)
    }

    /// `findValueByName(name)`.
    pub fn find_value_by_name(&self, name: &str) -> Option<&EnumValueDescriptor> {
        self.values.iter().find(|v| v.name == name)
    }
}

/// All descriptors of the crate's generated types.
#[derive(Debug)]
pub struct DescriptorPool {
    files: Vec<FileDescriptor>,
    messages: Vec<Descriptor>,
    enums: Vec<EnumDescriptor>,
    message_by_name: HashMap<String, usize>,
    enum_by_name: HashMap<String, usize>,
    file_by_name: HashMap<String, usize>,
}

/// The pool, decoded on first use.
pub fn pool() -> &'static DescriptorPool {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let fds = FileDescriptorSet::decode(DESCRIPTOR_SET).expect("embedded descriptor set");
        DescriptorPool::build(&fds)
    })
}

impl DescriptorPool {
    /// The message type with this full name.
    pub fn find_message_type_by_name(&self, full_name: &str) -> Option<&Descriptor> {
        self.message_by_name
            .get(full_name)
            .map(|&i| &self.messages[i])
    }

    /// The enum type with this full name.
    pub fn find_enum_type_by_name(&self, full_name: &str) -> Option<&EnumDescriptor> {
        self.enum_by_name.get(full_name).map(|&i| &self.enums[i])
    }

    /// The file with this name.
    pub fn find_file_by_name(&self, name: &str) -> Option<&FileDescriptor> {
        self.file_by_name.get(name).map(|&i| &self.files[i])
    }

    fn build(fds: &FileDescriptorSet) -> DescriptorPool {
        // Pass 1: names and indices.
        let mut message_by_name = HashMap::new();
        let mut enum_by_name = HashMap::new();
        let mut message_count = 0;
        let mut enum_count = 0;
        fn walk_names(
            prefix: &str,
            msgs: &[DescriptorProto],
            enums: &[EnumDescriptorProto],
            message_by_name: &mut HashMap<String, usize>,
            enum_by_name: &mut HashMap<String, usize>,
            message_count: &mut usize,
            enum_count: &mut usize,
        ) {
            for e in enums {
                enum_by_name.insert(join(prefix, e.name()), *enum_count);
                *enum_count += 1;
            }
            for m in msgs {
                let full = join(prefix, m.name());
                message_by_name.insert(full.clone(), *message_count);
                *message_count += 1;
                walk_names(
                    &full,
                    &m.nested_type,
                    &m.enum_type,
                    message_by_name,
                    enum_by_name,
                    message_count,
                    enum_count,
                );
            }
        }
        for file in &fds.file {
            walk_names(
                file.package(),
                &file.message_type,
                &file.enum_type,
                &mut message_by_name,
                &mut enum_by_name,
                &mut message_count,
                &mut enum_count,
            );
        }

        // Pass 2: the descriptors, in the same walk order.
        let mut pool = DescriptorPool {
            files: Vec::new(),
            messages: Vec::with_capacity(message_count),
            enums: Vec::with_capacity(enum_count),
            message_by_name,
            enum_by_name,
            file_by_name: HashMap::new(),
        };
        for (file_index, file) in fds.file.iter().enumerate() {
            let syntax = match file.syntax() {
                "" | "proto2" => Syntax::Proto2,
                "proto3" => Syntax::Proto3,
                other => unimplemented!("Java: Descriptors.FileDescriptor (syntax {other})"),
            };
            let mut top = Vec::new();
            pool.add_types(
                file_index,
                syntax,
                file.package(),
                None,
                &file.message_type,
                &file.enum_type,
                &mut top,
            );
            pool.file_by_name
                .insert(file.name().to_string(), file_index);
            pool.files.push(FileDescriptor {
                name: file.name().to_string(),
                package: file.package().to_string(),
                syntax,
                dependencies: file.dependency.clone(),
                message_types: top,
            });
        }
        pool
    }

    #[allow(clippy::too_many_arguments)]
    fn add_types(
        &mut self,
        file: usize,
        syntax: Syntax,
        prefix: &str,
        containing_type: Option<usize>,
        msgs: &[DescriptorProto],
        enums: &[EnumDescriptorProto],
        out: &mut Vec<usize>,
    ) {
        for e in enums {
            self.enums.push(EnumDescriptor {
                name: e.name().to_string(),
                full_name: join(prefix, e.name()),
                values: e
                    .value
                    .iter()
                    .enumerate()
                    .map(|(index, v)| EnumValueDescriptor {
                        name: v.name().to_string(),
                        number: v.number(),
                        index,
                    })
                    .collect(),
                closed: syntax == Syntax::Proto2,
            });
        }
        for m in msgs {
            let full_name = join(prefix, m.name());
            let index = self.messages.len();
            debug_assert_eq!(self.message_by_name[&full_name], index);
            let fields = m
                .field
                .iter()
                .enumerate()
                .map(|(field_index, f)| {
                    let type_ = Type::from_proto(f.r#type());
                    let resolve = |map: &HashMap<String, usize>| {
                        let name = f.type_name().trim_start_matches('.');
                        *map.get(name)
                            .unwrap_or_else(|| panic!("unresolved type {name}"))
                    };
                    FieldDescriptor {
                        name: f.name().to_string(),
                        full_name: join(&full_name, f.name()),
                        number: f.number(),
                        type_,
                        repeated: f.label() == ProtoLabel::Repeated,
                        required: f.label() == ProtoLabel::Required,
                        index: field_index,
                        containing_type: index,
                        message_type: matches!(type_, Type::Message | Type::Group)
                            .then(|| resolve(&self.message_by_name)),
                        enum_type: (type_ == Type::Enum).then(|| resolve(&self.enum_by_name)),
                        oneof_index: f.oneof_index.map(|i| i as usize),
                        proto3_optional: f.proto3_optional(),
                        packed_option: f.options.as_ref().and_then(|o| o.packed),
                        syntax,
                    }
                })
                .collect::<Vec<_>>();
            let oneofs = m
                .oneof_decl
                .iter()
                .enumerate()
                .map(|(i, o)| OneofDescriptor {
                    name: o.name().to_string(),
                    index: i,
                    synthetic: fields
                        .iter()
                        .any(|f| f.oneof_index == Some(i) && f.proto3_optional),
                })
                .collect();
            let options = m.options.as_ref();
            self.messages.push(Descriptor {
                name: m.name().to_string(),
                full_name: full_name.clone(),
                file,
                containing_type,
                fields,
                oneofs,
                nested_types: Vec::new(),
                map_entry: options.and_then(|o| o.map_entry).unwrap_or(false),
                message_set_wire_format: options
                    .and_then(|o| o.message_set_wire_format)
                    .unwrap_or(false),
                index,
            });
            out.push(index);
            let mut nested = Vec::new();
            self.add_types(
                file,
                syntax,
                &full_name,
                Some(index),
                &m.nested_type,
                &m.enum_type,
                &mut nested,
            );
            self.messages[index].nested_types = nested;
        }
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}
