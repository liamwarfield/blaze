//! Compiles the vendored `.proto` files (see `proto/`) with `protox` (pure Rust, no `protoc`)
//! into Rust types with `prost-build`, then generates `java_impls.rs`: one
//! `crate::JavaMessage` impl per message that encodes like Java's generated `writeTo` (fields in
//! field-number order, oneof members at their own number, map entries always writing key and
//! value) and prints like `com.google.protobuf.util.JsonFormat`.
//!
//! Map fields are generated as `Vec<FooEntry>` (repeated entry messages, the map's wire form)
//! instead of a hash map: Java's `MapField` keeps insertion order, and that order reaches both
//! the binary and the JSON output.

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use heck::{ToSnakeCase, ToUpperCamelCase};
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet};

/// Root files; their imports are compiled too.
const PROTOS: &[&str] = &[
    "src/main/protobuf/build.proto",
    "src/main/protobuf/analysis_v2.proto",
    "src/main/protobuf/stardoc_output.proto",
    "src/main/protobuf/spawn.proto",
    "src/main/java/com/google/devtools/build/lib/buildeventstream/proto/build_event_stream.proto",
    "src/main/protobuf/command_line.proto",
    "src/main/protobuf/failure_details.proto",
    "src/main/protobuf/invocation_policy.proto",
    "src/main/protobuf/option_filters.proto",
    "src/main/protobuf/action_cache.proto",
    "src/main/protobuf/test_status.proto",
    "src/main/protobuf/strategy_policy.proto",
    "src/main/java/com/google/devtools/build/lib/packages/metrics/package_load_metrics.proto",
    "src/main/protobuf/worker_protocol.proto",
    "src/main/protobuf/command_server.proto",
    "src/main/protobuf/crosstool_config.proto",
    "src/main/protobuf/bazel_flags.proto",
    "build/bazel/remote/execution/v2/remote_execution.proto",
    "build/bazel/semver/semver.proto",
    "google/rpc/status.proto",
    "google/longrunning/operations.proto",
    // Well-known types JsonFormat prints specially; compiled so that `Any` can hold them.
    "google/protobuf/wrappers.proto",
    "google/protobuf/field_mask.proto",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let proto_dir = manifest.join("proto");
    println!("cargo:rerun-if-changed=proto");
    println!("cargo:rerun-if-changed=build.rs");

    let fds = protox::Compiler::new([&proto_dir])
        .unwrap()
        .include_source_info(false)
        .include_imports(true)
        .open_files(PROTOS)
        .unwrap_or_else(|e| panic!("protox: {e}"))
        .file_descriptor_set();

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Types for prost-build: map entries become ordinary nested messages.
    let mut for_prost = fds.clone();
    for file in &mut for_prost.file {
        for msg in &mut file.message_type {
            clear_map_entry(msg);
        }
    }
    prost_build::Config::new()
        .compile_well_known_types()
        .include_file("_includes.rs")
        .out_dir(&out)
        .compile_fds(for_prost)
        .unwrap_or_else(|e| panic!("prost-build: {e}"));

    let generated = Generator::new(&fds).generate();
    fs::write(out.join("java_impls.rs"), generated).unwrap();
}

fn clear_map_entry(msg: &mut DescriptorProto) {
    if let Some(options) = msg.options.as_mut()
        && options.map_entry == Some(true)
    {
        options.map_entry = None;
    }
    for nested in &mut msg.nested_type {
        clear_map_entry(nested);
    }
}

// Naming, copied from prost-build 0.14 `ident.rs` so the generated paths match its output.
fn sanitize_identifier(ident: &str) -> String {
    match ident {
        "as" | "break" | "const" | "continue" | "else" | "enum" | "false" | "fn" | "for" | "if"
        | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref"
        | "return" | "static" | "struct" | "trait" | "true" | "type" | "unsafe" | "use"
        | "where" | "while" | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro"
        | "override" | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "async" | "await"
        | "try" | "gen" => format!("r#{ident}"),
        "_" | "super" | "self" | "Self" | "extern" | "crate" => format!("{ident}_"),
        s if s.starts_with(|c: char| c.is_numeric()) => format!("_{ident}"),
        _ => ident.to_string(),
    }
}

fn to_snake(s: &str) -> String {
    sanitize_identifier(&s.to_snake_case())
}

fn to_upper_camel(s: &str) -> String {
    sanitize_identifier(&s.to_upper_camel_case())
}

/// Java `Descriptors.FieldDescriptor` json name fallback (`fieldNameToJsonName`).
fn json_name(field: &FieldDescriptorProto) -> String {
    if let Some(name) = &field.json_name {
        return name.clone();
    }
    let mut result = String::new();
    let mut capitalize_next = false;
    for ch in field.name().chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

struct MsgInfo<'a> {
    full_name: String,
    rust_path: String,
    /// Path of the module holding this message's nested types and oneof enums.
    module_path: String,
    desc: &'a DescriptorProto,
    file: &'a FileDescriptorProto,
}

struct EnumInfo {
    /// proto2 enums are closed (`EnumDescriptor.isClosed`).
    closed: bool,
    fn_name: String,
    number_fn_name: String,
    /// name -> number, declaration order.
    by_name: Vec<(String, i32)>,
    /// (number, name), first declaration wins (Java `findValueByNumber`).
    values: BTreeMap<i32, String>,
}

struct Generator<'a> {
    fds: &'a FileDescriptorSet,
    messages: Vec<MsgInfo<'a>>,
    by_name: HashMap<String, usize>,
    enums: BTreeMap<String, EnumInfo>,
}

impl<'a> Generator<'a> {
    fn new(fds: &'a FileDescriptorSet) -> Self {
        let mut g = Generator {
            fds,
            messages: Vec::new(),
            by_name: HashMap::new(),
            enums: BTreeMap::new(),
        };
        for file in &fds.file {
            let package = file.package();
            let mut module = String::from("crate");
            for part in package.split('.').filter(|s| !s.is_empty()) {
                module.push_str("::");
                module.push_str(&to_snake(part));
            }
            for e in &file.enum_type {
                g.add_enum(file.syntax() != "proto3", package, e);
            }
            for msg in &file.message_type {
                g.add_message(file, package, &module, msg);
            }
        }
        g
    }

    fn add_enum(&mut self, closed: bool, scope: &str, e: &prost_types::EnumDescriptorProto) {
        let full = if scope.is_empty() {
            e.name().to_string()
        } else {
            format!("{scope}.{}", e.name())
        };
        let mut values = BTreeMap::new();
        for v in &e.value {
            values
                .entry(v.number())
                .or_insert_with(|| v.name().to_string());
        }
        let fn_name = format!("enum_name_{}", full.replace('.', "_"));
        let number_fn_name = format!("enum_number_{}", full.replace('.', "_"));
        let by_name = e
            .value
            .iter()
            .map(|v| (v.name().to_string(), v.number()))
            .collect();
        self.enums.insert(
            full,
            EnumInfo {
                closed,
                fn_name,
                number_fn_name,
                by_name,
                values,
            },
        );
    }

    fn add_message(
        &mut self,
        file: &'a FileDescriptorProto,
        scope: &str,
        module: &str,
        msg: &'a DescriptorProto,
    ) {
        let full = if scope.is_empty() {
            msg.name().to_string()
        } else {
            format!("{scope}.{}", msg.name())
        };
        let rust_path = format!("{module}::{}", to_upper_camel(msg.name()));
        let module_path = format!("{module}::{}", to_snake(msg.name()));
        self.by_name.insert(full.clone(), self.messages.len());
        self.messages.push(MsgInfo {
            full_name: full.clone(),
            rust_path,
            module_path: module_path.clone(),
            desc: msg,
            file,
        });
        for e in &msg.enum_type {
            self.add_enum(file.syntax() != "proto3", &full, e);
        }
        for nested in &msg.nested_type {
            self.add_message(file, &full, &module_path, nested);
        }
    }

    fn msg(&self, type_name: &str) -> &MsgInfo<'a> {
        let name = type_name.trim_start_matches('.');
        &self.messages[*self
            .by_name
            .get(name)
            .unwrap_or_else(|| panic!("unknown message {name}"))]
    }

    fn is_map_entry(desc: &DescriptorProto) -> bool {
        desc.options
            .as_ref()
            .and_then(|o| o.map_entry)
            .unwrap_or(false)
    }

    fn generate(&self) -> String {
        let mut out = String::new();
        out.push_str("// @generated by crates/blaze_protos/build.rs. Do not edit.\n\n");
        for (i, m) in self.messages.iter().enumerate() {
            self.gen_message(i, m, &mut out);
        }
        // Enum name tables.
        for e in self.enums.values() {
            let _ = writeln!(
                out,
                "#[allow(non_snake_case, clippy::match_single_binding)]\npub(crate) fn {}(n: i32) -> ::core::option::Option<&'static str> {{\n    match n {{",
                e.fn_name
            );
            for (num, name) in &e.values {
                let _ = writeln!(out, "        {num} => Some({name:?}),");
            }
            out.push_str("        _ => None,\n    }\n}\n\n");
            let _ = writeln!(
                out,
                "#[allow(non_snake_case, clippy::match_single_binding)]\npub(crate) fn {}(s: &str) -> ::core::option::Option<i32> {{\n    match s {{",
                e.number_fn_name
            );
            for (name, num) in &e.by_name {
                let _ = writeln!(out, "        {name:?} => Some({num}),");
            }
            out.push_str("        _ => None,\n    }\n}\n\n");
        }
        // Any registry: full name -> (file, content printer).
        out.push_str("pub(crate) fn any_entry(full_name: &str) -> ::core::option::Option<crate::json::AnyEntry> {\n    match full_name {\n");
        for m in &self.messages {
            if Self::is_map_entry(m.desc) {
                continue;
            }
            let _ = writeln!(
                out,
                "        {:?} => Some(crate::json::AnyEntry {{ file: {:?}, print: crate::json::print_any_content::<{p}>, parse: crate::json::parse_any_content::<{p}> }}),",
                m.full_name,
                m.file.name(),
                p = m.rust_path
            );
        }
        out.push_str("        _ => None,\n    }\n}\n\n");
        // File dependency table (TypeRegistry.Builder.addFile walks it).
        out.push_str("pub(crate) fn file_dependencies(file: &str) -> &'static [&'static str] {\n    match file {\n");
        for f in &self.fds.file {
            let _ = writeln!(out, "        {:?} => &{:?},", f.name(), f.dependency);
        }
        out.push_str("        _ => &[],\n    }\n}\n");
        out
    }

    fn gen_message(&self, _index: usize, m: &MsgInfo<'a>, out: &mut String) {
        let proto3 = m.file.syntax() == "proto3";
        let map_entry = Self::is_map_entry(m.desc);
        let mut fields: Vec<&FieldDescriptorProto> = m.desc.field.iter().collect();
        fields.sort_by_key(|f| f.number());

        let mut len = String::new();
        let mut enc = String::new();
        let mut json = String::new();
        let mut parse = String::new();
        for f in &fields {
            let fc = self.field_code(m, f, proto3, map_entry);
            len.push_str(&fc.len);
            enc.push_str(&fc.enc);
            json.push_str(&fc.json);
            parse.push_str(&self.parse_arm(m, f, proto3));
        }

        let wkt_print = match m.full_name.as_str() {
            "google.protobuf.Any" => {
                Some("crate::json::print_any(&self.type_url, &self.value, p)".to_string())
            }
            "google.protobuf.Timestamp" => {
                Some("p.print_timestamp(self.seconds, self.nanos)".to_string())
            }
            "google.protobuf.Duration" => {
                Some("p.print_duration(self.seconds, self.nanos)".to_string())
            }
            "google.protobuf.FieldMask" => Some("p.print_field_mask(&self.paths)".to_string()),
            "google.protobuf.Struct" | "google.protobuf.Value" | "google.protobuf.ListValue" => {
                Some("unimplemented!(\"Java: JsonFormat.PrinterImpl.printStruct\")".to_string())
            }
            "google.protobuf.DoubleValue"
            | "google.protobuf.FloatValue"
            | "google.protobuf.Int64Value"
            | "google.protobuf.UInt64Value"
            | "google.protobuf.Int32Value"
            | "google.protobuf.UInt32Value"
            | "google.protobuf.BoolValue"
            | "google.protobuf.StringValue"
            | "google.protobuf.BytesValue" => {
                let f = &m.desc.field[0];
                Some(format!(
                    "{{ let v = &self.value; {} Ok(()) }}",
                    self.print_value(f, "v", false)
                ))
            }
            _ => None,
        };

        let wkt_merge = match m.full_name.as_str() {
            "google.protobuf.Any" => Some(
                "crate::json::parse_any(&mut self.type_url, &mut self.value, v, p)".to_string(),
            ),
            "google.protobuf.Timestamp" => Some(
                "let (s, n) = p.parse_timestamp(v)?; self.seconds = s; self.nanos = n; Ok(())"
                    .to_string(),
            ),
            "google.protobuf.Duration" => Some(
                "let (s, n) = p.parse_duration(v)?; self.seconds = s; self.nanos = n; Ok(())"
                    .to_string(),
            ),
            "google.protobuf.FieldMask" => {
                Some("self.paths = p.parse_field_mask(v)?; Ok(())".to_string())
            }
            "google.protobuf.Struct" | "google.protobuf.Value" | "google.protobuf.ListValue" => {
                Some("unimplemented!(\"Java: JsonFormat.ParserImpl.mergeStruct\")".to_string())
            }
            "google.protobuf.DoubleValue"
            | "google.protobuf.FloatValue"
            | "google.protobuf.Int64Value"
            | "google.protobuf.UInt64Value"
            | "google.protobuf.Int32Value"
            | "google.protobuf.UInt32Value"
            | "google.protobuf.BoolValue"
            | "google.protobuf.StringValue"
            | "google.protobuf.BytesValue" => {
                let f = &m.desc.field[0];
                Some(format!("self.value = {}; Ok(())", self.parse_value(f, "v")))
            }
            _ => None,
        };
        let _ = writeln!(
            out,
            "impl crate::JavaMessage for {path} {{\n    const FULL_NAME: &'static str = {full:?};\n    const FILE: &'static str = {file:?};\n    const JSON_WELL_KNOWN: bool = {wkt};\n",
            path = m.rust_path,
            full = m.full_name,
            file = m.file.name(),
            wkt = wkt_print.is_some(),
        );
        let _ = writeln!(
            out,
            "    #[allow(unused_mut, unused_variables)]\n    fn java_encoded_len(&self) -> usize {{\n        let mut n = 0usize;\n{len}        n\n    }}\n"
        );
        let _ = writeln!(
            out,
            "    #[allow(unused_variables)]\n    fn java_encode_raw(&self, buf: &mut ::prost::alloc::vec::Vec<u8>) {{\n{enc}    }}\n"
        );
        let _ = writeln!(
            out,
            "    #[allow(unused_variables)]\n    fn print_json_fields(&self, p: &mut crate::json::JsonPrinter<'_>) -> ::core::result::Result<(), crate::json::InvalidProtocolBufferException> {{\n{json}        Ok(())\n    }}"
        );
        let _ = writeln!(
            out,
            "\n    #[allow(unused_variables, unreachable_code)]\n    fn merge_json_fields(&mut self, obj: &::serde_json::Map<::prost::alloc::string::String, ::serde_json::Value>, p: &crate::json::JsonParser<'_>) -> ::core::result::Result<(), crate::json::InvalidProtocolBufferException> {{\n        for (k, v) in obj {{\n            match k.as_str() {{\n{parse}                _ => {{ p.unknown_field(k, {:?})?; }}\n            }}\n        }}\n        Ok(())\n    }}",
            m.full_name
        );
        if let Some(body) = wkt_merge {
            let _ = writeln!(
                out,
                "\n    fn merge_json(&mut self, v: &::serde_json::Value, p: &crate::json::JsonParser<'_>) -> ::core::result::Result<(), crate::json::InvalidProtocolBufferException> {{\n        {body}\n    }}"
            );
        }
        if let Some(body) = wkt_print {
            let _ = writeln!(
                out,
                "\n    fn print_json(&self, p: &mut crate::json::JsonPrinter<'_>) -> ::core::result::Result<(), crate::json::InvalidProtocolBufferException> {{\n        {body}\n    }}"
            );
        }
        out.push_str("}\n\n");
    }

    /// (encoding module, kind) for a scalar field type.
    fn scalar(ty: Type) -> (&'static str, &'static str) {
        match ty {
            Type::Double => ("double", "f64"),
            Type::Float => ("float", "f32"),
            Type::Int64 => ("int64", "i64"),
            Type::Uint64 => ("uint64", "u64"),
            Type::Int32 => ("int32", "i32"),
            Type::Fixed64 => ("fixed64", "u64"),
            Type::Fixed32 => ("fixed32", "u32"),
            Type::Bool => ("bool", "bool"),
            Type::String => ("string", "str"),
            Type::Bytes => ("bytes", "bytes"),
            Type::Uint32 => ("uint32", "u32"),
            Type::Enum => ("int32", "enum"),
            Type::Sfixed32 => ("sfixed32", "i32"),
            Type::Sfixed64 => ("sfixed64", "i64"),
            Type::Sint32 => ("sint32", "i32"),
            Type::Sint64 => ("sint64", "i64"),
            Type::Message | Type::Group => unreachable!("not a scalar"),
        }
    }

    fn nondefault(kind: &str, v: &str) -> String {
        match kind {
            "f32" | "f64" => format!("{v}.to_bits() != 0"),
            "bool" => format!("*{v}"),
            "str" | "bytes" => format!("!{v}.is_empty()"),
            _ => format!("*{v} != 0"),
        }
    }

    /// Code printing one value (`printSingleFieldValue`) held by reference in `v`.
    fn print_value(&self, f: &FieldDescriptorProto, v: &str, always_with_quotes: bool) -> String {
        let q = always_with_quotes;
        match f.r#type() {
            Type::Message => format!("{v}.print_json(p)?;"),
            Type::Group => panic!("groups are not supported"),
            ty => {
                let (_, kind) = Self::scalar(ty);
                match kind {
                    "i32" => format!("p.print_i32(*{v}, {q});"),
                    "u32" => format!("p.print_u32(*{v}, {q});"),
                    "i64" => format!("p.print_i64(*{v});"),
                    "u64" => format!("p.print_u64(*{v});"),
                    "f32" => format!("p.print_f32(*{v}, {q});"),
                    "f64" => format!("p.print_f64(*{v}, {q});"),
                    "bool" => format!("p.print_bool(*{v}, {q});"),
                    "str" => format!("p.print_string({v});"),
                    "bytes" => format!("p.print_bytes({v});"),
                    "enum" => {
                        let en = f.type_name().trim_start_matches('.');
                        let info = &self.enums[en];
                        format!(
                            "p.print_enum(*{v}, {}, {}, {q});",
                            info.fn_name,
                            en == "google.protobuf.NullValue"
                        )
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    /// Expression parsing JSON value `v` for one element of field `f`
    /// (`ParserImpl.parseFieldValue`). Unknown enum names `continue` the enclosing loop.
    fn parse_value(&self, f: &FieldDescriptorProto, v: &str) -> String {
        match f.r#type() {
            Type::Message => {
                let path = &self.msg(f.type_name()).rust_path;
                format!("{{ let mut m = {path}::default(); m.merge_json({v}, p)?; m }}")
            }
            Type::Group => panic!("groups are not supported"),
            ty => {
                let (md, kind) = Self::scalar(ty);
                match (md, kind) {
                    (_, "i32") => format!("p.parse_i32({v})?"),
                    (_, "u32") => format!("p.parse_u32({v})?"),
                    (_, "i64") => format!("p.parse_i64({v})?"),
                    (_, "u64") => format!("p.parse_u64({v})?"),
                    (_, "f32") => format!("p.parse_f32({v})?"),
                    (_, "f64") => format!("p.parse_f64({v})?"),
                    (_, "bool") => format!("p.parse_bool({v})?"),
                    (_, "str") => format!("p.parse_string({v})?"),
                    (_, "bytes") => format!("p.parse_bytes({v})?"),
                    (_, "enum") => {
                        let en = f.type_name().trim_start_matches('.');
                        let info = &self.enums[en];
                        format!(
                            "match p.parse_enum({v}, {}, {}, {}, {en:?})? {{ ::core::option::Option::Some(n) => n, ::core::option::Option::None => continue }}",
                            info.number_fn_name, info.fn_name, info.closed
                        )
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    /// One `match` arm of `merge_json_fields` (`ParserImpl.mergeField`).
    fn parse_arm(&self, m: &MsgInfo<'a>, f: &FieldDescriptorProto, proto3: bool) -> String {
        let jname = json_name(f);
        let pname = f.name();
        let pat = if jname == pname {
            format!("{pname:?}")
        } else {
            format!("{jname:?} | {pname:?}")
        };
        let is_message = f.r#type() == Type::Message;
        let proto3_optional = f.proto3_optional.unwrap_or(false);
        let value = self.parse_value(f, "v");
        let wrap = if is_message { ".into()" } else { "" };
        if let Some(oneof_index) = f.oneof_index.filter(|_| !proto3_optional) {
            let oneof = &m.desc.oneof_decl[oneof_index as usize];
            let conflict = m
                .desc
                .nested_type
                .iter()
                .map(|t| t.name())
                .chain(m.desc.enum_type.iter().map(|e| e.name()))
                .any(|n| to_upper_camel(n) == to_upper_camel(oneof.name()));
            let mut ty = to_upper_camel(oneof.name());
            if conflict {
                ty.push_str("OneOf");
            }
            let variant = format!("{}::{}::{}", m.module_path, ty, to_upper_camel(f.name()));
            let rust = format!("self.{}", to_snake(oneof.name()));
            return format!(
                "                {pat} => {{ if v.is_null() {{ if matches!(&{rust}, ::core::option::Option::Some({variant}(_))) {{ {rust} = ::core::option::Option::None; }} continue; }} let x = {value}; {rust} = ::core::option::Option::Some({variant}(x{wrap})); }}\n"
            );
        }
        let rust = format!("self.{}", to_snake(f.name()));
        if f.label() == Label::Repeated {
            let em = is_message.then(|| self.msg(f.type_name()));
            if let Some(em) = em.filter(|em| Self::is_map_entry(em.desc)) {
                let key = em.desc.field.iter().find(|x| x.number() == 1).unwrap();
                let val = em.desc.field.iter().find(|x| x.number() == 2).unwrap();
                let key_parse = match Self::scalar(key.r#type()).1 {
                    "str" => "k.clone()".to_string(),
                    _ => self.parse_value(key, "&::serde_json::Value::String(k.clone())"),
                };
                let val_parse = self.parse_value(val, "v");
                let val_wrap = if val.r#type() == Type::Message {
                    "::core::option::Option::Some(x)"
                } else {
                    "x"
                };
                return format!(
                    "                {pat} => {{ if v.is_null() {{ {rust}.clear(); continue; }} let obj = p.expect_map(v)?; for (k, v) in obj {{ if v.is_null() {{ return Err(crate::json::InvalidProtocolBufferException::new(\"Map value cannot be null.\")); }} let key = {key_parse}; let x = {val_parse}; {rust}.push({} {{ key, value: {val_wrap} }}); }} }}\n",
                    em.rust_path
                );
            }
            return format!(
                "                {pat} => {{ if v.is_null() {{ {rust}.clear(); continue; }} let arr = p.expect_array(v, {pname:?})?; for v in arr {{ if v.is_null() {{ return Err(p.null_element({pname:?})); }} let x = {value}; {rust}.push(x); }} }}\n"
            );
        }
        let optional = proto3_optional || (f.label() == Label::Optional && (is_message || !proto3));
        if optional {
            return format!(
                "                {pat} => {{ if v.is_null() {{ {rust} = ::core::option::Option::None; continue; }} let x = {value}; {rust} = ::core::option::Option::Some(x{wrap}); }}\n"
            );
        }
        format!(
            "                {pat} => {{ if v.is_null() {{ {rust} = ::core::default::Default::default(); continue; }} let x = {value}; {rust} = x{wrap}; }}\n"
        )
    }

    fn field_code(
        &self,
        m: &MsgInfo<'a>,
        f: &FieldDescriptorProto,
        proto3: bool,
        map_entry: bool,
    ) -> FieldCode {
        let num = f.number();
        let jname = json_name(f);
        let pname = f.name();
        let is_message = matches!(f.r#type(), Type::Message | Type::Group);
        if f.r#type() == Type::Group {
            panic!("groups are not supported: {}", f.name());
        }
        let field_header = format!("p.field({jname:?}, {pname:?});");
        let proto3_optional = f.proto3_optional.unwrap_or(false);
        let real_oneof = f.oneof_index.filter(|_| !proto3_optional);

        // Oneof member.
        if let Some(oneof_index) = real_oneof {
            let oneof = &m.desc.oneof_decl[oneof_index as usize];
            let conflict = m
                .desc
                .nested_type
                .iter()
                .map(|t| t.name())
                .chain(m.desc.enum_type.iter().map(|e| e.name()))
                .any(|n| to_upper_camel(n) == to_upper_camel(oneof.name()));
            let mut ty = to_upper_camel(oneof.name());
            if conflict {
                ty.push_str("OneOf");
            }
            let pat = format!(
                "if let ::core::option::Option::Some({}::{}::{}(v)) = &self.{}",
                m.module_path,
                ty,
                to_upper_camel(f.name()),
                to_snake(oneof.name())
            );
            let (len, enc) = if is_message {
                (
                    format!("v.java_encoded_field_len({num})"),
                    format!("v.java_encode_field({num}, buf);"),
                )
            } else {
                let (md, _) = Self::scalar(f.r#type());
                (
                    format!("::prost::encoding::{md}::encoded_len({num}, v)"),
                    format!("::prost::encoding::{md}::encode({num}, v, buf);"),
                )
            };
            let json = format!(
                "        {pat} {{ {field_header} {} }}\n",
                self.print_value(f, "v", false)
            );
            return FieldCode {
                len: format!("        {pat} {{ n += {len}; }}\n"),
                enc: format!("        {pat} {{ {enc} }}\n"),
                json,
            };
        }

        let rust = format!("self.{}", to_snake(f.name()));

        // Repeated (including maps).
        if f.label() == Label::Repeated {
            let entry = if is_message {
                let em = self.msg(f.type_name());
                Self::is_map_entry(em.desc).then_some(em)
            } else {
                None
            };
            let (len, enc) = if is_message {
                (
                    format!(
                        "{rust}.iter().map(|v| v.java_encoded_field_len({num})).sum::<usize>()"
                    ),
                    format!("for v in &{rust} {{ v.java_encode_field({num}, buf); }}"),
                )
            } else {
                let (md, kind) = Self::scalar(f.r#type());
                let packable = !matches!(kind, "str" | "bytes");
                let explicit = f.options.as_ref().and_then(|o| o.packed);
                let packed = packable && explicit.unwrap_or(proto3);
                if packed {
                    (
                        format!("::prost::encoding::{md}::encoded_len_packed({num}, &{rust})"),
                        format!("::prost::encoding::{md}::encode_packed({num}, &{rust}, buf);"),
                    )
                } else {
                    (
                        format!("::prost::encoding::{md}::encoded_len_repeated({num}, &{rust})"),
                        format!("::prost::encoding::{md}::encode_repeated({num}, &{rust}, buf);"),
                    )
                }
            };
            let json = if let Some(em) = entry {
                let key = em.desc.field.iter().find(|x| x.number() == 1).unwrap();
                let value = em.desc.field.iter().find(|x| x.number() == 2).unwrap();
                let value_print = if value.r#type() == Type::Message {
                    let vm = self.msg(value.type_name());
                    format!(
                        "match &e.value {{ ::core::option::Option::Some(v) => {{ {} }} ::core::option::Option::None => {{ let v = &{}::default(); {} }} }}",
                        self.print_value(value, "v", false),
                        vm.rust_path,
                        self.print_value(value, "v", false)
                    )
                } else {
                    format!(
                        "{{ let v = &e.value; {} }}",
                        self.print_value(value, "v", false)
                    )
                };
                format!(
                    "        if !{rust}.is_empty() || p.always_print_fields_with_no_presence() {{ {field_header} p.begin_map(); for e in &{rust} {{ p.map_key(); {{ let v = &e.key; {} }} p.map_value(); {value_print} }} p.end_map(); }}\n",
                    self.print_value(key, "v", true)
                )
            } else {
                format!(
                    "        if !{rust}.is_empty() || p.always_print_fields_with_no_presence() {{ {field_header} p.begin_array(); for v in &{rust} {{ p.array_element(); {} }} p.end_array(); }}\n",
                    self.print_value(f, "v", false)
                )
            };
            return FieldCode {
                len: format!("        n += {len};\n"),
                enc: format!("        {enc}\n"),
                json,
            };
        }

        // Singular. Rust representation follows prost-build `CodeGenerator::optional`.
        let required = f.label() == Label::Required;
        let optional = proto3_optional || (f.label() == Label::Optional && (is_message || !proto3));
        let has_presence = !(proto3 && !optional && !is_message) || proto3_optional;
        if map_entry {
            // MapEntryLite.writeTo: key and value are always written.
            if optional {
                if !is_message {
                    panic!("proto2 map entries are not supported: {}", m.full_name);
                }
                let vm = self.msg(f.type_name());
                let d = format!("{}::default()", vm.rust_path);
                return FieldCode {
                    len: format!(
                        "        n += match &{rust} {{ ::core::option::Option::Some(v) => v.java_encoded_field_len({num}), ::core::option::Option::None => {d}.java_encoded_field_len({num}) }};\n"
                    ),
                    enc: format!(
                        "        match &{rust} {{ ::core::option::Option::Some(v) => v.java_encode_field({num}, buf), ::core::option::Option::None => {d}.java_encode_field({num}, buf) }}\n"
                    ),
                    json: String::new(),
                };
            }
            let (md, _) = Self::scalar(f.r#type());
            return FieldCode {
                len: format!(
                    "        n += ::prost::encoding::{md}::encoded_len({num}, &{rust});\n"
                ),
                enc: format!("        ::prost::encoding::{md}::encode({num}, &{rust}, buf);\n"),
                json: String::new(),
            };
        }

        if is_message {
            if required {
                return FieldCode {
                    len: format!("        n += {rust}.java_encoded_field_len({num});\n"),
                    enc: format!("        {rust}.java_encode_field({num}, buf);\n"),
                    json: format!(
                        "        {{ let v = &{rust}; {field_header} {} }}\n",
                        self.print_value(f, "v", false)
                    ),
                };
            }
            let pat = format!("if let ::core::option::Option::Some(v) = &{rust}");
            return FieldCode {
                len: format!("        {pat} {{ n += v.java_encoded_field_len({num}); }}\n"),
                enc: format!("        {pat} {{ v.java_encode_field({num}, buf); }}\n"),
                json: format!(
                    "        {pat} {{ {field_header} {} }}\n",
                    self.print_value(f, "v", false)
                ),
            };
        }

        let (md, kind) = Self::scalar(f.r#type());
        if optional {
            let pat = format!("if let ::core::option::Option::Some(v) = &{rust}");
            return FieldCode {
                len: format!(
                    "        {pat} {{ n += ::prost::encoding::{md}::encoded_len({num}, v); }}\n"
                ),
                enc: format!(
                    "        {pat} {{ ::prost::encoding::{md}::encode({num}, v, buf); }}\n"
                ),
                json: format!(
                    "        {pat} {{ {field_header} {} }}\n",
                    self.print_value(f, "v", false)
                ),
            };
        }
        if required {
            return FieldCode {
                len: format!(
                    "        n += ::prost::encoding::{md}::encoded_len({num}, &{rust});\n"
                ),
                enc: format!("        ::prost::encoding::{md}::encode({num}, &{rust}, buf);\n"),
                json: format!(
                    "        {{ let v = &{rust}; {field_header} {} }}\n",
                    self.print_value(f, "v", false)
                ),
            };
        }
        // proto3 implicit presence.
        assert!(!has_presence);
        let cond = Self::nondefault(kind, "v");
        FieldCode {
            len: format!(
                "        {{ let v = &{rust}; if {cond} {{ n += ::prost::encoding::{md}::encoded_len({num}, v); }} }}\n"
            ),
            enc: format!(
                "        {{ let v = &{rust}; if {cond} {{ ::prost::encoding::{md}::encode({num}, v, buf); }} }}\n"
            ),
            json: format!(
                "        {{ let v = &{rust}; if {cond} || p.always_print_fields_with_no_presence() {{ {field_header} {} }} }}\n",
                self.print_value(f, "v", false)
            ),
        }
    }
}

struct FieldCode {
    len: String,
    enc: String,
    json: String,
}
