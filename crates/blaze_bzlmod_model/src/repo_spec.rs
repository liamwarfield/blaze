//! Ports of `AttributeValues`, `AttributeValuesAdapter`, `RepoRuleId`, `RepoSpec`,
//! `ArchiveRepoSpecBuilder`, `GitRepoSpecBuilder` and `LocalPathRepoSpecs`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use blaze_cmdline::Label;
use blaze_util::string_encoding::{internal_to_unicode, unicode_to_internal};
use indexmap::IndexMap;

use crate::gson::{JsonValue, JsonWriter};

/// A Starlark attribute value of a repo rule or tag, as plain data. Java stores these as
/// Starlark values (`Starlark.NONE`, `Boolean`, `StarlarkInt`, `String`, `Label`, `StarlarkList`
/// or `Tuple`, `Dict`); this enum holds exactly the kinds `AttributeValuesAdapter` supports.
// PORT: StarlarkInt is arbitrary-precision in Java; lockfile serialization only supports values
// that fit an `int` (`toInt`), so i64 covers every value that can reach the lockfile.
#[derive(Debug, Clone)]
pub enum AttributeValue {
    None,
    Bool(bool),
    Int(i64),
    String(String),
    Label(Label),
    List(Vec<AttributeValue>),
    Dict(Vec<(AttributeValue, AttributeValue)>),
}

/// Starlark `Dict.equals` (`Map.equals`): entry order does not matter.
fn dict_eq<K: PartialEq, V: PartialEq>(a: &[(K, V)], b: &[(K, V)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.iter().any(|(k2, v2)| k == k2 && v == v2))
}

/// An order-independent hash of dict entries (Java `Map.hashCode` is a sum of entry hashes).
fn dict_hash<K: Hash, V: Hash, H: Hasher>(entries: &[(K, V)], state: &mut H) {
    let mut sum: u64 = 0;
    for (k, v) in entries {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        k.hash(&mut h);
        v.hash(&mut h);
        sum = sum.wrapping_add(h.finish());
    }
    entries.len().hash(state);
    sum.hash(state);
}

impl PartialEq for AttributeValue {
    fn eq(&self, other: &Self) -> bool {
        use AttributeValue::*;
        match (self, other) {
            (None, None) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (String(a), String(b)) => a == b,
            (Label(a), Label(b)) => a == b,
            (List(a), List(b)) => a == b,
            (Dict(a), Dict(b)) => dict_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for AttributeValue {}

impl Hash for AttributeValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            AttributeValue::None => {}
            AttributeValue::Bool(b) => b.hash(state),
            AttributeValue::Int(i) => i.hash(state),
            AttributeValue::String(s) => s.hash(state),
            AttributeValue::Label(l) => l.hash(state),
            AttributeValue::List(l) => l.hash(state),
            AttributeValue::Dict(d) => dict_hash(d, state),
        }
    }
}

impl PartialEq for AttributeValues {
    // AutoValue equals of the wrapped Dict (Map.equals).
    fn eq(&self, other: &Self) -> bool {
        dict_eq(&self.attributes, &other.attributes)
    }
}

impl Eq for AttributeValues {}

impl Hash for AttributeValues {
    fn hash<H: Hasher>(&self, state: &mut H) {
        dict_hash(&self.attributes, state);
    }
}

impl AttributeValue {
    pub fn string(s: impl Into<String>) -> AttributeValue {
        AttributeValue::String(s.into())
    }

    pub fn string_list<S: Into<String>>(items: impl IntoIterator<Item = S>) -> AttributeValue {
        AttributeValue::List(
            items
                .into_iter()
                .map(|s| AttributeValue::String(s.into()))
                .collect(),
        )
    }

    pub fn string_dict<K: Into<String>, V: Into<String>>(
        items: impl IntoIterator<Item = (K, V)>,
    ) -> AttributeValue {
        AttributeValue::Dict(
            items
                .into_iter()
                .map(|(k, v)| {
                    (
                        AttributeValue::String(k.into()),
                        AttributeValue::String(v.into()),
                    )
                })
                .collect(),
        )
    }
}

/// Wrapper around a dictionary of attribute names and values. Port of `AttributeValues`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:30 AttributeValues
#[derive(Debug, Clone, Default)]
pub struct AttributeValues {
    attributes: Vec<(String, AttributeValue)>,
}

impl AttributeValues {
    /// Creates attribute values; a repeated name keeps its first position and its last value
    /// (Starlark `Dict.Builder`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:32 AttributeValues.create
    pub fn create<K: Into<String>>(
        attribs: impl IntoIterator<Item = (K, AttributeValue)>,
    ) -> AttributeValues {
        let mut attributes: Vec<(String, AttributeValue)> = Vec::new();
        for (k, v) in attribs {
            let k = k.into();
            if let Some(slot) = attributes.iter_mut().find(|(n, _)| *n == k) {
                slot.1 = v;
            } else {
                attributes.push((k, v));
            }
        }
        AttributeValues { attributes }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValues.java:36 AttributeValues.attributes
    pub fn attributes(&self) -> &[(String, AttributeValue)] {
        &self.attributes
    }

    pub fn get(&self, name: &str) -> Option<&AttributeValue> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

/// Port of `AttributeValuesAdapter`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:47 AttributeValuesAdapter
pub struct AttributeValuesAdapter;

// Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:142 AttributeValuesAdapter.STRING_ESCAPE_SEQUENCE
pub const STRING_ESCAPE_SEQUENCE: &str = "'";

impl AttributeValuesAdapter {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:52 AttributeValuesAdapter.write
    pub fn write(out: &mut JsonWriter, attribute_values: &AttributeValues) {
        out.begin_object();
        for (key, value) in attribute_values.attributes() {
            // The attribute name is written as is (Java does not convert it either).
            out.name(key);
            // gson.toJson(JsonElement, out) with a default Gson: nulls in objects are dropped.
            out.json_value_with_gson_defaults(&Self::serialize_object(value));
        }
        out.end_object();
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:62 AttributeValuesAdapter.read
    pub fn read(json: &JsonValue) -> Result<AttributeValues, String> {
        let JsonValue::Object(members) = json else {
            return Err(format!("Not a JSON Object: {}", json_to_string(json)));
        };
        Ok(AttributeValues::create(
            members
                .iter()
                // The attribute name is kept as is (Java does not convert it either).
                .map(|(k, v)| Ok((k.clone(), Self::deserialize_object(v)?)))
                .collect::<Result<Vec<_>, String>>()?,
        ))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:77 AttributeValuesAdapter.serializeObject
    pub fn serialize_object(obj: &AttributeValue) -> JsonValue {
        match obj {
            AttributeValue::None => JsonValue::Null,
            AttributeValue::Bool(b) => JsonValue::Bool(*b),
            AttributeValue::Int(i) => {
                if *i < i32::MIN as i64 || *i > i32::MAX as i64 {
                    panic!(
                        "IllegalArgumentException: Unable to parse StarlarkInt to Integer: \
                         net.starlark.java.eval.EvalException: got {i} for serialization into the \
                         lockfile, want value in signed 32-bit range"
                    );
                }
                JsonValue::Number(i.to_string())
            }
            AttributeValue::String(_) | AttributeValue::Label(_) => {
                JsonValue::String(Self::serialize_obj_to_string(obj))
            }
            AttributeValue::Dict(entries) => JsonValue::Object(entries.iter().fold(
                Vec::<(String, JsonValue)>::new(),
                |mut acc, (k, v)| {
                    // JsonObject.add replaces an existing member in place.
                    let key = Self::serialize_obj_to_string(k);
                    let value = Self::serialize_object(v);
                    if let Some(slot) = acc.iter_mut().find(|(n, _)| *n == key) {
                        slot.1 = value;
                    } else {
                        acc.push((key, value));
                    }
                    acc
                },
            )),
            AttributeValue::List(items) => {
                JsonValue::Array(items.iter().map(Self::serialize_object).collect())
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:109 AttributeValuesAdapter.deserializeObject
    pub fn deserialize_object(json: &JsonValue) -> Result<AttributeValue, String> {
        Ok(match json {
            JsonValue::Null => AttributeValue::None,
            JsonValue::Bool(b) => AttributeValue::Bool(*b),
            JsonValue::Number(n) => AttributeValue::Int(json_number_get_as_int(n)? as i64),
            JsonValue::String(s) => Self::deserialize_string_to_object(s),
            JsonValue::Object(members) => AttributeValue::Dict(
                members
                    .iter()
                    .map(|(k, v)| {
                        Ok((
                            Self::deserialize_string_to_object(k),
                            Self::deserialize_object(v)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            ),
            JsonValue::Array(items) => AttributeValue::List(
                items
                    .iter()
                    .map(Self::deserialize_object)
                    .collect::<Result<Vec<_>, String>>()?,
            ),
        })
    }

    /// Serializes a Label or String (internal strings) to a Unicode JSON string. A string that
    /// looks like a label is escaped so it can be told apart from a label when deserializing.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:152 AttributeValuesAdapter.serializeObjToString
    fn serialize_obj_to_string(obj: &AttributeValue) -> String {
        match obj {
            AttributeValue::Label(label) => {
                let label_string = label.get_unambiguous_canonical_form();
                blaze_util::check_state!(label_string.starts_with("@@"));
                // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:156 AttributeValuesAdapter.serializeObjToString
                internal_to_unicode(&label_string).into_owned()
            }
            AttributeValue::String(string) => {
                if string.starts_with("@@")
                    || (string.starts_with(STRING_ESCAPE_SEQUENCE)
                        && string.ends_with(STRING_ESCAPE_SEQUENCE))
                {
                    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:165 AttributeValuesAdapter.serializeObjToString
                    internal_to_unicode(&format!(
                        "{STRING_ESCAPE_SEQUENCE}{string}{STRING_ESCAPE_SEQUENCE}"
                    ))
                    .into_owned()
                } else {
                    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:167 AttributeValuesAdapter.serializeObjToString
                    internal_to_unicode(string).into_owned()
                }
            }
            other => panic!("ClassCastException: {other:?} cannot be cast to java.lang.String"),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:177 AttributeValuesAdapter.deserializeStringToObject
    fn deserialize_string_to_object(unicode_value: &str) -> AttributeValue {
        // Java: com/google/devtools/build/lib/bazel/bzlmod/AttributeValuesAdapter.java:178 AttributeValuesAdapter.deserializeStringToObject
        let value = unicode_to_internal(unicode_value);
        let value: &str = &value;
        // A string represents a label if and only if it starts with "@@".
        if value.starts_with("@@") {
            return AttributeValue::Label(Label::parse_canonical_unchecked(value));
        }
        // Strings that start and end with the escape sequence always require one layer to be
        // stripped.
        if value.starts_with(STRING_ESCAPE_SEQUENCE) && value.ends_with(STRING_ESCAPE_SEQUENCE) {
            // PORT: Java substring(1, len-1) throws for the single-character string "'".
            let n = STRING_ESCAPE_SEQUENCE.len();
            if value.len() < 2 * n {
                panic!("StringIndexOutOfBoundsException: begin 1, end 0, length 1");
            }
            return AttributeValue::String(value[n..value.len() - n].to_string());
        }
        AttributeValue::String(value.to_string())
    }
}

/// `JsonPrimitive.getAsInt()` on a parsed number (`LazilyParsedNumber.intValue`).
pub(crate) fn json_number_get_as_int(n: &str) -> Result<i32, String> {
    if let Ok(i) = n.parse::<i32>() {
        return Ok(i);
    }
    if let Ok(l) = n.parse::<i64>() {
        return Ok(l as i32);
    }
    // PORT: BigDecimal(value).intValue() for fractional or huge numbers is not needed by any
    // file Bazel writes.
    blaze_util::unported!("LazilyParsedNumber.intValue", "{n}")
}

/// Compact JSON of a value (`JsonElement.toString`), for error messages.
pub(crate) fn json_to_string(json: &JsonValue) -> String {
    let mut w = JsonWriter::new(false);
    w.set_serialize_nulls(true);
    w.json_value(json);
    w.into_string()
}

/// Identifies a repo rule. Port of `RepoRuleId`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RepoRuleId.java:30 RepoRuleId
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRuleId {
    /// The label pointing to the .bzl file defining the repo rule. `None` only for ids read from
    /// a lockfile entry without `%` (Java's null).
    bzl_file_label: Option<Label>,
    rule_name: String,
}

impl RepoRuleId {
    // PORT: the AutoBuilder is `new` / `new_nullable` (a lockfile id may lack the .bzl label).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoRuleId.java:36 RepoRuleId.builder
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoRuleId.java:47 RepoRuleId.Builder.bzlFileLabel
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoRuleId.java:51 RepoRuleId.Builder.ruleName
    // PORT: skipped, no Rust caller copies or half-builds an id: RepoRuleId.toBuilder, RepoRuleId.Builder.isRuleNameSet
    pub fn new(bzl_file_label: Label, rule_name: impl Into<String>) -> RepoRuleId {
        RepoRuleId {
            bzl_file_label: Some(bzl_file_label),
            rule_name: rule_name.into(),
        }
    }

    pub(crate) fn new_nullable(
        bzl_file_label: Option<Label>,
        rule_name: impl Into<String>,
    ) -> RepoRuleId {
        RepoRuleId {
            bzl_file_label,
            rule_name: rule_name.into(),
        }
    }

    pub fn bzl_file_label(&self) -> Option<&Label> {
        self.bzl_file_label.as_ref()
    }

    pub fn rule_name(&self) -> &str {
        &self.rule_name
    }
}

impl fmt::Display for RepoRuleId {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoRuleId.java:32 RepoRuleId.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(label) = &self.bzl_file_label else {
            panic!("NullPointerException: RepoRuleId.bzlFileLabel is null");
        };
        write!(
            f,
            "{}%{}",
            label.get_unambiguous_canonical_form(),
            self.rule_name
        )
    }
}

/// Information about a repo definition: the repo rule and all its attributes (except `name`).
/// Port of `RepoSpec`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpec.java:29 RepoSpec
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoSpec {
    /// `None` only for specs read from a malformed lockfile (Java's null).
    repo_rule_id: Option<RepoRuleId>,
    attributes: AttributeValues,
}

impl RepoSpec {
    pub fn new(repo_rule_id: RepoRuleId, attributes: AttributeValues) -> RepoSpec {
        RepoSpec {
            repo_rule_id: Some(repo_rule_id),
            attributes,
        }
    }

    pub fn new_nullable(repo_rule_id: Option<RepoRuleId>, attributes: AttributeValues) -> RepoSpec {
        RepoSpec {
            repo_rule_id,
            attributes,
        }
    }

    pub fn repo_rule_id(&self) -> Option<&RepoRuleId> {
        self.repo_rule_id.as_ref()
    }

    pub fn attributes(&self) -> &AttributeValues {
        &self.attributes
    }
}

/// `ArchiveRepoSpecBuilder.HTTP_ARCHIVE`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:35 ArchiveRepoSpecBuilder.HTTP_ARCHIVE
pub static HTTP_ARCHIVE: LazyLock<RepoRuleId> = LazyLock::new(|| {
    RepoRuleId::new(
        Label::parse_canonical_unchecked("@@bazel_tools//tools/build_defs/repo:http.bzl"),
        "http_archive",
    )
});

/// `GitRepoSpecBuilder.GIT_REPOSITORY`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:33 GitRepoSpecBuilder.GIT_REPOSITORY
pub static GIT_REPOSITORY: LazyLock<RepoRuleId> = LazyLock::new(|| {
    RepoRuleId::new(
        Label::parse_canonical_unchecked("@@bazel_tools//tools/build_defs/repo:git.bzl"),
        "git_repository",
    )
});

// PORT: skipped, the private constructor of a static-only class: LocalPathRepoSpecs.<init>
/// `LocalPathRepoSpecs.LOCAL_REPOSITORY`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LocalPathRepoSpecs.java:27 LocalPathRepoSpecs.LOCAL_REPOSITORY
pub static LOCAL_REPOSITORY: LazyLock<RepoRuleId> = LazyLock::new(|| {
    RepoRuleId::new(
        Label::parse_canonical_unchecked("@@bazel_tools//tools/build_defs/repo:local.bzl"),
        "local_repository",
    )
});

/// A remote file with its integrity and URLs (`ArchiveRepoSpecBuilder.RemoteFile`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:113 ArchiveRepoSpecBuilder.RemoteFile
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub integrity: String,
    pub urls: Vec<String>,
}

impl RemoteFile {
    pub fn new(integrity: impl Into<String>, urls: Vec<String>) -> RemoteFile {
        RemoteFile {
            integrity: integrity.into(),
            urls,
        }
    }
}

/// Builder for a `RepoSpec` of `http_archive`. Port of `ArchiveRepoSpecBuilder`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:33 ArchiveRepoSpecBuilder
#[derive(Debug, Default)]
pub struct ArchiveRepoSpecBuilder {
    attrs: Vec<(String, AttributeValue)>,
}

impl ArchiveRepoSpecBuilder {
    pub fn new() -> Self {
        ArchiveRepoSpecBuilder::default()
    }

    fn put(&mut self, name: &str, value: AttributeValue) -> &mut Self {
        self.attrs.push((name.to_string(), value));
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:45 ArchiveRepoSpecBuilder.setUrls
    pub fn set_urls(&mut self, urls: Vec<String>) -> &mut Self {
        self.put("urls", AttributeValue::string_list(urls))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:51 ArchiveRepoSpecBuilder.setIntegrity
    pub fn set_integrity(&mut self, integrity: &str) -> &mut Self {
        self.put("integrity", AttributeValue::string(integrity))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:57 ArchiveRepoSpecBuilder.setStripPrefix
    pub fn set_strip_prefix(&mut self, strip_prefix: &str) -> &mut Self {
        self.put("strip_prefix", AttributeValue::string(strip_prefix))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:63 ArchiveRepoSpecBuilder.setPatches
    pub fn set_patches(&mut self, patches: Vec<Label>) -> &mut Self {
        self.put(
            "patches",
            AttributeValue::List(patches.into_iter().map(AttributeValue::Label).collect()),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:69 ArchiveRepoSpecBuilder.setRemotePatches
    pub fn set_remote_patches(&mut self, remote_patches: IndexMap<String, String>) -> &mut Self {
        self.put(
            "remote_patches",
            AttributeValue::string_dict(remote_patches),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:75 ArchiveRepoSpecBuilder.setOverlay
    pub fn set_overlay(&mut self, overlay: IndexMap<String, RemoteFile>) -> &mut Self {
        let remote_files = AttributeValue::Dict(
            overlay
                .iter()
                .map(|(k, rf)| {
                    (
                        AttributeValue::string(k.clone()),
                        AttributeValue::string_list(rf.urls.clone()),
                    )
                })
                .collect(),
        );
        let remote_files_integrity = AttributeValue::Dict(
            overlay
                .iter()
                .map(|(k, rf)| {
                    (
                        AttributeValue::string(k.clone()),
                        AttributeValue::string(rf.integrity.clone()),
                    )
                })
                .collect(),
        );
        self.put("remote_file_urls", remote_files);
        self.put("remote_file_integrity", remote_files_integrity)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:84 ArchiveRepoSpecBuilder.setRemoteModuleFile
    pub fn set_remote_module_file(&mut self, remote_module_file: RemoteFile) -> &mut Self {
        self.put(
            "remote_module_file_urls",
            AttributeValue::string_list(remote_module_file.urls),
        );
        self.put(
            "remote_module_file_integrity",
            AttributeValue::String(remote_module_file.integrity),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:92 ArchiveRepoSpecBuilder.setRemotePatchStrip
    pub fn set_remote_patch_strip(&mut self, remote_patch_strip: i32) -> &mut Self {
        self.put(
            "remote_patch_strip",
            AttributeValue::Int(remote_patch_strip as i64),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:98 ArchiveRepoSpecBuilder.setArchiveType
    pub fn set_archive_type(&mut self, archive_type: Option<&str>) -> &mut Self {
        if let Some(t) = archive_type.filter(|t| !t.is_empty()) {
            self.put("type", AttributeValue::string(t));
        }
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ArchiveRepoSpecBuilder.java:106 ArchiveRepoSpecBuilder.build
    pub fn build(&self) -> RepoSpec {
        RepoSpec::new(
            HTTP_ARCHIVE.clone(),
            AttributeValues::create(self.attrs.clone()),
        )
    }
}

/// Builder for a `RepoSpec` of `git_repository`. Port of `GitRepoSpecBuilder`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:31 GitRepoSpecBuilder
#[derive(Debug, Default)]
pub struct GitRepoSpecBuilder {
    attrs: Vec<(String, AttributeValue)>,
}

impl GitRepoSpecBuilder {
    pub fn new() -> Self {
        GitRepoSpecBuilder::default()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:104 GitRepoSpecBuilder.setAttr
    fn set_attr_str(&mut self, name: &str, value: Option<&str>) -> &mut Self {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            self.attrs
                .push((name.to_string(), AttributeValue::string(v)));
        }
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:112 GitRepoSpecBuilder.setAttr
    fn set_attr_bool(&mut self, name: &str, value: bool) -> &mut Self {
        self.attrs
            .push((name.to_string(), AttributeValue::Bool(value)));
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:118 GitRepoSpecBuilder.setAttr
    fn set_attr_list(&mut self, name: &str, value: &[String]) -> &mut Self {
        if !value.is_empty() {
            self.attrs.push((
                name.to_string(),
                AttributeValue::string_list(value.iter().cloned()),
            ));
        }
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:126 GitRepoSpecBuilder.setAttr
    fn set_attr_map(&mut self, name: &str, value: &IndexMap<String, String>) -> &mut Self {
        if !value.is_empty() {
            self.attrs.push((
                name.to_string(),
                AttributeValue::string_dict(value.iter().map(|(k, v)| (k.clone(), v.clone()))),
            ));
        }
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:134 GitRepoSpecBuilder.setAttr
    fn set_attr_int(&mut self, name: &str, value: i32) -> &mut Self {
        self.attrs
            .push((name.to_string(), AttributeValue::Int(value as i64)));
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:43 GitRepoSpecBuilder.setRemote
    pub fn set_remote(&mut self, remote_repo_url: Option<&str>) -> &mut Self {
        self.set_attr_str("remote", remote_repo_url)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:48 GitRepoSpecBuilder.setCommit
    pub fn set_commit(&mut self, git_commit_hash: Option<&str>) -> &mut Self {
        self.set_attr_str("commit", git_commit_hash)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:53 GitRepoSpecBuilder.setShallowSince
    pub fn set_shallow_since(&mut self, shallow_since: Option<&str>) -> &mut Self {
        self.set_attr_str("shallow_since", shallow_since)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:58 GitRepoSpecBuilder.setTag
    pub fn set_tag(&mut self, tag: Option<&str>) -> &mut Self {
        self.set_attr_str("tag", tag)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:63 GitRepoSpecBuilder.setInitSubmodules
    pub fn set_init_submodules(&mut self, init_submodules: bool) -> &mut Self {
        self.set_attr_bool("init_submodules", init_submodules);
        self.set_attr_bool("recursive_init_submodules", init_submodules)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:70 GitRepoSpecBuilder.setVerbose
    pub fn set_verbose(&mut self, verbose: bool) -> &mut Self {
        self.set_attr_bool("verbose", verbose)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:75 GitRepoSpecBuilder.setStripPrefix
    pub fn set_strip_prefix(&mut self, strip_prefix: Option<&str>) -> &mut Self {
        self.set_attr_str("strip_prefix", strip_prefix)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:80 GitRepoSpecBuilder.setRemotePatches
    pub fn set_remote_patches(&mut self, remote_patches: &IndexMap<String, String>) -> &mut Self {
        self.set_attr_map("remote_patches", remote_patches)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:85 GitRepoSpecBuilder.setRemoteModuleFile
    pub fn set_remote_module_file(&mut self, remote_module_file: &RemoteFile) -> &mut Self {
        self.set_attr_list("remote_module_file_urls", &remote_module_file.urls);
        self.set_attr_str(
            "remote_module_file_integrity",
            Some(&remote_module_file.integrity),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:93 GitRepoSpecBuilder.setRemotePatchStrip
    pub fn set_remote_patch_strip(&mut self, remote_patch_strip: i32) -> &mut Self {
        self.set_attr_int("remote_patch_strip", remote_patch_strip)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/GitRepoSpecBuilder.java:98 GitRepoSpecBuilder.build
    pub fn build(&self) -> RepoSpec {
        RepoSpec::new(
            GIT_REPOSITORY.clone(),
            AttributeValues::create(self.attrs.clone()),
        )
    }
}

/// Port of `LocalPathRepoSpecs.create`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LocalPathRepoSpecs.java:33 LocalPathRepoSpecs.create
pub fn local_path_repo_spec(path: &str) -> RepoSpec {
    RepoSpec::new(
        LOCAL_REPOSITORY.clone(),
        AttributeValues::create([("path", AttributeValue::string(path))]),
    )
}
