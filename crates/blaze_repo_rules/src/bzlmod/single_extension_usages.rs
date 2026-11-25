//! SINGLE_EXTENSION_USAGES: the usages of one module extension, taken out of the dependency
//! graph so that extension evaluation only reruns when they change, and their evaluation digest
//! (the lockfile's `usagesDigest`).
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java,
//! SingleExtensionUsagesFunction.java, and the parts of GsonTypeAdapterUtil
//! (`SINGLE_EXTENSION_USAGES_VALUE_GSON`) that serialize the trimmed value.

use std::fmt;

use blaze_bzlmod::skyframe::keys::BazelDepGraphKey;
use blaze_bzlmod_model::AbridgedModule;
use blaze_bzlmod_model::AttributeValuesAdapter;
use blaze_bzlmod_model::ModuleExtensionId;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::ModuleExtensionUsageProxy;
use blaze_bzlmod_model::ModuleKey;
use blaze_bzlmod_model::RepoOverride;
use blaze_bzlmod_model::Tag;
use blaze_bzlmod_model::gson::JsonWriter;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_events::Location;
use blaze_skyframe::Environment;
use blaze_skyframe::Key;
use blaze_skyframe::SkyFunctionName;
use blaze_skyframe::{ComputeError, FunctionHermeticity};
use blaze_util::string_encoding::internal_to_unicode;
use indexmap::IndexMap;
use sha2::Digest;
use sha2::Sha256;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:163 SkyFunctions.SINGLE_EXTENSION_USAGES
pub const SINGLE_EXTENSION_USAGES: SkyFunctionName = SkyFunctionName("SINGLE_EXTENSION_USAGES");

/// The result of [`compute`].
///
/// When adding or exposing new fields to extensions, make sure to update
/// [`SingleExtensionUsagesValue::trim_for_evaluation`] as well.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:43 SingleExtensionUsagesValue
#[derive(Debug, Clone, PartialEq)]
pub struct SingleExtensionUsagesValue {
    /// All usages of this extension, by the key of the module where the usage occurs.
    // Note: Equality of SingleExtensionUsagesValue does not check for equality of the order of the
    // entries of this map, but it is tracked implicitly via the order of the abridged modules.
    pub extension_usages: IndexMap<ModuleKey, ModuleExtensionUsage>,
    /// The "unique name" (see `BazelDepGraphValue::extension_unique_names`) of this extension.
    pub extension_unique_name: String,
    /// All `AbridgedModule`s in the dependency graph that used this extension.
    pub abridged_modules: Vec<AbridgedModule>,
    /// The repo mappings to use for each module that used this extension.
    pub repo_mappings: IndexMap<ModuleKey, RepositoryMapping>,
    /// Maps an extension-local repo name to the canonical name of the repo it is overridden with.
    pub repo_overrides: IndexMap<String, RepositoryName>,
}

impl SingleExtensionUsagesValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:63 SingleExtensionUsagesValue.create
    pub fn create(
        extension_usages: IndexMap<ModuleKey, ModuleExtensionUsage>,
        extension_unique_name: String,
        abridged_modules: Vec<AbridgedModule>,
        repo_mappings: IndexMap<ModuleKey, RepositoryMapping>,
        repo_overrides: IndexMap<String, RepositoryName>,
    ) -> SingleExtensionUsagesValue {
        SingleExtensionUsagesValue {
            extension_usages,
            extension_unique_name,
            abridged_modules,
            repo_mappings,
            repo_overrides,
        }
    }

    pub fn get_extension_usages(&self) -> &IndexMap<ModuleKey, ModuleExtensionUsage> {
        &self.extension_usages
    }

    pub fn get_extension_unique_name(&self) -> &str {
        &self.extension_unique_name
    }

    pub fn get_abridged_modules(&self) -> &[AbridgedModule] {
        &self.abridged_modules
    }

    pub fn get_repo_mappings(&self) -> &IndexMap<ModuleKey, RepositoryMapping> {
        &self.repo_mappings
    }

    pub fn get_repo_overrides(&self) -> &IndexMap<String, RepositoryName> {
        &self.repo_overrides
    }

    /// Turns the given usages value for a particular extension into a hash that can be compared
    /// for equality with another hash obtained in this way and compares equal only if the two
    /// values are equivalent for the purpose of evaluating the extension.
    ///
    /// Java: `Hashing.sha256().hashUnencodedChars(gson.toJson(usagesValue.trimForEvaluation()))`
    /// with `GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON`; `hashUnencodedChars` hashes
    /// the UTF-16 code units of the JSON, little-endian.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:78 SingleExtensionUsagesValue.hashForEvaluation
    pub fn hash_for_evaluation(usages_value: &SingleExtensionUsagesValue) -> Vec<u8> {
        let json = Self::to_json(&usages_value.trim_for_evaluation());
        let mut hasher = Sha256::new();
        for unit in json.encode_utf16() {
            hasher.update(unit.to_le_bytes());
        }
        hasher.finalize().to_vec()
    }

    /// `SINGLE_EXTENSION_USAGES_VALUE_GSON.toJson(value)`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/GsonTypeAdapterUtil.java:408 GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON
    pub fn to_json(value: &SingleExtensionUsagesValue) -> String {
        let mut out = JsonWriter::new(false);
        write_single_extension_usages_value(&mut out, value);
        out.into_string()
    }

    /// Returns a new value with only the information that influences the evaluation of the
    /// extension and isn't tracked elsewhere.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:88 SingleExtensionUsagesValue.trimForEvaluation
    pub fn trim_for_evaluation(&self) -> SingleExtensionUsagesValue {
        SingleExtensionUsagesValue::create(
            self.extension_usages
                .iter()
                .map(|(k, v)| (k.clone(), v.trim_for_evaluation()))
                .collect(),
            self.extension_unique_name.clone(),
            self.abridged_modules.clone(),
            // repoMappings: The usage of repo mappings by the extension's implementation function
            // is tracked on the level of individual entries and all label attributes are provided
            // as `Label`, which exclusively reference canonical repository names.
            IndexMap::new(),
            self.repo_overrides.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:101 SingleExtensionUsagesValue.key
    pub fn key(id: ModuleExtensionId) -> SingleExtensionUsagesKey {
        SingleExtensionUsagesKey(id)
    }
}

/// The `SkyKey` of a [`SingleExtensionUsagesValue`]. `Debug` is
/// `SINGLE_EXTENSION_USAGES:<id>` (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:106 SingleExtensionUsagesValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SingleExtensionUsagesKey(pub ModuleExtensionId);

impl SingleExtensionUsagesKey {
    pub fn key(id: ModuleExtensionId) -> SingleExtensionUsagesKey {
        SingleExtensionUsagesKey(id)
    }

    pub fn argument(&self) -> &ModuleExtensionId {
        &self.0
    }
}

impl fmt::Debug for SingleExtensionUsagesKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", SINGLE_EXTENSION_USAGES, self.0)
    }
}

impl Key for SingleExtensionUsagesKey {
    type Value = SingleExtensionUsagesValue;
    const FUNCTION: SkyFunctionName = SINGLE_EXTENSION_USAGES;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:164 SkyFunctions.SINGLE_EXTENSION_USAGES (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: an AutoValue of the usages, unique name, abridged modules, repo mappings and the main repo mapping.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesValue.java:43 SingleExtensionUsagesValue
    fn values_equal(old: &SingleExtensionUsagesValue, new: &SingleExtensionUsagesValue) -> bool {
        old == new
    }
}

/// A simple SkyFunction that takes the information needed by `SingleExtensionEvalFunction` out
/// of `BazelDepGraphValue` and stores it in a SkyValue.
///
/// The whole raison d'être of this function is to avoid unnecessary reruns of module
/// extensions: by storing the input needed by a single `SingleExtensionEvalFunction`, Skyframe's
/// change pruning makes sure that only the module extension whose input data actually changed
/// reruns.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionUsagesFunction.java:44 SingleExtensionUsagesFunction.compute
pub fn compute(
    key: &SingleExtensionUsagesKey,
    env: &mut Environment<'_>,
) -> Result<Option<SingleExtensionUsagesValue>, ComputeError> {
    let Some(bazel_dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };

    let id = key.argument();
    // We never request an extension without usages in Skyframe.
    let usages_table = &bazel_dep_graph_value.extension_usages_table;
    let row = usages_table.row(id).cloned().unwrap_or_default();
    let extension_unique_name = bazel_dep_graph_value
        .extension_unique_names
        .get(id)
        .unwrap_or_else(|| panic!("NullPointerException: no unique name for extension {id}"))
        .clone();
    // Filter abridged modules down to only those that actually used this extension.
    let abridged_modules = bazel_dep_graph_value
        .abridged_modules
        .iter()
        .filter(|module| usages_table.get(id, &module.key).is_some())
        .cloned()
        .collect();
    // TODO(wyv): Maybe cache these mappings?
    let repo_mappings = row
        .keys()
        .map(|key| {
            (
                key.clone(),
                bazel_dep_graph_value.get_full_repo_mapping(key),
            )
        })
        .collect();
    let repo_overrides = bazel_dep_graph_value
        .repo_overrides
        .row(id)
        .cloned()
        .unwrap_or_default();
    Ok(Some(SingleExtensionUsagesValue::create(
        row,
        extension_unique_name,
        abridged_modules,
        repo_mappings,
        repo_overrides,
    )))
}

// ---------------------------------------------------------------------------------------------
// SINGLE_EXTENSION_USAGES_VALUE_GSON: Gson without pretty printing, HTML escaping disabled,
// `serializeNulls` false, complex map key serialization, the auto-value-gson adapters
// (properties in declaration order, `get`/`is` prefixes stripped) and Bazel's type adapters.
// Values without an adapter (`Location`) use Gson's reflective adapter (fields in declaration
// order).

// Strings are written through GsonTypeAdapterUtil.STRING_ADAPTER and the other string adapters
// (MODULE_KEY, ISOLATION_KEY), which write `internalToUnicode(s)` (REPOSITORY_NAME and VERSION
// write theirs unconverted); map keys go through the same adapters (complex map key serialization).

/// The generated adapter of `@AutoValue @GenerateTypeAdapter SingleExtensionUsagesValue`.
fn write_single_extension_usages_value(out: &mut JsonWriter, value: &SingleExtensionUsagesValue) {
    out.begin_object();
    out.name("extensionUsages");
    // ImmutableMap<ModuleKey, _> through MapTypeAdapterFactory with complex map key
    // serialization: MODULE_KEY_TYPE_ADAPTER writes a string, so the map is a JSON object.
    out.begin_object();
    for (key, usage) in &value.extension_usages {
        out.name(&internal_to_unicode(&key.to_string()));
        write_module_extension_usage(out, usage);
    }
    out.end_object();
    out.name("extensionUniqueName");
    out.value_str(&internal_to_unicode(&value.extension_unique_name));
    out.name("abridgedModules");
    out.begin_array();
    for module in &value.abridged_modules {
        write_abridged_module(out, module);
    }
    out.end_array();
    out.name("repoMappings");
    out.begin_object();
    if let Some((key, _mapping)) = value.repo_mappings.iter().next() {
        out.name(&internal_to_unicode(&key.to_string()));
        // PORT: RepositoryMapping has no type adapter (Gson's reflective adapter would apply);
        // only trimmed values, whose repo mappings are empty, are serialized.
        blaze_util::unported!(
            "GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON",
            "reflective serialization of RepositoryMapping"
        );
    }
    out.end_object();
    out.name("repoOverrides");
    out.begin_object();
    for (name, repo) in &value.repo_overrides {
        out.name(&internal_to_unicode(name));
        // REPOSITORY_NAME_TYPE_ADAPTER
        out.value_str(repo.get_name());
    }
    out.end_object();
    out.end_object();
}

/// The generated adapter of `ModuleExtensionUsage`.
fn write_module_extension_usage(out: &mut JsonWriter, usage: &ModuleExtensionUsage) {
    out.begin_object();
    out.name("extensionBzlFile");
    out.value_str(&internal_to_unicode(&usage.extension_bzl_file));
    out.name("extensionName");
    out.value_str(&internal_to_unicode(&usage.extension_name));
    out.name("isolationKey");
    match &usage.isolation_key {
        // ISOLATION_KEY_TYPE_ADAPTER
        Some(key) => {
            out.value_str(&internal_to_unicode(&key.to_string()));
        }
        // OPTIONAL: an empty Optional of a non-String type writes a null with serializeNulls
        // false, which drops the name.
        None => {
            let old = out.get_serialize_nulls();
            out.set_serialize_nulls(false);
            out.null_value();
            out.set_serialize_nulls(old);
        }
    }
    out.name("proxies");
    out.begin_array();
    for proxy in &usage.proxies {
        write_proxy(out, proxy);
    }
    out.end_array();
    out.name("tags");
    out.begin_array();
    for tag in &usage.tags {
        write_tag(out, tag);
    }
    out.end_array();
    out.name("repoOverrides");
    out.begin_object();
    for (name, repo_override) in &usage.repo_overrides {
        out.name(&internal_to_unicode(name));
        write_repo_override(out, repo_override);
    }
    out.end_object();
    out.end_object();
}

/// The generated adapter of `ModuleExtensionUsage.Proxy`.
fn write_proxy(_out: &mut JsonWriter, _proxy: &ModuleExtensionUsageProxy) {
    // PORT: `containingModuleFilePath` (a PathFragment) has no type adapter (Gson's reflective
    // adapter would apply); trimmed usages have no proxies.
    blaze_util::unported!(
        "GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON",
        "serialization of ModuleExtensionUsage.Proxy"
    );
}

/// The generated adapter of the record `ModuleExtensionUsage.RepoOverride`.
fn write_repo_override(out: &mut JsonWriter, repo_override: &RepoOverride) {
    out.begin_object();
    out.name("overridingRepoName");
    out.value_str(&internal_to_unicode(&repo_override.overriding_repo_name));
    out.name("mustExist");
    out.value_bool(repo_override.must_exist);
    out.name("location");
    write_location(out, &repo_override.location);
    out.end_object();
}

/// The generated adapter of `Tag`.
fn write_tag(out: &mut JsonWriter, tag: &Tag) {
    out.begin_object();
    out.name("tagName");
    out.value_str(&internal_to_unicode(&tag.tag_name));
    out.name("attributeValues");
    AttributeValuesAdapter::write(out, &tag.attribute_values);
    out.name("devDependency");
    out.value_bool(tag.dev_dependency);
    out.name("location");
    write_location(out, &tag.location);
    out.end_object();
}

/// The generated adapter of `AbridgedModule` (VERSION_TYPE_ADAPTER and MODULE_KEY_TYPE_ADAPTER
/// write `toString()`).
fn write_abridged_module(out: &mut JsonWriter, module: &AbridgedModule) {
    out.begin_object();
    out.name("name");
    out.value_str(&internal_to_unicode(&module.name));
    out.name("version");
    // VERSION_TYPE_ADAPTER writes `version.toString()` unconverted.
    out.value_str(&module.version.to_string());
    out.name("key");
    out.value_str(&internal_to_unicode(&module.key.to_string()));
    out.end_object();
}

/// Gson's reflective adapter for `net.starlark.java.syntax.Location` (fields `file`, `line`,
/// `column`).
fn write_location(out: &mut JsonWriter, location: &Location) {
    out.begin_object();
    out.name("file");
    out.value_str(&internal_to_unicode(location.file()));
    out.name("line");
    out.value_number(&location.line().to_string());
    out.name("column");
    out.value_number(&location.column().to_string());
    out.end_object();
}
