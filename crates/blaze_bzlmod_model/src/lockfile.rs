//! Ports of `BazelLockFileValue`, `LockFileModuleExtension`, `ModuleExtensionEvalFactors`,
//! `LockfileModuleExtensionMetadata`, the lockfile's Gson serialization (`GsonTypeAdapterUtil`
//! `LOCKFILE_GSON` and the auto-value-gson adapters), the reading part of
//! `BazelLockFileFunction`, and `BazelLockFileModule.combineModuleExtensions`/`updateLockfile`.

use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

use blaze_cmdline::Label;
use blaze_util::string_encoding::{internal_to_unicode, unicode_to_internal};
use indexmap::IndexMap;
use regex::Regex;

use crate::checksum::{Checksum, KeyType, base64_decode, base64_encode};
use crate::external_deps_exception::{ExternalDepsCode, ExternalDepsException};
use crate::facts::{Facts, FactsAdapter};
use crate::gson::{JsonValue, JsonWriter, parse_lenient};
use crate::module_extension::ModuleExtensionId;
use crate::module_key::ModuleKey;
use crate::repo_spec::{AttributeValuesAdapter, RepoRuleId, RepoSpec};
use crate::resolution::LockfileMode;
use crate::version::java_string_compare;

// Gson's adapter registry has no counterpart: the writer and reader below apply each type's
// adapter directly at the field it serializes (marked per adapter).
// PORT: skipped GsonTypeAdapterUtil.newGsonBuilder: Gson adapter registry (see above).
// PORT: skipped OptionalChecksumTypeAdapterFactory.create: Gson adapter registry (see above).
// PORT: skipped DelegateTypeAdapterFactory.create: Gson adapter registry (see above).

/// The current lockfile version.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:46 BazelLockFileValue.LOCK_FILE_VERSION
pub const LOCK_FILE_VERSION: i32 = 28;

/// The evaluation factors (OS and architecture) of a module extension lockfile entry. Port of
/// `ModuleExtensionEvalFactors`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:30 ModuleExtensionEvalFactors
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ModuleExtensionEvalFactors {
    os: String,
    arch: String,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:32 ModuleExtensionEvalFactors.OS_KEY
const OS_KEY: &str = "os:";
const ARCH_KEY: &str = "arch:";
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:37 ModuleExtensionEvalFactors.GENERAL_EXTENSION
const GENERAL_EXTENSION: &str = "general";

impl ModuleExtensionEvalFactors {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:81 ModuleExtensionEvalFactors.create
    pub fn create(os: impl Into<String>, arch: impl Into<String>) -> Self {
        ModuleExtensionEvalFactors {
            os: os.into(),
            arch: arch.into(),
        }
    }

    pub fn get_os(&self) -> &str {
        &self.os
    }

    pub fn get_arch(&self) -> &str {
        &self.arch
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:48 ModuleExtensionEvalFactors.isEmpty
    pub fn is_empty(&self) -> bool {
        self.os.is_empty() && self.arch.is_empty()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:52 ModuleExtensionEvalFactors.hasSameDependenciesAs
    pub fn has_same_dependencies_as(&self, other: &ModuleExtensionEvalFactors) -> bool {
        self.os.is_empty() == other.os.is_empty() && self.arch.is_empty() == other.arch.is_empty()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:85 ModuleExtensionEvalFactors.parse
    pub fn parse(s: &str) -> Self {
        if s == GENERAL_EXTENSION {
            return Self::create("", "");
        }
        let mut os = "";
        let mut arch = "";
        for part in s.split(',') {
            if let Some(o) = part.strip_prefix(OS_KEY) {
                os = o;
            } else if let Some(a) = part.strip_prefix(ARCH_KEY) {
                arch = a;
            }
        }
        Self::create(os, arch)
    }
}

impl fmt::Display for ModuleExtensionEvalFactors {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:58 ModuleExtensionEvalFactors.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str(GENERAL_EXTENSION);
        }
        let mut parts = Vec::new();
        if !self.os.is_empty() {
            parts.push(format!("{OS_KEY}{}", self.os));
        }
        if !self.arch.is_empty() {
            parts.push(format!("{ARCH_KEY}{}", self.arch));
        }
        f.write_str(&parts.join(","))
    }
}

impl Ord for ModuleExtensionEvalFactors {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionEvalFactors.java:76 ModuleExtensionEvalFactors.compareTo
    fn cmp(&self, other: &Self) -> Ordering {
        java_string_compare(&self.to_string(), &other.to_string())
    }
}

impl PartialOrd for ModuleExtensionEvalFactors {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `ModuleExtensionMetadata.UseAllRepos`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionMetadata.java:176 ModuleExtensionMetadata.UseAllRepos
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UseAllRepos {
    No,
    Regular,
    Dev,
}

impl UseAllRepos {
    fn name(self) -> &'static str {
        match self {
            UseAllRepos::No => "NO",
            UseAllRepos::Regular => "REGULAR",
            UseAllRepos::Dev => "DEV",
        }
    }

    fn value_of(s: &str) -> Option<UseAllRepos> {
        match s {
            "NO" => Some(UseAllRepos::No),
            "REGULAR" => Some(UseAllRepos::Regular),
            "DEV" => Some(UseAllRepos::Dev),
            _ => None,
        }
    }
}

/// The part of `ModuleExtensionMetadata` persisted in the lockfile. Port of
/// `LockfileModuleExtensionMetadata` (data only; `generateFixup` belongs with extension
/// evaluation).
// Java: com/google/devtools/build/lib/bazel/bzlmod/LockfileModuleExtensionMetadata.java:51 LockfileModuleExtensionMetadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileModuleExtensionMetadata {
    pub explicit_root_module_direct_deps: Option<Vec<String>>,
    pub explicit_root_module_direct_dev_deps: Option<Vec<String>>,
    pub use_all_repos: UseAllRepos,
    pub reproducible: bool,
}

/// `RepoRecordedInput.WithValue`, kept in its lockfile string form.
// PORT: RepoRecordedInput lives in bazel/repository (not ported yet). Its `parse`/`toString`
// round-trip the strings Bazel writes; entries Java fails to parse become PARSE_FAILURE there,
// which this string form does not model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRecordedInputWithValue(pub String);

/// The transitive digest and generated repos of one module extension evaluation, as stored in
/// the lockfile. Port of `LockFileModuleExtension`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/LockFileModuleExtension.java:35 LockFileModuleExtension
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockFileModuleExtension {
    pub bzl_transitive_digest: Vec<u8>,
    pub usages_digest: Vec<u8>,
    pub recorded_inputs: Vec<RepoRecordedInputWithValue>,
    pub generated_repo_specs: IndexMap<String, RepoSpec>,
    pub module_extension_metadata: Option<LockfileModuleExtensionMetadata>,
}

impl LockFileModuleExtension {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/LockFileModuleExtension.java:55 LockFileModuleExtension.isReproducible
    pub fn is_reproducible(&self) -> bool {
        self.module_extension_metadata
            .as_ref()
            .is_some_and(|m| m.reproducible)
    }
}

/// A [`LockFileModuleExtension`] with its [`ModuleExtensionEvalFactors`].
// Java: com/google/devtools/build/lib/bazel/bzlmod/LockFileModuleExtension.java:79 LockFileModuleExtension.WithFactors
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockFileModuleExtensionWithFactors {
    pub extension_factors: ModuleExtensionEvalFactors,
    pub module_extension: LockFileModuleExtension,
}

/// The contents of a lockfile. Port of `BazelLockFileValue`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:41 BazelLockFileValue
#[derive(Debug, Clone, PartialEq)]
pub struct BazelLockFileValue {
    pub lock_file_version: i32,
    /// Hashes of files retrieved from registries (`None`: known to be missing).
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
    /// Selected module versions that are known to be yanked (and explicitly allowed).
    pub selected_yanked_versions: IndexMap<ModuleKey, String>,
    pub module_extensions:
        IndexMap<ModuleExtensionId, IndexMap<ModuleExtensionEvalFactors, LockFileModuleExtension>>,
    /// Per-extension facts.
    pub facts: IndexMap<ModuleExtensionId, Facts>,
    /// The `factsVersion` declared when the facts were written (default 0).
    pub facts_versions: IndexMap<ModuleExtensionId, i32>,
}

impl Default for BazelLockFileValue {
    fn default() -> Self {
        Self::empty_lockfile()
    }
}

/// A lockfile parse failure: the message of the Java exception (`JsonSyntaxException`,
/// `NullPointerException`, `IllegalArgumentException`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileParseError(pub String);

impl fmt::Display for LockfileParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LockfileParseError {}

fn err<T>(m: impl Into<String>) -> Result<T, LockfileParseError> {
    Err(LockfileParseError(m.into()))
}

fn expect_object<'a>(
    v: &'a JsonValue,
    path: &str,
) -> Result<&'a [(String, JsonValue)], LockfileParseError> {
    match v {
        JsonValue::Object(o) => Ok(o),
        other => err(format!(
            "java.lang.IllegalStateException: Expected BEGIN_OBJECT but was {} at path {path}",
            other.token_name()
        )),
    }
}

fn expect_string<'a>(v: &'a JsonValue, path: &str) -> Result<&'a str, LockfileParseError> {
    match v {
        JsonValue::String(s) | JsonValue::Number(s) => Ok(s),
        other => err(format!(
            "java.lang.IllegalStateException: Expected a string but was {} at path {path}",
            other.token_name()
        )),
    }
}

fn expect_int(v: &JsonValue, path: &str) -> Result<i32, LockfileParseError> {
    match v {
        JsonValue::Number(n) | JsonValue::String(n) => n.parse::<i32>().or_else(|_| {
            err(format!(
                "java.lang.NumberFormatException: Expected an int but was {n} at path {path}"
            ))
        }),
        other => err(format!(
            "java.lang.IllegalStateException: Expected an int but was {} at path {path}",
            other.token_name()
        )),
    }
}

fn expect_bool(v: &JsonValue, path: &str) -> Result<bool, LockfileParseError> {
    match v {
        JsonValue::Bool(b) => Ok(*b),
        JsonValue::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        other => err(format!(
            "java.lang.IllegalStateException: Expected a boolean but was {} at path {path}",
            other.token_name()
        )),
    }
}

/// Reads a map with Gson's `MapTypeAdapterFactory` (duplicate keys are an error).
fn read_map<K: std::hash::Hash + Eq, V>(
    v: &JsonValue,
    path: &str,
    mut key: impl FnMut(&str) -> Result<K, LockfileParseError>,
    mut value: impl FnMut(&JsonValue, &str) -> Result<V, LockfileParseError>,
) -> Result<IndexMap<K, V>, LockfileParseError> {
    let members = expect_object(v, path)?;
    let mut out = IndexMap::with_capacity(members.len());
    for (k, val) in members {
        let p = format!("{path}.{k}");
        let parsed_key = key(k)?;
        if out.insert(parsed_key, value(val, &p)?).is_some() {
            return err(format!("duplicate key: {k}"));
        }
    }
    Ok(out)
}

/// `GsonTypeAdapterUtil.STRING_ADAPTER.write`: a `String` field, list element or map key of a
/// Gson-serialized bzlmod value. The lockfile is Unicode JSON; Bazel's strings are internal.
// Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:64 GsonTypeAdapterUtil.STRING_ADAPTER
pub fn string_adapter_write(out: &mut JsonWriter, s: &str) {
    out.value_str(&internal_to_unicode(s));
}

/// `GsonTypeAdapterUtil.STRING_ADAPTER.read`: the internal string of a JSON string (or map key).
// Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:69 GsonTypeAdapterUtil.STRING_ADAPTER
pub fn string_adapter_read(json_string: &str) -> String {
    unicode_to_internal(json_string).into_owned()
}

/// `lockFileVersion` pattern used to sniff the version before parsing.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:56 BazelLockFileFunction.LOCKFILE_VERSION_PATTERN
// PORT: `\s` and `\d` of java.util.regex are ASCII-only (the pattern is compiled without
// UNICODE_CHARACTER_CLASS), hence `(?-u:...)`.
static LOCKFILE_VERSION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""lockFileVersion":(?-u:\s)*((?-u:\d)+)"#).unwrap());

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:59 BazelLockFileFunction.POSSIBLE_MERGE_CONFLICT_PATTERN
static POSSIBLE_MERGE_CONFLICT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<<<<<<<|=======|\|\|\|\|\|\|\||>>>>>>>").unwrap());

impl BazelLockFileValue {
    /// A valid empty lockfile (`EMPTY_LOCKFILE`, `builder().build()`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:49 BazelLockFileValue.EMPTY_LOCKFILE
    pub fn empty_lockfile() -> BazelLockFileValue {
        BazelLockFileValue {
            lock_file_version: LOCK_FILE_VERSION,
            registry_file_hashes: IndexMap::new(),
            selected_yanked_versions: IndexMap::new(),
            module_extensions: IndexMap::new(),
            facts: IndexMap::new(),
            facts_versions: IndexMap::new(),
        }
    }

    /// `LOCKFILE_GSON.toJson(value)` (no trailing newline).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:401 GsonTypeAdapterUtil.LOCKFILE_GSON
    pub fn to_json(&self) -> String {
        let mut out = JsonWriter::new(true);
        out.begin_object();
        out.name("lockFileVersion")
            .value_number(&self.lock_file_version.to_string());

        out.name("registryFileHashes").begin_object();
        for (url, checksum) in &self.registry_file_hashes {
            // Map key: STRING_ADAPTER.
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:64 GsonTypeAdapterUtil.STRING_ADAPTER
            out.name(&internal_to_unicode(url));
            // OptionalChecksumTypeAdapter: checksums are ASCII.
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:377 GsonTypeAdapterUtil.OptionalChecksumTypeAdapterFactory.OptionalChecksumTypeAdapter.write
            match checksum {
                Some(c) => out.value_str(&c.to_string()),
                None => out.value_str("not found"),
            };
        }
        out.end_object();

        out.name("selectedYankedVersions").begin_object();
        for (key, info) in &self.selected_yanked_versions {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:102 GsonTypeAdapterUtil.MODULE_KEY_TYPE_ADAPTER
            out.name(&internal_to_unicode(&key.to_string()));
            string_adapter_write(&mut out, info);
        }
        out.end_object();

        out.name("moduleExtensions").begin_object();
        for (id, per_factors) in &self.module_extensions {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:182 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
            out.name(&internal_to_unicode(&id.to_lockfile_form()))
                .begin_object();
            for (factors, ext) in per_factors {
                // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:228 GsonTypeAdapterUtil.MODULE_EXTENSION_FACTORS_TYPE_ADAPTER
                out.name(&internal_to_unicode(&factors.to_string()));
                write_lock_file_module_extension(&mut out, ext);
            }
            out.end_object();
        }
        out.end_object();

        out.name("facts").begin_object();
        for (id, facts) in &self.facts {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:182 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
            out.name(&internal_to_unicode(&id.to_lockfile_form()));
            FactsAdapter::write(&mut out, facts);
        }
        out.end_object();

        out.name("factsVersions").begin_object();
        for (id, v) in &self.facts_versions {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:182 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
            out.name(&internal_to_unicode(&id.to_lockfile_form()))
                .value_number(&v.to_string());
        }
        out.end_object();

        out.end_object();
        out.into_string()
    }

    /// The text `BazelLockFileModule.updateLockfile` writes: the JSON plus a newline. It is
    /// Unicode (the adapters convert Bazel's internal strings); the file holds its UTF-8 bytes
    /// (`OutputStreamWriter(outputStream, UTF_8)`), i.e. `as_bytes()`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:330 BazelLockFileModule.updateLockfile
    pub fn to_lockfile_contents(&self) -> String {
        let mut s = self.to_json();
        s.push('\n');
        s
    }

    /// `LOCKFILE_GSON.fromJson(json, BazelLockFileValue.class)`. `json` is Unicode text (the
    /// file decoded as UTF-8); the adapters convert its strings to internal strings. `Ok(None)`
    /// for an empty document (Gson returns null).
    pub fn from_json(json: &str) -> Result<Option<BazelLockFileValue>, LockfileParseError> {
        let tree = match parse_lenient(json) {
            Ok(Some(t)) => t,
            Ok(None) => return Ok(None),
            Err(e) => {
                return err(format!(
                    "{}\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json",
                    e.0
                ));
            }
        };
        if tree == JsonValue::Null {
            return Ok(None);
        }
        let members = expect_object(&tree, "$")?;
        let mut value = BazelLockFileValue::empty_lockfile();
        for (name, v) in members {
            let path = format!("$.{name}");
            let is_null = *v == JsonValue::Null;
            match name.as_str() {
                "lockFileVersion" => {
                    if is_null {
                        return err("Null lockFileVersion");
                    }
                    value.lock_file_version = expect_int(v, &path)?;
                }
                "registryFileHashes" => {
                    if is_null {
                        return err("Null registryFileHashes");
                    }
                    value.registry_file_hashes = read_map(
                        v,
                        &path,
                        |k| Ok(string_adapter_read(k)),
                        // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:386 GsonTypeAdapterUtil.OptionalChecksumTypeAdapterFactory.OptionalChecksumTypeAdapter.read
                        |c, p| {
                            let s = expect_string(c, p)?;
                            if s == "not found" {
                                return Ok(None);
                            }
                            match Checksum::from_string(KeyType::Sha256, s) {
                                Ok(c) => Ok(Some(c)),
                                Err(_) => err(format!("Invalid checksum: {s}")),
                            }
                        },
                    )?;
                }
                "selectedYankedVersions" => {
                    if is_null {
                        return err("Null selectedYankedVersions");
                    }
                    value.selected_yanked_versions = read_map(
                        v,
                        &path,
                        |k| {
                            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:107 GsonTypeAdapterUtil.MODULE_KEY_TYPE_ADAPTER
                            let json_string = unicode_to_internal(k);
                            ModuleKey::from_string(&json_string).or_else(|_| {
                                err(format!(
                                    "Unable to parse ModuleKey {json_string} version from the lockfile"
                                ))
                            })
                        },
                        |s, p| Ok(string_adapter_read(expect_string(s, p)?)),
                    )?;
                }
                "moduleExtensions" => {
                    if is_null {
                        // isValidLockfile treats a null map as an invalid lockfile.
                        return err("Null moduleExtensions");
                    }
                    value.module_extensions =
                        read_map(v, &path, read_module_extension_id, |per_factors, p| {
                            read_map(
                                per_factors,
                                p,
                                // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:233 GsonTypeAdapterUtil.MODULE_EXTENSION_FACTORS_TYPE_ADAPTER
                                |k| Ok(ModuleExtensionEvalFactors::parse(&unicode_to_internal(k))),
                                read_lock_file_module_extension,
                            )
                        })?;
                }
                "facts" => {
                    if is_null {
                        return err("Null facts");
                    }
                    value.facts = read_map(v, &path, read_module_extension_id, |f, _| {
                        FactsAdapter::read(f).map_err(LockfileParseError)
                    })?;
                }
                "factsVersions" => {
                    if is_null {
                        return err("Null factsVersions");
                    }
                    value.facts_versions =
                        read_map(v, &path, read_module_extension_id, expect_int)?;
                }
                // Unknown names are skipped.
                _ => {}
            }
        }
        Ok(Some(value))
    }
}

/// `MODULE_EXTENSION_ID_TYPE_ADAPTER.read` of a map key.
// Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:191 GsonTypeAdapterUtil.MODULE_EXTENSION_ID_TYPE_ADAPTER
fn read_module_extension_id(k: &str) -> Result<ModuleExtensionId, LockfileParseError> {
    ModuleExtensionId::parse_lockfile_form(&unicode_to_internal(k)).map_err(LockfileParseError)
}

// Generated adapter of LockFileModuleExtension (auto-value-gson; property order).
fn write_lock_file_module_extension(out: &mut JsonWriter, ext: &LockFileModuleExtension) {
    out.begin_object();
    out.name("bzlTransitiveDigest")
        .value_str(&base64_encode(&ext.bzl_transitive_digest));
    out.name("usagesDigest")
        .value_str(&base64_encode(&ext.usages_digest));
    out.name("recordedInputs").begin_array();
    for input in &ext.recorded_inputs {
        // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:339 GsonTypeAdapterUtil.REPO_RECORDED_INPUT_WITH_VALUE_TYPE_ADAPTER
        out.value_str(&internal_to_unicode(&input.0));
    }
    out.end_array();
    out.name("generatedRepoSpecs").begin_object();
    for (name, spec) in &ext.generated_repo_specs {
        // Map key: STRING_ADAPTER.
        out.name(&internal_to_unicode(name));
        write_repo_spec(out, spec);
    }
    out.end_object();
    out.name("moduleExtensionMetadata");
    match &ext.module_extension_metadata {
        // Optional.empty with serializeNulls false: the name is dropped.
        // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:306 GsonTypeAdapterUtil.OptionalTypeAdapter.write
        None => {
            out.null_value();
        }
        Some(m) => {
            out.begin_object();
            out.name("explicitRootModuleDirectDeps");
            write_nullable_string_list(out, m.explicit_root_module_direct_deps.as_deref());
            out.name("explicitRootModuleDirectDevDeps");
            write_nullable_string_list(out, m.explicit_root_module_direct_dev_deps.as_deref());
            out.name("useAllRepos").value_str(m.use_all_repos.name());
            out.name("reproducible").value_bool(m.reproducible);
            out.end_object();
        }
    }
    out.end_object();
}

fn write_nullable_string_list(out: &mut JsonWriter, list: Option<&[String]>) {
    match list {
        None => {
            out.null_value();
        }
        Some(items) => {
            out.begin_array();
            for i in items {
                string_adapter_write(out, i);
            }
            out.end_array();
        }
    }
}

// Gson adapter of the RepoSpec record.
fn write_repo_spec(out: &mut JsonWriter, spec: &RepoSpec) {
    out.begin_object();
    out.name("repoRuleId");
    match spec.repo_rule_id() {
        None => {
            out.null_value();
        }
        Some(id) => {
            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:143 GsonTypeAdapterUtil.REPO_RULE_ID_TYPE_ADAPTER
            out.value_str(&internal_to_unicode(&id.to_string()));
        }
    }
    out.name("attributes");
    AttributeValuesAdapter::write(out, spec.attributes());
    out.end_object();
}

fn read_digest(v: &JsonValue, path: &str) -> Result<Vec<u8>, LockfileParseError> {
    let s = expect_string(v, path)?;
    base64_decode(s).map_err(LockfileParseError)
}

fn read_nullable_string_list(
    v: &JsonValue,
    path: &str,
) -> Result<Option<Vec<String>>, LockfileParseError> {
    match v {
        JsonValue::Null => Ok(None),
        JsonValue::Array(items) => {
            let mut out: Vec<String> = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let s = string_adapter_read(expect_string(item, &format!("{path}[{i}]"))?);
                // ImmutableSet: duplicates collapse.
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            Ok(Some(out))
        }
        other => err(format!(
            "java.lang.IllegalStateException: Expected BEGIN_ARRAY but was {} at path {path}",
            other.token_name()
        )),
    }
}

fn read_lock_file_module_extension(
    v: &JsonValue,
    path: &str,
) -> Result<LockFileModuleExtension, LockfileParseError> {
    let members = expect_object(v, path)?;
    let mut bzl_transitive_digest = None;
    let mut usages_digest = None;
    let mut recorded_inputs = None;
    let mut generated_repo_specs = None;
    let mut module_extension_metadata = None;
    for (name, val) in members {
        let p = format!("{path}.{name}");
        match name.as_str() {
            "bzlTransitiveDigest" => {
                if *val == JsonValue::Null {
                    return err("Null bzlTransitiveDigest");
                }
                bzl_transitive_digest = Some(read_digest(val, &p)?);
            }
            "usagesDigest" => {
                if *val == JsonValue::Null {
                    return err("Null usagesDigest");
                }
                usages_digest = Some(read_digest(val, &p)?);
            }
            "recordedInputs" => {
                if *val == JsonValue::Null {
                    return err("Null recordedInputs");
                }
                let JsonValue::Array(items) = val else {
                    return err(format!(
                        "java.lang.IllegalStateException: Expected BEGIN_ARRAY but was {} at path {p}",
                        val.token_name()
                    ));
                };
                recorded_inputs = Some(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:344 GsonTypeAdapterUtil.REPO_RECORDED_INPUT_WITH_VALUE_TYPE_ADAPTER
                            Ok(RepoRecordedInputWithValue(string_adapter_read(
                                expect_string(s, &format!("{p}[{i}]"))?,
                            )))
                        })
                        .collect::<Result<Vec<_>, LockfileParseError>>()?,
                );
            }
            "generatedRepoSpecs" => {
                if *val == JsonValue::Null {
                    // isValidLockfile: a null map makes the lockfile invalid.
                    return err("Null generatedRepoSpecs");
                }
                generated_repo_specs = Some(read_map(
                    val,
                    &p,
                    |k| Ok(string_adapter_read(k)),
                    read_repo_spec,
                )?);
            }
            "moduleExtensionMetadata" => {
                // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:322 GsonTypeAdapterUtil.OptionalTypeAdapter.read
                module_extension_metadata = match val {
                    JsonValue::Null => None,
                    other => Some(read_metadata(other, &p)?),
                };
            }
            _ => {}
        }
    }
    let mut missing = Vec::new();
    if bzl_transitive_digest.is_none() {
        missing.push("bzlTransitiveDigest");
    }
    if usages_digest.is_none() {
        missing.push("usagesDigest");
    }
    if recorded_inputs.is_none() {
        missing.push("recordedInputs");
    }
    if generated_repo_specs.is_none() {
        missing.push("generatedRepoSpecs");
    }
    if !missing.is_empty() {
        // PORT: AutoValue's builder throws IllegalStateException here, which
        // BazelLockFileFunction does not catch (a crash in Java); reported as a parse error.
        return err(format!(
            "Missing required properties: {}",
            missing.join(" ")
        ));
    }
    Ok(LockFileModuleExtension {
        bzl_transitive_digest: bzl_transitive_digest.unwrap(),
        usages_digest: usages_digest.unwrap(),
        recorded_inputs: recorded_inputs.unwrap(),
        generated_repo_specs: generated_repo_specs.unwrap(),
        module_extension_metadata,
    })
}

fn read_metadata(
    v: &JsonValue,
    path: &str,
) -> Result<LockfileModuleExtensionMetadata, LockfileParseError> {
    let members = expect_object(v, path)?;
    let mut deps = None;
    let mut dev_deps = None;
    let mut use_all_repos = None;
    let mut reproducible = None;
    for (name, val) in members {
        let p = format!("{path}.{name}");
        match name.as_str() {
            "explicitRootModuleDirectDeps" => deps = read_nullable_string_list(val, &p)?,
            "explicitRootModuleDirectDevDeps" => dev_deps = read_nullable_string_list(val, &p)?,
            "useAllRepos" => {
                if *val == JsonValue::Null {
                    return err("Null useAllRepos");
                }
                // Gson's enum adapter maps unknown names to null.
                let s = expect_string(val, &p)?;
                match UseAllRepos::value_of(s) {
                    Some(u) => use_all_repos = Some(u),
                    None => return err("Null useAllRepos"),
                }
            }
            "reproducible" => reproducible = Some(expect_bool(val, &p)?),
            _ => {}
        }
    }
    let Some(use_all_repos) = use_all_repos else {
        return err("Missing required properties: useAllRepos");
    };
    let Some(reproducible) = reproducible else {
        return err("Missing required properties: reproducible");
    };
    Ok(LockfileModuleExtensionMetadata {
        explicit_root_module_direct_deps: deps,
        explicit_root_module_direct_dev_deps: dev_deps,
        use_all_repos,
        reproducible,
    })
}

// Gson record adapter of RepoSpec with REPO_RULE_ID_TYPE_ADAPTER and AttributeValuesAdapter.
fn read_repo_spec(v: &JsonValue, path: &str) -> Result<RepoSpec, LockfileParseError> {
    let members = expect_object(v, path)?;
    let mut repo_rule_id = None;
    let mut attributes = None;
    for (name, val) in members {
        let p = format!("{path}.{name}");
        match name.as_str() {
            "repoRuleId" => {
                repo_rule_id = match val {
                    JsonValue::Null => None,
                    other => {
                        // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:148 GsonTypeAdapterUtil.REPO_RULE_ID_TYPE_ADAPTER
                        let s = string_adapter_read(expect_string(other, &p)?);
                        let s = s.as_str();
                        Some(match s.find('%') {
                            None => RepoRuleId::new_nullable(None, s),
                            Some(percent) => RepoRuleId::new_nullable(
                                Some(Label::parse_canonical_unchecked(&s[..percent])),
                                &s[percent + 1..],
                            ),
                        })
                    }
                }
            }
            "attributes" => {
                attributes = match val {
                    JsonValue::Null => None,
                    other => Some(AttributeValuesAdapter::read(other).map_err(LockfileParseError)?),
                }
            }
            _ => {}
        }
    }
    let Some(attributes) = attributes else {
        // PORT: Java keeps a null AttributeValues and fails later; reported here instead.
        return err(format!("Null attributes at path {path}"));
    };
    Ok(RepoSpec::new_nullable(repo_rule_id, attributes))
}

/// The reading part of `BazelLockFileFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:52 BazelLockFileFunction
pub struct BazelLockFileFunction;

impl BazelLockFileFunction {
    /// `compute` given the lockfile's contents (`Ok(None)`: the file does not exist).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:69 BazelLockFileFunction.compute
    pub fn compute(
        contents: Result<Option<String>, std::io::Error>,
        for_hidden_lockfile: bool,
        lockfile_mode: LockfileMode,
    ) -> Result<BazelLockFileValue, ExternalDepsException> {
        if lockfile_mode == LockfileMode::Off {
            return Ok(BazelLockFileValue::empty_lockfile());
        }
        let mode = if for_hidden_lockfile {
            LockfileMode::Update
        } else {
            lockfile_mode
        };
        let error_message: String = match contents {
            Err(e) => e.to_string(),
            Ok(contents) => match Self::get_lockfile_value(contents.as_deref(), mode) {
                Ok(Ok(v)) => return Ok(v),
                // A BazelLockfileFunctionException is thrown as is.
                Ok(Err(e)) => return Err(e),
                Err(parse_error) => parse_error.0,
            },
        };
        if for_hidden_lockfile {
            return Ok(BazelLockFileValue::empty_lockfile());
        }
        let action_suffix = if POSSIBLE_MERGE_CONFLICT_PATTERN.is_match(&error_message) {
            " This looks like a merge conflict. See \
             https://bazel.build/external/lockfile#merge-conflicts for advice."
        } else {
            " Try deleting it and rerun the build."
        };
        Err(ExternalDepsException::with_message(
            ExternalDepsCode::BadLockfile,
            format!(
                "Failed to read and parse the MODULE.bazel.lock file with error: \
                 {error_message}.{action_suffix}"
            ),
        ))
    }

    /// Parses lockfile contents. The outer error is a parse failure (caught by `compute`); the
    /// inner error is a `BazelLockfileFunctionException`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:124 BazelLockFileFunction.getLockfileValue
    #[allow(clippy::type_complexity)]
    pub fn get_lockfile_value(
        contents: Option<&str>,
        lockfile_mode: LockfileMode,
    ) -> Result<Result<BazelLockFileValue, ExternalDepsException>, LockfileParseError> {
        let Some(json) = contents else {
            // FileNotFoundException
            return Ok(Ok(BazelLockFileValue::empty_lockfile()));
        };
        // int version = matcher.find() ? Integer.parseInt(matcher.group(1)) : -1;
        let version = match LOCKFILE_VERSION_PATTERN.captures(json) {
            Some(c) => match c[1].parse::<i32>() {
                Ok(version) => version,
                // Integer.parseInt throws NumberFormatException (an IllegalArgumentException,
                // which BazelLockFileFunction.compute catches) for digits beyond the int range.
                // Java: java.base/java/lang/NumberFormatException.java NumberFormatException.forInputString
                Err(_) => return err(format!("For input string: \"{}\"", &c[1])),
            },
            None => -1,
        };
        let unsupported = || {
            ExternalDepsException::with_message(
                ExternalDepsCode::BadLockfile,
                "The version of MODULE.bazel.lock is not supported by this version of Bazel. \
                 Please run `bazel mod deps --lockfile_mode=update` to update your lockfile.",
            )
        };
        if version == LOCK_FILE_VERSION {
            let lock_file_value = match BazelLockFileValue::from_json(json) {
                Ok(v) => v,
                // isValidLockfile treats null maps (reported as errors above) as invalid.
                Err(e) if e.0 == "Null moduleExtensions" || e.0 == "Null generatedRepoSpecs" => {
                    None
                }
                Err(e) => return Err(e),
            };
            if !Self::is_valid_lockfile(lock_file_value.as_ref()) {
                if lockfile_mode == LockfileMode::Error {
                    return Ok(Err(unsupported()));
                }
                return Ok(Ok(BazelLockFileValue::empty_lockfile()));
            }
            Ok(Ok(lock_file_value.unwrap()))
        } else {
            // This is an old version, its information can't be used.
            if lockfile_mode == LockfileMode::Error {
                return Ok(Err(unsupported()));
            }
            Ok(Ok(BazelLockFileValue::empty_lockfile()))
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:167 BazelLockFileFunction.isValidLockfile
    fn is_valid_lockfile(lock_file_value: Option<&BazelLockFileValue>) -> bool {
        let Some(v) = lock_file_value else {
            return false;
        };
        v.module_extensions.values().all(|per_factors| {
            per_factors.values().all(|ext| {
                ext.generated_repo_specs
                    .values()
                    .all(|s| s.repo_rule_id().is_some())
            })
        })
    }
}

/// The lockfile-combination logic of `BazelLockFileModule`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:47 BazelLockFileModule
pub struct BazelLockFileModule;

type ExtensionsMap =
    IndexMap<ModuleExtensionId, IndexMap<ModuleExtensionEvalFactors, LockFileModuleExtension>>;

impl BazelLockFileModule {
    /// Combines the old extensions stored in the lockfile (if they are still used and have the
    /// same dependence on os/arch) with the newly evaluated ones; sorted by extension id.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:256 BazelLockFileModule.combineModuleExtensions
    pub fn combine_module_extensions(
        old_extension_infos: &ExtensionsMap,
        new_extension_infos: &IndexMap<ModuleExtensionId, LockFileModuleExtensionWithFactors>,
        has_usages: impl Fn(&ModuleExtensionId) -> bool,
        reproducible: bool,
    ) -> ExtensionsMap {
        let mut updated: ExtensionsMap = IndexMap::new();
        // Keep the per-factor results that are still used per the static information in the
        // extension declaration (dependence on os and arch).
        for (module_extension_id, per_factors) in old_extension_infos {
            if !has_usages(module_extension_id) {
                // Extensions without any usages are not needed anymore.
                continue;
            }
            let Some(new_extension_info) = new_extension_infos.get(module_extension_id) else {
                // No information based on which we could invalidate old entries, keep all.
                updated.insert(module_extension_id.clone(), per_factors.clone());
                continue;
            };
            let new_factors = &new_extension_info.extension_factors;
            // Prefer the new result for its particular set of factors.
            let mut to_keep: Vec<(ModuleExtensionEvalFactors, LockFileModuleExtension)> =
                per_factors
                    .iter()
                    .filter(|(old, _)| {
                        old.has_same_dependencies_as(new_factors) && *old != new_factors
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
            if to_keep.is_empty() {
                continue;
            }
            to_keep.sort_by(|a, b| a.0.cmp(&b.0));
            updated.insert(module_extension_id.clone(), to_keep.into_iter().collect());
        }
        // Add the new resolved extensions.
        for (id, info) in new_extension_infos {
            let extension = &info.module_extension;
            if extension.is_reproducible() != reproducible {
                continue;
            }
            let factors = &info.extension_factors;
            let entries = match updated.get(id) {
                Some(old) => {
                    // buildKeepingLast, then ImmutableSortedMap.
                    let mut merged: Vec<(ModuleExtensionEvalFactors, LockFileModuleExtension)> =
                        old.iter()
                            .filter(|(k, _)| *k != factors)
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                    merged.push((factors.clone(), extension.clone()));
                    merged.sort_by(|a, b| a.0.cmp(&b.0));
                    merged.into_iter().collect()
                }
                None => IndexMap::from([(factors.clone(), extension.clone())]),
            };
            updated.insert(id.clone(), entries);
        }
        // Skyframe evaluation order is non-deterministic; sort for a deterministic lockfile.
        updated.sort_by(|a, _, b, _| ModuleExtensionId::lexicographic_cmp(a, b));
        updated
    }

    /// Writes the lockfile to `lockfile_root/MODULE.bazel.lock`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:330 BazelLockFileModule.updateLockfile
    pub fn update_lockfile(
        lockfile_root: &std::path::Path,
        updated_lockfile: &BazelLockFileValue,
    ) -> std::io::Result<()> {
        // PORT: Java logs IO errors at SEVERE and continues; the caller decides here.
        // `lockfile_root` is a platform path; the contents are written as UTF-8
        // (`new OutputStreamWriter(outputStream, UTF_8)`).
        std::fs::write(
            lockfile_root.join("MODULE.bazel.lock"),
            updated_lockfile.to_lockfile_contents(),
        )
    }

    /// `filterRelevantFacts`: facts of extensions that still have usages and are non-empty,
    /// sorted by extension id.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:225 BazelLockFileModule.filterRelevantFacts
    pub fn filter_relevant_facts(
        combined_facts: &IndexMap<ModuleExtensionId, Facts>,
        has_usages: impl Fn(&ModuleExtensionId) -> bool,
    ) -> IndexMap<ModuleExtensionId, Facts> {
        let mut out: IndexMap<ModuleExtensionId, Facts> = combined_facts
            .iter()
            .filter(|(id, f)| has_usages(id) && **f != Facts::empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.sort_by(|a, _, b, _| ModuleExtensionId::lexicographic_cmp(a, b));
        out
    }

    /// Only keeps non-zero versions for extensions that have facts persisted.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:238 BazelLockFileModule.filterRelevantFactsVersions
    pub fn filter_relevant_facts_versions(
        combined_facts_versions: &IndexMap<ModuleExtensionId, i32>,
        relevant_facts: &IndexMap<ModuleExtensionId, Facts>,
    ) -> IndexMap<ModuleExtensionId, i32> {
        let mut out: IndexMap<ModuleExtensionId, i32> = combined_facts_versions
            .iter()
            .filter(|(id, v)| relevant_facts.contains_key(*id) && **v != 0)
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        out.sort_by(|a, _, b, _| ModuleExtensionId::lexicographic_cmp(a, b));
        out
    }

    /// The registry file hashes that go into the lockfile: local (`file:`) registries are
    /// excluded; sorted by URL (`ImmutableSortedMap`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:157 BazelLockFileModule.afterCommand
    pub fn remote_registry_file_hashes(
        registry_file_hashes: &IndexMap<String, Option<Checksum>>,
    ) -> IndexMap<String, Option<Checksum>> {
        let mut out: IndexMap<String, Option<Checksum>> = registry_file_hashes
            .iter()
            .filter(|(url, _)| !url.starts_with("file:"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.sort_by(|a, _, b, _| java_string_compare(a, b));
        out
    }
}
