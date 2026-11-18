//! Ports of `analysis/configuredtargets/*` (`AbstractConfiguredTarget`, `RuleConfiguredTarget`,
//! `FileConfiguredTarget` with `InputFileConfiguredTarget` and `OutputFileConfiguredTarget`,
//! `PackageGroupConfiguredTarget`, `EnvironmentGroupConfiguredTarget`,
//! `MergedConfiguredTarget`), `rules/AliasConfiguredTarget` and `analysis/Util.findImplicitDeps`,
//! plus the `AbstractConfiguredTarget` lookups every configured target shares (`get`,
//! `getProvider`, `DefaultInfo.build`) as functions over `crate::configured_target::ConfiguredTarget`.
//!
//! Each struct's `providers()` is its `TransitiveInfoProviderMap`. For the classes whose Java
//! `getProvider` synthesizes providers (file targets, package groups), the map holds those
//! providers, in the order Java's `getProvidersDictForQuery` lists them, so the lead's
//! `ConfiguredTarget::get_provider` finds them.

pub mod target_value;

use std::sync::{Arc, OnceLock};

use blaze_actions::{ActionLookupKeyRef, ActionRef, Artifact};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::rule_class::{
    AdvertisedProviderSet, ProviderKey, RequiredProviders, StarlarkProviderIdentifier,
};
use blaze_packages::starlark_defs::builtin_providers::{
    DEFAULT_INFO, INSTRUMENTED_FILES_INFO, OUTPUT_GROUP_INFO,
};
use blaze_packages::target::{PackageGroup, RuleOrMacroInstance};
use blaze_util::{check_argument, check_state};
use indexmap::{IndexMap, IndexSet};
use starlark::values::OwnedFrozenValue;

use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::configured_target::{ConfiguredTarget, ConfiguredTargetBase};
use crate::configured_target_key::ConfiguredTargetKey;
use crate::info::downcast_info;
use crate::provider_map::{
    ProviderInstance, ProviderMapKey, TransitiveInfoProvider, TransitiveInfoProviderMap,
    TransitiveInfoProviderMapBuilder,
};
use crate::providers::alias_provider::AliasProvider;
use crate::providers::default_info;
use crate::providers::file_provider::{FileProvider, FilesToRunProvider, RunfilesProvider};
use crate::providers::output_group_info::{self, OutputGroupInfo};
use crate::providers::package_specification_provider::{
    PackageSpecificationProvider, TransitiveVisibilityProvider,
};
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::providers::visibility::{
    Visibility, VisibilityProvider, VisibilityProviderData, private_visibility, public_visibility,
};
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::rule::target_context::TargetContext;
use crate::toolchains::toolchain_context::ToolchainContext;

fn base_of(lookup_key: ActionLookupKeyRef, visibility: Visibility) -> ConfiguredTargetBase {
    ConfiguredTargetBase {
        lookup_key,
        visibility: Some(Arc::new(VisibilityProviderData::new(visibility))),
    }
}

fn visibility_of(base: &ConfiguredTargetBase) -> Visibility {
    base.visibility
        .as_ref()
        .map(|v| v.visibility.clone())
        .unwrap_or_else(private_visibility)
}

/// Marks targets instantiated in a symbolic macro (Java
/// `RuleConfiguredTarget.CreatedInSymbolicMacroMarker`).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:178 RuleConfiguredTarget.CreatedInSymbolicMacroMarker
#[derive(Clone, Debug)]
pub struct CreatedInSymbolicMacroMarker;

impl TransitiveInfoProvider for CreatedInSymbolicMacroMarker {
    fn java_class_name(&self) -> &'static str {
        "CreatedInSymbolicMacroMarker"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// -------------------------------------------------------------------------------------------
// RuleConfiguredTarget

/// A `ConfiguredTarget` that is produced by a rule.
///
/// Created by `RuleConfiguredTargetBuilder`. There is an instance of this class for every
/// analyzed rule. For more information about how analysis works, see
/// `RuleConfiguredTargetFactory`.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:63 RuleConfiguredTarget
#[derive(Debug)]
pub struct RuleConfiguredTarget {
    pub base: ConfiguredTargetBase,
    /// The configuration transition keys of implicit deps (Java `implicitDeps`).
    implicit_deps: IndexSet<ConfiguredTargetKey>,
    providers: TransitiveInfoProviderMap,
    config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
    /// Java `ruleClassId.name()`.
    rule_class_name: String,
    actions: Vec<ActionRef>,
    default_info: OnceLock<OwnedFrozenValue>,
}

impl RuleConfiguredTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:86 RuleConfiguredTarget.<init>
    fn new_internal(
        action_lookup_key: ActionLookupKeyRef,
        visibility: Visibility,
        is_created_in_symbolic_macro: bool,
        providers: TransitiveInfoProviderMap,
        config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
        implicit_deps: IndexSet<ConfiguredTargetKey>,
        rule_class_name: String,
        actions: Vec<ActionRef>,
    ) -> RuleConfiguredTarget {
        let mut provider_builder = TransitiveInfoProviderMapBuilder::default();
        provider_builder.add_all(&providers);
        if is_created_in_symbolic_macro {
            provider_builder.add_native(CreatedInSymbolicMacroMarker);
        }
        check_state!(
            provider_builder.contains_native::<RunfilesProvider>(),
            "{}",
            action_lookup_key
        );
        check_state!(
            provider_builder.contains_native::<FileProvider>(),
            "{}",
            action_lookup_key
        );
        check_state!(
            provider_builder.contains_native::<FilesToRunProvider>(),
            "{}",
            action_lookup_key
        );
        // PORT: StarlarkApiProvider.init (legacy struct-field providers of native rules) has no
        // ported instances.
        RuleConfiguredTarget {
            base: base_of(action_lookup_key, visibility),
            implicit_deps,
            providers: provider_builder.build(),
            config_conditions,
            rule_class_name,
            actions,
            default_info: OnceLock::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:115 RuleConfiguredTarget.<init>
    pub fn new(
        rule_context: &RuleContext,
        providers: TransitiveInfoProviderMap,
        actions: Vec<ActionRef>,
    ) -> RuleConfiguredTarget {
        let t = Self::new_internal(
            rule_context.get_owner().clone(),
            rule_context.get_visibility().clone(),
            /* is_created_in_symbolic_macro= */
            blaze_packages::target::Target::Rule(rule_context.get_rule().clone())
                .is_created_in_symbolic_macro(),
            providers,
            rule_context.get_config_conditions().clone(),
            find_implicit_deps(rule_context),
            rule_context
                .get_rule()
                .get_rule_class_object()
                .get_name()
                .to_owned(),
            actions,
        );
        // If this rule is the run_under target, then check that we have an executable; note
        // that this can only be set to a non-null value if the target is run in the target
        // configuration and the rule is an executable rule.
        if !rule_context.has_errors() {
            for out in rule_context.get_rule().get_output_files() {
                rule_context.create_output_artifact_for(&out);
            }
        }
        t
    }

    /// Constructor for rules that are incompatible with the target platform (no actions; the
    /// providers include `IncompatiblePlatformProvider`).
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:135 RuleConfiguredTarget.<init>
    pub fn new_incompatible(
        action_lookup_key: ActionLookupKeyRef,
        visibility: Visibility,
        is_created_in_symbolic_macro: bool,
        providers: TransitiveInfoProviderMap,
        config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
        rule_class_name: String,
    ) -> RuleConfiguredTarget {
        Self::new_internal(
            action_lookup_key,
            visibility,
            is_created_in_symbolic_macro,
            providers,
            config_conditions,
            IndexSet::new(),
            rule_class_name,
            Vec::new(),
        )
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:185 RuleConfiguredTarget.isCreatedInSymbolicMacro
    pub fn is_created_in_symbolic_macro(&self) -> bool {
        self.providers
            .get_provider::<CreatedInSymbolicMacroMarker>()
            .is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:190 RuleConfiguredTarget.getConfigConditions
    pub fn get_config_conditions(&self) -> &Arc<IndexMap<Label, ConfigMatchingProvider>> {
        &self.config_conditions
    }

    /// The implicit dependencies (Java `getImplicitDeps()`), used by cquery's
    /// `--noimplicit_deps`.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:200 RuleConfiguredTarget.getImplicitDeps
    pub fn get_implicit_deps(&self) -> &IndexSet<ConfiguredTargetKey> {
        &self.implicit_deps
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:205 RuleConfiguredTarget.getRuleClassString
    pub fn get_rule_class_string(&self) -> &str {
        &self.rule_class_name
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:296 RuleConfiguredTarget.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }

    /// Returns the output artifact of an output file of this rule.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:300 RuleConfiguredTarget.findArtifactByOutputLabel
    pub fn find_artifact_by_output_label(&self, output_label: &Label) -> Artifact {
        let label = self.base.lookup_key.get_label().expect("label");
        check_argument!(
            output_label.get_package_identifier() == label.get_package_identifier(),
            "{} not in same package as {}",
            output_label,
            label
        );
        let relative_output_path = output_label.to_path_fragment();
        for action in &self.actions {
            for output in action.get_outputs() {
                if output.get_exec_path().ends_with(&relative_output_path) {
                    return output;
                }
            }
        }
        panic!(
            "IllegalArgumentException: No output matching {} in ConfiguredTarget({}, {})",
            output_label,
            label,
            self.base
                .lookup_key
                .get_configuration_checksum()
                .unwrap_or_else(|| "null".to_owned())
        );
    }
}

/// Returns the implicit dependencies of a rule (attribute values not explicitly specified, the
/// target platform and resolved toolchains), minus the explicit ones.
// Java: com/google/devtools/build/lib/analysis/Util.java:63 Util.findImplicitDeps
pub fn find_implicit_deps(rule_context: &RuleContext) -> IndexSet<ConfiguredTargetKey> {
    let mut maybe_implicit_deps: IndexSet<ConfiguredTargetKey> = IndexSet::new();
    let mut explicit_deps: IndexSet<ConfiguredTargetKey> = IndexSet::new();
    let attributes = rule_context.attributes();
    for attr_name in attributes.get_attribute_names() {
        let attr_values = rule_context.get_prerequisite_configured_targets(&attr_name);
        if !attr_values.is_empty() {
            let set = if attributes.is_attribute_value_explicitly_specified(&attr_name) {
                &mut explicit_deps
            } else {
                &mut maybe_implicit_deps
            };
            // Java: Util.addLabelsAndConfigs.
            for dep in attr_values {
                set.insert(
                    ConfiguredTargetKey::builder()
                        .set_label(dep.get_configured_target().get_original_label())
                        .set_configuration(dep.get_configuration().map(|c| c.as_ref()))
                        .build(),
                );
            }
        }
    }

    if rule_context.get_rule().use_toolchain_resolution() {
        if let Some(platform_configuration) = rule_context
            .get_configuration()
            .get_fragment::<blaze_build_config::fragments::platform_configuration::PlatformConfiguration>()
        {
            maybe_implicit_deps.insert(
                ConfiguredTargetKey::builder()
                    .set_label(platform_configuration.get_target_platform().clone())
                    .set_configuration_key(Some(BuildConfigurationKey::create(Arc::new(
                        blaze_build_config::transitions::EMPTY_OPTIONS.clone(),
                    ))))
                    .build(),
            );
        }
    }

    if let Some(toolchain_contexts) = rule_context.get_toolchain_contexts() {
        for toolchain_context in toolchain_contexts.context_map().values() {
            let target_configuration = rule_context.get_configuration();
            for toolchain in toolchain_context.resolved_toolchain_labels() {
                maybe_implicit_deps.insert(
                    ConfiguredTargetKey::builder()
                        .set_label(toolchain.clone())
                        .set_configuration(Some(target_configuration.as_ref()))
                        .set_execution_platform_label(Some(
                            toolchain_context.execution_platform().label().clone(),
                        ))
                        .build(),
                );
            }
        }
    }
    // Java: ImmutableSet.copyOf(Sets.difference(maybeImplicitDeps, explicitDeps)).
    maybe_implicit_deps
        .into_iter()
        .filter(|k| !explicit_deps.contains(k))
        .collect()
}

// -------------------------------------------------------------------------------------------
// File targets

/// The common part of input and output file targets (Java `FileConfiguredTarget`).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:39 FileConfiguredTarget
fn file_target_providers(
    visibility: &Visibility,
    is_created_in_symbolic_macro: bool,
    artifact: &Artifact,
    transitive_visibility: Option<TransitiveVisibilityProvider>,
) -> (NestedSet<Artifact>, TransitiveInfoProviderMapBuilder) {
    let single_file = NestedSetBuilder::create(Order::STABLE_ORDER, [artifact.clone()]);
    let mut b = TransitiveInfoProviderMapBuilder::default();
    // Java: FileConfiguredTarget.getProvidersDictForQuery order: VisibilityProvider,
    // FileProvider, FilesToRunProvider.
    b.add_native(VisibilityProvider::new(
        visibility.clone(),
        is_created_in_symbolic_macro,
    ));
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:94 FileConfiguredTarget.createFileProvider
    b.add_native_arc(FileProvider::of(single_file.clone()));
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:98 FileConfiguredTarget.createFilesToRunProvider
    b.add_native_arc(FilesToRunProvider::create(
        single_file.clone(),
        None,
        Some(artifact.clone()),
    ));
    if let Some(tv) = transitive_visibility {
        b.add_native(tv);
    }
    (single_file, b)
}

/// A `ConfiguredTarget` for an input file.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/InputFileConfiguredTarget.java:37 InputFileConfiguredTarget
#[derive(Debug)]
pub struct InputFileConfiguredTarget {
    pub base: ConfiguredTargetBase,
    single_file: NestedSet<Artifact>,
    is_created_in_symbolic_macro: bool,
    providers: TransitiveInfoProviderMap,
    default_info: OnceLock<OwnedFrozenValue>,
}

impl InputFileConfiguredTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/InputFileConfiguredTarget.java:42 InputFileConfiguredTarget.<init>
    pub fn new(target_context: &TargetContext, artifact: Artifact) -> InputFileConfiguredTarget {
        check_argument!(
            target_context.get_target().is_input_file(),
            "{}",
            target_context.get_target()
        );
        let is_created_in_symbolic_macro =
            target_context.get_target().is_created_in_symbolic_macro();
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/InputFileConfiguredTarget.java:79 InputFileConfiguredTarget.createTransitiveVisibilityProvider
        let tv = target_context
            .get_transitive_visibility_imposed_by_this_package()
            .map(|p| TransitiveVisibilityProvider::new(vec![p.clone()]));
        let (single_file, b) = file_target_providers(
            target_context.get_visibility(),
            is_created_in_symbolic_macro,
            &artifact,
            tv,
        );
        let t = InputFileConfiguredTarget {
            base: base_of(
                target_context
                    .get_analysis_environment()
                    .get_owner()
                    .clone(),
                target_context.get_visibility().clone(),
            ),
            single_file,
            is_created_in_symbolic_macro,
            providers: b.build(),
            default_info: OnceLock::new(),
        };
        // Java: checkArgument(getConfigurationKey() == null, getLabel()).
        check_argument!(
            t.base.lookup_key.get_configuration_checksum().is_none(),
            "{}",
            target_context.get_label()
        );
        t
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:58 FileConfiguredTarget.getArtifact
    pub fn get_artifact(&self) -> &Artifact {
        self.single_file.get_singleton()
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/InputFileConfiguredTarget.java:68 InputFileConfiguredTarget.isCreatedInSymbolicMacro
    pub fn is_created_in_symbolic_macro(&self) -> bool {
        self.is_created_in_symbolic_macro
    }
}

/// A `ConfiguredTarget` for an output file.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:39 OutputFileConfiguredTarget
#[derive(Debug)]
pub struct OutputFileConfiguredTarget {
    pub base: ConfiguredTargetBase,
    single_file: NestedSet<Artifact>,
    generating_rule: Arc<RuleConfiguredTarget>,
    providers: TransitiveInfoProviderMap,
    default_info: OnceLock<OwnedFrozenValue>,
}

impl OutputFileConfiguredTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:44 OutputFileConfiguredTarget.<init>
    pub fn new(
        target_context: &TargetContext,
        output_artifact: Artifact,
        generating_rule: Arc<RuleConfiguredTarget>,
    ) -> OutputFileConfiguredTarget {
        check_argument!(
            target_context.get_target().is_output_file(),
            "{}",
            target_context.get_target()
        );
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:75 OutputFileConfiguredTarget.createTransitiveVisibilityProvider
        let tv = generating_rule
            .providers()
            .get_provider::<TransitiveVisibilityProvider>()
            .cloned();
        let (single_file, mut b) = file_target_providers(
            target_context.get_visibility(),
            generating_rule.is_created_in_symbolic_macro(),
            &output_artifact,
            tv,
        );
        // Java: OutputFileConfiguredTarget.rawGetStarlarkProvider and getProvidersDictForQuery:
        // InstrumentedFilesInfo, the validation OutputGroupInfo, and the rule's
        // RequiredConfigFragmentsProvider.
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:89 OutputFileConfiguredTarget.rawGetStarlarkProvider
        let instrumented = generating_rule
            .providers()
            .get(&INSTRUMENTED_FILES_INFO.key())
            .cloned()
            .unwrap_or_else(crate::test::instrumented_files_info::empty_instrumented_files_info);
        b.put_info(instrumented);
        if let Some(ogi) = generating_rule.providers().get(&OUTPUT_GROUP_INFO.key()) {
            let validation_outputs = downcast_info::<OutputGroupInfo>(ogi)
                .expect("OutputGroupInfo")
                .get_output_group(output_group_info::VALIDATION);
            if !validation_outputs.is_empty() {
                b.put_info(output_group_info::freeze(OutputGroupInfo::single_group(
                    output_group_info::VALIDATION,
                    validation_outputs,
                )));
            }
        }
        if let Some(p) = generating_rule
            .providers()
            .get_provider::<RequiredConfigFragmentsProvider>()
        {
            b.add_native(p.clone());
        }
        OutputFileConfiguredTarget {
            base: base_of(
                target_context
                    .get_analysis_environment()
                    .get_owner()
                    .clone(),
                target_context.get_visibility().clone(),
            ),
            single_file,
            generating_rule,
            providers: b.build(),
            default_info: OnceLock::new(),
        }
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:58 FileConfiguredTarget.getArtifact
    pub fn get_artifact(&self) -> &Artifact {
        self.single_file.get_singleton()
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:62 OutputFileConfiguredTarget.getGeneratingRule
    pub fn get_generating_rule(&self) -> &Arc<RuleConfiguredTarget> {
        &self.generating_rule
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:67 OutputFileConfiguredTarget.isCreatedInSymbolicMacro
    pub fn is_created_in_symbolic_macro(&self) -> bool {
        self.generating_rule.is_created_in_symbolic_macro()
    }
}

// -------------------------------------------------------------------------------------------
// Package groups and environment groups

/// Dummy ConfiguredTarget for package groups. Contains no functionality, since package groups
/// are not really first-class Targets.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/PackageGroupConfiguredTarget.java:36 PackageGroupConfiguredTarget
#[derive(Debug)]
pub struct PackageGroupConfiguredTarget {
    pub base: ConfiguredTargetBase,
    package_specification_provider: Arc<PackageSpecificationProvider>,
    providers: TransitiveInfoProviderMap,
    default_info: OnceLock<OwnedFrozenValue>,
}

impl PackageGroupConfiguredTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/PackageGroupConfiguredTarget.java:52 PackageGroupConfiguredTarget.<init>
    pub fn new(
        action_lookup_key: ActionLookupKeyRef,
        target_context: &TargetContext,
        package_group: &PackageGroup,
    ) -> PackageGroupConfiguredTarget {
        Self::from_provider(
            action_lookup_key,
            Arc::new(PackageSpecificationProvider::create(
                target_context,
                package_group,
            )),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/PackageGroupConfiguredTarget.java:59 PackageGroupConfiguredTarget.<init>
    pub fn from_provider(
        action_lookup_key: ActionLookupKeyRef,
        package_specification_provider: Arc<PackageSpecificationProvider>,
    ) -> PackageGroupConfiguredTarget {
        let mut b = TransitiveInfoProviderMapBuilder::default();
        // Java: PackageGroupConfiguredTarget.getProvider (FileProvider.EMPTY, the
        // PackageSpecificationProvider, and itself as the VisibilityProvider) and
        // rawGetStarlarkProvider (the PackageSpecificationInfo).
        b.add_native(VisibilityProvider::new(public_visibility(), false));
        b.add_native_arc(FileProvider::empty());
        b.add_native_arc(package_specification_provider.clone());
        b.put_info(crate::info::freeze_native_info(
            package_specification_provider.as_ref().clone(),
        ));
        PackageGroupConfiguredTarget {
            base: base_of(action_lookup_key, public_visibility()),
            package_specification_provider,
            providers: b.build(),
            default_info: OnceLock::new(),
        }
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    pub fn get_package_specification_provider(&self) -> &Arc<PackageSpecificationProvider> {
        &self.package_specification_provider
    }
}

/// Dummy ConfiguredTarget for environment groups. Contains no functionality, since environment
/// groups are not really first-class Targets.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/EnvironmentGroupConfiguredTarget.java:30 EnvironmentGroupConfiguredTarget
#[derive(Debug)]
pub struct EnvironmentGroupConfiguredTarget {
    pub base: ConfiguredTargetBase,
    providers: TransitiveInfoProviderMap,
    default_info: OnceLock<OwnedFrozenValue>,
}

impl EnvironmentGroupConfiguredTarget {
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/EnvironmentGroupConfiguredTarget.java:33 EnvironmentGroupConfiguredTarget.<init>
    pub fn new(action_lookup_key: ActionLookupKeyRef) -> EnvironmentGroupConfiguredTarget {
        check_state!(
            action_lookup_key.get_configuration_checksum().is_none(),
            "{}",
            action_lookup_key
        );
        let mut b = TransitiveInfoProviderMapBuilder::default();
        b.add_native(VisibilityProvider::new(private_visibility(), false));
        EnvironmentGroupConfiguredTarget {
            base: base_of(action_lookup_key, private_visibility()),
            providers: b.build(),
            default_info: OnceLock::new(),
        }
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }
}

// -------------------------------------------------------------------------------------------
// Alias and merged targets

/// This configured target pretends to be whatever type of target "actual" is, returning its
/// label, transitive info providers and target, but returning its own visibility.
// Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:54 AliasConfiguredTarget
#[derive(Debug)]
pub struct AliasConfiguredTarget {
    pub base: ConfiguredTargetBase,
    pub actual: ConfiguredTarget,
    /// Java `overrides` merged over the actual target's providers (overrides win).
    providers: TransitiveInfoProviderMap,
    config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
}

impl AliasConfiguredTarget {
    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:56 AliasConfiguredTarget.create
    pub fn create(
        rule_context: &RuleContext,
        actual: ConfiguredTarget,
        visibility: Visibility,
    ) -> AliasConfiguredTarget {
        Self::create_with_overrides(
            rule_context,
            actual,
            visibility,
            TransitiveInfoProviderMap::default(),
        )
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:64 AliasConfiguredTarget.createWithOverrides
    pub fn create_with_overrides(
        rule_context: &RuleContext,
        actual: ConfiguredTarget,
        visibility: Visibility,
        overrides: TransitiveInfoProviderMap,
    ) -> AliasConfiguredTarget {
        let mut all = TransitiveInfoProviderMapBuilder::default();
        all.add_all(&overrides);
        all.add_native(AliasProvider::from_alias_rule(
            rule_context.get_rule(),
            &actual,
        ));
        all.add_native(VisibilityProvider::new(
            visibility.clone(),
            blaze_packages::target::Target::Rule(rule_context.get_rule().clone())
                .is_created_in_symbolic_macro(),
        ));
        if let Some(r) = rule_context.get_required_config_fragments() {
            all.add_native(r);
        }
        Self::new(
            rule_context.get_owner().clone(),
            actual,
            all.build(),
            rule_context.get_config_conditions().clone(),
            visibility,
        )
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:93 AliasConfiguredTarget.<init>
    pub fn new(
        action_lookup_key: ActionLookupKeyRef,
        actual: ConfiguredTarget,
        overrides: TransitiveInfoProviderMap,
        config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
        visibility: Visibility,
    ) -> AliasConfiguredTarget {
        // Java: getProvider returns the override, else the actual target's provider.
        let mut merged = TransitiveInfoProviderMapBuilder::default();
        merged.add_all(actual.providers());
        merged.add_all(&overrides);
        AliasConfiguredTarget {
            base: base_of(action_lookup_key, visibility),
            actual,
            providers: merged.build(),
            config_conditions,
        }
    }

    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:107 AliasConfiguredTarget.getActualNoFollow
    pub fn get_actual_no_follow(&self) -> &ConfiguredTarget {
        &self.actual
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:170 AliasConfiguredTarget.getActual
    pub fn get_actual(&self) -> ConfiguredTarget {
        self.actual.get_actual()
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:176 AliasConfiguredTarget.getOriginalLabel
    pub fn get_original_label(&self) -> Label {
        self.base.lookup_key.get_label().expect("label")
    }

    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:122 AliasConfiguredTarget.getConfigConditions
    pub fn get_config_conditions(&self) -> &Arc<IndexMap<Label, ConfigMatchingProvider>> {
        &self.config_conditions
    }
}

/// A single dependency with its configured target and aspects merged together.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/MergedConfiguredTarget.java:61 MergedConfiguredTarget
#[derive(Debug)]
pub struct MergedConfiguredTarget {
    pub base: ConfiguredTargetBase,
    pub base_configured_target: ConfiguredTarget,
    /// The base target's providers overridden by the aspects' (Java `nonBaseProviders` first).
    pub providers: TransitiveInfoProviderMap,
}

impl MergedConfiguredTarget {
    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    /// Creates an instance based on a configured target and a set of aspects.
    // Java: com/google/devtools/build/lib/analysis/configuredtargets/MergedConfiguredTarget.java:196 MergedConfiguredTarget.of
    pub fn of(_base: ConfiguredTarget) -> ConfiguredTarget {
        // PORT: needs ConfiguredAspect (aspects are not ported).
        blaze_util::unported!("MergedConfiguredTarget.of")
    }

    // Java: com/google/devtools/build/lib/analysis/configuredtargets/MergedConfiguredTarget.java:388 MergedConfiguredTarget.getBaseConfiguredTarget
    pub fn get_base_configured_target(&self) -> &ConfiguredTarget {
        &self.base_configured_target
    }
}

// -------------------------------------------------------------------------------------------
// The `ConfiguredTarget` interface methods shared by all classes.

/// Java `ConfiguredTarget.getLabel()`: an alias reports its actual target's label, a merged
/// target its base's.
// Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:132 AliasConfiguredTarget.getLabel
pub fn get_label(ct: &ConfiguredTarget) -> Label {
    match ct {
        ConfiguredTarget::Alias(a) => get_label(&a.actual),
        ConfiguredTarget::Merged(m) => get_label(&m.base_configured_target),
        _ => ct.get_label(),
    }
}

/// Java `getProvider(Class)`.
pub fn get_provider<P: TransitiveInfoProvider>(ct: &ConfiguredTarget) -> Option<&P> {
    ct.providers().get_provider::<P>()
}

/// Java `getProvider(FilesToRunProvider.class)` as a shared handle.
pub fn get_files_to_run_provider(ct: &ConfiguredTarget) -> Option<Arc<FilesToRunProvider>> {
    // PORT: the provider is cheap to clone (a NestedSet and shared handles); Java returns the
    // same object, which only matters for identity-based hashing of the Starlark value.
    get_provider::<FilesToRunProvider>(ct).map(|p| Arc::new(p.clone()))
}

/// Java `getProvider(VisibilityProvider.class)`: the target itself for every
/// `AbstractConfiguredTarget`, or an alias's override.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:104 AbstractConfiguredTarget.getProvider
pub fn get_visibility_provider(ct: &ConfiguredTarget) -> VisibilityProvider {
    if let Some(v) = get_provider::<VisibilityProvider>(ct) {
        return v.clone();
    }
    let is_created_in_symbolic_macro = match ct {
        ConfiguredTarget::Rule(r) => r.is_created_in_symbolic_macro(),
        ConfiguredTarget::InputFile(t) => t.is_created_in_symbolic_macro(),
        ConfiguredTarget::OutputFile(t) => t.is_created_in_symbolic_macro(),
        _ => false,
    };
    let visibility = ct
        .get_visibility()
        .map(|v| v.visibility.clone())
        .unwrap_or_else(private_visibility);
    VisibilityProvider::new(visibility, is_created_in_symbolic_macro)
}

/// Java `get(Provider.Key)`: `DefaultInfo` is built from the target's native providers; other
/// infos come from the provider map (an alias forwards to its actual target).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:209 AbstractConfiguredTarget.get
pub fn get_info(ct: &ConfiguredTarget, key: &ProviderKey) -> Option<OwnedFrozenValue> {
    if let ConfiguredTarget::Alias(a) = ct {
        // Java: AliasConfiguredTarget.get(Provider.Key) -> actual.get(providerKey).
        return get_info(&a.actual, key);
    }
    if *key == DEFAULT_INFO.key() {
        return Some(get_default_info(ct));
    }
    ct.providers().get(key).cloned()
}

/// Java `DefaultInfo.build(this)` (a `DelegatingDefaultInfo`), cached per target.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:72 DefaultInfo.build
pub fn get_default_info(ct: &ConfiguredTarget) -> OwnedFrozenValue {
    let build = || {
        let files_to_build = get_provider::<FileProvider>(ct)
            .map(|p| p.get_files_to_build().clone())
            .unwrap_or_else(|| NestedSetBuilder::empty_set(Order::STABLE_ORDER));
        default_info::delegating(
            files_to_build,
            get_files_to_run_provider(ct),
            get_provider::<RunfilesProvider>(ct).cloned(),
        )
    };
    match ct {
        ConfiguredTarget::Rule(t) => t.default_info.get_or_init(build).clone(),
        ConfiguredTarget::InputFile(t) => t.default_info.get_or_init(build).clone(),
        ConfiguredTarget::OutputFile(t) => t.default_info.get_or_init(build).clone(),
        ConfiguredTarget::PackageGroup(t) => t.default_info.get_or_init(build).clone(),
        ConfiguredTarget::EnvironmentGroup(t) => t.default_info.get_or_init(build).clone(),
        ConfiguredTarget::Alias(a) => get_default_info(&a.actual),
        ConfiguredTarget::Merged(_) => build(),
    }
}

/// Java `get(String legacyKey)` (the legacy struct fields of a target).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:217 AbstractConfiguredTarget.get
pub fn get_legacy(ct: &ConfiguredTarget, key: &str) -> Option<OwnedFrozenValue> {
    match ct {
        ConfiguredTarget::Alias(a) => get_legacy(&a.actual, key),
        _ => ct.providers().get_legacy(key).cloned(),
    }
}

/// The providers a configured target has, as an advertised provider set (builtin providers by
/// Java simple class name, Starlark providers by key), for `RequiredProviders` checks.
fn advertised_set_of(ct: &ConfiguredTarget) -> AdvertisedProviderSet {
    let mut b = AdvertisedProviderSet::builder();
    // Every AbstractConfiguredTarget is a VisibilityProvider.
    b.add_builtin("VisibilityProvider");
    for (k, v) in ct.providers().entries() {
        match (k, v) {
            (ProviderMapKey::Class(_), ProviderInstance::Native(p)) => {
                b.add_builtin(p.java_class_name());
            }
            (ProviderMapKey::Info(key), _) => {
                b.add_starlark(StarlarkProviderIdentifier::for_key(key.clone()));
            }
            _ => {}
        }
    }
    b.add_starlark(StarlarkProviderIdentifier::for_key(DEFAULT_INFO.key()));
    b.build()
}

/// Java `TransitiveInfoCollection.satisfies(RequiredProviders)`.
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoCollection.java:89 TransitiveInfoCollection.satisfies
pub fn satisfies(ct: &ConfiguredTarget, required_providers: &RequiredProviders) -> bool {
    let set = advertised_set_of(ct);
    required_providers.is_satisfied_by_predicates(
        &|name: &String| set.get_builtin_providers().contains(name),
        &|id: &StarlarkProviderIdentifier| set.get_starlark_providers().contains(id),
    )
}

/// Java `TransitiveInfoCollection.missingProviders(RequiredProviders)`.
// Java: com/google/devtools/build/lib/analysis/TransitiveInfoCollection.java:101 TransitiveInfoCollection.missingProviders
pub fn missing_providers(
    ct: &ConfiguredTarget,
    required_providers: &RequiredProviders,
) -> RequiredProviders {
    required_providers.get_missing(&advertised_set_of(ct))
}

/// Whether the target has the native provider with the given Java simple class name.
pub fn has_builtin_provider_named(ct: &ConfiguredTarget, name: &str) -> bool {
    advertised_set_of(ct).get_builtin_providers().contains(name)
}

/// Java `getRuleClassString()` ("" for non-rules).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:236 AbstractConfiguredTarget.getRuleClassString
pub fn get_rule_class_string(ct: &ConfiguredTarget) -> String {
    match ct {
        ConfiguredTarget::Rule(r) => r.get_rule_class_string().to_owned(),
        ConfiguredTarget::Merged(m) => get_rule_class_string(&m.base_configured_target),
        _ => String::new(),
    }
}

/// Java `toString()`: `ConfiguredTarget(<label>, <config checksum>)`.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:98 AbstractConfiguredTarget.toString
pub fn to_string(ct: &ConfiguredTarget) -> String {
    match ct {
        ConfiguredTarget::InputFile(_) => format!("InputFileConfiguredTarget({})", ct.get_label()),
        _ => format!(
            "ConfiguredTarget({}, {})",
            ct.get_label(),
            ct.get_configuration_checksum()
                .unwrap_or_else(|| "null".to_owned())
        ),
    }
}

/// Keeps `RuleErrorConsumer` in scope for trait method calls on the rule context.
#[allow(unused)]
fn _uses(_: &dyn RuleErrorConsumer, _: &dyn AttributeMap, _: &dyn RuleOrMacroInstance) {}

/// The visibility of a configured target's base (for callers holding the struct).
pub fn base_visibility(base: &ConfiguredTargetBase) -> Visibility {
    visibility_of(base)
}

/// A configured target as a Starlark `Target` value.
///
/// PORT: Java reads `--incompatible_disable_target_default_provider_fields` from the thread's
/// semantics when a field is accessed; a `Heap` carries no semantics, so this uses the flag's
/// default (false). Callers with an evaluator can use `target_value::StarlarkTarget::new`.
pub fn target_to_starlark<'v>(
    target: &ConfiguredTarget,
    heap: starlark::values::Heap<'v>,
) -> starlark::values::Value<'v> {
    heap.alloc(target_value::StarlarkTarget::new(target.clone(), false))
}

/// The configured target of a Starlark `Target` value.
pub fn target_from_starlark(v: starlark::values::Value) -> Option<ConfiguredTarget> {
    target_value::StarlarkTarget::configured_target_of(v).cloned()
}
