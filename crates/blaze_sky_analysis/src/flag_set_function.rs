//! Ports of `skyframe/config/FlagSetFunction.java`, `skyframe/config/FlagSetValue.java` and
//! `runtime/ConfigFlagDefinitions.java`: the flags a project file's buildable unit (selected by
//! `--scl_config` or the project's default) adds to a build.
//!
//! PORT: `ConfigFlagDefinitions` is a `lib.runtime` class; it lives here, next to its one reader,
//! because FLAG_SET (below `blaze_runtime`) holds it in its key.

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::{Label, LabelRepoContextImpl};
use blaze_events::Event;
use blaze_options::global_rc::GlobalRcUtils;
use blaze_options::parser::STARLARK_SKIPPED_PREFIXES;
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::project_function::{
    BuildableUnit, EnforcementPolicy, ProjectKey, ProjectValue,
};
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, Transience,
};
use indexmap::{IndexMap, IndexSet};

// -------------------------------------------------------------------------------------------
// ConfigFlagDefinitions

/// One `<command>:<config>` rc definition (Java `ConfigFlagDefinitions.ConfigDefinition`).
// Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:122 ConfigFlagDefinitions.ConfigDefinition
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigDefinition {
    pub flags: Vec<String>,
    pub rc_source: String,
}

// Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:33 ConfigFlagDefinitions
/// The `--config` definitions valid for this command (config name -> definitions, in rc order).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConfigFlagDefinitions {
    /// `ImmutableListMultimap<String, ConfigDefinition>`.
    definitions: Vec<(String, ConfigDefinition)>,
}

/// `ConfigFlagDefinitions.ConfigValue`.
// Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:56 ConfigFlagDefinitions.ConfigValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigValue {
    pub flags: Vec<String>,
    pub rc_sources: IndexSet<String>,
}

impl ConfigFlagDefinitions {
    /// `hashCode()`: `definitions.hashCode()`, an `ImmutableListMultimap`'s `asMap().hashCode()`
    /// (the sum of `key.hashCode() ^ list.hashCode()`); a `ConfigDefinition` record hashes as
    /// `31 * flags.hashCode() + rcSource.hashCode()`.
    // Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:145 ConfigFlagDefinitions.hashCode
    pub fn java_hash_code(&self) -> i32 {
        let list_hash = |items: &mut dyn Iterator<Item = i32>| {
            items.fold(1i32, |h, x| h.wrapping_mul(31).wrapping_add(x))
        };
        let mut grouped: IndexMap<&str, Vec<&ConfigDefinition>> = IndexMap::new();
        for (name, definition) in &self.definitions {
            grouped.entry(name).or_default().push(definition);
        }
        grouped.iter().fold(0i32, |h, (name, definitions)| {
            let definitions_hash = list_hash(&mut definitions.iter().map(|d| {
                let flags = list_hash(&mut d.flags.iter().map(|f| java_hash::string_hash_code(f)));
                flags
                    .wrapping_mul(31)
                    .wrapping_add(java_hash::string_hash_code(&d.rc_source))
            }));
            h.wrapping_add(java_hash::string_hash_code(name) ^ definitions_hash)
        })
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:46 ConfigFlagDefinitions.<init>
    pub fn new(definitions: Vec<(String, ConfigDefinition)>) -> Self {
        ConfigFlagDefinitions { definitions }
    }

    fn get_definitions(&self, name: &str) -> Vec<&ConfigDefinition> {
        self.definitions
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, d)| d)
            .collect()
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:63 ConfigFlagDefinitions.get
    /// `Err` is the `OptionsParsingException` message.
    pub fn get(
        config_name: &str,
        definitions: &ConfigFlagDefinitions,
    ) -> Result<ConfigValue, String> {
        if definitions.get_definitions(config_name).is_empty() {
            return Err(format!("--config={} doesn't exist", config_name));
        }
        let mut flags = Vec::new();
        let mut rc_sources = IndexSet::new();
        let mut seen_configs = HashSet::new();
        Self::apply_definition(
            config_name,
            config_name,
            definitions,
            &mut flags,
            &mut rc_sources,
            &mut seen_configs,
        )?;
        Ok(ConfigValue { flags, rc_sources })
    }

    // Java: com/google/devtools/build/lib/runtime/ConfigFlagDefinitions.java:77 ConfigFlagDefinitions.applyDefinition
    fn apply_definition(
        orig_config_name: &str,
        config_name: &str,
        definitions: &ConfigFlagDefinitions,
        flags: &mut Vec<String>,
        rc_sources: &mut IndexSet<String>,
        seen_configs: &mut HashSet<String>,
    ) -> Result<(), String> {
        let direct_flags = definitions.get_definitions(config_name);
        if !seen_configs.insert(config_name.to_owned()) {
            return Err(format!(
                "--config={} can't be evaluated because its definition has a cycle.",
                orig_config_name
            ));
        }
        if direct_flags.is_empty() {
            return Err(format!(
                "--config={} doesn't resolve because it expands to non-existent --config={}",
                orig_config_name, config_name
            ));
        }
        // The underlying data stores a multimap: { name: Collection<Definition> }. This is more
        // important for non-config definitions like "build --a=b" where each rc file defines its
        // own "build" args. For --config, just choose the first.
        let def = direct_flags[0];
        rc_sources.insert(def.rc_source.clone());
        for flag in &def.flags {
            if let Some(name) = flag.strip_prefix("--config=") {
                Self::apply_definition(
                    orig_config_name,
                    name,
                    definitions,
                    flags,
                    rc_sources,
                    seen_configs,
                )?;
            } else {
                flags.push(flag.clone());
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------
// FlagSetValue

/// `SkyFunctions.FLAG_SET`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java SkyFunctions.FLAG_SET
pub const FLAG_SET: SkyFunctionName = SkyFunctionName("FLAG_SET");

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:41 FlagSetValue.Key
/// The key of a `FlagSetValue`.
#[derive(Clone)]
pub struct FlagSetKey {
    pub targets: IndexSet<Label>,
    pub project_file: Label,
    /// `nullToEmpty(sclConfig)`.
    pub scl_config: String,
    pub target_options: Arc<BuildOptions>,
    pub all_option_names: Arc<IndexSet<String>>,
    pub user_options: Arc<IndexMap<String, String>>,
    pub config_flag_definitions: Arc<ConfigFlagDefinitions>,
    pub enforce_canonical: bool,
}

impl PartialEq for FlagSetKey {
    // Java: the record's equals.
    fn eq(&self, other: &Self) -> bool {
        self.targets == other.targets
            && self.project_file == other.project_file
            && self.scl_config == other.scl_config
            && *self.target_options == *other.target_options
            && *self.all_option_names == *other.all_option_names
            && *self.user_options == *other.user_options
            && *self.config_flag_definitions == *other.config_flag_definitions
            && self.enforce_canonical == other.enforce_canonical
    }
}

impl Eq for FlagSetKey {}

impl Hash for FlagSetKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.project_file.hash(state);
        self.scl_config.hash(state);
        self.target_options.hash(state);
        self.enforce_canonical.hash(state);
    }
}

impl FlagSetKey {
    /// The record's `hashCode()` (`ObjectMethods`: `31 * result + hash(component)` from 0, in
    /// component order). Callers that iterate a `HashMap` keyed by `FlagSetValue.Key` (the
    /// `EvaluationResult` of `Project.applySclConfig`) order by it.
    // Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:47 FlagSetValue.Key
    pub fn java_hash_code(&self) -> i32 {
        use java_hash::*;
        // ImmutableSet: the sum of the elements' hashes.
        let targets = self
            .targets
            .iter()
            .fold(0i32, |h, l| h.wrapping_add(label_hash_code(l)));
        // BuildOptions.hashCode: 31 + checksum().hashCode().
        let target_options = 31i32.wrapping_add(string_hash_code(self.target_options.checksum()));
        let all_option_names = self
            .all_option_names
            .iter()
            .fold(0i32, |h, n| h.wrapping_add(string_hash_code(n)));
        // ImmutableMap: the sum of key.hashCode() ^ value.hashCode().
        let user_options = self.user_options.iter().fold(0i32, |h, (k, v)| {
            h.wrapping_add(string_hash_code(k) ^ string_hash_code(v))
        });
        let config_flag_definitions = self.config_flag_definitions.java_hash_code();
        let enforce_canonical = if self.enforce_canonical { 1231 } else { 1237 };
        [
            targets,
            label_hash_code(&self.project_file),
            string_hash_code(&self.scl_config),
            target_options,
            all_option_names,
            user_options,
            config_flag_definitions,
            enforce_canonical,
        ]
        .iter()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(*c))
    }
}

/// `hashCode()` of the Java types in `FlagSetValue.Key`.
mod java_hash {
    use blaze_cmdline::Label;

    /// `String.hashCode()` (UTF-16 code units).
    pub fn string_hash_code(value: &str) -> i32 {
        blaze_util::profiler::java_hash_order::string_hash_code(value)
    }

    /// `HashCodes.hashObjects(o1, ..., on)`.
    // Java: com/google/devtools/build/lib/util/HashCodes.java:46 HashCodes.hashObjects
    fn hash_objects(hashes: &[i32]) -> i32 {
        hashes
            .iter()
            .fold(1i32, |h, x| h.wrapping_mul(31).wrapping_add(*x))
    }

    /// `Label.hashCode()`: `hashCode(name, packageIdentifier)`;
    /// `PackageIdentifier.hashCode()`: `hashObjects(repository, pkgName)`;
    /// `RepositoryName.hashCode()`: `hashObjects(name, contextRepoIfNotVisible,
    /// didYouMeanSuffix)`.
    ///
    /// PORT: the labels here are targets and project files of the build, in visible
    /// repositories (`contextRepoIfNotVisible` and `didYouMeanSuffix` are null).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
    pub fn label_hash_code(label: &Label) -> i32 {
        let package_identifier = label.get_package_identifier();
        let repository = hash_objects(&[
            string_hash_code(package_identifier.get_repository().get_name()),
            0,
            0,
        ]);
        let package = hash_objects(&[
            repository,
            string_hash_code(package_identifier.get_package_fragment().get_path_string()),
        ]);
        let result = 31i32.wrapping_add(string_hash_code(label.get_name()));
        31i32.wrapping_mul(result).wrapping_add(package)
    }
}

impl fmt::Debug for FlagSetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key[targets={:?}, projectFile={}, sclConfig={}, targetOptions={}, enforceCanonical={}]",
            self.targets,
            self.project_file,
            self.scl_config,
            self.target_options.checksum(),
            self.enforce_canonical
        )
    }
}

impl Key for FlagSetKey {
    type Value = FlagSetValue;
    const FUNCTION: SkyFunctionName = FLAG_SET;
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:32 FlagSetValue
/// The flags a project's selected config adds, and the messages to show for it.
#[derive(Clone, Debug)]
pub struct FlagSetValue {
    flags: IndexSet<String>,
    persistent_messages: Vec<Event>,
}

impl FlagSetValue {
    // Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:105 FlagSetValue.create
    pub fn create(flags: IndexSet<String>, persistent_messages: Vec<Event>) -> Self {
        FlagSetValue {
            flags,
            persistent_messages,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:115 FlagSetValue.getOptionsFromFlagset
    pub fn get_options_from_flagset(&self) -> &IndexSet<String> {
        &self.flags
    }

    // Java: com/google/devtools/build/lib/skyframe/config/FlagSetValue.java:120 FlagSetValue.getPersistentMessages
    pub fn get_persistent_messages(&self) -> &[Event] {
        &self.persistent_messages
    }
}

// -------------------------------------------------------------------------------------------
// FlagSetFunction

/// `FlagSetFunction.UnsupportedConfigException` (the FLAG_SET node's error).
// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:713 FlagSetFunction.UnsupportedConfigException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct UnsupportedConfigException(pub String);

fn flag_set_function_exception(message: String) -> ComputeError {
    SkyFunctionException::new(UnsupportedConfigException(message), Transience::Persistent).into()
}

/// Registers FLAG_SET.
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.typed_function::<FlagSetKey, _>(compute)
}

/// Java `ImmutableSet.toString()` / `List.toString()`.
fn java_collection_to_string<'a>(items: impl IntoIterator<Item = &'a String>) -> String {
    format!(
        "[{}]",
        items
            .into_iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `Iterables.get(Splitter.on("=").split(s), 0)`.
fn before_equals(s: &str) -> &str {
    s.split('=').next().unwrap_or("")
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:74 FlagSetFunction.compute
fn compute(
    key: &FlagSetKey,
    env: &mut Environment<'_>,
) -> Result<Option<FlagSetValue>, ComputeError> {
    if !key.enforce_canonical {
        if !key.scl_config.is_empty() {
            env.listener().handle(&Event::info(format!(
                "Ignoring --scl_config={} because --enforce_project_configs is not set",
                key.scl_config
            )));
        }
        // --noenforce_project_configs. Nothing to do.
        return Ok(Some(FlagSetValue::create(IndexSet::new(), Vec::new())));
    }
    let Some(project_value) = env.get_value(ProjectKey::new(key.project_file.clone()))? else {
        return Ok(None);
    };
    // Skyframe doesn't replay warnings or info messages on cache hits: see
    // Event.storeForReplay and Reportable.storeForReplay. We want some flag set messages to be
    // more persistent, so we return them in the Skyvalue for the caller to emit.
    let mut persistent_messages: Vec<Event> = Vec::new();
    let Some(scl_config_as_starlark_list) =
        get_scl_config(key, &project_value, &mut persistent_messages, env)?
    else {
        return Ok(None);
    };
    Ok(Some(FlagSetValue::create(
        scl_config_as_starlark_list,
        persistent_messages,
    )))
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:107 FlagSetFunction.getSclConfig
/// Given an .scl file and `--scl_config` value, returns the flags denoted by that
/// `--scl_config`. `None` if Skyframe dependencies need to be evaluated.
fn get_scl_config(
    key: &FlagSetKey,
    scl_content: &ProjectValue,
    persistent_messages: &mut Vec<Event>,
    env: &mut Environment<'_>,
) -> Result<Option<IndexSet<String>>, ComputeError> {
    let project_file = &key.project_file;
    let scl_config_name = key.scl_config.as_str();
    let enforcement_policy = scl_content.get_enforcement_policy();
    let configs = match scl_content.get_buildable_units() {
        Some(c) if !c.is_empty() => c,
        // This project file doesn't define configs, so it must not be used for canonical
        // configs.
        _ => return Ok(Some(IndexSet::new())),
    };

    let mut scl_config_name_for_message = scl_config_name.to_owned();
    let mut scl_config_value: Vec<String>;
    if scl_config_name.is_empty() {
        // If there's no --scl_config, try to use the default_config.
        let default_buildable_units = filter_projects(&key.targets, configs)?;
        // check that all targets resolves to the same set of flags.
        // orders of flags should not matter here.
        let resolved_default_buildable_unit =
            resolve_single_matching_default_buildable_unit_for_all_targets(
                &default_buildable_units,
            );
        if resolved_default_buildable_unit.len() > 1 {
            return Err(flag_set_function_exception(
                "Building target(s) with different configurations are not supported.".to_owned(),
            ));
        }
        if resolved_default_buildable_unit.is_empty() {
            return Err(flag_set_function_exception(format!(
                "This project's builds must set --scl_config because no default config is defined.\n{}",
                supported_configs_desc(project_file, configs)
            )));
        }
        let buildable_unit = resolved_default_buildable_unit[0];
        scl_config_value = buildable_unit.flags().to_vec();
        scl_config_name_for_message = buildable_unit.name().to_owned();
    } else {
        let Some(unit) = configs.get(scl_config_name) else {
            // The user set --scl_config to an unknown config.
            return Err(flag_set_function_exception(format!(
                "--scl_config={} is not a valid configuration for this project.{}",
                scl_config_name,
                supported_configs_desc(project_file, configs)
            )));
        };
        scl_config_value = unit.flags().to_vec();
    }

    // Canonicalize space-separated flags to equals-separated flags.
    scl_config_value = scl_config_value
        .into_iter()
        .map(|flag| {
            // Leave normal and malformed flags alone.
            if !flag.starts_with("--") || !flag.contains(' ') {
                return flag;
            }
            let space_index = flag.find(' ').map(|i| i as i64).unwrap_or(-1);
            let equals_index = flag.find('=').map(|i| i as i64).unwrap_or(-1);
            // Space-separated flags will always have the space before the equals sign.
            // e.g. we need to canonicalize --define bar=baz, but not --foo='bar baz'
            if space_index < equals_index {
                let i = space_index as usize;
                return format!("{}={}", &flag[..i], &flag[i + 1..]);
            }
            flag
        })
        .collect();

    let build_options_as_strings = get_build_options_as_strings(&key.target_options);
    let directly_set_flags = group_flags(
        &scl_config_value,
        &key.all_option_names,
        &build_options_as_strings,
    );
    // Error on unrecognized native flags.
    let eval_context = format!(
        "Applying config '{}' in {}",
        scl_config_name_for_message, project_file
    );
    if !directly_set_flags.unrecognized_flags.is_empty() {
        return Err(flag_set_function_exception(format!(
            "{}: unrecognized option{}: {}.",
            eval_context,
            if directly_set_flags.unrecognized_flags.len() == 1 {
                ""
            } else {
                "s"
            },
            directly_set_flags
                .unrecognized_flags
                .iter()
                .map(|f| format!("--{}", f))
                .collect::<Vec<_>>()
                .join(", ")
        )));
        // Error on native flags that project files don't support, i.e. non-output affecting
        // flags.
    } else if !directly_set_flags.unsupported_flags.is_empty() {
        return Err(flag_set_function_exception(format!(
            "{}: project flags don't support non-output affecting option{}: {}.",
            eval_context,
            if directly_set_flags.unsupported_flags.len() == 1 {
                ""
            } else {
                "s"
            },
            directly_set_flags
                .unsupported_flags
                .iter()
                .map(|f| format!("--{}", f))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    // Check that directly set Starlark flags are valid.
    if !validate_starlark_flags(&directly_set_flags.starlark_flags, &eval_context, env)? {
        return Ok(None);
    }

    // Replace --config=foo entries with their expanded definitions.
    let scl_config_value = expand_config_flags(
        scl_config_name,
        &scl_config_value,
        &key.config_flag_definitions,
    )?;
    // TODO: b/388289978 - Fail on unrecognized options from --config and warn when ignoring
    //     recognized options that are filtered out here.
    let options_to_apply = filter_options(&scl_config_value, &build_options_as_strings);
    if options_to_apply.is_empty() {
        return Ok(Some(IndexSet::new()));
    }

    let always_allowed_configs: Vec<String> = scl_content
        .get_always_allowed_configs()
        .map(|c| c.to_vec())
        .unwrap_or_default();
    validate_no_extra_flags_set(
        enforcement_policy,
        &always_allowed_configs,
        &build_options_as_strings,
        &key.user_options,
        &options_to_apply,
        persistent_messages,
        project_file,
    )?;
    persistent_messages.push(Event::info(format!(
        "Applying flags from the config '{}' defined in {}: {} ",
        scl_config_name_for_message,
        project_file,
        java_collection_to_string(&options_to_apply)
    )));
    Ok(Some(options_to_apply))
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:262 FlagSetFunction.filterProjects
/// Returns all default buildable units that contain the specific target in the
/// `targetPatterns` field. If there are multiple matching default buildable units, an exception
/// will be thrown.
fn filter_projects<'a>(
    targets: &IndexSet<Label>,
    buildable_units: &'a IndexMap<String, BuildableUnit>,
) -> Result<Vec<&'a BuildableUnit>, ComputeError> {
    // PORT: java-hash-order. Java collects the matches in a HashMap<Label, BuildableUnit> and
    // returns its values; the order only decides which unit is "first" among units with equal
    // flags, which have the same effect. Targets are kept in their order here.
    let mut targets_and_matching_default_buildable_units: IndexMap<&Label, &'a BuildableUnit> =
        IndexMap::new();
    for target in targets {
        for buildable_unit in buildable_units.values() {
            if does_buildable_unit_match_target(buildable_unit, target)
                && buildable_unit.is_default()
                && targets_and_matching_default_buildable_units
                    .insert(target, buildable_unit)
                    .is_some()
            {
                return Err(flag_set_function_exception(format!(
                    "Multiple matching default configs found for target {}. Please check your project file and ensure that for target {}, there should be only 1 matching default config.",
                    target, target
                )));
            }
        }
    }
    Ok(targets_and_matching_default_buildable_units
        .into_values()
        .collect())
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:293 FlagSetFunction.resolveSingleMatchingDefaultBuildableUnitForAllTargets
/// Takes a list of default buildable units and compares the flags values of all buildable units.
/// Returns the first matching buildable unit for each distinct set of flags.
fn resolve_single_matching_default_buildable_unit_for_all_targets<'a>(
    default_buildable_units_for_all_targets: &[&'a BuildableUnit],
) -> Vec<&'a BuildableUnit> {
    let mut flags_to_first_buildable_unit: IndexMap<&[String], &'a BuildableUnit> = IndexMap::new();
    for buildable_unit in default_buildable_units_for_all_targets {
        flags_to_first_buildable_unit
            .entry(buildable_unit.flags())
            .or_insert(buildable_unit);
    }
    // ImmutableSet.copyOf(values): BuildableUnit equality.
    let mut result: Vec<&'a BuildableUnit> = Vec::new();
    for unit in flags_to_first_buildable_unit.into_values() {
        if !result.iter().any(|u| **u == *unit) {
            result.push(unit);
        }
    }
    result
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:310 FlagSetFunction.doesBuildableUnitMatchTarget
/// Returns `true` iff the `specific_target` matches the target patterns in the buildable unit.
pub fn does_buildable_unit_match_target(
    buildable_unit: &BuildableUnit,
    specific_target: &Label,
) -> bool {
    if buildable_unit.target_pattern_matcher().is_empty() {
        return true;
    }
    buildable_unit
        .target_pattern_matcher()
        .contains(specific_target)
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:317 FlagSetFunction.getBuildOptionsAsStrings
fn get_build_options_as_strings(target_options: &BuildOptions) -> Vec<String> {
    let mut all_options_as_strings: Vec<String> = Vec::new();
    // Collect a list of BuildOptions, excluding TestOptions.
    for key in target_options.get_starlark_options().keys() {
        all_options_as_strings.push(key.to_string());
    }
    for (class, fragment_options) in target_options.get_native_options() {
        if class.get_name()
            == "com.google.devtools.build.lib.analysis.test.TestConfiguration$TestOptions"
        {
            continue;
        }
        all_options_as_strings.extend(fragment_options.as_map().into_keys());
    }
    all_options_as_strings
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:343 FlagSetFunction.filterOptions
/// Filters the options from the selected config to only those that are part of `BuildOptions`,
/// excluding `TestConfiguration.TestOptions`.
fn filter_options(
    flags_from_selected_config: &[String],
    build_options_as_strings: &[String],
) -> IndexSet<String> {
    let mut filtered_flags = IndexSet::new();
    for flag_setting in flags_from_selected_config {
        // Remove options that aren't part of BuildOptions from the selected config.
        let name = before_equals(flag_setting)
            .replacen("--", "", 1)
            // Don't strip out negative boolean flags, e.g --nostamp, even though they aren't
            // part of BuildOptions in the negative form.
            .replacen("no", "", 1)
            .replace('\'', "");
        if build_options_as_strings.contains(&name) {
            filtered_flags.insert(flag_setting.clone());
        } else if STARLARK_SKIPPED_PREFIXES
            .iter()
            .any(|p| flag_setting.starts_with(p))
        {
            // Because the BuildOptions might not already include Starlark flags that are set in
            // the flagset, explicitly add them to the set of options to return.
            filtered_flags.insert(flag_setting.clone());
        }
    }
    filtered_flags
}

/// Project-declared flag settings grouped by flag type, all in `"flag_name"` form.
// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:378 FlagSetFunction.FlagTypes
struct FlagTypes {
    #[allow(dead_code)]
    supported_flags: IndexSet<String>,
    unsupported_flags: IndexSet<String>,
    unrecognized_flags: IndexSet<String>,
    starlark_flags: IndexSet<String>,
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:392 FlagSetFunction.groupFlags
fn group_flags(
    flag_settings: &[String],
    all_option_names: &IndexSet<String>,
    build_options_as_strings: &[String],
) -> FlagTypes {
    let mut supported_flags = IndexSet::new();
    let mut unsupported_flags = IndexSet::new();
    let mut unrecognized_flags = IndexSet::new();
    let mut starlark_flags = IndexSet::new();
    for flag_setting in flag_settings {
        let flag_name = before_equals(flag_setting).replacen("--", "", 1);
        if flag_name == "config" {
            supported_flags.insert(flag_setting.clone());
        } else if STARLARK_SKIPPED_PREFIXES
            .iter()
            .any(|p| flag_setting.starts_with(p))
        {
            starlark_flags.insert(flag_name);
        } else if !all_option_names.contains(&flag_name) {
            unrecognized_flags.insert(flag_name);
        } else if !build_options_as_strings.contains(&flag_name)
            // We really only want to check the option without its "no" prefix if the flag is a
            // boolean flag, but it's hard to know what type it is at this point in the build.
            && !build_options_as_strings.contains(&flag_name.replacen("no", "", 1))
        {
            unsupported_flags.insert(flag_name);
        } else {
            supported_flags.insert(flag_name);
        }
    }
    FlagTypes {
        supported_flags,
        unsupported_flags,
        unrecognized_flags,
        starlark_flags,
    }
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:434 FlagSetFunction.validateStarlarkFlags
/// Checks that Starlark flags exist. Returns `Ok(false)` if Skyframe dependencies need to be
/// evaluated.
fn validate_starlark_flags(
    starlark_flags: &IndexSet<String>,
    eval_context: &str,
    env: &mut Environment<'_>,
) -> Result<bool, ComputeError> {
    // Maps Starlark flag labels to the strings they appear in in in the project file.
    let mut flag_labels: IndexMap<Label, String> = IndexMap::new();
    let mut bad_flags: IndexSet<String> = IndexSet::new();
    // Get the context we need to properly parse labels that may come from other repos.
    let Some(main_repo_mapping) = env.get_value(RepositoryMappingKey::key(
        blaze_cmdline::repository_name::MAIN.clone(),
    ))?
    else {
        return Ok(false);
    };
    let main_repo_context = LabelRepoContextImpl::of(
        blaze_cmdline::repository_name::MAIN.clone(),
        main_repo_mapping
            .repository_mapping()
            .expect("NullPointerException: repositoryMapping()")
            .clone(),
    );
    // Parse the flags into repo-aware labels.
    for starlark_flag in starlark_flags {
        match Label::parse_with_repo_context(starlark_flag, &main_repo_context) {
            Ok(label) => {
                flag_labels.insert(label, starlark_flag.clone());
            }
            Err(_) => {
                bad_flags.insert(starlark_flag.clone());
            }
        }
    }
    // Skyframe-load their packages.
    let mut package_keys: IndexSet<PackageKey> = IndexSet::new();
    for label in flag_labels.keys() {
        package_keys.insert(PackageKey::key(label.get_package_identifier().clone()));
    }
    let sky_keys: Vec<SkyKey> = package_keys
        .iter()
        .map(|k| SkyKey::new(k.clone()))
        .collect();
    let evaluated = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(false);
    }
    // Check that they're real, flag-typed targets.
    for (label, given) in &flag_labels {
        let key = PackageKey::key(label.get_package_identifier().clone());
        let is_build_setting =
            match evaluated.get_or_throw_typed::<PackageKey, NoSuchPackageException>(&key) {
                Ok(Some(pkg)) => match pkg.get_package().get_target(label.get_name()) {
                    Ok(target) => target.as_rule().is_some_and(|rule| rule.is_build_setting()),
                    Err(_) => false,
                },
                _ => false,
            };
        if !is_build_setting {
            bad_flags.insert(given.clone());
        }
    }
    // Report bad results.
    if !bad_flags.is_empty() {
        return Err(flag_set_function_exception(format!(
            "{}: unrecognized Starlark flag{}: {}. {}",
            eval_context,
            if bad_flags.len() == 1 { "" } else { "s" },
            bad_flags
                .iter()
                .map(|f| format!("--{}", f))
                .collect::<Vec<_>>()
                .join(", "),
            if bad_flags.len() == 1 {
                "Check that this is a valid target that can be set as a flag."
            } else {
                "Check that these are valid targets that can be set as flags."
            }
        )));
    }
    Ok(true)
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:516 FlagSetFunction.expandConfigFlags
/// In-place expands `--config=foo` entries in `input_flags`.
fn expand_config_flags(
    scl_config_name: &str,
    input_flags: &[String],
    config_flag_definitions: &ConfigFlagDefinitions,
) -> Result<Vec<String>, ComputeError> {
    // First look for dupes.
    let mut dupe_checker: HashSet<&String> = HashSet::new();
    for flag in input_flags {
        if flag.starts_with("--config=") && !dupe_checker.insert(flag) {
            return Err(flag_set_function_exception(format!(
                "--scl_config={}: {} appears multiple times. Please ensure it appears at most once.",
                scl_config_name, flag
            )));
        }
    }
    // Now rebuild the input list while in-place expanding each "--config=foo" entry.
    let mut ans: Vec<String> = Vec::new();
    for flag in input_flags {
        if !flag.starts_with("--config=") {
            ans.push(flag.clone());
            continue;
        }
        // TODO: b/388289978 - fail when a --config sets non-BuildOptions flags.
        let name = &flag[flag.find('=').unwrap() + 1..];
        let expanded_flags =
            ConfigFlagDefinitions::get(name, config_flag_definitions).map_err(|e| {
                flag_set_function_exception(format!("--scl_config={}: {}", scl_config_name, e))
            })?;
        for rc_source in &expanded_flags.rc_sources {
            if !GlobalRcUtils::is_global_rc_file(rc_source) {
                return Err(flag_set_function_exception(format!(
                    "--scl_config={}: can't set {} because its definition depends on {} which isn't a global rc file.",
                    scl_config_name, flag, rc_source
                )));
            }
        }
        ans.extend(expanded_flags.flags);
    }
    Ok(ans)
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:583 FlagSetFunction.validateNoExtraFlagsSet
/// Enforces the project's `enforcement_policy` against the output-affecting options the user
/// set on the command line or in a user bazelrc.
fn validate_no_extra_flags_set(
    enforcement_policy: EnforcementPolicy,
    always_allowed_configs: &[String],
    build_options_as_strings: &[String],
    user_options: &IndexMap<String, String>,
    flags_from_selected_config: &IndexSet<String>,
    persistent_messages: &mut Vec<Event>,
    project_file: &Label,
) -> Result<(), ComputeError> {
    let mut overlap: IndexSet<String> = IndexSet::new();
    for (option, expanded_from) in user_options {
        // Remove options that aren't part of BuildOptions. This section can be removed once
        // we only include BuildOptions in the passed userOptions.
        let name = before_equals(option)
            .replacen("--", "", 1)
            .replace('\'', "");
        if !build_options_as_strings.contains(&name) {
            continue;
        }
        if option.starts_with("--scl_config") {
            continue;
        }
        if flags_from_selected_config.contains(option) {
            continue;
        }
        // Remove options that are expanded from always-allowed configs either defined in the
        // project file...
        if always_allowed_configs.contains(expanded_from) {
            continue;
        }
        // ... or globally
        if GlobalRcUtils::ALLOWED_GLOBAL_CONFIGS.contains(&expanded_from.as_str()) {
            continue;
        }
        overlap.insert(if expanded_from.is_empty() {
            format!("'{}'", option)
        } else {
            format!("'{}'", expanded_from)
        });
    }
    if overlap.is_empty() {
        return Ok(());
    }
    match enforcement_policy {
        EnforcementPolicy::WARN => {}
        EnforcementPolicy::COMPATIBLE => {
            let option_names_from_selected_config: IndexSet<String> = flags_from_selected_config
                .iter()
                .map(|flag| before_equals(flag).replace('\'', ""))
                .collect();
            let conflicting_options: IndexSet<String> = overlap
                .iter()
                .filter(|option| {
                    option_names_from_selected_config
                        .contains(&before_equals(option).replace('\'', ""))
                })
                .cloned()
                .collect();
            if !conflicting_options.is_empty() {
                return Err(flag_set_function_exception(format!(
                    "This build uses a project file ({}) that does not allow conflicting flags in the command line or user bazelrc. Found {}. Please remove these flags or disable project file resolution via --noenforce_project_configs.",
                    project_file,
                    java_collection_to_string(&conflicting_options)
                )));
            }
        }
        EnforcementPolicy::STRICT => {
            return Err(flag_set_function_exception(format!(
                "This build uses a project file ({}) that does not allow output-affecting flags in the command line or user bazelrc. Found {}. Please remove these flags or disable project file resolution via --noenforce_project_configs.",
                project_file,
                java_collection_to_string(&overlap)
            )));
        }
    }
    // This appears in the WARN case, or for a COMPATIBLE project file that doesn't have
    // conflicting flags. We never hit this in the STRICT case, since we've already thrown.
    persistent_messages.push(Event::warn(format!(
        "This build uses a project file ({}), but also sets output-affecting flags in the command line or user bazelrc: {}. Please consider removing these flags.",
        project_file,
        java_collection_to_string(&overlap)
    )));
    Ok(())
}

// Java: com/google/devtools/build/lib/skyframe/config/FlagSetFunction.java:681 FlagSetFunction.supportedConfigsDesc
/// Returns a user-friendly description of project-supported configurations.
fn supported_configs_desc(
    project_file: &Label,
    configs: &IndexMap<String, BuildableUnit>,
) -> String {
    let mut ans = "\nThis project supports:\n".to_owned();
    let longest_name_length = configs.keys().map(|k| k.chars().count()).max().unwrap();
    for (name, unit) in configs {
        // Strings.padEnd(name, longestNameLength, ' ')
        let padding = " ".repeat(longest_name_length - name.chars().count());
        ans.push_str(&format!("  --scl_config={}{} -> ", name, padding));
        let desc = unit.description();
        // Add user-friendly description if specified, else list of applied flags.
        if desc.is_empty() || desc == name {
            ans.push_str(&format!("[{}]", unit.flags().join(" ")));
        } else {
            ans.push_str(desc);
        }
        ans.push('\n');
    }
    ans.push_str(&format!(
        "\nThis policy is defined in {}.\n",
        project_file.to_path_fragment()
    ));
    ans
}
