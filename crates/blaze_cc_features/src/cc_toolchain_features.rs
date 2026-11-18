//! Port of `com.google.devtools.build.lib.rules.cpp.CcToolchainFeatures`: access to the features
//! supported by a specific toolchain, feature selection and flag/environment expansion.

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::actions::{InputMetadataProvider, NOOP, PathMapper};
use crate::artifact_category::ArtifactCategory;
use crate::cc_toolchain_config_info::CcToolchainConfigInfo;
use crate::cc_toolchain_variables::{
    CcToolchainVariables, CcToolchainVariablesRef, StringChunk, VariableObject, get_sequence_value,
};
use crate::errors::{CollidingProvidesException, EvalException, ExpansionException};
use crate::feature_configuration_for_starlark::FeatureConfigurationForStarlark;
use crate::feature_selection::FeatureSelection;

/// Error message thrown when a toolchain enables two features that provide the same string.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:106 CcToolchainFeatures.COLLIDING_PROVIDES_ERROR
pub fn colliding_provides_error(symbol: &str, features: &str) -> String {
    format!("Symbol {symbol} is provided by all of the following features: {features}")
}

/// Java's `ImmutableSet<String>` inside hashed records: set equality, order-independent hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StringSet(pub IndexSet<String>);

impl Hash for StringSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Java's Set.hashCode is the sum of the element hashes.
        let mut sum: u64 = 0;
        for s in &self.0 {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            sum = sum.wrapping_add(h.finish());
        }
        sum.hash(state);
    }
}

impl StringSet {
    pub fn of<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> StringSet {
        StringSet(items.into_iter().map(|s| s.as_ref().to_string()).collect())
    }

    pub fn contains(&self, s: &str) -> bool {
        self.0.contains(s)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.0.iter()
    }
}

/// A flag or flag group that can be expanded under a set of variables.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:256 CcToolchainVariables.Expandable
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Expandable {
    Flag(Flag),
    SingleChunkFlag(SingleChunkFlag),
    FlagGroup(Box<FlagGroup>),
}

impl Expandable {
    /// Expands the current expandable under the given `variables`, adding new flags to
    /// `command_line`.
    pub fn expand(
        &self,
        variables: &CcToolchainVariablesRef,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        match self {
            Expandable::Flag(f) => f.expand(variables, path_mapper, command_line),
            Expandable::SingleChunkFlag(f) => f.expand(variables, path_mapper, command_line),
            Expandable::FlagGroup(g) => g.expand(
                variables,
                input_metadata_provider,
                path_mapper,
                command_line,
            ),
        }
    }
}

/// A single flag to be expanded under a set of variables.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:112 CcToolchainFeatures.Flag
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Flag {
    pub chunks: Vec<StringChunk>,
}

impl Flag {
    /// Expand this flag into a single new entry in `command_line`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:115 CcToolchainFeatures.Flag.expand
    fn expand(
        &self,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        let mut flag = String::new();
        for chunk in &self.chunks {
            flag.push_str(&chunk.expand(variables, path_mapper)?);
        }
        command_line.push(flag);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:129 CcToolchainFeatures.Flag.create
    pub fn create(chunks: Vec<StringChunk>) -> Expandable {
        if chunks.len() == 1 {
            return Expandable::SingleChunkFlag(SingleChunkFlag {
                chunk: chunks.into_iter().next().unwrap(),
            });
        }
        Expandable::Flag(Flag { chunks })
    }
}

/// Optimization for single-chunk case.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:139 CcToolchainFeatures.Flag.SingleChunkFlag
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SingleChunkFlag {
    pub chunk: StringChunk,
}

impl SingleChunkFlag {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:142 CcToolchainFeatures.Flag.SingleChunkFlag.expand
    fn expand(
        &self,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        command_line.push(self.chunk.expand(variables, path_mapper)?);
        Ok(())
    }
}

/// A single environment key/value pair to be expanded under a set of variables.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:156 CcToolchainFeatures.EnvEntry
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnvEntry {
    pub key: String,
    pub value_chunks: Vec<StringChunk>,
    pub expand_if_all_available: StringSet,
}

impl EnvEntry {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:161 CcToolchainFeatures.EnvEntry.canBeExpanded
    fn can_be_expanded(&self, variables: &CcToolchainVariables) -> bool {
        for variable in self.expand_if_all_available.iter() {
            if !variables.is_available(variable) {
                return false;
            }
        }
        true
    }

    /// Adds the key/value pair this object represents to the given map of environment
    /// variables. The value of the entry is expanded with the given `variables`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:174 CcToolchainFeatures.EnvEntry.addEnvEntry
    pub fn add_env_entry(
        &self,
        variables: &CcToolchainVariables,
        env_builder: &mut ImmutableMapBuilder,
        path_mapper: &dyn PathMapper,
    ) -> Result<(), ExpansionException> {
        if !self.can_be_expanded(variables) {
            return Ok(());
        }
        let mut value = String::new();
        for chunk in &self.value_chunks {
            value.push_str(&chunk.expand(variables, path_mapper)?);
        }
        env_builder.put(self.key.clone(), value);
        Ok(())
    }
}

/// Java's `ImmutableMap.Builder<String, String>` with `buildOrThrow` duplicate-key failure.
#[derive(Default)]
pub struct ImmutableMapBuilder {
    entries: Vec<(String, String)>,
}

impl ImmutableMapBuilder {
    pub fn put(&mut self, key: String, value: String) {
        self.entries.push((key, value));
    }

    /// Java: `buildOrThrow()`; duplicate keys throw `IllegalArgumentException`.
    pub fn build_or_throw(self) -> IndexMap<String, String> {
        let mut map: IndexMap<String, String> = IndexMap::new();
        for (key, value) in self.entries {
            if let Some(existing) = map.get(&key) {
                // PORT: Guava's message names both entries; the order of the two entries in
                // Guava's text is not verified.
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {key}={value} and \
                     {key}={existing}"
                );
            }
            map.insert(key, value);
        }
        map
    }
}

/// Used for equality check between a variable and a specific value.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:193 CcToolchainFeatures.VariableWithValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariableWithValue {
    pub variable: String,
    pub value: String,
}

/// A group of flags. When iterateOverVariable is specified, we assume the variable is a sequence
/// and the flag_group will be expanded repeatedly for every value in the sequence.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:201 CcToolchainFeatures.FlagGroup
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FlagGroup {
    pub expandables: Vec<Expandable>,
    pub iterate_over_variable: Option<String>,
    pub expand_if_all_available: StringSet,
    pub expand_if_none_available: StringSet,
    pub expand_if_true: Option<String>,
    pub expand_if_false: Option<String>,
    pub expand_if_equal: Option<VariableWithValue>,
}

impl FlagGroup {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:212 CcToolchainFeatures.FlagGroup.expand
    pub fn expand(
        &self,
        variables: &CcToolchainVariablesRef,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        if !self.can_be_expanded(variables, input_metadata_provider, path_mapper)? {
            return Ok(());
        }
        if let Some(iterate_over_variable) = &self.iterate_over_variable {
            for variable_value in get_sequence_value(
                iterate_over_variable,
                &variables.get_variable_with_metadata(
                    iterate_over_variable,
                    input_metadata_provider,
                    path_mapper,
                )?,
            )? {
                let nested_variables = CcToolchainVariables::new_single_variables(
                    Some(variables.clone()),
                    iterate_over_variable,
                    match variable_value {
                        Some(v) => VariableObject::Value(v),
                        None => VariableObject::Null,
                    },
                );
                for expandable in &self.expandables {
                    expandable.expand(
                        &nested_variables,
                        input_metadata_provider,
                        path_mapper,
                        command_line,
                    )?;
                }
            }
        } else {
            for expandable in &self.expandables {
                expandable.expand(
                    variables,
                    input_metadata_provider,
                    path_mapper,
                    command_line,
                )?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:239 CcToolchainFeatures.FlagGroup.canBeExpanded
    fn can_be_expanded(
        &self,
        variables: &CcToolchainVariables,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<bool, ExpansionException> {
        for variable in self.expand_if_all_available.iter() {
            if !variables.is_available_with_metadata(variable, input_metadata_provider) {
                return Ok(false);
            }
        }
        for variable in self.expand_if_none_available.iter() {
            if variables.is_available_with_metadata(variable, input_metadata_provider) {
                return Ok(false);
            }
        }
        if let Some(expand_if_true) = &self.expand_if_true
            && (!variables.is_available_with_metadata(expand_if_true, input_metadata_provider)
                || !variables
                    .get_variable(expand_if_true, path_mapper)?
                    .is_truthy())
        {
            return Ok(false);
        }
        if let Some(expand_if_false) = &self.expand_if_false
            && (!variables.is_available_with_metadata(expand_if_false, input_metadata_provider)
                || variables
                    .get_variable(expand_if_false, path_mapper)?
                    .is_truthy())
        {
            return Ok(false);
        }
        if let Some(expand_if_equal) = &self.expand_if_equal
            && (!variables
                .is_available_with_metadata(&expand_if_equal.variable, input_metadata_provider)
                || variables
                    .get_variable(&expand_if_equal.variable, path_mapper)?
                    .get_string_value(&expand_if_equal.variable, path_mapper)?
                    != expand_if_equal.value)
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Expands all flags in this group and adds them to `command_line`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:290 CcToolchainFeatures.FlagGroup.expandCommandLine
    fn expand_command_line(
        &self,
        variables: &CcToolchainVariablesRef,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        self.expand(
            variables,
            input_metadata_provider,
            path_mapper,
            command_line,
        )
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:300 CcToolchainFeatures.isWithFeaturesSatisfied
fn is_with_features_satisfied<'a>(
    with_feature_sets: impl IntoIterator<Item = &'a WithFeatureSet>,
    enabled_feature_names: &IndexSet<String>,
) -> bool {
    let mut any = false;
    for feature_set in with_feature_sets {
        any = true;
        if feature_set
            .features
            .iter()
            .all(|f| enabled_feature_names.contains(f))
            && !feature_set
                .not_features
                .iter()
                .any(|f| enabled_feature_names.contains(f))
        {
            return true;
        }
    }
    // withFeatureSets.isEmpty() -> true
    !any
}

/// Groups a set of flags to apply for certain actions.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:317 CcToolchainFeatures.FlagSet
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FlagSet {
    pub actions: StringSet,
    pub expand_if_all_available: StringSet,
    pub with_feature_sets: WithFeatureSetSet,
    pub flag_groups: Vec<FlagGroup>,
}

impl FlagSet {
    /// Adds the flags that apply to the given `action` to `command_line`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:324 CcToolchainFeatures.FlagSet.expandCommandLine
    fn expand_command_line(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
        enabled_feature_names: &IndexSet<String>,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        for variable in self.expand_if_all_available.iter() {
            if !variables.is_available_with_metadata(variable, input_metadata_provider) {
                return Ok(());
            }
        }
        if !is_with_features_satisfied(self.with_feature_sets.iter(), enabled_feature_names) {
            return Ok(());
        }
        if !self.actions.contains(action) {
            return Ok(());
        }
        for flag_group in &self.flag_groups {
            flag_group.expand_command_line(
                variables,
                input_metadata_provider,
                path_mapper,
                command_line,
            )?;
        }
        Ok(())
    }
}

/// A set of positive and negative features. This stanza will evaluate to true when every
/// 'feature' is enabled, and every 'not_feature' is not enabled.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:355 CcToolchainFeatures.WithFeatureSet
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WithFeatureSet {
    pub features: StringSet,
    pub not_features: StringSet,
}

/// Java's `ImmutableSet<WithFeatureSet>`: insertion order, set equality.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WithFeatureSetSet(pub IndexSet<WithFeatureSet>);

impl Hash for WithFeatureSetSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut sum: u64 = 0;
        for s in &self.0 {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            sum = sum.wrapping_add(h.finish());
        }
        sum.hash(state);
    }
}

impl WithFeatureSetSet {
    pub fn iter(&self) -> impl Iterator<Item = &WithFeatureSet> {
        self.0.iter()
    }
}

/// Groups a set of environment variables to apply for certain actions.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:360 CcToolchainFeatures.EnvSet
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnvSet {
    pub actions: StringSet,
    pub env_entries: Vec<EnvEntry>,
    pub with_feature_sets: WithFeatureSetSet,
}

impl EnvSet {
    /// Adds the environment key/value pairs that apply to the given `action` to `env_builder`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:369 CcToolchainFeatures.EnvSet.expandEnvironment
    fn expand_environment(
        &self,
        action: &str,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
        enabled_feature_names: &IndexSet<String>,
        env_builder: &mut ImmutableMapBuilder,
    ) -> Result<(), ExpansionException> {
        if !self.actions.contains(action) {
            return Ok(());
        }
        if !is_with_features_satisfied(self.with_feature_sets.iter(), enabled_feature_names) {
            return Ok(());
        }
        for env_entry in &self.env_entries {
            env_entry.add_env_entry(variables, env_builder, path_mapper)?;
        }
        Ok(())
    }
}

/// Contains flags for a specific feature.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:404 CcToolchainFeatures.Feature
// PORT: the weak interner is a memory optimization and is skipped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
    name: String,
    flag_sets: Vec<FlagSet>,
    env_sets: Vec<EnvSet>,
    enabled: bool,
    requires: Vec<StringSet>,
    implies: Vec<String>,
    provides: Vec<String>,
}

impl Feature {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:415 CcToolchainFeatures.Feature.<init>
    pub fn new(
        name: String,
        flag_sets: Vec<FlagSet>,
        env_sets: Vec<EnvSet>,
        enabled: bool,
        requires: Vec<StringSet>,
        implies: Vec<String>,
        provides: Vec<String>,
    ) -> Feature {
        Feature {
            name,
            flag_sets,
            env_sets,
            enabled,
            requires,
            implies,
            provides,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:439 CcToolchainFeatures.Feature.getName
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Adds environment variables for the given action to the provided builder.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:444 CcToolchainFeatures.Feature.expandEnvironment
    fn expand_environment(
        &self,
        action: &str,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
        enabled_feature_names: &IndexSet<String>,
        env_builder: &mut ImmutableMapBuilder,
    ) -> Result<(), ExpansionException> {
        for env_set in &self.env_sets {
            env_set.expand_environment(
                action,
                variables,
                path_mapper,
                enabled_feature_names,
                env_builder,
            )?;
        }
        Ok(())
    }

    /// Adds the flags that apply to the given `action` to `command_line`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:457 CcToolchainFeatures.Feature.expandCommandLine
    fn expand_command_line(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
        enabled_feature_names: &IndexSet<String>,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        for flag_set in &self.flag_sets {
            flag_set.expand_command_line(
                action,
                variables,
                enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                command_line,
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:471 CcToolchainFeatures.Feature.getFlagSets
    pub fn get_flag_sets(&self) -> &[FlagSet] {
        &self.flag_sets
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:475 CcToolchainFeatures.Feature.getEnvSets
    pub fn get_env_sets(&self) -> &[EnvSet] {
        &self.env_sets
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:501 CcToolchainFeatures.Feature.isEnabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:505 CcToolchainFeatures.Feature.getRequires
    pub fn get_requires(&self) -> &[StringSet] {
        &self.requires
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:509 CcToolchainFeatures.Feature.getImplies
    pub fn get_implies(&self) -> &[String] {
        &self.implies
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:513 CcToolchainFeatures.Feature.getProvides
    pub fn get_provides(&self) -> &[String] {
        &self.provides
    }
}

/// Where a tool path is relative to.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:524 CcToolchainFeatures.Tool.PathOrigin
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathOrigin {
    CROSSTOOL_PACKAGE,
    FILESYSTEM_ROOT,
    WORKSPACE_ROOT,
}

/// An executable to be invoked by a blaze action. Can carry information on its platform
/// restrictions.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:523 CcToolchainFeatures.Tool
// PORT: Java caches the CROSSTOOL_PACKAGE path string; it is recomputed here (pure).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tool {
    tool_path_fragment: PathFragment,
    tool_path_origin: PathOrigin,
    execution_requirements: StringSet,
    with_feature_set_sets: WithFeatureSetSet,
}

impl Tool {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:539 CcToolchainFeatures.Tool.<init>
    pub fn new(
        tool_path_fragment: PathFragment,
        tool_path_origin: PathOrigin,
        execution_requirements: StringSet,
        with_feature_set_sets: WithFeatureSetSet,
    ) -> Result<Tool, EvalException> {
        Tool::check_tool_path(&tool_path_fragment, tool_path_origin)?;
        Ok(Tool {
            tool_path_fragment,
            tool_path_origin,
            execution_requirements,
            with_feature_set_sets,
        })
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:566 CcToolchainFeatures.Tool.checkToolPath
    fn check_tool_path(tool_path: &PathFragment, origin: PathOrigin) -> Result<(), EvalException> {
        match origin {
            // For legacy reasons, we allow absolute and relative paths here.
            PathOrigin::CROSSTOOL_PACKAGE => Ok(()),
            PathOrigin::FILESYSTEM_ROOT => {
                if !tool_path.is_absolute() {
                    return Err(EvalException::new(format!(
                        "Tool-path with origin FILESYSTEM_ROOT must be absolute, got '{}'.",
                        tool_path.get_path_string()
                    )));
                }
                Ok(())
            }
            PathOrigin::WORKSPACE_ROOT => {
                if tool_path.is_absolute() {
                    return Err(EvalException::new(format!(
                        "Tool-path with origin WORKSPACE_ROOT must be relative, got '{}'.",
                        tool_path.get_path_string()
                    )));
                }
                Ok(())
            }
        }
    }

    /// Returns the path to this action's tool relative to the provided crosstool path.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:595 CcToolchainFeatures.Tool.getToolPathString
    pub fn get_tool_path_string(&self, cc_toolchain_path: &PathFragment) -> String {
        match self.tool_path_origin {
            // Legacy behavior.
            PathOrigin::CROSSTOOL_PACKAGE => cc_toolchain_path
                .get_relative(&self.tool_path_fragment)
                .get_safe_path_string()
                .to_string(),
            PathOrigin::FILESYSTEM_ROOT | PathOrigin::WORKSPACE_ROOT => {
                self.tool_path_fragment.get_safe_path_string().to_string()
            }
        }
    }

    /// Returns a list of requirement hints that apply to the execution of this tool.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:609 CcToolchainFeatures.Tool.getExecutionRequirements
    pub fn get_execution_requirements(&self) -> &StringSet {
        &self.execution_requirements
    }

    /// Returns the `WithFeatureSet`s used to decide whether to use this tool given a set of
    /// enabled features.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:617 CcToolchainFeatures.Tool.getWithFeatureSetSets
    pub fn get_with_feature_set_sets(&self) -> &WithFeatureSetSet {
        &self.with_feature_set_sets
    }
}

/// A container for information on a particular blaze action.
///
/// An ActionConfig can select a tool for its blaze action based on the set of active features.
/// For a given feature configuration, the ActionConfig will consider the first tool in its list
/// with a feature set that matches the configuration to be the tool for its blaze action.
///
/// ActionConfigs can be activated by features. It is invalid for a toolchain to contain two
/// action configs for the same blaze action.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:643 CcToolchainFeatures.ActionConfig
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActionConfig {
    config_name: String,
    action_name: String,
    tools: Vec<Tool>,
    flag_sets: Vec<FlagSet>,
    enabled: bool,
    implies: Vec<String>,
}

impl ActionConfig {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:644 CcToolchainFeatures.ActionConfig.FLAG_SET_WITH_ACTION_ERROR
    pub fn flag_set_with_action_error(action_config: &str) -> String {
        format!(
            "action_config {action_config} specifies actions.  An action_config's flag sets \
             automatically apply to the configured action.  Thus, you must not specify action \
             lists in an action_config's flag set."
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:659 CcToolchainFeatures.ActionConfig.<init>
    pub fn new(
        config_name: String,
        action_name: String,
        tools: Vec<Tool>,
        flag_sets: Vec<FlagSet>,
        enabled: bool,
        implies: Vec<String>,
    ) -> ActionConfig {
        ActionConfig {
            config_name,
            action_name,
            tools,
            flag_sets,
            enabled,
            implies,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:681 CcToolchainFeatures.ActionConfig.getName
    pub fn get_name(&self) -> &str {
        &self.config_name
    }

    /// Returns the name of the blaze action this action config applies to.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:686 CcToolchainFeatures.ActionConfig.getActionName
    pub fn get_action_name(&self) -> &str {
        &self.action_name
    }

    /// Returns the tool for this action given a set of enabled features.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:694 CcToolchainFeatures.ActionConfig.getTool
    /// `Err` holds the message of the `IllegalArgumentException` Java throws.
    fn get_tool(&self, enabled_feature_names: &IndexSet<String>) -> Result<&Tool, String> {
        match self.tools.iter().find(|t| {
            is_with_features_satisfied(t.get_with_feature_set_sets().iter(), enabled_feature_names)
        }) {
            Some(t) => Ok(t),
            None => Err(format!(
                "Matching tool for action {} not found for given feature configuration",
                self.get_action_name()
            )),
        }
    }

    /// Adds the flags that apply to this action to `command_line`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:706 CcToolchainFeatures.ActionConfig.expandCommandLine
    fn expand_command_line(
        &self,
        variables: &CcToolchainVariablesRef,
        enabled_feature_names: &IndexSet<String>,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        command_line: &mut Vec<String>,
    ) -> Result<(), ExpansionException> {
        for flag_set in &self.flag_sets {
            flag_set.expand_command_line(
                &self.action_name,
                variables,
                enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                command_line,
            )?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:724 CcToolchainFeatures.ActionConfig.isEnabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:728 CcToolchainFeatures.ActionConfig.getImplies
    pub fn get_implies(&self) -> &[String] {
        &self.implies
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:754 CcToolchainFeatures.ActionConfig.getTools
    pub fn get_tools(&self) -> &[Tool] {
        &self.tools
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:758 CcToolchainFeatures.ActionConfig.getFlagSets
    pub fn get_flag_sets(&self) -> &[FlagSet] {
        &self.flag_sets
    }
}

/// A feature or an action config: Java's `CrosstoolSelectable`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:394 CcToolchainFeatures.CrosstoolSelectable
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrosstoolSelectable {
    Feature(Arc<Feature>),
    ActionConfig(Arc<ActionConfig>),
}

impl CrosstoolSelectable {
    /// Returns the name of this selectable.
    pub fn get_name(&self) -> &str {
        match self {
            CrosstoolSelectable::Feature(f) => f.get_name(),
            CrosstoolSelectable::ActionConfig(a) => a.get_name(),
        }
    }
}

/// A description of how artifacts of a certain type are named.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:766 CcToolchainFeatures.ArtifactNamePattern
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtifactNamePattern {
    pub prefix: String,
    pub extension: String,
}

impl ArtifactNamePattern {
    /// Returns the artifact name that this pattern selects.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:769 CcToolchainFeatures.ArtifactNamePattern.getArtifactName
    fn get_artifact_name(&self, base_name: &str) -> String {
        format!("{}{}{}", self.prefix, base_name, self.extension)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:766 CcToolchainFeatures.ArtifactNamePattern.extension
    pub fn extension(&self) -> &str {
        &self.extension
    }
}

/// Maps artifact categories to their name patterns.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:774 CcToolchainFeatures.ArtifactNamePatternMapper
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArtifactNamePatternMapper {
    prefix_extension_overrides: IndexMap<ArtifactCategory, ArtifactNamePattern>,
}

impl ArtifactNamePatternMapper {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:789 CcToolchainFeatures.ArtifactNamePatternMapper.get
    pub fn get(&self, category: ArtifactCategory) -> ArtifactNamePattern {
        match self.prefix_extension_overrides.get(&category) {
            Some(result) => result.clone(),
            // DEFAULT_PATTERNS
            None => ArtifactNamePattern {
                prefix: category.get_default_prefix().to_string(),
                extension: category.get_default_extension().to_string(),
            },
        }
    }
}

/// Builder of [`ArtifactNamePatternMapper`].
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:794 CcToolchainFeatures.ArtifactNamePatternMapper.Builder
#[derive(Default)]
pub struct ArtifactNamePatternMapperBuilder {
    overrides: Vec<(ArtifactCategory, ArtifactNamePattern)>,
}

impl ArtifactNamePatternMapperBuilder {
    pub fn new() -> ArtifactNamePatternMapperBuilder {
        ArtifactNamePatternMapperBuilder::default()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:799 CcToolchainFeatures.ArtifactNamePatternMapper.Builder.addOverride
    pub fn add_override(&mut self, category: ArtifactCategory, prefix: &str, extension: &str) {
        if category.get_default_prefix() != prefix || category.get_default_extension() != extension
        {
            self.overrides.push((
                category,
                ArtifactNamePattern {
                    prefix: prefix.to_string(),
                    extension: extension.to_string(),
                },
            ));
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:807 CcToolchainFeatures.ArtifactNamePatternMapper.Builder.build
    pub fn build(self) -> ArtifactNamePatternMapper {
        let mut map = IndexMap::new();
        for (category, pattern) in self.overrides {
            if let Some(existing) = map.get(&category) {
                // ImmutableMap.Builder.buildOrThrow.
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {category}={pattern:?} \
                     and {category}={existing:?}"
                );
            }
            map.insert(category, pattern);
        }
        ArtifactNamePatternMapper {
            prefix_extension_overrides: map,
        }
    }
}

/// Captures the set of enabled features and action configs for a rule.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:817 CcToolchainFeatures.FeatureConfiguration
// PORT: the weak interner and the serialization codec are skipped.
#[derive(Clone, Debug)]
pub struct FeatureConfiguration {
    requested_features: IndexSet<String>,
    enabled_feature_names: IndexSet<String>,
    enabled_features: Vec<Arc<Feature>>,
    enabled_action_config_action_names: IndexSet<String>,
    action_config_by_action_name: Arc<IndexMap<String, Arc<ActionConfig>>>,
    cc_toolchain_path: PathFragment,
}

impl FeatureConfiguration {
    /// `FeatureConfiguration` instance that doesn't produce any command lines. This is to be used
    /// when creation of the real `FeatureConfiguration` failed, the rule error was reported, but
    /// the analysis continues to collect more rule errors.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:836 CcToolchainFeatures.FeatureConfiguration.EMPTY
    pub fn empty() -> FeatureConfiguration {
        FeatureConfiguration::new(
            IndexSet::new(),
            Vec::new(),
            IndexSet::new(),
            Arc::new(IndexMap::new()),
            PathFragment::empty_fragment(),
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:848 CcToolchainFeatures.FeatureConfiguration.<init>
    pub(crate) fn new(
        requested_features: IndexSet<String>,
        enabled_features: Vec<Arc<Feature>>,
        enabled_action_config_action_names: IndexSet<String>,
        action_config_by_action_name: Arc<IndexMap<String, Arc<ActionConfig>>>,
        cc_toolchain_path: PathFragment,
    ) -> FeatureConfiguration {
        let mut enabled_feature_names = IndexSet::new();
        for feature in &enabled_features {
            enabled_feature_names.insert(feature.get_name().to_string());
        }
        FeatureConfiguration {
            requested_features,
            enabled_feature_names,
            enabled_features,
            enabled_action_config_action_names,
            action_config_by_action_name,
            cc_toolchain_path,
        }
    }

    /// Returns whether the given `feature` is enabled.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:892 CcToolchainFeatures.FeatureConfiguration.isEnabled
    pub fn is_enabled(&self, feature: &str) -> bool {
        self.enabled_feature_names.contains(feature)
    }

    /// The list of requested features, even if they do not exist in CROSSTOOLs.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:897 CcToolchainFeatures.FeatureConfiguration.getRequestedFeatures
    pub fn get_requested_features(&self) -> &IndexSet<String> {
        &self.requested_features
    }

    /// Returns whether an action config for the blaze action with the given name is enabled.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:904 CcToolchainFeatures.FeatureConfiguration.actionIsConfigured
    pub fn action_is_configured(&self, action_name: &str) -> bool {
        self.enabled_action_config_action_names
            .contains(action_name)
    }

    /// Returns the command line for the given `action`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:911 CcToolchainFeatures.FeatureConfiguration.getCommandLine
    pub fn get_command_line(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
    ) -> Result<Vec<String>, ExpansionException> {
        self.get_command_line_with(action, variables, None, &NOOP)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:916 CcToolchainFeatures.FeatureConfiguration.getCommandLine
    pub fn get_command_line_with(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, ExpansionException> {
        let mut command_line = Vec::new();
        if self.action_is_configured(action) {
            self.action_config_by_action_name[action].expand_command_line(
                variables,
                &self.enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                &mut command_line,
            )?;
        }

        for feature in &self.enabled_features {
            feature.expand_command_line(
                action,
                variables,
                &self.enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                &mut command_line,
            )?;
        }

        Ok(command_line)
    }

    /// Returns the flags expanded for the given `action` in per-feature buckets.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:941 CcToolchainFeatures.FeatureConfiguration.getPerFeatureExpansions
    pub fn get_per_feature_expansions(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<(String, Vec<String>)>, ExpansionException> {
        self.get_per_feature_expansions_with(action, variables, None, path_mapper)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:947 CcToolchainFeatures.FeatureConfiguration.getPerFeatureExpansions
    pub fn get_per_feature_expansions_with(
        &self,
        action: &str,
        variables: &CcToolchainVariablesRef,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<(String, Vec<String>)>, ExpansionException> {
        let mut per_feature_expansions = Vec::new();
        if self.action_is_configured(action) {
            let mut command_line = Vec::new();
            let action_config = &self.action_config_by_action_name[action];
            action_config.expand_command_line(
                variables,
                &self.enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                &mut command_line,
            )?;
            per_feature_expansions.push((action_config.get_name().to_string(), command_line));
        }

        for feature in &self.enabled_features {
            let mut command_line = Vec::new();
            feature.expand_command_line(
                action,
                variables,
                &self.enabled_feature_names,
                input_metadata_provider,
                path_mapper,
                &mut command_line,
            )?;
            per_feature_expansions.push((feature.get_name().to_string(), command_line));
        }

        Ok(per_feature_expansions)
    }

    /// Returns the environment variables (key/value pairs) for the given `action`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:976 CcToolchainFeatures.FeatureConfiguration.getEnvironmentVariables
    pub fn get_environment_variables(
        &self,
        action: &str,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
    ) -> Result<IndexMap<String, String>, ExpansionException> {
        let mut env_builder = ImmutableMapBuilder::default();
        for feature in &self.enabled_features {
            feature.expand_environment(
                action,
                variables,
                path_mapper,
                &self.enabled_feature_names,
                &mut env_builder,
            )?;
        }
        Ok(env_builder.build_or_throw())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:986 CcToolchainFeatures.FeatureConfiguration.getToolPathForAction
    // Unchecked IllegalArgumentExceptions become panics; see `try_get_tool_path_for_action`.
    pub fn get_tool_path_for_action(&self, action_name: &str) -> String {
        self.try_get_tool_path_for_action(action_name)
            .unwrap_or_else(|e| panic!("IllegalArgumentException: {e}"))
    }

    /// Java's `getToolPathForAction` with its `IllegalArgumentException` message as `Err` (callers
    /// such as `cc_common.get_tool_for_action` catch it).
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:986 CcToolchainFeatures.FeatureConfiguration.getToolPathForAction
    pub fn try_get_tool_path_for_action(&self, action_name: &str) -> Result<String, String> {
        let Some(action_config) = self.action_config_by_action_name.get(action_name) else {
            return Err(format!(
                "Action {action_name} does not have an enabled configuration in the toolchain."
            ));
        };
        Ok(action_config
            .get_tool(&self.enabled_feature_names)?
            .get_tool_path_string(&self.cc_toolchain_path))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:995 CcToolchainFeatures.FeatureConfiguration.getToolRequirementsForAction
    pub fn get_tool_requirements_for_action(&self, action_name: &str) -> StringSet {
        blaze_util::check_argument!(
            self.action_config_by_action_name.contains_key(action_name),
            "Action {} does not have an enabled configuration in the toolchain.",
            action_name
        );
        let action_config = &self.action_config_by_action_name[action_name];
        action_config
            .get_tool(&self.enabled_feature_names)
            .unwrap_or_else(|e| panic!("IllegalArgumentException: {e}"))
            .get_execution_requirements()
            .clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1029 CcToolchainFeatures.FeatureConfiguration.getEnabledFeatureNames
    pub fn get_enabled_feature_names(&self) -> &IndexSet<String> {
        &self.enabled_feature_names
    }
}

impl PartialEq for FeatureConfiguration {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1005 CcToolchainFeatures.FeatureConfiguration.equals
    fn eq(&self, that: &Self) -> bool {
        // Only compare actionConfigByActionName, enabledActionConfigActionnames and
        // enabledFeatures because enabledFeatureNames is based on the list of Features.
        *self.action_config_by_action_name == *that.action_config_by_action_name
            && self
                .enabled_action_config_action_names
                .iter()
                .eq(that.enabled_action_config_action_names.iter())
            && self.enabled_features == that.enabled_features
    }
}

impl Eq for FeatureConfiguration {}

/// Provides access to features supported by a specific toolchain.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:80 CcToolchainFeatures
pub struct CcToolchainFeatures {
    artifact_name_patterns: ArtifactNamePatternMapper,

    /// All features and action configs in the order in which they were specified in the
    /// configuration.
    ///
    /// We guarantee the command line to be in the order in which the flags were specified in the
    /// configuration.
    selectables: Vec<CrosstoolSelectable>,

    /// Maps the selectables's name to the selectable (index into `selectables`).
    // PORT: Java copies a HashMap; its key order is only observable through
    // getActivatableNames (tests). Insertion order is used here.
    selectables_by_name: IndexMap<String, usize>,

    /// Maps an action's name to the ActionConfig.
    action_configs_by_action_name: Arc<IndexMap<String, Arc<ActionConfig>>>,

    /// Maps from a selectable to a set of all the selectables it has a direct 'implies' edge to.
    implies: Vec<Vec<usize>>,

    /// Maps from a selectable to all features that have an direct 'implies' edge to this
    /// selectable.
    implied_by: Vec<Vec<usize>>,

    /// Maps from a selectable to a set of selectable sets, where a selectable set satisfies the
    /// 'requires' condition if all selectables in it are enabled, and the 'requires' condition is
    /// satisfied if at least one of the selectable sets satisfies it.
    requires: Vec<Vec<IndexSet<usize>>>,

    /// Maps from a string to the set of selectables that 'provide' it.
    provides: IndexMap<String, Vec<usize>>,

    /// Maps from a selectable to all selectables that have a requirement referencing it.
    required_by: Vec<Vec<usize>>,

    default_selectables: Vec<String>,

    /// A cache of feature selection results, so we do not recalculate the feature selection for
    /// all actions.
    // PORT: Caffeine's 10000-entry bound is not reproduced (the cache is unbounded); keys are
    // compared as sets, as ImmutableSet.equals does.
    configuration_cache: Mutex<HashMap<BTreeSet<String>, Arc<FeatureConfiguration>>>,

    cc_toolchain_path: PathFragment,
}

impl CcToolchainFeatures {
    /// Constructs the feature configuration from a [`CcToolchainConfigInfo`].
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1099 CcToolchainFeatures.<init>
    pub fn new(
        cc_toolchain_config_info: &CcToolchainConfigInfo,
        cc_toolchain_path: PathFragment,
    ) -> Result<CcToolchainFeatures, EvalException> {
        let features = cc_toolchain_config_info.get_features()?;
        let action_configs = cc_toolchain_config_info.get_action_configs()?;
        let artifact_name_patterns = cc_toolchain_config_info.get_artifact_name_patterns()?;
        CcToolchainFeatures::from_parts(
            features,
            action_configs,
            artifact_name_patterns,
            cc_toolchain_path,
        )
    }

    /// The constructor body, from already converted features and action configs.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1099 CcToolchainFeatures.<init>
    pub fn from_parts(
        features: Vec<Feature>,
        action_configs: Vec<ActionConfig>,
        artifact_name_patterns: ArtifactNamePatternMapper,
        cc_toolchain_path: PathFragment,
    ) -> Result<CcToolchainFeatures, EvalException> {
        // Build up the feature/action config graph.  We refer to features/action configs as
        // 'selectables'.
        // First, we build up the map of name -> selectables in one pass, so that earlier
        // selectables can reference later features in their configuration.
        let mut selectables: Vec<CrosstoolSelectable> = Vec::new();
        let mut selectables_by_name: IndexMap<String, usize> = IndexMap::new();

        // Also build a map from action -> action_config, for use in tool lookups
        let mut action_configs_by_action_name_builder: Vec<(String, Arc<ActionConfig>)> =
            Vec::new();

        let mut default_selectables = Vec::new();
        let features: Vec<Arc<Feature>> = features.into_iter().map(Arc::new).collect();
        for feature in &features {
            selectables.push(CrosstoolSelectable::Feature(feature.clone()));
            selectables_by_name.insert(feature.get_name().to_string(), selectables.len() - 1);
            if feature.is_enabled() {
                default_selectables.push(feature.get_name().to_string());
            }
        }

        let action_configs: Vec<Arc<ActionConfig>> =
            action_configs.into_iter().map(Arc::new).collect();
        for action_config in &action_configs {
            selectables.push(CrosstoolSelectable::ActionConfig(action_config.clone()));
            selectables_by_name.insert(action_config.get_name().to_string(), selectables.len() - 1);
            action_configs_by_action_name_builder.push((
                action_config.get_action_name().to_string(),
                action_config.clone(),
            ));
            if action_config.is_enabled() {
                default_selectables.push(action_config.get_name().to_string());
            }
        }

        check_for_action_name_dups(&action_configs)?;
        check_for_activatable_dups(&selectables)?;

        let action_configs_by_action_name: IndexMap<String, Arc<ActionConfig>> =
            action_configs_by_action_name_builder.into_iter().collect();

        let n = selectables.len();
        let mut this = CcToolchainFeatures {
            artifact_name_patterns,
            selectables,
            selectables_by_name,
            action_configs_by_action_name: Arc::new(action_configs_by_action_name),
            implies: vec![Vec::new(); n],
            implied_by: vec![Vec::new(); n],
            requires: vec![Vec::new(); n],
            provides: IndexMap::new(),
            required_by: vec![Vec::new(); n],
            default_selectables,
            configuration_cache: Mutex::new(HashMap::new()),
            cc_toolchain_path,
        };

        // Next, we build up all forward references for 'implies', 'requires', and 'provides'
        // edges.
        let mut provides_forward: Vec<(usize, String)> = Vec::new();
        for feature in &features {
            let name = feature.get_name();
            let selectable = this.selectables_by_name[name];
            for required_features in feature.get_requires() {
                let mut all_of = IndexSet::new();
                for required_name in required_features.iter() {
                    let required = this.get_activatable_or_fail(required_name, name)?;
                    all_of.insert(required);
                    this.required_by[required].push(selectable);
                }
                this.requires[selectable].push(all_of);
            }
            for implied_name in feature.get_implies() {
                let implied = this.get_activatable_or_fail(implied_name, name)?;
                this.implied_by[implied].push(selectable);
                this.implies[selectable].push(implied);
            }
            for provides_name in feature.get_provides() {
                provides_forward.push((selectable, provides_name.clone()));
            }
        }

        for action_config in &action_configs {
            let name = action_config.get_name();
            let selectable = this.selectables_by_name[name];
            for implied_name in action_config.get_implies() {
                let implied = this.get_activatable_or_fail(implied_name, name)?;
                this.implied_by[implied].push(selectable);
                this.implies[selectable].push(implied);
            }
        }

        // provides.build().inverse(): keys in order of first appearance.
        for (selectable, provided) in provides_forward {
            this.provides.entry(provided).or_default().push(selectable);
        }
        Ok(this)
    }

    /// Java: `configure_features(requested_features)` (Starlark method). The private-API allowlist
    /// check is the Starlark wrapper's job.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1234 CcToolchainFeatures.configureFeatures
    pub fn configure_features(
        &self,
        requested_features: &[String],
    ) -> Result<FeatureConfigurationForStarlark, EvalException> {
        let requested: IndexSet<String> = requested_features.iter().cloned().collect();
        match self.get_feature_configuration(&requested) {
            Ok(fc) => Ok(FeatureConfigurationForStarlark::from(fc)),
            Err(ex) => Err(ex.into()),
        }
    }

    /// Given a list of `requested_selectables`, returns all features that are enabled by the
    /// toolchain configuration.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1260 CcToolchainFeatures.getFeatureConfiguration
    pub fn get_feature_configuration(
        &self,
        requested_selectables: &IndexSet<String>,
    ) -> Result<Arc<FeatureConfiguration>, CollidingProvidesException> {
        let key: BTreeSet<String> = requested_selectables.iter().cloned().collect();
        if let Some(cached) = self.configuration_cache.lock().get(&key) {
            return Ok(cached.clone());
        }
        let computed = Arc::new(self.compute_feature_configuration(requested_selectables)?);
        Ok(self
            .configuration_cache
            .lock()
            .entry(key)
            .or_insert(computed)
            .clone())
    }

    /// Given `requested_selectables`, returns a FeatureConfiguration with all requested features
    /// enabled.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1283 CcToolchainFeatures.computeFeatureConfiguration
    pub fn compute_feature_configuration(
        &self,
        requested_selectables: &IndexSet<String>,
    ) -> Result<FeatureConfiguration, CollidingProvidesException> {
        // Command line flags will be output in the order in which they are specified in the
        // toolchain configuration.
        FeatureSelection::new(
            requested_selectables.clone(),
            &self.selectables_by_name,
            &self.selectables,
            &self.provides,
            &self.implies,
            &self.implied_by,
            &self.requires,
            &self.required_by,
            self.action_configs_by_action_name.clone(),
            self.cc_toolchain_path.clone(),
        )
        .run()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1305 CcToolchainFeatures.getDefaultFeaturesAndActionConfigsForStarlark
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1311 CcToolchainFeatures.getDefaultFeaturesAndActionConfigs
    pub fn get_default_features_and_action_configs(&self) -> &[String] {
        &self.default_selectables
    }

    /// Returns the selectable with the given `name`, or an error if no selectable with the given
    /// name was configured.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1319 CcToolchainFeatures.getActivatableOrFail
    fn get_activatable_or_fail(&self, name: &str, reference: &str) -> Result<usize, EvalException> {
        match self.selectables_by_name.get(name) {
            Some(&i) => Ok(i),
            None => Err(EvalException::new(format!(
                "Invalid toolchain configuration: feature '{}', which is referenced from feature \
                 '{}', is not defined.",
                name, reference
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1331 CcToolchainFeatures.getActivatableNames
    pub fn get_activatable_names(&self) -> Vec<String> {
        self.selectables_by_name.keys().cloned().collect()
    }

    /// Returns the artifact selected by the toolchain for the given action type and action
    /// category.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1338 CcToolchainFeatures.getArtifactNameForCategory
    pub fn get_artifact_name_for_category(
        &self,
        artifact_category: ArtifactCategory,
        output_name: &str,
    ) -> String {
        let output = PathFragment::create(output_name);
        let parent = output
            .get_parent_directory()
            .unwrap_or_else(|| panic!("NullPointerException"));
        parent
            .get_child(
                &self
                    .artifact_name_patterns
                    .get(artifact_category)
                    .get_artifact_name(output.get_base_name()),
            )
            .get_path_string()
            .to_string()
    }

    /// Returns the artifact name extension selected by the toolchain for the given artifact
    /// category.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1349 CcToolchainFeatures.getArtifactNameExtensionForCategory
    pub fn get_artifact_name_extension_for_category(
        &self,
        artifact_category: ArtifactCategory,
    ) -> String {
        self.artifact_name_patterns
            .get(artifact_category)
            .extension()
            .to_string()
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1194 CcToolchainFeatures.checkForActivatableDups
fn check_for_activatable_dups(selectables: &[CrosstoolSelectable]) -> Result<(), EvalException> {
    let mut names = std::collections::HashSet::new();
    for selectable in selectables {
        if !names.insert(selectable.get_name()) {
            return Err(EvalException::new(format!(
                "Invalid toolchain configuration: feature or action config '{}' was specified \
                 multiple times.",
                selectable.get_name()
            )));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:1207 CcToolchainFeatures.checkForActionNameDups
fn check_for_action_name_dups(action_configs: &[Arc<ActionConfig>]) -> Result<(), EvalException> {
    let mut action_names = std::collections::HashSet::new();
    for action_config in action_configs {
        if !action_names.insert(action_config.get_action_name()) {
            return Err(EvalException::new(format!(
                "Invalid toolchain configuration: multiple action configs for action '{}'",
                action_config.get_action_name()
            )));
        }
    }
    Ok(())
}
