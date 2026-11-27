//! Port of `analysis/RuleContext.java` (with its `Builder`, prerequisite validation and the
//! `TargetContext` it extends).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use blaze_actions::{ActionLookupKeyRef, ActionOwner, ActionRef, Artifact, ArtifactRoot};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::{Label, PackageIdentifier, RepositoryName};
use blaze_collect::NestedSet;
use blaze_config::transition_factories::DEFAULT_EXEC_GROUP_NAME;
use blaze_packages::aspect::{Aspect, AspectClass, AspectDescriptor};
use blaze_packages::attribute::{Attribute, AttributeMap};
use blaze_packages::package::ConfigSettingVisibilityPolicy;
use blaze_packages::package::package_args::FeatureSet;
use blaze_packages::rule_class::ConfigurationFragmentPolicy;
use blaze_packages::rule_class::rule_class::RuleClassNamePredicateType;
use blaze_packages::target::{OutputFile, OutputFileKind, Rule, Target};
use blaze_packages::types::{AttrValue, FileTypeSet, Type};
use blaze_starlark::StarlarkSemantics;
use blaze_util::{check_argument, check_state};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use starlark::values::OwnedFrozenValue;

use crate::aspect_context::{AspectContext, to_action_aspect_descriptors};
use crate::config::config_conditions::ConfigConditions;
use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use crate::configured_rule_class_provider::ConfiguredRuleClassProvider;
use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::dependency_kind::DependencyKind;
use crate::platform::platform_info::PlatformInfo;
use crate::providers::alias_provider::{AliasProvider, TargetMode};
use crate::providers::file_provider::{FileProvider, FilesToRunProvider};
use crate::providers::package_specification_provider::PackageSpecificationProvider;
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::providers::visibility::Visibility;
use crate::rule::analysis_environment::CachingAnalysisEnvironment;
use crate::rule::analysis_utils;
use crate::rule::artifact_roots::to_actions_root;
use crate::rule::common_prerequisite_validator::BazelPrerequisiteValidator;
use crate::rule::config_util;
use crate::rule::configuration_make_variable_context::ConfigurationMakeVariableContext;
use crate::rule::configured_target_and_data_ext::ConfiguredTargetAndDataExt;
use crate::rule::expander::Expander;
use crate::rule::prerequisite_artifacts::PrerequisiteArtifacts;
use crate::rule::prerequisites_collection::PrerequisitesCollection;
use crate::rule::rule_error_consumer::{
    ErrorReporter, RuleErrorConsumer, RuleReporter, SuppressingErrorReporter,
};
use crate::rule::stringtemplate::TemplateContext;
use crate::rule::target_context::TargetContext;
use crate::rule::to_event_location;
use crate::toolchains::aspect_base_target_resolved_toolchain_context::AspectBaseTargetResolvedToolchainContext;
use crate::toolchains::exec_group_collection::{
    ExecGroupCollection, ExecGroupCollectionBuilder, InvalidExecGroupException,
};
use crate::toolchains::resolved_toolchain_context::ResolvedToolchainContext;
use crate::toolchains::toolchain_collection::ToolchainCollection;
use crate::toolchains::toolchain_context::ToolchainContext;

/// Java `OrderedSetMultimap<DependencyKind, ConfiguredTargetAndData>`: keys in insertion order,
/// each key's values in insertion order (the caller guarantees set semantics).
pub type PrerequisiteMap = IndexMap<DependencyKind, Vec<ConfiguredTargetAndData>>;

/// Java `RuleClass.EXEC_PROPERTIES_ATTR`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:329 RuleClass.EXEC_PROPERTIES_ATTR
pub const EXEC_PROPERTIES_ATTR: &str = "exec_properties";

/// Java `RuleClass.DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:183 RuleClass.DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME
pub const DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME: &str = "test";

/// Java `RuleClass.CONFIG_SETTING_DEPS_ATTRIBUTE`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:324 RuleClass.CONFIG_SETTING_DEPS_ATTRIBUTE
pub const CONFIG_SETTING_DEPS_ATTRIBUTE: &str = "$config_dependencies";

/// The make-variable attributes (`RuleContext.DEFAULT_MAKE_VARIABLE_ATTRIBUTES`).
// TODO(b/37567440): Remove when Starlark callers can be updated to get this from
// CcToolchainProvider. We should use CcCommon.CC_TOOLCHAIN_ATTRIBUTE_NAME, but we didn't want to
// pollute core with C++ specific constant.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:929 RuleContext.DEFAULT_MAKE_VARIABLE_ATTRIBUTES
pub const DEFAULT_MAKE_VARIABLE_ATTRIBUTES: [&str; 4] = [
    "toolchains",
    ":cc_toolchain",
    "$toolchains",
    "$cc_toolchain",
];

/// Bazel's universal configuration fragments (Java simple class names).
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:235 BazelRuleClassProvider.BAZEL_SETUP
const UNIVERSAL_FRAGMENTS: [&str; 3] = [
    "ShellConfiguration",
    "PlatformConfiguration",
    "StrictActionEnvConfiguration",
];

/// The (lazily built) toolchain attribute used for toolchain-related validation errors.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:156 RuleContext.TOOLCHAIN_ATTRIBUTE
fn toolchain_attribute() -> &'static Attribute {
    static ATTR: OnceLock<Attribute> = OnceLock::new();
    ATTR.get_or_init(|| {
        blaze_packages::attribute::AttributeBuilder::new(
            RuleContext::TOOLCHAIN_ATTR_NAME,
            Type::LabelList,
        )
        .build()
    })
}

// PORT: skipped, Blaze's native rules call no Starlark-defined builtins: RuleContext.getStarlarkDefinedBuiltin, RuleContext.callStarlarkOrThrowRuleError
// PORT: skipped, the Starlark thread and context are the evaluator's (thread_payload): RuleContext.getStarlarkThread, RuleContext.getStarlarkRuleContext
/// The totality of data available during the analysis of a rule.
///
/// These objects should not outlast the analysis phase. Do not pass them to `Action` objects or
/// other persistent objects. There are internal tests to ensure that RuleContext objects are not
/// persisted that check the name of this class, so update those tests if you change this class's
/// name.
///
/// PORT: every method takes `&self` (Java mutates a few caches and lists); the mutable state is
/// behind locks, so the Starlark rule context (agent S) can share a `&RuleContext`.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:131 RuleContext
pub struct RuleContext {
    target_context: TargetContext,
    rule: Arc<Rule>,
    configuration: Arc<BuildConfigurationValue>,
    /// If this `RuleContext` is for rule evaluation, this holds the attribute-based prerequisites
    /// of the rule.
    prerequisites_collection: PrerequisitesCollection,
    config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
    /// Java `attributes`: the rule's `ConfiguredAttributeMapper`, or for aspects the
    /// `AspectAwareAttributeMapper` over it and the aspects' attributes.
    attributes: RuleContextAttributes,
    features: FeatureSet,
    rule_class_name_for_logging: String,
    configuration_fragment_policy: ConfigurationFragmentPolicy,
    rule_class_provider: Arc<ConfiguredRuleClassProvider>,
    reporter: Arc<RuleReporter>,
    toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
    exec_group_collection: ExecGroupCollection,
    required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    transitive_packages_for_runfile_repo_mapping_manifest: Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    >,
    /// The looked-up variables of each `Expander` created by this context (Java
    /// `makeVariableExpanders`).
    make_variable_expanders: Mutex<Vec<Arc<Mutex<BTreeSet<String>>>>>,
    /// Map of exec group names to ActionOwners (`None`: no toolchain context for the group).
    action_owners: Mutex<HashMap<String, Option<ActionOwner>>>,
    /// Java `actionOwnerSymbolGenerator`'s owner.
    action_owner_symbol: ActionLookupKeyRef,
    /// Lazily computed cache for Make variables.
    configuration_make_variable_context: OnceLock<Arc<ConfigurationMakeVariableContext>>,
    /// Java `starlarkRuleContext.lookedUpVariables()`, recorded by the Starlark rule context
    /// (agent S) when it closes; `None` while no Starlark rule context exists.
    starlark_looked_up_variables: Mutex<Option<Vec<String>>>,
    starlark_semantics: Arc<StarlarkSemantics>,
    /// The state of Java's subclass `AspectContext` (`extends RuleContext`): `Some` iff this
    /// context is for the evaluation of an aspect. The methods `AspectContext` overrides
    /// dispatch on it.
    aspect_context: Option<AspectContext>,
    /// `conflictFinder`.
    conflict_finder: crate::rule::action_conflict_exception::ConflictFinderSupplier,
}

impl RuleContext {
    /// Returns the conflict finder if `ConflictCheckingMode.UPON_CONFIGURED_OBJECT_CREATION`
    /// and `None` otherwise.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1052 RuleContext.getConflictFinder
    pub fn get_conflict_finder(
        &self,
    ) -> Option<Arc<dyn crate::rule::action_conflict_exception::ConflictFinder>> {
        (self.conflict_finder)()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:153 RuleContext.TOOLCHAIN_ATTR_NAME
    pub const TOOLCHAIN_ATTR_NAME: &'static str = "$toolchain";

    /// The constructor is intentionally private to be only used by the builder.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:211 RuleContext.<init>
    pub(crate) fn new(
        builder: &RuleContextBuilder,
        attributes: Arc<dyn AttributeMap + Send + Sync>,
        prerequisites_collection: PrerequisitesCollection,
        exec_group_collection: ExecGroupCollection,
    ) -> RuleContext {
        let rule = builder.get_rule().clone();
        let target_context = TargetContext::new(
            builder.env.clone(),
            builder.target.clone(),
            Some(builder.configuration.clone()),
            &Self::get_direct_prerequisites(builder.prerequisite_map.as_ref().unwrap()),
            builder.visibility.clone().expect("visibility"),
            builder
                .transitive_visibility_imposed_by_this_package
                .clone(),
        );
        let features = Self::compute_features(&rule, attributes.as_ref(), &builder.configuration);
        let attributes = RuleContextAttributes(attributes);
        RuleContext {
            conflict_finder: builder.conflict_finder.clone(),
            target_context,
            rule,
            configuration: builder.configuration.clone(),
            prerequisites_collection,
            config_conditions: builder
                .config_conditions
                .as_ref()
                .expect("configConditions")
                .as_providers()
                .clone()
                .into(),
            attributes,
            features,
            rule_class_name_for_logging: builder.get_rule_class_name_for_logging(),
            configuration_fragment_policy: builder
                .configuration_fragment_policy
                .clone()
                .expect("configurationFragmentPolicy"),
            rule_class_provider: builder
                .rule_class_provider
                .clone()
                .expect("ruleClassProvider"),
            reporter: builder.reporter.clone(),
            toolchain_contexts: builder.toolchain_contexts.clone(),
            exec_group_collection,
            required_config_fragments: builder.required_config_fragments.clone(),
            transitive_packages_for_runfile_repo_mapping_manifest: builder
                .transitive_packages_for_runfile_repo_mapping_manifest
                .clone(),
            make_variable_expanders: Mutex::new(Vec::new()),
            action_owners: Mutex::new(HashMap::new()),
            action_owner_symbol: builder
                .action_owner_symbol
                .clone()
                .expect("actionOwnerSymbol"),
            configuration_make_variable_context: OnceLock::new(),
            starlark_looked_up_variables: Mutex::new(None),
            starlark_semantics: builder.env.get_starlark_semantics().clone(),
            aspect_context: None,
        }
    }

    /// Java `AspectContext.<init>`'s own fields, set right after `super(...)` (the
    /// `RuleContext` constructor).
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:69 AspectContext.<init>
    pub(crate) fn set_aspect_context(&mut self, aspect_context: AspectContext) {
        check_state!(self.aspect_context.is_none());
        self.aspect_context = Some(aspect_context);
    }

    /// Java `(AspectContext) ruleContext`: the aspect part of this context, `None` for a rule.
    pub fn as_aspect_context(&self) -> Option<&AspectContext> {
        self.aspect_context.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:243 RuleContext.create
    fn create(
        builder: &RuleContextBuilder,
        rule_attributes: Arc<ConfiguredAttributeMapper>,
        targets_map: Vec<(DependencyKind, Vec<ConfiguredTargetAndData>)>,
        exec_group_collection: ExecGroupCollection,
    ) -> RuleContext {
        // Java: ImmutableSortedKeyListMultimap (sorted by attribute name).
        let mut attr_name_to_targets: BTreeMap<String, Vec<ConfiguredTargetAndData>> =
            BTreeMap::new();
        for (kind, values) in targets_map {
            let name = kind
                .get_attribute()
                .expect("attribute dependency")
                .get_name()
                .to_owned();
            attr_name_to_targets.entry(name).or_default().extend(values);
        }
        let prerequisites = PrerequisitesCollection::new(
            attr_name_to_targets,
            crate::rule::prerequisites_collection::AttributesMapper::of_attribute_map(
                rule_attributes.clone(),
            ),
            builder.get_error_consumer().clone(),
            builder.get_rule().clone(),
            builder.get_rule_class_name_for_logging(),
        );
        RuleContext::new(
            builder,
            rule_attributes,
            prerequisites,
            exec_group_collection,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:268 RuleContext.computeFeatures
    fn compute_features(
        rule: &Rule,
        attributes: &dyn AttributeMap,
        configuration: &BuildConfigurationValue,
    ) -> FeatureSet {
        let pkg = rule
            .get_package_declarations()
            .get_package_args()
            .features()
            .clone();
        let rule_features = if attributes.has_typed("features", Type::StringList) {
            let features: Vec<String> = attributes
                .get("features", Type::StringList)
                .and_then(|v| v.as_string_list().map(|s| s.to_vec()))
                .unwrap_or_default();
            FeatureSet::parse(&features)
        } else {
            FeatureSet::empty()
        };
        // PORT: the configuration's default features are `blaze_build_config::FeatureSet`;
        // re-parse its string form into the packages `FeatureSet` (same class in Java).
        let global = FeatureSet::parse(&configuration.get_default_features().to_string_list());
        FeatureSet::merge_with_global_features(&FeatureSet::merge(&pkg, &rule_features), &global)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:278 RuleContext.getDirectPrerequisites
    fn get_direct_prerequisites(
        prerequisite_map: &PrerequisiteMap,
    ) -> Vec<ConfiguredTargetAndData> {
        let mut out: Vec<ConfiguredTargetAndData> = Vec::new();
        for (kind, values) in prerequisite_map {
            if kind.get_attribute().is_none() {
                out.extend(values.iter().cloned());
            }
        }
        out
    }

    // ---------------------------------------------------------------------------------------
    // TargetContext

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:82 TargetContext.getAnalysisEnvironment
    pub fn get_analysis_environment(&self) -> &Arc<CachingAnalysisEnvironment> {
        self.target_context.get_analysis_environment()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:86 TargetContext.getActionKeyContext
    pub fn get_action_key_context(&self) -> &Arc<blaze_actions::ActionKeyContext> {
        self.target_context.get_action_key_context()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:90 TargetContext.getTarget
    pub fn get_target(&self) -> &Target {
        self.target_context.get_target()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:94 TargetContext.getLabel
    pub fn get_label(&self) -> &Label {
        self.target_context.get_label()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:99 TargetContext.getPackageContext
    pub fn get_package_context(&self) -> blaze_cmdline::label::LabelPackageContext {
        self.target_context.get_package_context()
    }

    /// The configuration of this rule (never null for rules).
    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:109 TargetContext.getConfiguration
    pub fn get_configuration(&self) -> &Arc<BuildConfigurationValue> {
        &self.configuration
    }

    /// The configuration as the action layer reads it in `SpawnAction.Builder.build(owner,
    /// configuration)`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java RuleContext.getConfiguration
    pub fn get_spawn_action_configuration(
        &self,
    ) -> Arc<dyn blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration> {
        Arc::new(
            crate::rule::config_util::ConfigurationSpawnActionConfiguration(
                self.get_configuration().clone(),
            ),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:113 TargetContext.getVisibility
    pub fn get_visibility(&self) -> &Visibility {
        self.target_context.get_visibility()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:118 TargetContext.getTransitiveVisibilityImposedByThisPackage
    pub fn get_transitive_visibility_imposed_by_this_package(
        &self,
    ) -> Option<&Arc<PackageSpecificationProvider>> {
        self.target_context
            .get_transitive_visibility_imposed_by_this_package()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:128 TargetContext.findDirectPrerequisite
    pub fn find_direct_prerequisite(
        &self,
        label: &Label,
        config: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<ConfiguredTarget> {
        self.target_context.find_direct_prerequisite(label, config)
    }

    /// The Starlark semantics of this analysis (Java `getAnalysisEnvironment().getStarlarkSemantics()`).
    pub fn get_starlark_semantics(&self) -> &Arc<StarlarkSemantics> {
        &self.starlark_semantics
    }

    // ---------------------------------------------------------------------------------------
    // RuleContext

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:286 RuleContext.isAllowTagsPropagation
    pub fn is_allow_tags_propagation(&self) -> bool {
        self.starlark_semantics.incompatible_allow_tags_propagation
    }

    /// If this `RuleContext` is for rule evaluation, returns the attribute-based prerequisites of
    /// the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:297 RuleContext.getRulePrerequisitesCollection
    pub fn get_rule_prerequisites_collection(&self) -> &PrerequisitesCollection {
        &self.prerequisites_collection
    }

    /// Prerequisite lookup methods use this method to find the `PrerequisitesCollection` owning
    /// an attribute with the given name.
    ///
    /// For aspect evaluation, `AspectContext` overrides this to select the correct owning
    /// collection: the main aspect or the underlying rule and base aspects.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:310 RuleContext.getOwningPrerequisitesCollection
    pub(crate) fn get_owning_prerequisites_collection(
        &self,
        attribute_name: &str,
    ) -> &PrerequisitesCollection {
        if let Some(aspect_context) = &self.aspect_context {
            return aspect_context.get_owning_prerequisites_collection(self, attribute_name);
        }
        &self.prerequisites_collection
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:314 RuleContext.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        self.rule.get_repository()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:319 RuleContext.getBinDirectory
    pub fn get_bin_directory(&self) -> ArtifactRoot {
        to_actions_root(
            &self
                .configuration
                .get_bin_directory(self.get_label().get_repository()),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:323 RuleContext.getGenfilesDirectory
    pub fn get_genfiles_directory(&self) -> ArtifactRoot {
        to_actions_root(
            &self
                .configuration
                .get_genfiles_directory(self.get_label().get_repository()),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:327 RuleContext.getTestLogsDirectory
    pub fn get_test_logs_directory(&self) -> ArtifactRoot {
        to_actions_root(
            &self
                .configuration
                .get_test_logs_directory(self.get_label().get_repository()),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:331 RuleContext.getBinFragment
    pub fn get_bin_fragment(&self) -> PathFragment {
        self.configuration
            .get_bin_fragment(self.get_label().get_repository())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:335 RuleContext.getGenfilesFragment
    pub fn get_genfiles_fragment(&self) -> PathFragment {
        self.configuration
            .get_genfiles_fragment(self.get_label().get_repository())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:339 RuleContext.getRule
    pub fn get_rule(&self) -> &Arc<Rule> {
        &self.rule
    }

    /// All aspects applied to the target (empty for a rule; the main aspect is the last one).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:343 RuleContext.getAspects
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:254 AspectContext.getAspects
    pub fn get_aspects(&self) -> &[Aspect] {
        match &self.aspect_context {
            Some(aspect_context) => aspect_context.get_aspects(),
            None => &[],
        }
    }

    /// If this target's configuration suppresses analysis failures, this returns the
    /// descriptions of the errors that occurred during processing this target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:355 RuleContext.getSuppressedErrorMessages
    pub fn get_suppressed_error_messages(&self) -> Vec<String> {
        check_state!(
            config_util::allow_analysis_failures(&self.configuration),
            "Error messages can only be retrieved via RuleContext if allow_analysis_failures is true"
        );
        match self.reporter.as_ref() {
            RuleReporter::Suppressing(r) => r.get_error_messages(),
            RuleReporter::Events(_) => panic!("IllegalStateException: Unexpected error reporter"),
        }
    }

    /// Returns a rule class name suitable for log messages, including an aspect name if
    /// applicable.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:378 RuleContext.getRuleClassNameForLogging
    pub fn get_rule_class_name_for_logging(&self) -> &str {
        &self.rule_class_name_for_logging
    }

    /// Returns the workspace name for the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:383 RuleContext.getWorkspaceName
    pub fn get_workspace_name(&self) -> &str {
        &self.rule.get_package_metadata().workspace_name
    }

    /// The configuration conditions that trigger this rule's configurable attributes.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:388 RuleContext.getConfigConditions
    pub fn get_config_conditions(&self) -> &Arc<IndexMap<Label, ConfigMatchingProvider>> {
        &self.config_conditions
    }

    /// All aspects applied to the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:393 RuleContext.getAspectDescriptors
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:271 AspectContext.getAspectDescriptors
    pub fn get_aspect_descriptors(&self) -> &[AspectDescriptor] {
        match &self.aspect_context {
            Some(aspect_context) => aspect_context.get_aspect_descriptors(),
            None => &[],
        }
    }

    /// If this `RuleContext` is for an aspect implementation, returns that aspect (it is the
    /// last aspect in the list of aspects applied to a target; all other aspects are the ones
    /// main aspect sees as specified by its "required_aspect_providers"). Otherwise returns
    /// `None`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:371 RuleContext.getMainAspect
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:265 AspectContext.getMainAspect
    pub fn get_main_aspect(&self) -> Option<&Aspect> {
        self.aspect_context
            .as_ref()
            .map(|aspect_context| aspect_context.get_main_aspect())
    }

    /// Accessor for the attributes of the rule and its aspects.
    ///
    /// The rule's native attributes can be queried both on their structure / existence and
    /// values. Aspect attributes can only be queried on their structure.
    ///
    /// This should be the sole interface for reading rule/aspect attributes in `RuleContext`.
    /// Don't expose other access points through new public methods.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:406 RuleContext.attributes
    pub fn attributes(&self) -> &RuleContextAttributes {
        &self.attributes
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:411 RuleContext.hasErrors
    pub fn has_errors(&self) -> bool {
        self.reporter.has_errors()
    }

    /// Returns a list of all prerequisites as `ConfiguredTarget` objects.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:416 RuleContext.getAllPrerequisites
    pub fn get_all_prerequisites(&self) -> Vec<ConfiguredTarget> {
        if let Some(aspect_context) = &self.aspect_context {
            return aspect_context.get_all_prerequisites(self);
        }
        self.prerequisites_collection.get_all_prerequisites()
    }

    /// Returns the `ConfiguredTargetAndData` of the given attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:421 RuleContext.getPrerequisiteConfiguredTargets
    pub fn get_prerequisite_configured_targets(
        &self,
        attribute_name: &str,
    ) -> &[ConfiguredTargetAndData] {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisite_configured_targets(attribute_name)
    }

    /// Returns a special action owner for test actions. Test actions should run on the target
    /// platform rather than the host platform. Note that the value is not cached (on the
    /// assumption that this method is only called once).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:431 RuleContext.getTestActionOwner
    pub fn get_test_action_owner(&self) -> ActionOwner {
        let (test_execution_platform, test_exec_properties) = match &self.toolchain_contexts {
            // If we have a toolchain, pull the target platform out of it.
            // TODO(https://github.com/bazelbuild/bazel/issues/17466): This doesn't respect
            // execution properties coming from the target's `exec_properties` attribute.
            Some(tc) => {
                let platform = tc.get_target_platform().clone();
                let props = platform.exec_properties().clone();
                (Some(platform), props)
            }
            None => (
                None,
                self.get_exec_groups()
                    .get_exec_properties(DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME),
            ),
        };
        Self::create_action_owner(
            &self.rule,
            self.get_aspect_descriptors(),
            &self.configuration,
            test_exec_properties,
            test_execution_platform,
        )
    }

    /// The execution platform `get_test_action_owner` uses (Java: `toolchainContexts
    /// .getTargetPlatform()`, or null without toolchain contexts).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:431 RuleContext.getTestActionOwner
    pub fn get_test_execution_platform(&self) -> Option<Arc<PlatformInfo>> {
        self.toolchain_contexts
            .as_ref()
            .map(|tc| tc.get_target_platform().clone())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:457 RuleContext.getActionOwner
    pub fn get_action_owner(&self) -> ActionOwner {
        self.get_action_owner_for(DEFAULT_EXEC_GROUP_NAME)
            .expect("default exec group has an action owner")
    }

    /// Java `getActionOwner(String execGroup)`; `None` if the exec group has no toolchain
    /// context.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:463 RuleContext.getActionOwner
    pub fn get_action_owner_for(&self, exec_group: &str) -> Option<ActionOwner> {
        if let Some(owner) = self.action_owners.lock().get(exec_group) {
            return owner.clone();
        }
        if let Some(tc) = &self.toolchain_contexts {
            if !tc.has_toolchain_context(exec_group) {
                return None;
            }
        }
        let action_owner = Self::create_action_owner(
            &self.rule,
            self.get_aspect_descriptors(),
            &self.configuration,
            self.exec_group_collection.get_exec_properties(exec_group),
            self.get_execution_platform_for(exec_group),
        );
        self.action_owners
            .lock()
            .insert(exec_group.to_owned(), Some(action_owner.clone()));
        Some(action_owner)
    }

    /// The owner symbol of objects created by this context (Java `getSymbolGenerator()`'s owner).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:487 RuleContext.getSymbolGenerator
    pub fn get_action_owner_symbol(&self) -> &ActionLookupKeyRef {
        &self.action_owner_symbol
    }

    /// Returns a configuration fragment for this target. The fragment class is named by its Java
    /// simple class name and its `@StarlarkBuiltin` name, if any.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:493 RuleContext.getFragment
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:497 RuleContext.getFragment
    pub fn get_fragment<T: blaze_build_config::fragment::Fragment>(
        &self,
        simple_name: &str,
        starlark_name: Option<&str>,
    ) -> Option<&T> {
        check_argument!(
            self.is_legal_fragment(simple_name, starlark_name),
            "{} has to declare '{}' as a required fragment in order to access it.{}",
            self.rule_class_name_for_logging,
            simple_name,
            ""
        );
        self.configuration.get_fragment::<T>()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:536 RuleContext.isLegalFragment
    pub fn is_legal_fragment(
        &self,
        fragment_simple_name: &str,
        starlark_name: Option<&str>,
    ) -> bool {
        // PORT: Java consults `ruleClassProvider.getFragmentRegistry().getUniversalFragments()`;
        // Bazel's (BazelRuleClassProvider.java:235) are ShellConfiguration,
        // PlatformConfiguration and StrictActionEnvConfiguration.
        if UNIVERSAL_FRAGMENTS.contains(&fragment_simple_name) {
            return true;
        }
        self.configuration_fragment_policy
            .is_legal_configuration_fragment(fragment_simple_name, starlark_name)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:542 RuleContext.getOwner
    pub fn get_owner(&self) -> &ActionLookupKeyRef {
        self.get_analysis_environment().get_owner()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:547 RuleContext.createActionOwner
    pub fn create_action_owner(
        rule: &Rule,
        aspect_descriptors: &[AspectDescriptor],
        build_configuration_value: &BuildConfigurationValue,
        exec_properties: IndexMap<String, String>,
        execution_platform: Option<Arc<PlatformInfo>>,
    ) -> ActionOwner {
        ActionOwner::create(
            Some(rule.get_label().clone()),
            to_event_location(rule.get_location()),
            blaze_packages::target::TargetData::get_target_kind(rule),
            config_util::to_build_configuration_info(build_configuration_value),
            execution_platform.map(|p| p as blaze_actions::PlatformInfoRef),
            // PORT: the actions layer's AspectDescriptor (blaze_actions) is a converted copy of
            // the packages one.
            to_action_aspect_descriptors(aspect_descriptors),
            exec_properties,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:564 RuleContext.registerAction
    pub fn register_action(&self, action: ActionRef) {
        self.get_analysis_environment().register_action(action);
    }

    /// Returns the reporter (Java `RuleErrorConsumer`) of this context.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1407 RuleContext.getRuleErrorConsumer
    pub fn get_rule_error_consumer(&self) -> &dyn RuleErrorConsumer {
        self
    }

    /// Returns an artifact beneath the root of either the "bin" or "genfiles" tree, whose path is
    /// based on the name of this target and the current configuration.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:614 RuleContext.createOutputArtifact
    pub fn create_output_artifact(&self) -> Artifact {
        let target = self.get_target();
        let root_relative_path = self
            .get_package_directory()
            .get_relative(&PathFragment::create(target.get_name()));
        self.internal_create_output_artifact(
            &root_relative_path,
            target.get_label(),
            OutputFileKind::File,
        )
    }

    /// Returns the output artifact of an `OutputFile` of this target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:627 RuleContext.createOutputArtifact
    pub fn create_output_artifact_for(&self, out: &OutputFile) -> Artifact {
        let package_relative_path = self
            .get_package_directory()
            .get_relative(&PathFragment::create(out.get_name()));
        self.internal_create_output_artifact(
            &package_relative_path,
            out.get_label(),
            out.get_kind(),
        )
    }

    /// Returns an artifact whose path is based on the name of this target with a script suffix
    /// appropriate for the default exec group's execution platform.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:640 RuleContext.createOutputArtifactScriptForAnalysisTest
    pub fn create_output_artifact_script_for_analysis_test(&self) -> Artifact {
        let target = self.get_target();
        let file_extension = if self.is_default_exec_group_executing_on_windows() {
            ".cmd"
        } else {
            ".sh"
        };
        let root_relative_path = self
            .get_package_directory()
            .get_relative(&PathFragment::create(&format!(
                "{}{}",
                target.get_name(),
                file_extension
            )));
        self.internal_create_output_artifact(
            &root_relative_path,
            target.get_label(),
            OutputFileKind::File,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:656 RuleContext.internalCreateOutputArtifact
    fn internal_create_output_artifact(
        &self,
        root_relative_path: &PathFragment,
        target_label: &Label,
        output_file_kind: OutputFileKind,
    ) -> Artifact {
        check_state!(
            target_label.get_package_identifier() == self.get_label().get_package_identifier(),
            "Creating output artifact for target '{}' in different package than the rule '{}' being analyzed",
            target_label,
            self.get_label()
        );
        let root = self.get_bin_or_genfiles_directory();
        match output_file_kind {
            OutputFileKind::File => self.get_derived_artifact(root_relative_path, &root),
            OutputFileKind::Fileset => self
                .get_analysis_environment()
                .get_fileset_artifact(root_relative_path, &root),
        }
    }

    /// Returns the root of either the "bin" or "genfiles" tree, based on this target and the
    /// current configuration.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:678 RuleContext.getBinOrGenfilesDirectory
    pub fn get_bin_or_genfiles_directory(&self) -> ArtifactRoot {
        if self.rule.outputs_to_bindir() {
            self.get_bin_directory()
        } else {
            self.get_genfiles_directory()
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:688 RuleContext.getBinArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:692 RuleContext.getBinArtifact
    pub fn get_bin_artifact(&self, relative: &PathFragment) -> Artifact {
        self.get_package_relative_artifact(relative, &self.get_bin_directory())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:701 RuleContext.getGenfilesArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:705 RuleContext.getGenfilesArtifact
    pub fn get_genfiles_artifact(&self, relative: &PathFragment) -> Artifact {
        self.get_package_relative_artifact(relative, &self.get_genfiles_directory())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:711 RuleContext.getShareableArtifact
    pub fn get_shareable_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.get_analysis_environment()
            .get_derived_artifact(root_relative_path, root)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:716 RuleContext.getPackageRelativeArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:725 RuleContext.getPackageRelativeArtifact
    pub fn get_package_relative_artifact(
        &self,
        relative: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.get_derived_artifact(&self.get_package_directory().get_relative(relative), root)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:730 RuleContext.getPackageDirectory
    pub fn get_package_directory(&self) -> PathFragment {
        self.get_label().get_package_identifier().get_package_path(
            config_util::is_sibling_repository_layout(&self.configuration),
        )
    }

    /// Creates an artifact under a given root with the given root-relative path, verifying that
    /// it is in the root-relative directory corresponding to the package of the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:744 RuleContext.getDerivedArtifact
    pub fn get_derived_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(
            root_relative_path.starts_with(&self.get_package_directory()),
            "Output artifact '{}' not under package directory '{}' for target '{}'",
            root_relative_path,
            self.get_package_directory(),
            self.get_label()
        );
        self.get_analysis_environment()
            .get_derived_artifact(root_relative_path, root)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:756 RuleContext.getTreeArtifact
    pub fn get_tree_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        check_state!(
            root_relative_path.starts_with(&self.get_package_directory()),
            "Output artifact '{}' not under package directory '{}' for target '{}'",
            root_relative_path,
            self.get_package_directory(),
            self.get_label()
        );
        self.get_analysis_environment()
            .get_tree_artifact(root_relative_path, root)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:770 RuleContext.getPackageRelativeTreeArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:774 RuleContext.getPackageRelativeTreeArtifact
    pub fn get_package_relative_tree_artifact(
        &self,
        relative: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.get_tree_artifact(&self.get_package_directory().get_relative(relative), root)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:788 RuleContext.getUniqueDirectoryArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:782 RuleContext.getUniqueDirectoryArtifact
    pub fn get_unique_directory_artifact(
        &self,
        unique_directory_suffix: &str,
        relative: &str,
    ) -> Artifact {
        self.get_unique_directory_artifact_in(
            unique_directory_suffix,
            &PathFragment::create(relative),
            &self.get_bin_or_genfiles_directory(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:798 RuleContext.getUniqueDirectoryArtifact
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:792 RuleContext.getUniqueDirectoryArtifact
    pub fn get_unique_directory_artifact_in(
        &self,
        unique_directory: &str,
        relative: &PathFragment,
        root: &ArtifactRoot,
    ) -> Artifact {
        self.get_derived_artifact(
            &self
                .get_unique_directory(unique_directory)
                .get_relative(relative),
            root,
        )
    }

    /// Returns true iff the rule, or any attached aspect, has an attribute with the given name
    /// and type.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:807 RuleContext.isAttrDefined
    pub fn is_attr_defined(&self, attr_name: &str, type_: Type) -> bool {
        self.attributes().has_typed(attr_name, type_)
    }

    /// Returns the specified native provider of the prerequisite referenced by the attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:816 RuleContext.getPrerequisite
    pub fn get_prerequisite_provider<P: crate::provider_map::TransitiveInfoProvider + Clone>(
        &self,
        attribute_name: &str,
    ) -> Option<P> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisite_provider::<P>(attribute_name)
    }

    /// Returns the transitive info collection that feeds into this target through the specified
    /// attribute. Returns `None` if the attribute is empty.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:826 RuleContext.getPrerequisite
    pub fn get_prerequisite(&self, attribute_name: &str) -> Option<ConfiguredTarget> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisite(attribute_name)
    }

    /// Returns the declared provider (native and Starlark) for the given key under the specified
    /// attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:836 RuleContext.getPrerequisite
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:842 RuleContext.getPrerequisite
    pub fn get_prerequisite_info(
        &self,
        attribute_name: &str,
        key: &blaze_packages::rule_class::ProviderKey,
    ) -> Option<OwnedFrozenValue> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisite_info(attribute_name, key)
    }

    /// Returns the `--run_under` prerequisite based on the value of
    /// `--incompatible_bazel_test_exec_run_under`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:853 RuleContext.getRunUnderPrerequisite
    pub fn get_run_under_prerequisite(&self) -> Option<ConfiguredTarget> {
        self.get_prerequisite(
            if config_util::run_under_exec_config_for_tests(&self.configuration) {
                ":run_under_exec_config"
            } else {
                ":run_under_target_config"
            },
        )
    }

    /// Returns the list of transitive info collections that feed into this target through the
    /// specified attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:864 RuleContext.getPrerequisites
    pub fn get_prerequisites(&self, attribute_name: &str) -> Vec<ConfiguredTarget> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisites(attribute_name)
    }

    /// Returns all the native providers of the specified type under the attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:872 RuleContext.getPrerequisites
    pub fn get_prerequisites_providers<P: crate::provider_map::TransitiveInfoProvider + Clone>(
        &self,
        attribute_name: &str,
    ) -> Vec<P> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisites_providers::<P>(attribute_name)
    }

    /// Returns all the declared providers for the given key under the attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:882 RuleContext.getPrerequisites
    pub fn get_prerequisites_infos(
        &self,
        attribute_name: &str,
        key: &blaze_packages::rule_class::ProviderKey,
    ) -> Vec<OwnedFrozenValue> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_prerequisites_infos(attribute_name, key)
    }

    /// Returns the prerequisite referred to by the specified attribute, checking that it is
    /// executable.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:896 RuleContext.getExecutablePrerequisite
    pub fn get_executable_prerequisite(
        &self,
        attribute_name: &str,
    ) -> Option<Arc<FilesToRunProvider>> {
        self.get_owning_prerequisites_collection(attribute_name)
            .get_executable_prerequisite(attribute_name)
    }

    /// The `TemplateVariableInfo` variables of the given attributes' prerequisites.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:900 RuleContext.fromAttributes
    fn from_attributes(&self, attribute_names: &[&str]) -> Vec<IndexMap<String, String>> {
        let key = crate::platform::template_variable_info::TemplateVariableInfo::provider_key();
        let mut out = Vec::new();
        for attr_name in attribute_names {
            // Only process this attribute it if is present in the rule.
            if !self.attributes().has(attr_name) {
                continue;
            }
            // Get the TemplateVariableInfo providers from this attribute.
            for info in self.get_prerequisites_infos(attr_name, &key) {
                out.push(
                    crate::platform::template_variable_info::TemplateVariableInfo::variables_of(
                        &info,
                    ),
                );
            }
        }
        out
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:914 RuleContext.fromToolchains
    fn from_toolchains(&self) -> Vec<IndexMap<String, String>> {
        let Some(tc) = self.get_toolchain_contexts() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for context in tc.context_map().values() {
            for info in context.template_variable_providers() {
                out.push(
                    crate::platform::template_variable_info::TemplateVariableInfo::variables_of(
                        info,
                    ),
                );
            }
        }
        out
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:932 RuleContext.getDefaultTemplateVariableProviders
    // Java: com/google/devtools/build/lib/analysis/AspectContext.java:336 AspectContext.getDefaultTemplateVariableProviders
    pub fn get_default_template_variable_providers(&self) -> Vec<IndexMap<String, String>> {
        if let Some(aspect_context) = &self.aspect_context {
            let mut v = aspect_context
                .get_template_variables_from_aspect_attributes(&DEFAULT_MAKE_VARIABLE_ATTRIBUTES);
            v.extend(self.from_toolchains());
            return v;
        }
        let mut v = self.from_attributes(&DEFAULT_MAKE_VARIABLE_ATTRIBUTES);
        v.extend(self.from_toolchains());
        v
    }

    /// Returns an `Expander` over the given template context, recording its looked-up variables.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:942 RuleContext.getExpander
    pub fn get_expander_for(
        &self,
        template_context: Arc<dyn TemplateContext + Send + Sync>,
    ) -> Expander<'_> {
        let expander = Expander::new(self, template_context, None);
        self.make_variable_expanders
            .lock()
            .push(expander.looked_up_variables_handle());
        expander
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:948 RuleContext.getExpander
    pub fn get_expander(&self) -> Expander<'_> {
        self.get_expander_for(self.get_configuration_make_variable_context())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:954 RuleContext.getExpander
    pub fn get_expander_with_label_map(
        &self,
        label_map: IndexMap<Label, Vec<Artifact>>,
    ) -> Expander<'_> {
        let expander = Expander::new(
            self,
            self.get_configuration_make_variable_context(),
            Some(label_map),
        );
        self.make_variable_expanders
            .lock()
            .push(expander.looked_up_variables_handle());
        expander
    }

    /// Returns a cached context that maps Make variable names (string) to values (string)
    /// without any extra `MakeVariableSupplier`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:966 RuleContext.getConfigurationMakeVariableContext
    // (AspectContext.getConfigurationMakeVariableContext only calls super: the context of the
    // aspect itself, including toolchains but not the underlying target.)
    pub fn get_configuration_make_variable_context(&self) -> Arc<ConfigurationMakeVariableContext> {
        self.configuration_make_variable_context
            .get_or_init(|| {
                Arc::new(ConfigurationMakeVariableContext::new(
                    self.rule.get_package_declarations(),
                    &self.configuration,
                    self.get_default_template_variable_providers(),
                ))
            })
            .clone()
    }

    /// Records the variables the Starlark rule context looked up through `ctx.var` (Java
    /// `starlarkRuleContext.lookedUpVariables()`, read by `getRequiredConfigFragments`).
    pub fn set_starlark_looked_up_variables(&self, variables: Vec<String>) {
        *self.starlark_looked_up_variables.lock() = Some(variables);
    }

    /// Prepares Starlark objects created during this target's analysis for use by others.
    ///
    /// PORT: the Starlark thread, its mutability and the `StarlarkRuleContext` belong to agent S
    /// (`crate::starlark`), which freezes its module when the rule implementation returns; this
    /// is a no-op kept for call-site fidelity.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1061 RuleContext.close
    pub fn close(&self) {}

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1069 RuleContext.useAutoExecGroups
    pub fn use_auto_exec_groups(&self) -> bool {
        if let Some(aspect_context) = &self.aspect_context {
            return aspect_context.use_auto_exec_groups(self);
        }
        self.rule
            .get_rule_class_object()
            .get_auto_exec_groups_mode()
            .is_enabled(
                self.attributes(),
                config_util::use_auto_exec_groups(&self.configuration),
            )
    }

    /// Returns the toolchain context from the default exec group.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1081 RuleContext.getToolchainContext
    pub fn get_toolchain_context(&self) -> Option<&ResolvedToolchainContext> {
        self.toolchain_contexts
            .as_ref()
            .map(|tc| tc.get_default_toolchain_context())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1086 RuleContext.getToolchainContext
    fn get_toolchain_context_for(&self, exec_group: &str) -> Option<&ResolvedToolchainContext> {
        self.toolchain_contexts
            .as_ref()
            .and_then(|tc| tc.get_toolchain_context(exec_group))
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1091 RuleContext.getToolchainContextForToolchainType
    fn get_toolchain_context_for_toolchain_type(
        &self,
        toolchain_type: &Label,
    ) -> Option<&ResolvedToolchainContext> {
        let tcs = self.toolchain_contexts.as_ref()?;
        if let Some(toolchain_context) = tcs.get_toolchain_context(&toolchain_type.to_string()) {
            if toolchain_context
                .for_toolchain_type(toolchain_type)
                .is_some()
            {
                // Return early if name of the Automatic Exec Group (AEG) and toolchain type
                // matches.
                return Some(toolchain_context);
            }
        }
        // Alias can be used for toolchains, in which case name of AEG will not match with the
        // toolchain type in its ResolvedToolchainContext (AEGs are created before toolchain
        // context is resolved).
        let alias_name = tcs
            .get_exec_group_names()
            .into_iter()
            .filter(|name| blaze_packages::rule_class::DeclaredExecGroup::is_automatic(name))
            .find(|name| {
                tcs.get_toolchain_context(name).is_some_and(|context| {
                    context
                        .requested_toolchain_type_labels()
                        .contains_key(toolchain_type)
                })
            });
        alias_name.and_then(|n| tcs.get_toolchain_context(n))
    }

    /// Returns the toolchain info (a `ToolchainInfo` value) for the given type.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1121 RuleContext.getToolchainInfo
    pub fn get_toolchain_info(&self, toolchain_type: &Label) -> Option<OwnedFrozenValue> {
        let toolchain_context = if self.use_auto_exec_groups() {
            self.get_toolchain_context_for_toolchain_type(toolchain_type)
        } else {
            self.get_toolchain_context()
        };
        toolchain_context.and_then(|c| c.for_toolchain_type(toolchain_type))
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1131 RuleContext.hasToolchainContext
    pub fn has_toolchain_context(&self, exec_group: &str) -> bool {
        self.toolchain_contexts
            .as_ref()
            .is_some_and(|tc| tc.has_toolchain_context(exec_group))
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1136 RuleContext.getToolchainContexts
    pub fn get_toolchain_contexts(
        &self,
    ) -> Option<&Arc<ToolchainCollection<ResolvedToolchainContext>>> {
        self.toolchain_contexts.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1140 RuleContext.getExecGroups
    pub fn get_exec_groups(&self) -> &ExecGroupCollection {
        &self.exec_group_collection
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1152 RuleContext.getRuleClassProvider
    pub fn get_rule_class_provider(&self) -> &Arc<ConfiguredRuleClassProvider> {
        &self.rule_class_provider
    }

    /// Returns the configuration fragments this rule uses if it should be included for this
    /// rule. Otherwise it returns `None`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1161 RuleContext.getRequiredConfigFragments
    pub fn get_required_config_fragments(&self) -> Option<RequiredConfigFragmentsProvider> {
        let required = self.required_config_fragments.as_ref()?;
        let mut merged: Option<crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProviderBuilder> = None;
        // Add variables accessed through ctx.var, if this is a Starlark rule.
        if let Some(vars) = self.starlark_looked_up_variables.lock().as_ref() {
            for make_variable in vars {
                if self.is_user_defined_make_variable(make_variable) {
                    merged
                        .get_or_insert_with(|| {
                            let mut b = RequiredConfigFragmentsProvider::builder();
                            b.merge(Some(required));
                            b
                        })
                        .add_define(make_variable);
                }
            }
        }
        // Add variables accessed through Make variable substitution.
        for expander in self.make_variable_expanders.lock().iter() {
            for make_variable in expander.lock().iter() {
                if self.is_user_defined_make_variable(make_variable) {
                    merged
                        .get_or_insert_with(|| {
                            let mut b = RequiredConfigFragmentsProvider::builder();
                            b.merge(Some(required));
                            b
                        })
                        .add_define(make_variable);
                }
            }
        }
        Some(match merged {
            None => required.clone(),
            Some(m) => m.build(),
        })
    }

    /// Returns the set of transitive packages, only intended to be used to create the repo
    /// mapping manifest for the runfiles tree.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1202 RuleContext.getTransitivePackagesForRunfileRepoMappingManifest
    pub fn get_transitive_packages_for_runfile_repo_mapping_manifest(
        &self,
    ) -> Option<
        &NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        self.transitive_packages_for_runfile_repo_mapping_manifest
            .as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1206 RuleContext.isUserDefinedMakeVariable
    fn is_user_defined_make_variable(&self, make_variable: &str) -> bool {
        // User-defined make values may be set either in "--define foo=bar" or in a vardef in the
        // rule's package. Both are equivalent for these purposes, since in both cases setting
        // "--define foo=bar" impacts the rule's output.
        self.rule
            .get_package_declarations()
            .get_make_environment()
            .contains_key(make_variable)
            || self
                .configuration
                .get_command_line_build_variables()
                .contains_key(make_variable)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1144 RuleContext.targetPlatformHasConstraint
    pub fn target_platform_has_constraint(
        &self,
        constraint_value: &crate::platform::ConstraintValueInfo,
    ) -> bool {
        // PORT: ToolchainCollection.getTargetPlatform() is never null in the Rust port.
        let Some(toolchain_contexts) = &self.toolchain_contexts else {
            return false;
        };
        // All toolchain contexts should have the same target platform so we access via the
        // default.
        toolchain_contexts
            .get_target_platform()
            .constraints()
            .has_constraint_value(constraint_value)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1216 RuleContext.getExecutionPlatform
    pub fn get_execution_platform(&self) -> Option<Arc<PlatformInfo>> {
        self.get_toolchain_context()
            .map(|c| c.execution_platform().clone())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1225 RuleContext.getExecutionPlatform
    pub fn get_execution_platform_for(&self, exec_group: &str) -> Option<Arc<PlatformInfo>> {
        self.toolchain_contexts.as_ref()?;
        self.get_toolchain_context_for(exec_group)
            .map(|c| c.execution_platform().clone())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1234 RuleContext.getExecutionPlatformForToolchainType
    pub fn get_execution_platform_for_toolchain_type(
        &self,
        toolchain_type: &Label,
    ) -> Option<Arc<PlatformInfo>> {
        self.toolchain_contexts.as_ref()?;
        self.get_toolchain_context_for_toolchain_type(toolchain_type)
            .map(|c| c.execution_platform().clone())
    }

    /// For the specified attribute (which must be of type list(label)), resolves all the labels
    /// into ConfiguredTargets and returns their build artifacts.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1249 RuleContext.getPrerequisiteArtifacts
    pub fn get_prerequisite_artifacts(&self, attribute_name: &str) -> PrerequisiteArtifacts {
        PrerequisiteArtifacts::get(self, attribute_name)
    }

    /// For the specified attribute (which must be of type label), resolves the ConfiguredTarget
    /// and returns its single build artifact.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1261 RuleContext.getPrerequisiteArtifact
    pub fn get_prerequisite_artifact(&self, attribute_name: &str) -> Option<Artifact> {
        // Java reads `prerequisitesCollection` directly (the rule and base aspects' collection
        // for aspects), not `getOwningPrerequisitesCollection`.
        self.prerequisites_collection
            .get_prerequisite_artifact(attribute_name)
    }

    /// Returns a path fragment qualified by the rule name and unique fragment to disambiguate
    /// artifacts produced from the source file appearing in multiple rules.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1271 RuleContext.getUniqueDirectory
    pub fn get_unique_directory(&self, fragment: &str) -> PathFragment {
        self.get_unique_directory_fragment(&PathFragment::create(fragment))
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1282 RuleContext.getUniqueDirectory
    pub fn get_unique_directory_fragment(&self, fragment: &PathFragment) -> PathFragment {
        analysis_utils::get_unique_directory(
            self.get_label(),
            fragment,
            config_util::is_sibling_repository_layout(&self.configuration),
        )
    }

    /// Check that all targets that were specified as sources are from the same package as this
    /// rule. Output a warning or an error for every target that is imported from a different
    /// package.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1291 RuleContext.checkSrcsSamePackage
    pub fn check_srcs_same_package(&self, only_warn: bool) {
        let package_name = self.get_label().get_package_fragment().clone();
        for src_item in PrerequisiteArtifacts::get(self, "srcs").list() {
            if !src_item.is_source_artifact() {
                // In theory, we should not do this check. However, in practice, we
                // have a couple of rules that do not obey the "srcs must contain
                // files and only files" rule. Thus, we are stuck with this hack here :(
                continue;
            }
            let associated_label = src_item.get_owner().expect("source artifact owner");
            let item_package_name = associated_label.get_package_fragment();
            if *item_package_name != package_name {
                let message = format!(
                    "please do not import '{associated_label}' directly. You should either move the file to this package or depend on an appropriate rule there"
                );
                if only_warn {
                    self.attribute_warning("srcs", &message);
                } else {
                    self.attribute_error("srcs", &message);
                }
            }
        }
    }

    /// Only use from Starlark. Returns the implicit output artifact for a given output path.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1334 RuleContext.getImplicitOutputArtifact
    pub fn get_implicit_output_artifact(&self, path: &str) -> Artifact {
        self.get_package_relative_artifact(
            &PathFragment::create(path),
            &self.get_bin_or_genfiles_directory(),
        )
    }

    /// Returns the (unmodifiable, ordered) list of artifacts which are the outputs of this
    /// target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1344 RuleContext.getOutputArtifacts
    pub fn get_output_artifacts(&self) -> Vec<Artifact> {
        self.rule
            .get_output_files()
            .iter()
            .map(|out| self.create_output_artifact_for(out))
            .collect()
    }

    /// Like `get_output_artifacts` but for a singular output item. Reports an error if the "out"
    /// attribute is not a singleton.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1360 RuleContext.getOutputArtifact
    pub fn get_output_artifact(&self) -> Option<Artifact> {
        let outs = self.get_output_artifacts();
        if outs.len() != 1 {
            self.attribute_error("out", "exactly one output file required");
            if outs.is_empty() {
                return None;
            }
        }
        Some(outs[0].clone())
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1372 RuleContext.getRelatedArtifact
    pub fn get_related_artifact(&self, path_fragment: &PathFragment, extension: &str) -> Artifact {
        // Java: FileSystemUtils.replaceExtension(pathFragment, extension).
        let base = path_fragment.get_base_name();
        let stem = match base.rfind('.') {
            Some(i) => &base[..i],
            None => base,
        };
        let file = path_fragment
            .replace_name(&format!("{stem}{extension}"))
            .expect("non-empty path");
        self.get_derived_artifact(&file, &self.get_bin_directory())
    }

    /// Returns true if the target for this context is a test target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1378 RuleContext.isTestTarget
    pub fn is_test_target(&self) -> bool {
        blaze_packages::target::target_utils::is_test_rule(self.get_target())
    }

    /// Returns true if the testonly attribute is set on this context.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1383 RuleContext.isTestOnlyTarget
    pub fn is_test_only_target(&self) -> bool {
        self.attributes().has_typed("testonly", Type::Boolean)
            && self
                .attributes()
                .get("testonly", Type::Boolean)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    }

    /// Returns true if the execution platform of the default exec group is Windows.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1388 RuleContext.isDefaultExecGroupExecutingOnWindows
    pub fn is_default_exec_group_executing_on_windows(&self) -> bool {
        crate::platform::constraint_constants::get_os_from_constraints_or_host(
            self.get_execution_platform().as_deref(),
        ) == blaze_shell::util::os::OS::Windows
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1395 RuleContext.getFeatures
    pub fn get_features(&self) -> &BTreeSet<String> {
        self.features.on()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1402 RuleContext.getDisabledFeatures
    pub fn get_disabled_features(&self) -> &BTreeSet<String> {
        self.features.off()
    }

    /// Returns true if a `RequiredConfigFragmentsProvider` should be included for this rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1415 RuleContext.shouldIncludeRequiredConfigFragmentsProvider
    pub fn should_include_required_config_fragments_provider(&self) -> bool {
        self.required_config_fragments.is_some()
    }
}

impl std::fmt::Display for RuleContext {
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1420 RuleContext.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RuleContext({}, {:?})",
            self.get_label(),
            self.configuration
        )
    }
}

impl RuleErrorConsumer for RuleContext {
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:582 RuleContext.ruleWarning
    fn rule_warning(&self, message: &str) {
        self.reporter.rule_warning(message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:573 RuleContext.ruleError
    fn rule_error(&self, message: &str) {
        self.reporter.rule_error(message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:604 RuleContext.attributeWarning
    fn attribute_warning(&self, attr_name: &str, message: &str) {
        self.reporter.attribute_warning(attr_name, message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:593 RuleContext.attributeError
    fn attribute_error(&self, attr_name: &str, message: &str) {
        self.reporter.attribute_error(attr_name, message);
    }

    fn has_errors(&self) -> bool {
        self.reporter.has_errors()
    }
}

/// The value of `RuleContext.attributes()` (Java's `AttributeMap` field): the rule's
/// `ConfiguredAttributeMapper`, or for an aspect the `AspectAwareAttributeMapper` over it.
///
/// PORT: a concrete type that implements `AttributeMap` by delegation, so callers read it
/// through the trait like Java's `AttributeMap` interface.
pub struct RuleContextAttributes(Arc<dyn AttributeMap + Send + Sync>);

impl AttributeMap for RuleContextAttributes {
    fn describe_rule(&self) -> String {
        self.0.describe_rule()
    }

    fn get_label(&self) -> &Label {
        self.0.get_label()
    }

    fn has(&self, attr_name: &str) -> bool {
        self.0.has(attr_name)
    }

    fn has_typed(&self, attr_name: &str, type_: Type) -> bool {
        self.0.has_typed(attr_name, type_)
    }

    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        self.0.get(attribute_name, type_)
    }

    fn get_or_default(
        &self,
        attribute_name: &str,
        type_: Type,
        default_value: Option<AttrValue>,
    ) -> Option<AttrValue> {
        self.0.get_or_default(attribute_name, type_, default_value)
    }

    fn is_configurable(&self, attribute_name: &str) -> bool {
        self.0.is_configurable(attribute_name)
    }

    fn get_attribute_names(&self) -> Vec<String> {
        self.0.get_attribute_names()
    }

    fn get_attribute_type(&self, attr_name: &str) -> Option<Type> {
        self.0.get_attribute_type(attr_name)
    }

    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        self.0.get_attribute_definition(attr_name)
    }

    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
        self.0
            .is_attribute_value_explicitly_specified(attribute_name)
    }

    fn visit_all_labels(&self, consumer: &mut dyn FnMut(&Attribute, &Label)) {
        self.0.visit_all_labels(consumer)
    }

    fn visit_labels(&self, attribute_name: &str, consumer: &mut dyn FnMut(&Label)) {
        self.0.visit_labels(attribute_name, consumer)
    }

    fn visit_labels_with_filter(
        &self,
        filter: &blaze_packages::attribute::DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        self.0.visit_labels_with_filter(filter, consumer)
    }

    fn get_package_args(&self) -> blaze_packages::package::PackageArgs {
        self.0.get_package_args()
    }
}

/// Builder class for a RuleContext.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:1425 RuleContext.Builder
pub struct RuleContextBuilder {
    /// `conflictFinder` (`() -> null` by default).
    conflict_finder: crate::rule::action_conflict_exception::ConflictFinderSupplier,
    env: Arc<CachingAnalysisEnvironment>,
    target: Target,
    aspects: Vec<Aspect>,
    configuration: Arc<BuildConfigurationValue>,
    reporter: Arc<RuleReporter>,
    rule_class_provider: Option<Arc<ConfiguredRuleClassProvider>>,
    configuration_fragment_policy: Option<ConfigurationFragmentPolicy>,
    action_owner_symbol: Option<ActionLookupKeyRef>,
    prerequisite_map: Option<PrerequisiteMap>,
    allow_materializer_rule_real_deps: bool,
    materializer_targets: Option<PrerequisiteMap>,
    config_conditions: Option<ConfigConditions>,
    visibility: Option<Visibility>,
    transitive_visibility_imposed_by_this_package: Option<Arc<PackageSpecificationProvider>>,
    toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
    base_target_toolchain_contexts:
        Option<Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>>,
    exec_group_collection_builder: Option<ExecGroupCollectionBuilder>,
    raw_exec_properties: Option<IndexMap<String, String>>,
    required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    transitive_packages_for_runfile_repo_mapping_manifest: Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    >,
}

impl RuleContextBuilder {
    /// `aspects` are the aspects applied to the target, the main aspect last (empty for a
    /// rule).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1456 RuleContext.Builder.<init>
    pub fn new(
        env: Arc<CachingAnalysisEnvironment>,
        target: Target,
        aspects: Vec<Aspect>,
        configuration: Arc<BuildConfigurationValue>,
    ) -> RuleContextBuilder {
        let rule = target
            .get_associated_rule()
            .expect("NullPointerException: target has no associated rule")
            .clone();
        let reporter = if config_util::allow_analysis_failures(&configuration) {
            RuleReporter::Suppressing(SuppressingErrorReporter::new())
        } else {
            RuleReporter::Events(ErrorReporter::new(
                env.clone(),
                rule.clone(),
                configuration.clone(),
                Self::rule_class_name_for_logging(&rule, &aspects),
            ))
        };
        RuleContextBuilder {
            conflict_finder: crate::rule::action_conflict_exception::no_conflict_finder(),
            env,
            target,
            aspects,
            configuration,
            reporter: Arc::new(reporter),
            rule_class_provider: None,
            configuration_fragment_policy: None,
            action_owner_symbol: None,
            prerequisite_map: None,
            allow_materializer_rule_real_deps: false,
            materializer_targets: None,
            config_conditions: None,
            visibility: None,
            transitive_visibility_imposed_by_this_package: None,
            toolchain_contexts: None,
            base_target_toolchain_contexts: None,
            exec_group_collection_builder: None,
            raw_exec_properties: None,
            required_config_fragments: None,
            transitive_packages_for_runfile_repo_mapping_manifest: None,
        }
    }

    /// Same as `build`, except without some attribute checks. Don't use outside of testing.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1482 RuleContext.Builder.unsafeBuild
    pub fn unsafe_build(&self) -> Result<RuleContext, InvalidExecGroupException> {
        self.build_with(false)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1487 RuleContext.Builder.build
    pub fn build(&self) -> Result<RuleContext, InvalidExecGroupException> {
        self.build_with(true)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1491 RuleContext.Builder.build
    fn build_with(&self, attribute_checks: bool) -> Result<RuleContext, InvalidExecGroupException> {
        let rule_class_provider = self
            .rule_class_provider
            .as_ref()
            .expect("ruleClassProvider");
        let _ = rule_class_provider;
        check_state!(self.configuration_fragment_policy.is_some());
        check_state!(self.action_owner_symbol.is_some());
        check_state!(self.prerequisite_map.is_some());
        check_state!(self.config_conditions.is_some());
        check_state!(self.visibility.is_some());
        let rule_attributes = Arc::new(ConfiguredAttributeMapper::of(
            self.get_rule().clone(),
            self.config_conditions
                .as_ref()
                .unwrap()
                .as_providers()
                .clone()
                .into(),
            &self.configuration,
        ));
        let target_map = self.create_target_map();
        self.validate_extra_prerequisites(attribute_checks, &rule_attributes);

        let exec_group_collection = self.create_exec_group_collection(rule_attributes.as_ref())?;
        if self.aspects.is_empty() {
            Ok(RuleContext::create(
                self,
                rule_attributes,
                target_map,
                exec_group_collection,
            ))
        } else {
            Ok(AspectContext::create(
                self,
                rule_attributes,
                target_map,
                exec_group_collection,
                self.base_target_toolchain_contexts.clone(),
            ))
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1515 RuleContext.Builder.createExecGroupCollection
    fn create_exec_group_collection(
        &self,
        attributes: &dyn AttributeMap,
    ) -> Result<ExecGroupCollection, InvalidExecGroupException> {
        let raw_exec_properties = match &self.raw_exec_properties {
            Some(p) => p.clone(),
            None => {
                if !attributes.has_typed(EXEC_PROPERTIES_ATTR, Type::StringDict) {
                    IndexMap::new()
                } else {
                    attributes
                        .get(EXEC_PROPERTIES_ATTR, Type::StringDict)
                        .and_then(|v| v.as_string_dict().cloned())
                        .unwrap_or_default()
                }
            }
        };
        self.exec_group_collection_builder
            .as_ref()
            .expect("NullPointerException: execGroupCollectionBuilder")
            .build(
                self.toolchain_contexts.as_deref(),
                &raw_exec_properties,
                &Target::Rule(self.get_rule().clone()).get_display_form_label(),
            )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1532 RuleContext.Builder.checkAttributesNonEmpty
    fn check_attributes_non_empty(&self, attributes: &dyn AttributeMap) {
        for attribute_name in attributes.get_attribute_names() {
            let attr = attributes
                .get_attribute_definition(&attribute_name)
                .expect("attribute");
            if !attr.is_non_empty() {
                continue;
            }
            let attribute_value = attributes.get(&attribute_name, attr.get_type());
            // TODO(adonovan): define in terms of Starlark.len?
            let is_empty = match &attribute_value {
                Some(AttrValue::StringList(l)) => l.is_empty(),
                Some(AttrValue::LabelList(l)) => l.is_empty(),
                Some(AttrValue::IntList(l)) => l.is_empty(),
                Some(AttrValue::StringSet(l)) => l.is_empty(),
                Some(AttrValue::StringDict(m)) => m.is_empty(),
                Some(AttrValue::StringListDict(m)) => m.is_empty(),
                Some(AttrValue::LabelDictUnary(m)) => m.is_empty(),
                Some(AttrValue::LabelKeyedStringDict(m)) => m.is_empty(),
                Some(AttrValue::LabelListDict(m)) => m.is_empty(),
                _ => false,
            };
            if is_empty {
                self.reporter
                    .attribute_error(attr.get_name(), "attribute must be non empty");
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1553 RuleContext.Builder.checkAttributesForDuplicateLabels
    fn check_attributes_for_duplicate_labels(&self, attributes: &ConfiguredAttributeMapper) {
        for attribute_name in attributes.get_attribute_names() {
            let attr = attributes
                .get_attribute_definition(&attribute_name)
                .expect("attribute");
            if attr.get_type() != Type::LabelList {
                // It is not obvious but correct to skip LABEL_LIST_DICT here: since concatenating
                // selects of dicts of lists does not concatenate the lists but picks the last one
                // for each key, all possible duplicates have already been ruled out by
                // AggregatingAttributeMapper.
                continue;
            }
            let duplicates = attributes.check_for_duplicate_labels(attr);
            for label in duplicates {
                self.reporter
                    .attribute_error(attr.get_name(), &format!("Label '{label}' is duplicated"));
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1571 RuleContext.Builder.setRuleClassProvider
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1699 RuleContext.Builder.setConflictFinder
    pub fn set_conflict_finder(
        &mut self,
        conflict_finder: crate::rule::action_conflict_exception::ConflictFinderSupplier,
    ) -> &mut Self {
        self.conflict_finder = conflict_finder;
        self
    }

    pub fn set_rule_class_provider(&mut self, p: Arc<ConfiguredRuleClassProvider>) -> &mut Self {
        self.rule_class_provider = Some(p);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1577 RuleContext.Builder.setAllowMaterializerRuleRealDeps
    pub fn set_allow_materializer_rule_real_deps(&mut self, v: bool) -> &mut Self {
        self.allow_materializer_rule_real_deps = v;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1583 RuleContext.Builder.setConfigurationFragmentPolicy
    pub fn set_configuration_fragment_policy(
        &mut self,
        policy: ConfigurationFragmentPolicy,
    ) -> &mut Self {
        self.configuration_fragment_policy = Some(policy);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1589 RuleContext.Builder.setActionOwnerSymbol
    pub fn set_action_owner_symbol(&mut self, symbol: ActionLookupKeyRef) -> &mut Self {
        self.action_owner_symbol = Some(symbol);
        self
    }

    /// PORT: Java's `setMutability` creates the Starlark thread's mutability; the Starlark
    /// thread is agent S's, so there is nothing to set.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1595 RuleContext.Builder.setMutability
    pub fn set_mutability(&mut self, _what: &str) -> &mut Self {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1601 RuleContext.Builder.setVisibility
    pub fn set_visibility(&mut self, visibility: Visibility) -> &mut Self {
        self.visibility = Some(visibility);
        self
    }

    /// Sets the prerequisites and checks their visibility.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1611 RuleContext.Builder.setPrerequisites
    pub fn set_prerequisites(&mut self, prerequisite_map: PrerequisiteMap) -> &mut Self {
        self.prerequisite_map = Some(prerequisite_map);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1618 RuleContext.Builder.setMaterializerTargets
    pub fn set_materializer_targets(&mut self, targets: Option<PrerequisiteMap>) -> &mut Self {
        self.materializer_targets = targets;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1629 RuleContext.Builder.setConfigConditions
    pub fn set_config_conditions(&mut self, config_conditions: ConfigConditions) -> &mut Self {
        self.config_conditions = Some(config_conditions);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1635 RuleContext.Builder.setTransitiveVisibilityImposedByThisPackage
    pub fn set_transitive_visibility_imposed_by_this_package(
        &mut self,
        transitive_visibility: Option<Arc<PackageSpecificationProvider>>,
    ) -> &mut Self {
        self.transitive_visibility_imposed_by_this_package = transitive_visibility;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1644 RuleContext.Builder.setToolchainContexts
    pub fn set_toolchain_contexts(
        &mut self,
        toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
    ) -> &mut Self {
        check_state!(
            self.toolchain_contexts.is_none(),
            "toolchainContexts has already been set for this Builder"
        );
        self.toolchain_contexts = toolchain_contexts;
        self
    }

    /// Sets the collection of `AspectBaseTargetResolvedToolchainContext`s available to this
    /// aspect from its base target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1658 RuleContext.Builder.setBaseTargetToolchainContexts
    pub fn set_base_target_toolchain_contexts(
        &mut self,
        base_target_toolchain_contexts: Option<
            Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>,
        >,
    ) -> &mut Self {
        check_state!(
            self.base_target_toolchain_contexts.is_none(),
            "baseTargetToolchainContexts has already been set for this Builder"
        );
        self.base_target_toolchain_contexts = base_target_toolchain_contexts;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1668 RuleContext.Builder.setExecGroupCollectionBuilder
    pub fn set_exec_group_collection_builder(
        &mut self,
        b: ExecGroupCollectionBuilder,
    ) -> &mut Self {
        self.exec_group_collection_builder = Some(b);
        self
    }

    /// Warning: if you set the exec properties using this method any exec_properties attribute
    /// value will be ignored in favor of this value.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1679 RuleContext.Builder.setExecProperties
    pub fn set_exec_properties(&mut self, exec_properties: IndexMap<String, String>) -> &mut Self {
        self.raw_exec_properties = Some(exec_properties);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1685 RuleContext.Builder.setRequiredConfigFragments
    pub fn set_required_config_fragments(
        &mut self,
        required: Option<RequiredConfigFragmentsProvider>,
    ) -> &mut Self {
        self.required_config_fragments = required;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1692 RuleContext.Builder.setTransitivePackagesForRunfileRepoMappingManifest
    pub fn set_transitive_packages_for_runfile_repo_mapping_manifest(
        &mut self,
        packages: Option<
            NestedSet<
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
            >,
        >,
    ) -> &mut Self {
        self.transitive_packages_for_runfile_repo_mapping_manifest = packages;
        self
    }

    /// Filter only attribute-based prerequisites, validate them and return them in a map from
    /// `DependencyKind` to list of configured targets.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1708 RuleContext.Builder.createTargetMap
    fn create_target_map(&self) -> Vec<(DependencyKind, Vec<ConfiguredTargetAndData>)> {
        let mut map: IndexMap<DependencyKind, Vec<ConfiguredTargetAndData>> = IndexMap::new();
        for (kind, values) in self.prerequisite_map.as_ref().unwrap() {
            let Some(attribute) = kind.get_attribute() else {
                continue;
            };
            if attribute.is_single_artifact() && values.len() > 1 {
                self.attribute_error(attribute.get_name(), "must contain a single dependency");
                continue;
            }
            for configured_target in values {
                let passes_filter = !attribute.is_silent_rule_class_filter()
                    || attribute
                        .get_allowed_rule_classes_for_labels()
                        .as_predicate_of_rule_class(&configured_target.get_rule_class());
                if passes_filter {
                    if self.aspects.is_empty()
                        || Some(self.get_main_aspect().expect("aspects").get_aspect_class())
                            == kind.get_owning_aspect()
                    {
                        // During aspects evaluation, only validate the dependencies of the main
                        // aspect. Dependencies of base aspects as well as the rule itself are
                        // checked when they are evaluated.
                        self.validate_direct_prerequisite(attribute, configured_target);
                    }
                    map.entry(kind.clone())
                        .or_default()
                        .push(configured_target.clone());
                }

                if attribute.is_for_dependency_resolution() {
                    if !configured_target.is_for_dependency_resolution()
                        && !matches!(
                            configured_target.get_configured_target(),
                            ConfiguredTarget::PackageGroup(_)
                        )
                    {
                        self.attribute_error(
                            attribute.get_name(),
                            &format!(
                                "attribute marked as available in materializers but prerequisite {} isn't",
                                AliasProvider::describe_target_with_aliases(
                                    configured_target,
                                    TargetMode::WithKind
                                )
                            ),
                        );
                    }
                }

                // Run materializer attribute validation only when constructing a context for
                // rules and not for aspects because aspects aren't materializer rules.
                if !self.for_aspect() && !self.allow_materializer_rule_real_deps {
                    // Materializer rules can depend only on dependency resolution rules via
                    // "normal" / non-dormant attributes, and everything else only via dormant
                    // attributes, except for PackageGroupConfiguredTargets and build settings.
                    if self
                        .get_rule()
                        .get_rule_class_object()
                        .is_materializer_rule()
                        && !(attribute.get_type() == Type::DormantLabel
                            || attribute.get_type() == Type::DormantLabelList)
                        && !configured_target.is_for_dependency_resolution()
                        && !matches!(
                            configured_target.get_configured_target(),
                            ConfiguredTarget::PackageGroup(_)
                        )
                    {
                        self.attribute_error(
                            attribute.get_name(),
                            &format!(
                                "materializer rules can depend on only dependency resolution rules via non-dormant attributes; {} is a non-dormant attribute, and {} is not a dependency resolution rule",
                                attribute.get_name(),
                                configured_target.get_target_label()
                            ),
                        );
                    }
                }
            }
        }

        if let Some(materializer_targets) = &self.materializer_targets {
            for (kind, values) in materializer_targets {
                let Some(attribute) = kind.get_attribute() else {
                    continue;
                };
                for materializer_target in values {
                    self.validate_direct_prerequisite(attribute, materializer_target);
                }
            }
        }

        map.into_iter().collect()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1819 RuleContext.Builder.badPrerequisiteMessage
    fn bad_prerequisite_message(
        prerequisite: &ConfiguredTargetAndData,
        reason: Option<&str>,
        is_warning: bool,
    ) -> String {
        let msg_reason = match reason {
            Some(r) => format!(" ({r})"),
            None => String::new(),
        };
        if is_warning {
            return format!(
                "{} is unexpected here{}; continuing anyway",
                AliasProvider::describe_target_with_aliases(prerequisite, TargetMode::WithKind),
                msg_reason
            );
        }
        format!(
            "{} is misplaced here{}",
            AliasProvider::describe_target_with_aliases(prerequisite, TargetMode::WithKind),
            msg_reason
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1832 RuleContext.Builder.reportBadPrerequisite
    fn report_bad_prerequisite(
        &self,
        attribute: &Attribute,
        prerequisite: &ConfiguredTargetAndData,
        reason: Option<&str>,
        is_warning: bool,
    ) {
        let message = Self::bad_prerequisite_message(prerequisite, reason, is_warning);
        if is_warning {
            self.attribute_warning(attribute.get_name(), &message);
        } else {
            self.attribute_error(attribute.get_name(), &message);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1845 RuleContext.Builder.validateDirectPrerequisiteType
    fn validate_direct_prerequisite_type(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        if prerequisite
            .get_rule_class_object()
            .is_some_and(|rc| rc.is_materializer_rule())
        {
            // Materializer rules pass along other targets, so don't check their providers.
            return;
        }

        let rule_class = prerequisite.get_rule_class();
        if !rule_class.is_empty() {
            self.validate_rule_dependency(prerequisite, attribute);
            return;
        }

        if !(prerequisite.is_target_file() && attribute.is_strict_label_checking_enabled()) {
            return;
        }

        let prerequisite_target_label = prerequisite.get_target_label();
        let allowed = attribute.get_allowed_file_types_predicate();
        if allowed.is_some_and(|a| a.apply(prerequisite_target_label.get_name())) {
            return;
        }

        if prerequisite.is_target_input_file()
            && !prerequisite.get_input_path().is_some_and(|p| p.exists())
        {
            // Misplaced labels, no corresponding target exists
            if allowed.is_some_and(|a| *a == FileTypeSet::NoFile)
                && !prerequisite_target_label.get_name().contains('.')
            {
                // There are no allowed files in the attribute but it's not a valid rule,
                // and the filename doesn't contain a dot --> probably a misspelled rule
                self.attribute_error(
                    attribute.get_name(),
                    &format!("rule '{prerequisite_target_label}' does not exist"),
                );
            } else {
                self.attribute_error(
                    attribute.get_name(),
                    &format!("target '{prerequisite_target_label}' does not exist"),
                );
            }
            return;
        }
        // The file exists but has a bad extension
        self.report_bad_prerequisite(
            attribute,
            prerequisite,
            Some(&format!("expected {}", file_type_set_to_string(allowed))),
            false,
        );
    }

    /// Returns whether the context being constructed is for the evaluation of an aspect.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1889 RuleContext.Builder.forAspect
    pub fn for_aspect(&self) -> bool {
        !self.aspects.is_empty()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1893 RuleContext.Builder.getRule
    pub fn get_rule(&self) -> &Arc<Rule> {
        self.target
            .get_associated_rule()
            .expect("target has an associated rule")
    }

    /// Returns the `StarlarkSemantics` governing the building of this rule.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1901 RuleContext.Builder.getStarlarkSemantics
    pub fn get_starlark_semantics(&self) -> &Arc<StarlarkSemantics> {
        self.env.get_starlark_semantics()
    }

    /// Returns a rule class name suitable for log messages, including an aspect name if
    /// applicable.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1909 RuleContext.Builder.getRuleClassNameForLogging
    pub fn get_rule_class_name_for_logging(&self) -> String {
        Self::rule_class_name_for_logging(self.get_rule(), &self.aspects)
    }

    /// The body of `getRuleClassNameForLogging` (also used by the constructor, before `self`
    /// exists).
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1909 RuleContext.Builder.getRuleClassNameForLogging
    fn rule_class_name_for_logging(rule: &Rule, aspects: &[Aspect]) -> String {
        if aspects.is_empty() {
            return rule.get_rule_class().to_owned();
        }
        // Java: Joiner.on(",").join(descriptors) (AspectDescriptor.toString() is
        // getDescription()).
        let descriptors: Vec<String> = aspects
            .iter()
            .map(|a| a.get_descriptor().get_description())
            .collect();
        format!(
            "{} aspect on {}",
            descriptors.join(","),
            rule.get_rule_class()
        )
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1919 RuleContext.Builder.getErrorConsumer
    pub(crate) fn get_error_consumer(&self) -> &Arc<RuleReporter> {
        &self.reporter
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1923 RuleContext.Builder.getConfiguration
    pub fn get_configuration(&self) -> &Arc<BuildConfigurationValue> {
        &self.configuration
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1928 RuleContext.Builder.getMainAspect
    pub fn get_main_aspect(&self) -> Option<&Aspect> {
        self.aspects.last()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1932 RuleContext.Builder.getAspects
    pub fn get_aspects(&self) -> &[Aspect] {
        &self.aspects
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1936 RuleContext.Builder.isStarlarkRuleOrAspect
    pub fn is_starlark_rule_or_aspect(&self) -> bool {
        if let Some(main_aspect) = self.get_main_aspect() {
            return matches!(main_aspect.get_aspect_class(), AspectClass::Starlark(_));
        }
        self.get_rule()
            .get_rule_class_object()
            .get_rule_definition_environment_label()
            .is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1945 RuleContext.Builder.validateDirectPrerequisiteFileTypes
    fn validate_direct_prerequisite_file_types(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        if attribute.is_skip_analysis_time_file_type_check() {
            return;
        }
        let Some(allowed_file_types) = attribute.get_allowed_file_types_predicate() else {
            // It's not a label or label_list attribute.
            return;
        };
        if *allowed_file_types == FileTypeSet::AnyFile
            && !attribute.is_non_empty()
            && !attribute.is_single_artifact()
        {
            return;
        }

        // If we allow any file we still need to check if there are actually files generated
        // Note that this check only runs for ANY_FILE predicates if the attribute is NON_EMPTY
        // or SINGLE_ARTIFACT
        // If we performed this check when allowedFileTypes == NO_FILE this would
        // always throw an error in those cases
        if *allowed_file_types != FileTypeSet::NoFile {
            let artifacts = crate::configuredtargets::get_provider::<FileProvider>(
                prerequisite.get_configured_target(),
            )
            .expect("FileProvider")
            .get_files_to_build()
            .clone();
            if attribute.is_single_artifact() && !artifacts.is_singleton() {
                self.attribute_error(
                    attribute.get_name(),
                    &format!(
                        "'{}' must produce a single file",
                        prerequisite.get_target_label()
                    ),
                );
                return;
            }
            for source_artifact in artifacts.to_list().iter() {
                if allowed_file_types.apply(source_artifact.get_filename()) {
                    return;
                }
                if source_artifact.is_tree_artifact() {
                    return;
                }
            }
            self.attribute_error(
                attribute.get_name(),
                &format!(
                    "'{}' does not produce any {} {} files (expected {})",
                    prerequisite.get_target_label(),
                    self.get_rule_class_name_for_logging(),
                    attribute.get_name(),
                    file_type_set_to_string(Some(allowed_file_types))
                ),
            );
        }
    }

    /// A dependency is valid if it is from a rule in allowedRuledClasses, OR if all of the
    /// providers in requiredProviders are provided by the target.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2002 RuleContext.Builder.validateRuleDependency
    fn validate_rule_dependency(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) {
        let mut unfulfilled_requirements: IndexSet<String> = IndexSet::new();
        if Self::check_rule_dependency_class(prerequisite, attribute, &mut unfulfilled_requirements)
        {
            return;
        }

        if self.check_rule_dependency_class_warnings(prerequisite, attribute) {
            return;
        }

        if Self::check_rule_dependency_mandatory_providers(
            prerequisite,
            attribute,
            &mut unfulfilled_requirements,
        ) {
            return;
        }

        // not allowed rule class and some mandatory providers missing => reject.
        if !unfulfilled_requirements.is_empty() {
            let reqs: Vec<&String> = unfulfilled_requirements.iter().collect();
            self.attribute_error(
                attribute.get_name(),
                &blaze_packages::attribute::string_util::join_english_list_full(
                    &reqs, "and", "", true,
                ),
            );
        }
    }

    /// Check if prerequisite should be allowed based on its rule class.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2025 RuleContext.Builder.checkRuleDependencyClass
    fn check_rule_dependency_class(
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
        unfulfilled_requirements: &mut IndexSet<String>,
    ) -> bool {
        let predicate = attribute.get_allowed_rule_classes_for_labels();
        if predicate.get_predicate_type() == RuleClassNamePredicateType::Unspecified {
            // alwaysTrue is a special sentinel value. See
            // RuleClass.Builder.RuleClassNamePredicate.unspecified.
            return false;
        }
        if predicate.as_predicate_of_rule_class(&prerequisite.get_rule_class()) {
            // prerequisite has an allowed rule class => accept.
            return true;
        }
        // remember that the rule class that was not allowed;
        // but maybe prerequisite provides required providers? do not reject yet.
        unfulfilled_requirements.insert(Self::bad_prerequisite_message(
            prerequisite,
            Some(&format!("expected {}", predicate.describe())),
            false,
        ));
        false
    }

    /// Check if prerequisite should be allowed with warning based on its rule class. If yes, also
    /// issues said warning.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2052 RuleContext.Builder.checkRuleDependencyClassWarnings
    fn check_rule_dependency_class_warnings(
        &self,
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
    ) -> bool {
        // PORT: Java's unset warning predicate is `Predicates.alwaysFalse()` (the builder's
        // default); an unspecified Rust predicate stands for it.
        let warning_predicate = attribute.get_allowed_rule_classes_for_labels_warning();
        if warning_predicate.get_predicate_type() == RuleClassNamePredicateType::Unspecified
            || !warning_predicate.as_predicate_of_rule_class(&prerequisite.get_rule_class())
        {
            return false;
        }
        let allowed_rule_classes = attribute.get_allowed_rule_classes_for_labels();
        let reason = if allowed_rule_classes.get_predicate_type()
            == RuleClassNamePredicateType::Unspecified
        {
            None
        } else {
            Some(format!("expected {}", allowed_rule_classes.describe()))
        };
        self.report_bad_prerequisite(attribute, prerequisite, reason.as_deref(), true);
        // prerequisite has a rule class allowed with a warning => accept, emitting a warning.
        true
    }

    /// Check if prerequisite should be allowed based on required providers on the attribute.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2071 RuleContext.Builder.checkRuleDependencyMandatoryProviders
    fn check_rule_dependency_mandatory_providers(
        prerequisite: &ConfiguredTargetAndData,
        attribute: &Attribute,
        unfulfilled_requirements: &mut IndexSet<String>,
    ) -> bool {
        let required_providers = attribute.get_required_providers();
        if required_providers.accepts_any() {
            // If no required providers specified, we do not know if we should accept.
            return false;
        }
        if crate::configuredtargets::satisfies(
            prerequisite.get_configured_target(),
            required_providers,
        ) {
            return true;
        }
        unfulfilled_requirements.insert(format!(
            "'{}' does not have mandatory providers: {}",
            prerequisite.get_target_label(),
            crate::configuredtargets::missing_providers(
                prerequisite.get_configured_target(),
                required_providers
            )
            .get_description()
        ));
        false
    }

    /// Perform extra validation of prerequisites. Standard attribute-based dependencies are
    /// already validated as part of `create_target_map`.
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2102 RuleContext.Builder.validateExtraPrerequisites
    fn validate_extra_prerequisites(
        &self,
        attribute_checks: bool,
        attributes: &ConfiguredAttributeMapper,
    ) {
        // These checks can fail when ConfigConditions.EMPTY are empty, resulting in noMatchError
        // accessing attributes without a default condition.
        if attribute_checks && self.target.is_rule() {
            self.check_attributes_non_empty(attributes);
            self.check_attributes_for_duplicate_labels(attributes);
        }

        // This conditionally checks visibility on config_setting rules based on
        // --config_setting_visibility_policy. This should be removed as soon as it's deemed safe
        // to unconditionally check visibility. See
        // https://github.com/bazelbuild/bazel/issues/12669.
        let config_setting_visibility_policy = self
            .target
            .get_package_metadata()
            .config_setting_visibility_policy;
        if config_setting_visibility_policy != Some(ConfigSettingVisibilityPolicy::LegacyOff) {
            // Validate config conditions.
            // Java: a rule class without `$config_dependencies` (a dependency resolution rule)
            // has no config conditions, so the null attribute is never used.
            let config_setting_attr = attributes
                .get_attribute_definition(CONFIG_SETTING_DEPS_ATTRIBUTE)
                .cloned();
            for condition in self
                .config_conditions
                .as_ref()
                .unwrap()
                .as_configured_targets()
                .values()
            {
                // When just --incompatible_enforce_config_setting_visibility is on, that means
                // "enforce config_setting visibility with public default". In that case, we
                // continue to ignore the alias' visibility in preference for the config_setting.
                // When we also turn on --incompatible_config_setting_private_default_visibility,
                // we expect full standard visibility compliance.
                let checked = if config_setting_visibility_policy
                    == Some(ConfigSettingVisibilityPolicy::DefaultPublic)
                {
                    condition.from_configured_target_no_check(
                        condition.get_configured_target().get_actual(),
                    )
                } else {
                    condition.clone()
                };
                self.validate_direct_prerequisite(
                    config_setting_attr
                        .as_ref()
                        .expect("NullPointerException: $config_dependencies"),
                    &checked,
                );
            }
        }

        // Validate toolchains.
        if let Some(tcs) = &self.toolchain_contexts {
            for toolchain_context in tcs.context_map().values() {
                for prerequisite in toolchain_context.prerequisite_targets() {
                    self.validate_direct_prerequisite(toolchain_attribute(), prerequisite);
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:2165 RuleContext.Builder.validateDirectPrerequisite
    fn validate_direct_prerequisite(
        &self,
        attribute: &Attribute,
        prerequisite: &ConfiguredTargetAndData,
    ) {
        self.validate_direct_prerequisite_type(prerequisite, attribute);
        self.validate_direct_prerequisite_file_types(prerequisite, attribute);
        if attribute.perform_prereq_validator_check() {
            // PORT: Java calls `ruleClassProvider.getPrerequisiteValidator()`; Bazel's is
            // always `BazelPrerequisiteValidator`.
            BazelPrerequisiteValidator.validate(self, prerequisite, attribute);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1893 RuleContext.Builder.getRule
    pub fn get_target(&self) -> &Target {
        &self.target
    }
}

impl RuleErrorConsumer for RuleContextBuilder {
    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1805 RuleContext.Builder.ruleWarning
    fn rule_warning(&self, message: &str) {
        self.reporter.rule_warning(message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1795 RuleContext.Builder.ruleError
    fn rule_error(&self, message: &str) {
        self.reporter.rule_error(message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1810 RuleContext.Builder.attributeWarning
    fn attribute_warning(&self, attr_name: &str, message: &str) {
        self.reporter.attribute_warning(attr_name, message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1800 RuleContext.Builder.attributeError
    fn attribute_error(&self, attr_name: &str, message: &str) {
        self.reporter.attribute_error(attr_name, message);
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:1815 RuleContext.Builder.hasErrors
    fn has_errors(&self) -> bool {
        self.reporter.has_errors()
    }
}

/// Java `FileTypeSet.toString()`.
// Java: com/google/devtools/build/lib/util/FileTypeSet.java:144 FileTypeSet.toString
// Java: com/google/devtools/build/lib/util/FileTypeSet.java:136 FileTypeSet.getExtensions
pub fn file_type_set_to_string(set: Option<&FileTypeSet>) -> String {
    match set {
        None => "null".to_owned(),
        Some(FileTypeSet::AnyFile) => "any files".to_owned(),
        Some(FileTypeSet::NoFile) => "no files".to_owned(),
        Some(FileTypeSet::Of(types)) => {
            let extensions: Vec<String> = types
                .iter()
                .flat_map(|t| t.get_extensions().iter().cloned())
                .collect();
            blaze_packages::attribute::string_util::join_english_list(&extensions)
        }
    }
}

/// Java `RuleContext.getRelatedArtifact`'s helper exposed for the Starlark context.
pub fn output_file_artifact(rule_context: &RuleContext, out: &OutputFile) -> Artifact {
    rule_context.create_output_artifact_for(out)
}

/// Re-exported so callers can name the rule's package-relative root types.
pub type RulePackageIdentifier = PackageIdentifier;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_type_set_strings() {
        assert_eq!(
            file_type_set_to_string(Some(&FileTypeSet::AnyFile)),
            "any files"
        );
        assert_eq!(
            file_type_set_to_string(Some(&FileTypeSet::NoFile)),
            "no files"
        );
        assert_eq!(
            file_type_set_to_string(Some(&FileTypeSet::of(vec![
                blaze_packages::types::FileType::of(&[".cc"]),
                blaze_packages::types::FileType::of(&[".h"]),
            ]))),
            ".cc or .h"
        );
    }

    #[test]
    #[ignore = "needs a loaded package, configuration and ConfiguredAttributeMapper (RuleContextTest)"]
    fn rule_context_test() {}

    #[test]
    #[ignore = "needs analysis of BUILD files (RuleContextTest.testNestedSetFileTypeFiltering etc.)"]
    fn test_attribute_errors_with_location() {}
}
