//! Port of `analysis/RuleConfiguredTargetBuilder.java`.

use crate::constraints::environment_collection::EnvironmentCollectionBuilder;
use crate::constraints::rule_context_constraint_semantics::RuleContextConstraintSemantics;
use crate::constraints::supported_environments::SupportedEnvironments;
use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::starlark_defs::builtin_providers::{
    ANALYSIS_TEST_RESULT_INFO, INSTRUMENTED_FILES_INFO, OUTPUT_GROUP_INFO,
};
use blaze_packages::target::RuleOrMacroInstance;
use blaze_packages::target::target_utils;
use blaze_packages::types::{LabelClass, Type};
use starlark::values::OwnedFrozenValue;

use crate::configured_target::{ConfiguredTarget, ConfiguredTargetBase};
use crate::configuredtargets::RuleConfiguredTarget;
use crate::info::{downcast_info, info_provider_key};
use crate::provider_map::{
    ProviderMapKey, TransitiveInfoProvider, TransitiveInfoProviderMapBuilder,
};
use crate::providers::file_provider::{FileProvider, FilesToRunProvider, RunfilesProvider};
use crate::providers::output_group_info::{self, OutputGroupInfo};
use crate::providers::package_specification_provider::{
    PackageSpecificationProvider, TransitiveVisibilityProvider, add_unique,
};
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::rule::actions_util::{self, AssignOwnersError};
use crate::rule::config_util;
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::test::instrumented_files_info::InstrumentedFilesCollector;
use crate::test::test_action_builder::TestActionBuilder;
use crate::test::test_provider::{TestProvider, TestTagsProvider};

/// Errors of `build()` (Java `ActionConflictException`, `InterruptedException`).
#[derive(Debug, thiserror::Error)]
pub enum RuleConfiguredTargetBuildError {
    /// Java `ActionConflictException` (message).
    #[error("{0}")]
    ActionConflict(crate::rule::action_conflict_exception::ActionConflictException),
    #[error("interrupted")]
    Interrupted,
}

/// A marker provider of targets analyzed under `evaluatingForAnalysisTest` (Java
/// `RuleConfiguredTargetBuilder.TransitiveLabelsInfo`).
// Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:668 RuleConfiguredTargetBuilder.TransitiveLabelsInfo
#[derive(Clone, Debug)]
pub struct TransitiveLabelsInfo {
    labels: NestedSet<blaze_cmdline::Label>,
}

impl TransitiveLabelsInfo {
    pub fn get_labels(&self) -> &NestedSet<blaze_cmdline::Label> {
        &self.labels
    }
}

impl TransitiveInfoProvider for TransitiveLabelsInfo {
    fn java_class_name(&self) -> &'static str {
        "TransitiveLabelsInfo"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Builder class for analyzed rule instances.
///
/// This is used to tell Bazel which `TransitiveInfoProvider`s are produced by the analysis of a
/// configured target. For more information about analysis, see `RuleConfiguredTargetFactory`.
// Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:84 RuleConfiguredTargetBuilder
pub struct RuleConfiguredTargetBuilder<'a> {
    rule_context: &'a RuleContext,
    providers_builder: TransitiveInfoProviderMapBuilder,
    /// Java `TreeMap<String, NestedSetBuilder<Artifact>>` (sorted by group name).
    output_group_builders: BTreeMap<String, NestedSetBuilder<Artifact>>,
    additional_test_action_tools: Vec<Artifact>,
    files_to_build: NestedSet<Artifact>,
    runfiles_support: Option<Arc<RunfilesSupport>>,
    executable: Option<Artifact>,
}

impl<'a> RuleConfiguredTargetBuilder<'a> {
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:99 RuleConfiguredTargetBuilder.<init>
    pub fn new(rule_context: &'a RuleContext) -> RuleConfiguredTargetBuilder<'a> {
        RuleConfiguredTargetBuilder {
            rule_context,
            providers_builder: TransitiveInfoProviderMapBuilder::default(),
            output_group_builders: BTreeMap::new(),
            additional_test_action_tools: Vec::new(),
            files_to_build: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            runfiles_support: None,
            executable: None,
        }
    }

    /// Constructs the RuleConfiguredTarget instance based on the values set for this Builder.
    /// Returns `Ok(None)` if there were rule errors reported.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:108 RuleConfiguredTargetBuilder.build
    pub fn build(mut self) -> Result<Option<ConfiguredTarget>, RuleConfiguredTargetBuildError> {
        let rule_context = self.rule_context;
        let configuration = rule_context.get_configuration().clone();
        // If allowing analysis failures, the current target may not propagate all of the
        // expected providers; be lenient on such cases (for example, avoid precondition checks).
        let allow_analysis_failures = config_util::allow_analysis_failures(&configuration);

        if config_util::enforce_constraints(&configuration) {
            self.check_constraints();
        }

        for allowlist_checker in rule_context
            .get_rule()
            .get_rule_class_object()
            .get_allowlist_checkers()
            .to_vec()
        {
            self.handle_allowlist_checker(&allowlist_checker);
        }
        if config_util::enforce_transitive_visibility(&configuration) {
            // Gather the transitive_visibility from this target's package and deps. If there are
            // any, propagate their union in a TransitiveVisibilityProvider.
            let mut tv: Vec<Arc<PackageSpecificationProvider>> = Vec::new();
            if let Some(p) = rule_context.get_transitive_visibility_imposed_by_this_package() {
                add_unique(&mut tv, p.clone());
            }
            for attribute_name in rule_context.attributes().get_attribute_names() {
                let attribute = rule_context
                    .attributes()
                    .get_attribute_definition(&attribute_name)
                    .expect("attribute");
                if attribute.get_type().get_label_class() == LabelClass::Dependency {
                    for dep in rule_context.get_prerequisites(&attribute_name) {
                        if let Some(provider) = crate::configuredtargets::get_provider::<
                            TransitiveVisibilityProvider,
                        >(&dep)
                        {
                            for p in provider.get_transitive_visibility() {
                                add_unique(&mut tv, p.clone());
                            }
                        }
                    }
                }
            }
            if !tv.is_empty() {
                self.add_provider(TransitiveVisibilityProvider::new(tv));
            }
        }

        if rule_context.has_errors() && !allow_analysis_failures {
            return Ok(None);
        }

        self.maybe_add_required_config_fragments_provider();

        let runfiles_trees: NestedSet<Artifact> = match &self.runfiles_support {
            Some(rs) => NestedSetBuilder::create(
                Order::STABLE_ORDER,
                [rs.get_runfiles_tree_artifact().clone()],
            ),
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        };

        let files_to_run_provider = FilesToRunProvider::create(
            self.build_files_to_run(&runfiles_trees, &self.files_to_build),
            self.runfiles_support.clone(),
            self.executable.clone(),
        );
        self.providers_builder
            .add_native_arc(FileProvider::of(self.files_to_build.clone()));
        self.providers_builder
            .add_native_arc(files_to_run_provider.clone());

        if self.runfiles_support.is_some() {
            // If a binary is built, build its runfiles, too
            self.add_output_group(output_group_info::HIDDEN_TOP_LEVEL, &runfiles_trees);
        } else if self.providers_builder.contains_native::<RunfilesProvider>() {
            // If we don't have a RunfilesSupport (probably because this is not a binary rule),
            // we still want to build the files this rule contributes to runfiles of dependent
            // rules so that we report an error if one of these is broken.
            //
            // Note that this is a best-effort thing: there is .getDataRunfiles() and all the
            // language-specific *RunfilesProvider classes, which we don't add here for reasons
            // that are lost in the mists of time.
            let all = self
                .providers_builder
                .get_provider::<RunfilesProvider>()
                .unwrap()
                .get_default_runfiles()
                .get_all_artifacts();
            self.add_output_group(output_group_info::HIDDEN_TOP_LEVEL, &all);
        }

        if self.propagate_validation_action_output_group() {
            self.propagate_transitive_validation_output_groups();
        }

        // Add a default provider that forwards InstrumentedFilesInfo from dependencies, even if
        // this rule doesn't configure InstrumentedFilesInfo. This needs to be done for non-test
        // rules as well, but should be done before initializeTestProvider, which uses that.
        if config_util::is_code_coverage_enabled(&configuration)
            && !self
                .providers_builder
                .contains_info(&INSTRUMENTED_FILES_INFO.key())
            && !self
                .providers_builder
                .contains_info(&crate::platform::toolchain_info::ToolchainInfo::provider_key())
        {
            let info = InstrumentedFilesCollector::forward_all(rule_context);
            self.add_native_declared_provider(info);
        }
        // Create test action and artifacts if target was successfully initialized and is a
        // test. Also, as an extreme hack, only bother doing this if the TestConfiguration is
        // actually present.
        if target_utils::is_test_rule(rule_context.get_target()) {
            let test_tags: Vec<String> = rule_context
                .get_rule()
                .get_rule_tags()
                .into_iter()
                .collect();
            self.add_provider(TestTagsProvider::new(test_tags));
            if config_util::has_fragment_named(&configuration, "TestConfiguration") {
                if self.runfiles_support.is_some() {
                    let test_provider = self.initialize_test_provider(&files_to_run_provider);
                    self.add_provider(test_provider);
                } else if !allow_analysis_failures {
                    panic!("IllegalStateException: Test rules must have runfiles");
                }
            }
        }

        // Only add ExtraActionProvider if extra action listeners are applied
        if !config_util::get_action_listeners(&configuration).is_empty() {
            blaze_util::unported!("RuleConfiguredTargetBuilder.createExtraActionProvider");
        }

        if !self.output_group_builders.is_empty() {
            let info = OutputGroupInfo::from_builders(&self.output_group_builders);
            self.add_native_declared_provider(output_group_info::freeze(info));
        }

        if config_util::evaluating_for_analysis_test(&configuration) {
            if rule_context.get_rule().is_analysis_test() {
                rule_context.rule_error(&format!(
                    "analysis_test rule '{}' cannot be transitively depended on by another analysis test rule",
                    rule_context.get_label()
                ));
                return Ok(None);
            }
            let labels = self.transitive_labels();
            self.add_provider(TransitiveLabelsInfo { labels });
        }

        if rule_context.get_rule().has_analysis_test_transition() {
            let labels = self.transitive_labels();
            let dep_count = labels.memoized_flatten_and_get_size();
            let limit = config_util::analysis_testing_deps_limit(&configuration);
            if dep_count > limit {
                rule_context.rule_error(&format!(
                    "analysis test rule exceeded maximum dependency edge count. Count: {dep_count}. Limit is {limit}. This limit is imposed on analysis test rules which use analysis_test_transition attribute transitions. Exceeding this limit indicates either the analysis_test has too many dependencies, or the underlying toolchains may be large. Try decreasing the number of test dependencies, (Analysis tests should not be very large!) or, if possible, try not using configuration transitions. If underlying toolchain size is to blame, it might be worth considering increasing --analysis_testing_deps_limit. (Beware, however, that large values of this flag can lead to no safeguards against giant test suites that can lead to Out Of Memory exceptions in the build server.)"
                ));
                return Ok(None);
            }
        }

        if rule_context.get_rule().is_build_setting() {
            let build_setting = rule_context
                .get_rule()
                .get_rule_class_object()
                .get_build_setting()
                .expect("build setting rule has a BuildSetting")
                .clone();
            let default_value = rule_context.attributes().get(
                blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME,
                build_setting.get_type(),
            );
            self.providers_builder.add_native(
                crate::config::build_setting_provider::BuildSettingProvider::new(
                    build_setting,
                    default_value,
                    rule_context.get_label().clone(),
                ),
            );
        }
        let providers = self.providers_builder.build();

        if rule_context.get_rule().is_analysis_test() {
            // If the target is an analysis test that returned AnalysisTestResultInfo, register a
            // test pass/fail action on behalf of the target.
            let Some(test_result_info) = providers.get(&ANALYSIS_TEST_RESULT_INFO.key()) else {
                rule_context.rule_error(
                    "rules with analysis_test=true must return an instance of AnalysisTestResultInfo",
                );
                return Ok(None);
            };
            let test_result_info = crate::info::downcast_info::<
                crate::test::execution_info::AnalysisTestResultInfo,
            >(test_result_info)
            .expect("AnalysisTestResultInfo");
            crate::test::analysis_test_action_builder::AnalysisTestActionBuilder::write_analysis_test_action(
                rule_context,
                test_result_info,
            );
        }

        let analysis_environment = rule_context.get_analysis_environment();
        let actions = analysis_environment.get_registered_actions();
        match actions_util::assign_owners_and_throw_if_conflict_tolerating_shared_actions(
            analysis_environment.get_action_key_context(),
            &actions,
            rule_context.get_owner(),
        ) {
            Ok(()) => {}
            Err(AssignOwnersError::ArtifactGeneratedByOtherRule(m)) => {
                rule_context.rule_error(&m);
                return Ok(None);
            }
            Err(AssignOwnersError::ActionConflict(m)) => {
                return Err(RuleConfiguredTargetBuildError::ActionConflict(m));
            }
            Err(AssignOwnersError::Interrupted) => {
                return Err(RuleConfiguredTargetBuildError::Interrupted);
            }
        }

        // PORT: the conflict finder (ConflictCheckingMode.UPON_CONFIGURED_OBJECT_CREATION) is
        // not used by Blaze.
        Ok(Some(ConfiguredTarget::Rule(Arc::new(
            RuleConfiguredTarget::new(rule_context, providers, actions),
        ))))
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:297 RuleConfiguredTargetBuilder.propagateValidationActionOutputGroup
    fn propagate_validation_action_output_group(&self) -> bool {
        !self.rule_context.get_rule().is_analysis_test()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:301 RuleConfiguredTargetBuilder.handleAllowlistChecker
    fn handle_allowlist_checker(
        &self,
        allowlist_checker: &blaze_packages::rule_class::AllowlistChecker,
    ) {
        use blaze_packages::rule_class::LocationCheck;
        let rule_context = self.rule_context;
        if let Some(trigger) = &allowlist_checker.attribute_set_trigger {
            if !rule_context
                .get_rule()
                .is_attribute_value_explicitly_specified(trigger)
            {
                return;
            }
        }
        let attr = &allowlist_checker.allowlist_attr;
        let passing = match allowlist_checker.location_check {
            LocationCheck::Instance => allowlist::is_available(rule_context, attr),
            LocationCheck::Definition => {
                allowlist::is_available_based_on_rule_location(rule_context, attr)
            }
            LocationCheck::InstanceOrDefinition => {
                allowlist::is_available(rule_context, attr)
                    || allowlist::is_available_based_on_rule_location(rule_context, attr)
            }
        };
        if !passing {
            rule_context.rule_error(&allowlist_checker.error_message);
        }
    }

    // TODO(blaze-team): Simplify the conditional logic and make it easier to understand.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:333 RuleConfiguredTargetBuilder.maybeAddRequiredConfigFragmentsProvider
    fn maybe_add_required_config_fragments_provider(&mut self) {
        if self
            .rule_context
            .should_include_required_config_fragments_provider()
            && !self
                .providers_builder
                .contains_native::<RequiredConfigFragmentsProvider>()
        {
            if let Some(p) = self.rule_context.get_required_config_fragments() {
                self.add_provider(p);
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:341 RuleConfiguredTargetBuilder.transitiveLabels
    fn transitive_labels(&self) -> NestedSet<blaze_cmdline::Label> {
        let mut nested_set_builder = NestedSetBuilder::stable_order();
        let attributes = self.rule_context.attributes();
        for attribute_name in attributes.get_attribute_names() {
            let attribute_type = attributes
                .get_attribute_definition(&attribute_name)
                .expect("attribute")
                .get_type();
            if attribute_type.get_label_class() == LabelClass::Dependency {
                for labels_info in self
                    .rule_context
                    .get_prerequisites_providers::<TransitiveLabelsInfo>(&attribute_name)
                {
                    nested_set_builder.add_transitive(labels_info.get_labels());
                }
            }
        }
        nested_set_builder.add(self.rule_context.get_label().clone());
        nested_set_builder.build()
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:359 RuleConfiguredTargetBuilder.propagateTransitiveValidationOutputGroups
    fn propagate_transitive_validation_output_groups(&mut self) {
        if self
            .output_group_builders
            .contains_key(output_group_info::VALIDATION_TRANSITIVE)
        {
            let rde_label = self
                .rule_context
                .get_rule()
                .get_rule_class_object()
                .get_rule_definition_environment_label()
                .cloned();
            // only allow native and builtins to override transitive validation propagation
            if let Some(rde_label) = rde_label {
                let main_repo_mapping = self
                    .rule_context
                    .get_analysis_environment()
                    .get_main_repo_mapping()
                    .expect("main repo mapping");
                if crate::rule::builtin_restriction::is_not_allowed(
                    &rde_label,
                    main_repo_mapping,
                    crate::rule::builtin_restriction::INTERNAL_STARLARK_API_ALLOWLIST,
                ) {
                    self.rule_context.rule_error(&format!(
                        "{rde_label} cannot access the _transitive_validation private API"
                    ));
                    return;
                }
            }
            let transitive = self
                .output_group_builders
                .remove(output_group_info::VALIDATION_TRANSITIVE)
                .unwrap()
                .build();
            self.add_output_group(output_group_info::VALIDATION, &transitive);
        } else {
            let mut collected: Vec<NestedSet<Artifact>> = Vec::new();
            Self::collect_transitive_validation_output_groups(
                self.rule_context,
                &|_| true,
                &mut |validation_artifacts| collected.push(validation_artifacts),
            );
            for v in collected {
                self.add_output_group(output_group_info::VALIDATION, &v);
            }
        }
    }

    /// Collects the validation output groups of direct dependencies (excluding tools and
    /// implicit dependencies).
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:385 RuleConfiguredTargetBuilder.collectTransitiveValidationOutputGroups
    pub fn collect_transitive_validation_output_groups(
        rule_context: &RuleContext,
        include_attribute: &dyn Fn(&str) -> bool,
        consumer: &mut dyn FnMut(NestedSet<Artifact>),
    ) {
        let attributes = rule_context.attributes();
        for attribute_name in attributes.get_attribute_names() {
            if !include_attribute(&attribute_name) {
                continue;
            }
            // Validation actions for tools, or from implicit deps should not fail the overall
            // build, since those dependencies should have their own builds and tests that should
            // surface any failing validations.
            let attribute = attributes
                .get_attribute_definition(&attribute_name)
                .expect("attribute");
            if !attribute.skip_validations()
                && !attribute.is_tool_dependency()
                && !attribute.is_implicit()
                && attribute.get_type().get_label_class() == LabelClass::Dependency
            {
                for output_group in
                    rule_context.get_prerequisites_infos(&attribute_name, &OUTPUT_GROUP_INFO.key())
                {
                    let info =
                        downcast_info::<OutputGroupInfo>(&output_group).expect("OutputGroupInfo");
                    let validation_artifacts = info.get_output_group(output_group_info::VALIDATION);
                    if !validation_artifacts.is_empty() {
                        consumer(validation_artifacts);
                    }
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:412 RuleConfiguredTargetBuilder.buildFilesToRun
    fn build_files_to_run(
        &self,
        runfiles_trees: &NestedSet<Artifact>,
        files_to_build: &NestedSet<Artifact>,
    ) -> NestedSet<Artifact> {
        let mut builder = NestedSetBuilder::stable_order();
        builder.add_transitive(files_to_build);
        builder.add_transitive(runfiles_trees);
        if let Some(executable) = &self.executable {
            if self
                .rule_context
                .get_rule()
                .get_rule_class_object()
                .is_starlark()
            {
                builder.add(executable.clone());
            }
        }
        builder.build()
    }

    /// Invokes Blaze's constraint enforcement system: checks that this rule's dependencies
    /// support its environments and reports appropriate errors if violations are found.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:450 RuleConfiguredTargetBuilder.checkConstraints
    fn check_constraints(&mut self) {
        if !self
            .rule_context
            .get_rule()
            .get_rule_class_object()
            .supports_constraint_checking()
        {
            return;
        }
        // PORT: `ruleContext.getRuleClassProvider().getConstraintSemantics()` is always
        // RuleContextConstraintSemantics in Bazel.
        let supported_environments =
            RuleContextConstraintSemantics::get_supported_environments(self.rule_context);
        if let Some(supported_environments) = supported_environments {
            let mut refined_environments = EnvironmentCollectionBuilder::new();
            let mut removed_environment_culprits = indexmap::IndexMap::new();
            RuleContextConstraintSemantics::check_constraints(
                self.rule_context,
                &supported_environments,
                &mut refined_environments,
                &mut removed_environment_culprits,
            );
            self.add_provider(SupportedEnvironments::create(
                supported_environments,
                refined_environments.build(),
                removed_environment_culprits,
            ));
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:445 RuleConfiguredTargetBuilder.initializeTestProvider
    fn initialize_test_provider(
        &self,
        files_to_run_provider: &Arc<FilesToRunProvider>,
    ) -> TestProvider {
        let rule_context = self.rule_context;
        let explicit_shard_count = rule_context
            .attributes()
            .get("shard_count", Type::Integer)
            .and_then(|v| v.as_int())
            .unwrap_or(0);
        if explicit_shard_count < 0
            && rule_context
                .get_rule()
                .is_attribute_value_explicitly_specified("shard_count")
        {
            rule_context.attribute_error("shard_count", "Must not be negative.");
        }
        if explicit_shard_count > 50 {
            rule_context.attribute_error(
                "shard_count",
                "Having more than 50 shards is indicative of poor test organization. Please reduce the number of shards.",
            );
        }
        let instrumented_files = self
            .providers_builder
            .get_info(&INSTRUMENTED_FILES_INFO.key())
            .cloned();
        let execution_info = self
            .providers_builder
            .get_info(&crate::test::execution_info::EXECUTION_INFO.key())
            .cloned();
        let mut test_action_builder = TestActionBuilder::new(rule_context);
        test_action_builder.set_instrumented_files(instrumented_files);
        test_action_builder
            .set_files_to_run_provider(files_to_run_provider)
            .add_tools(&self.additional_test_action_tools)
            .set_execution_requirements(execution_info);
        let test_params = test_action_builder.build();
        TestProvider::new(test_params)
    }

    /// Adds a native provider keyed by its class.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:494 RuleConfiguredTargetBuilder.addProvider
    pub fn add_provider<P: TransitiveInfoProvider>(&mut self, provider: P) -> &mut Self {
        self.providers_builder.add_native(provider);
        self
    }

    /// Adds a shared native provider keyed by its class.
    pub fn add_provider_arc<P: TransitiveInfoProvider>(&mut self, provider: Arc<P>) -> &mut Self {
        self.providers_builder.add_native_arc(provider);
        self
    }

    /// Add multiple providers with given values.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:507 RuleConfiguredTargetBuilder.addProviders
    pub fn add_providers(
        &mut self,
        providers: &crate::provider_map::TransitiveInfoProviderMap,
    ) -> &mut Self {
        self.providers_builder.add_all(providers);
        self
    }

    /// Adds a "declared provider" defined in Starlark to the rule. Use this method for declared
    /// providers defined in Starlark. `OutputGroupInfo` is merged into the builder's output
    /// groups.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:519 RuleConfiguredTargetBuilder.addStarlarkDeclaredProvider
    pub fn add_starlark_declared_provider(&mut self, provider: OwnedFrozenValue) -> &mut Self {
        let key = info_provider_key(provider.value()).unwrap_or_else(|| {
            panic!(
                "IllegalArgumentException: not an Info: {}",
                provider.value()
            )
        });
        if key == OUTPUT_GROUP_INFO.key() {
            let output_group_info = downcast_info::<OutputGroupInfo>(&provider)
                .expect("OutputGroupInfo")
                .clone();
            for output_group in output_group_info.groups() {
                let files = output_group_info.get_output_group(&output_group);
                self.add_output_group(&output_group, &files);
            }
        } else {
            self.providers_builder.put_info(provider);
        }
        self
    }

    /// Adds "declared providers" defined in native code to the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:537 RuleConfiguredTargetBuilder.addNativeDeclaredProviders
    pub fn add_native_declared_providers(
        &mut self,
        providers: impl IntoIterator<Item = OwnedFrozenValue>,
    ) -> &mut Self {
        for provider in providers {
            self.add_native_declared_provider(provider);
        }
        self
    }

    /// Adds a "declared provider" defined in native code to the rule.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:548 RuleConfiguredTargetBuilder.addNativeDeclaredProvider
    pub fn add_native_declared_provider(&mut self, provider: OwnedFrozenValue) -> &mut Self {
        self.providers_builder.put_info(provider);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:555 RuleConfiguredTargetBuilder.containsProviderKey
    pub fn contains_provider_key(
        &self,
        provider_key: &blaze_packages::rule_class::ProviderKey,
    ) -> bool {
        self.providers_builder.contains_info(provider_key)
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:559 RuleConfiguredTargetBuilder.containsLegacyKey
    pub fn contains_legacy_key(&self, legacy_id: &str) -> bool {
        self.providers_builder
            .contains(&ProviderMapKey::Legacy(legacy_id.to_owned()))
    }

    /// Add a Starlark transitive info. The provider value must be safe.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:566 RuleConfiguredTargetBuilder.addStarlarkTransitiveInfo
    pub fn add_starlark_transitive_info(
        &mut self,
        name: &str,
        value: OwnedFrozenValue,
    ) -> &mut Self {
        self.providers_builder.put_legacy(name, value);
        self
    }

    /// Set the runfiles support for executable targets.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:573 RuleConfiguredTargetBuilder.setRunfilesSupport
    pub fn set_runfiles_support(
        &mut self,
        runfiles_support: Option<Arc<RunfilesSupport>>,
        executable: Option<Artifact>,
    ) -> &mut Self {
        self.runfiles_support = runfiles_support;
        self.executable = executable;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:581 RuleConfiguredTargetBuilder.addTestActionTools
    pub fn add_test_action_tools(&mut self, tools: &[Artifact]) -> &mut Self {
        self.additional_test_action_tools
            .extend(tools.iter().cloned());
        self
    }

    /// Set the files to build.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:588 RuleConfiguredTargetBuilder.setFilesToBuild
    pub fn set_files_to_build(&mut self, files_to_build: NestedSet<Artifact>) -> &mut Self {
        self.files_to_build = files_to_build;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:593 RuleConfiguredTargetBuilder.getOutputGroupBuilder
    fn get_output_group_builder(&mut self, name: &str) -> &mut NestedSetBuilder<Artifact> {
        self.output_group_builders
            .entry(name.to_owned())
            .or_insert_with(NestedSetBuilder::stable_order)
    }

    /// Adds a set of files to an output group.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:606 RuleConfiguredTargetBuilder.addOutputGroup
    pub fn add_output_group(&mut self, name: &str, artifacts: &NestedSet<Artifact>) -> &mut Self {
        self.get_output_group_builder(name)
            .add_transitive(artifacts);
        self
    }

    /// Adds a file to an output group.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:613 RuleConfiguredTargetBuilder.addOutputGroup
    pub fn add_output_group_artifact(&mut self, name: &str, artifact: Artifact) -> &mut Self {
        self.get_output_group_builder(name).add(artifact);
        self
    }

    /// Adds multiple output groups.
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetBuilder.java:620 RuleConfiguredTargetBuilder.addOutputGroups
    pub fn add_output_groups(
        &mut self,
        groups: &indexmap::IndexMap<String, NestedSet<Artifact>>,
    ) -> &mut Self {
        for (k, v) in groups {
            self.get_output_group_builder(k).add_transitive(v);
        }
        self
    }

    /// The rule context this builder builds for.
    pub fn get_rule_context(&self) -> &'a RuleContext {
        self.rule_context
    }
}

/// Helper so the base of a rule configured target can be built where the lead's enum expects it.
pub fn rule_configured_target_base(rule_context: &RuleContext) -> ConfiguredTargetBase {
    ConfiguredTargetBase {
        lookup_key: rule_context.get_owner().clone(),
        visibility: Some(Arc::new(
            crate::providers::visibility::VisibilityProviderData::new(
                rule_context.get_visibility().clone(),
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs a RuleContext over a loaded rule (RuleConfiguredTargetBuilderTest)"]
    fn build_adds_file_and_files_to_run_providers() {}
}

/// The `RuleContext`-based helpers of `analysis/Allowlist.java`.
pub mod allowlist {
    use blaze_cmdline::Label;
    use blaze_packages::types::Type;

    use crate::providers::package_specification_provider::PackageSpecificationProvider;
    use crate::rule::rule_context::RuleContext;

    /// Returns whether the rule in the given RuleContext *was defined* in an allowlist.
    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:62 Allowlist.isAvailableBasedOnRuleLocation
    pub fn is_available_based_on_rule_location(
        rule_context: &RuleContext,
        allowlist_name: &str,
    ) -> bool {
        let label = rule_context
            .get_rule()
            .get_rule_class_object()
            .get_rule_definition_environment_label()
            .cloned();
        match label {
            Some(l) => is_available_for(rule_context, allowlist_name, &l),
            // Java: isAvailableFor(..., null) would throw on getPackageIdentifier.
            None => panic!("NullPointerException: rule definition environment label"),
        }
    }

    /// Returns whether the rule in the given RuleContext *was instantiated* in an allowlist.
    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:76 Allowlist.isAvailable
    pub fn is_available(rule_context: &RuleContext, allowlist_name: &str) -> bool {
        is_available_for(rule_context, allowlist_name, rule_context.get_label())
    }

    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:87 Allowlist.isAvailableFor
    pub fn is_available_for(
        rule_context: &RuleContext,
        allowlist_name: &str,
        relevant_label: &Label,
    ) -> bool {
        let provider = fetch_package_specification_provider(rule_context, allowlist_name);
        PackageSpecificationProvider::is_available_for(
            provider.get_package_specifications(),
            relevant_label,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:99 Allowlist.fetchPackageSpecificationProvider
    pub fn fetch_package_specification_provider(
        rule_context: &RuleContext,
        allowlist_name: &str,
    ) -> PackageSpecificationProvider {
        fetch_package_specification_provider_or_null(rule_context, allowlist_name).unwrap_or_else(
            || panic!("NullPointerException: Allowlist argument for {allowlist_name} not found"),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:108 Allowlist.fetchPackageSpecificationProviderOrNull
    pub fn fetch_package_specification_provider_or_null(
        rule_context: &RuleContext,
        allowlist_name: &str,
    ) -> Option<PackageSpecificationProvider> {
        for attribute_name in get_attribute_name_from_allowlist_name(allowlist_name) {
            if !rule_context.is_attr_defined(&attribute_name, Type::Label) {
                continue;
            }
            let package_group = rule_context
                .get_prerequisite(&attribute_name)
                .expect("allowlist prerequisite");
            let provider = crate::configuredtargets::get_provider::<PackageSpecificationProvider>(
                &package_group,
            )
            .cloned();
            return Some(provider.unwrap_or_else(|| {
                panic!(
                    "NullPointerException: {}",
                    crate::configuredtargets::get_label(&package_group)
                )
            }));
        }
        None
    }

    // Java: com/google/devtools/build/lib/analysis/Allowlist.java:136 Allowlist.getAttributeNameFromAllowlistName
    fn get_attribute_name_from_allowlist_name(allowlist_name: &str) -> [String; 2] {
        [
            format!("$whitelist_{allowlist_name}"),
            format!("$allowlist_{allowlist_name}"),
        ]
    }
}
