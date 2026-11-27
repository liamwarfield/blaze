//! Port of `analysis/ConfiguredTargetFactory.java`.

use std::sync::Arc;

use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::{ActionLookupKeyRef, ArtifactOwner};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::{Event, EventHandler};
use blaze_packages::aspect::Aspect;
use blaze_packages::package::package::Package;
use blaze_packages::package::rule_visibility::RuleVisibility;
use blaze_packages::rule_class::MissingFragmentPolicy;
use blaze_packages::starlark_defs::builtin_providers::ANALYSIS_FAILURE_INFO;
use blaze_packages::target::{Rule, Target};

use crate::aspect_key_creator::AspectKey;
use crate::config::config_conditions::ConfigConditions;
use crate::configured_aspect::ConfiguredAspect;
use crate::configured_aspect_factory::ConfiguredAspectFactory;
use crate::configured_rule_class_provider::{
    ConfiguredRuleClassProvider, CreateConfiguredTargetError,
};
use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::configured_target_key::ConfiguredTargetKey;
use crate::configuredtargets::{
    EnvironmentGroupConfiguredTarget, InputFileConfiguredTarget, OutputFileConfiguredTarget,
    PackageGroupConfiguredTarget,
};
use crate::dependency_kind::DependencyKind;
use crate::providers::file_provider::RunfilesProvider;
use crate::providers::package_specification_provider::PackageSpecificationProvider;
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProvider;
use crate::providers::visibility::{
    PackageGroupContentsRef, Visibility, private_visibility, public_visibility,
};
use crate::rule::analysis_environment::CachingAnalysisEnvironment;
use crate::rule::config_util;
use crate::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use crate::rule::rule_context::{PrerequisiteMap, RuleContext, RuleContextBuilder};
use crate::rule::rule_error_consumer::{RuleErrorConsumer, RuleErrorException};
use crate::rule::target_context::TargetContext;
use crate::rule::to_event_location;
use crate::test::analysis_failure_info::{AnalysisFailure, AnalysisFailureInfo};
use crate::toolchains::aspect_base_target_resolved_toolchain_context::AspectBaseTargetResolvedToolchainContext;
use crate::toolchains::exec_group_collection::{
    ExecGroupCollectionBuilder, InvalidExecGroupException,
};
use crate::toolchains::resolved_toolchain_context::ResolvedToolchainContext;
use crate::toolchains::toolchain_collection::ToolchainCollection;

/// Errors of `createConfiguredTarget` beyond those of a rule factory (Java
/// `InvalidExecGroupException`, `AnalysisFailurePropagationException`).
#[derive(Debug, thiserror::Error)]
pub enum ConfiguredTargetFactoryError {
    #[error(transparent)]
    Create(#[from] CreateConfiguredTargetError),
    #[error(transparent)]
    InvalidExecGroup(#[from] InvalidExecGroupException),
    /// Java `AnalysisFailurePropagationException(label, messages)`.
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailurePropagationException.java:37 AnalysisFailurePropagationException.<init>
    #[error(
        "Error while collecting analysis-phase failure information for '{label}': {}",
        messages.join("; ")
    )]
    AnalysisFailurePropagation { label: Label, messages: Vec<String> },
}

impl From<RuleConfiguredTargetBuildError> for ConfiguredTargetFactoryError {
    fn from(e: RuleConfiguredTargetBuildError) -> Self {
        match e {
            RuleConfiguredTargetBuildError::ActionConflict(m) => {
                ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::ActionConflict(m))
            }
            RuleConfiguredTargetBuildError::Interrupted => {
                ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::Interrupted)
            }
        }
    }
}

/// This class creates `ConfiguredTarget` instances using a given `ConfiguredRuleClassProvider`.
// Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:92 ConfiguredTargetFactory
pub struct ConfiguredTargetFactory {
    rule_class_provider: Arc<ConfiguredRuleClassProvider>,
    conflict_finder: crate::rule::action_conflict_exception::ConflictFinderSupplier,
}

impl ConfiguredTargetFactory {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:100 ConfiguredTargetFactory.<init>
    pub fn new(rule_class_provider: Arc<ConfiguredRuleClassProvider>) -> ConfiguredTargetFactory {
        Self::with_conflict_finder(
            rule_class_provider,
            crate::rule::action_conflict_exception::no_conflict_finder(),
        )
    }

    /// `new ConfiguredTargetFactory(ruleClassProvider, conflictFinder)`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:100 ConfiguredTargetFactory.<init>
    pub fn with_conflict_finder(
        rule_class_provider: Arc<ConfiguredRuleClassProvider>,
        conflict_finder: crate::rule::action_conflict_exception::ConflictFinderSupplier,
    ) -> ConfiguredTargetFactory {
        ConfiguredTargetFactory {
            rule_class_provider,
            conflict_finder,
        }
    }

    /// Returns the visibility of the given target, as represented by
    /// `VisibilityProvider.getVisibility`.
    ///
    /// This is constructed by starting with the value obtained from either `Target.getVisibility`
    /// or `Target.getActualVisibility`, and recursively expanding package groups. Errors during
    /// package group resolution are reported to the event handler.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:115 ConfiguredTargetFactory.convertVisibility
    fn convert_visibility(
        prerequisite_map: &PrerequisiteMap,
        reporter: &CachingAnalysisEnvironment,
        target: &Target,
    ) -> Visibility {
        // Optimization: don't use actual visibility if not in a symbolic macro. See javadoc on
        // VisibilityProvider#getVisibility. Since the actual visibility only ever adds a
        // ...:__pkg__ item, not a package group, it doesn't need to be handled here.
        let rule_visibility = if target.is_created_in_symbolic_macro() {
            target.get_actual_visibility()
        } else {
            target.get_visibility()
        };
        let package_groups_visibility = match &rule_visibility {
            RuleVisibility::Public => return public_visibility(),
            RuleVisibility::Private => return private_visibility(),
            RuleVisibility::PackageGroups(p) => p.clone(),
        };

        let mut result: NestedSetBuilder<PackageGroupContentsRef> =
            NestedSetBuilder::stable_order();
        for group_label in package_groups_visibility.get_package_groups() {
            // PackageGroupsConfiguredTargets are always in the package-group configuration.
            let group = Self::find_visibility_prerequisite(prerequisite_map, group_label);
            // group == null can only happen if the package group list comes from a
            // default_visibility attribute, because in every other case, this missing link is
            // caught during transitive closure visitation or if the RuleConfiguredTargetGraph
            // threw out a visibility edge because if would have caused a cycle. The filtering
            // should be done in a single place, ConfiguredTargetGraph, but for now, this is the
            // minimally invasive way of providing a sane error message in case a cycle is
            // created by a visibility attribute.
            let provider = group.as_ref().and_then(|g| {
                crate::configuredtargets::get_provider::<PackageSpecificationProvider>(g).cloned()
            });
            match provider {
                Some(p) => {
                    result.add_transitive(p.get_package_specifications());
                }
                None => {
                    reporter.get_event_handler().handle(&Event::error_at(
                        Some(to_event_location(&target.get_location())),
                        format!("Label '{group_label}' does not refer to a package group"),
                    ));
                }
            }
        }
        result.add(PackageGroupContentsRef::new(
            package_groups_visibility.get_direct_packages().clone(),
        ));
        result.build()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:163 ConfiguredTargetFactory.findVisibilityPrerequisite
    fn find_visibility_prerequisite(
        prerequisite_map: &PrerequisiteMap,
        label: &Label,
    ) -> Option<ConfiguredTarget> {
        let prerequisites = prerequisite_map.get(&DependencyKind::VISIBILITY_DEPENDENCY)?;
        for prerequisite in prerequisites {
            if prerequisite.get_target_label() == *label
                && prerequisite.get_configuration().is_none()
            {
                return Some(prerequisite.get_configured_target().clone());
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:175 ConfiguredTargetFactory.findTransitiveVisibilityPrerequisite
    fn find_transitive_visibility_prerequisite(
        prerequisite_map: &PrerequisiteMap,
        label: &Label,
    ) -> Option<ConfiguredTarget> {
        let prerequisites =
            prerequisite_map.get(&DependencyKind::TRANSITIVE_VISIBILITY_DEPENDENCY)?;
        for prerequisite in prerequisites {
            // Just return the first one.
            if prerequisite.get_target_label() == *label
                && prerequisite.get_configuration().is_none()
            {
                return Some(prerequisite.get_configured_target().clone());
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:188 ConfiguredTargetFactory.getTransitiveVisibilityForCurrentPackage
    fn get_transitive_visibility_for_current_package(
        prerequisite_map: &PrerequisiteMap,
        reporter: &CachingAnalysisEnvironment,
        target: &Target,
    ) -> Option<Arc<PackageSpecificationProvider>> {
        let package_args = target.get_package_declarations().get_package_args();
        let tv_label = package_args.transitive_visibility()?.clone();
        let prerequisite =
            Self::find_transitive_visibility_prerequisite(prerequisite_map, &tv_label)?;
        let provider =
            crate::configuredtargets::get_provider::<PackageSpecificationProvider>(&prerequisite)
                .cloned();
        if provider.is_none() {
            reporter.get_event_handler().handle(&Event::error_at(
                Some(to_event_location(&target.get_location())),
                format!(
                    "Label '{tv_label}' in transitive_visibility does not refer to a package group"
                ),
            ));
        }
        provider.map(Arc::new)
    }

    /// Invokes the appropriate constructor to create a `ConfiguredTarget` instance.
    ///
    /// For use in `ConfiguredTargetFunction`. Returns `Ok(None)` if Skyframe deps are missing or
    /// upon certain errors.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:220 ConfiguredTargetFactory.createConfiguredTarget
    pub fn create_configured_target(
        &self,
        analysis_environment: &Arc<CachingAnalysisEnvironment>,
        artifact_factory: &ArtifactFactory,
        target: &Target,
        package: &Package,
        config: Option<&Arc<BuildConfigurationValue>>,
        configured_target_key: &ConfiguredTargetKey,
        prerequisite_map: &PrerequisiteMap,
        materializer_targets: Option<&PrerequisiteMap>,
        config_conditions: &ConfigConditions,
        toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
        transitive_packages: Option<
            NestedSet<
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
            >,
        >,
        exec_group_collection_builder: ExecGroupCollectionBuilder,
        required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredTarget>, ConfiguredTargetFactoryError> {
        if let Target::Rule(rule) = target {
            // PORT: CurrentRuleTracker (a debugging aid) is skipped.
            return self.create_rule(
                analysis_environment,
                rule,
                package,
                config.expect("rules have a configuration"),
                configured_target_key,
                prerequisite_map,
                materializer_targets,
                config_conditions,
                toolchain_contexts,
                transitive_packages,
                exec_group_collection_builder,
                required_config_fragments,
            );
        }

        // Enforce that targets whose names are outside their declaring macro's namespace cannot
        // be analyzed. (createRule() already enforces this above for rule targets, with optional
        // error interception through analysis_test.)
        if let Err(e) = check_macro_namespace_compliance(package, target) {
            analysis_environment
                .get_event_handler()
                .handle(&Event::error_at(
                    Some(to_event_location(&target.get_location())),
                    e,
                ));
            return Ok(None);
        }

        // Visibility, like all package groups, doesn't have a configuration
        let visibility = Self::convert_visibility(prerequisite_map, analysis_environment, target);
        // For InputFiles, we're not gating on --experimental_enforce_transitive_visibility
        // because they have no config, so we can't check whether
        // --experimental_enforce_transitive_visibility is set.
        let transitive_visibility = if config
            .is_some_and(|c| config_util::enforce_transitive_visibility(c))
            || target.is_input_file()
        {
            Self::get_transitive_visibility_for_current_package(
                prerequisite_map,
                analysis_environment,
                target,
            )
        } else {
            None
        };
        let empty: Vec<ConfiguredTargetAndData> = Vec::new();
        match target {
            Target::OutputFile(output_file) => {
                let target_context = TargetContext::new(
                    analysis_environment.clone(),
                    target.clone(),
                    config.cloned(),
                    prerequisite_map
                        .get(&DependencyKind::OUTPUT_FILE_RULE_DEPENDENCY)
                        .unwrap_or(&empty),
                    visibility,
                    // We are passing around this object because it looks nice, but it's never
                    // used. OutputFiles get the transitive visibility from their generating rule.
                    transitive_visibility,
                );
                let rule_label = output_file.get_generating_rule().get_label();
                let rule = target_context
                    .find_direct_prerequisite(
                        rule_label,
                        // Don't pass a specific configuration, as we don't care what
                        // configuration the generating rule is in. There can only be one actual
                        // dependency here, which is the target that generated the output file.
                        None,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "VerifyException: While analyzing {output_file}, missing generating rule {rule_label}"
                        )
                    });
                let ConfiguredTarget::Rule(rule) = rule else {
                    panic!("ClassCastException: generating rule is not a RuleConfiguredTarget");
                };
                // If analysis failures are allowed and the generating rule has failure info,
                // just propagate it. The output artifact won't exist, so we can't create an
                // OutputFileConfiguredTarget.
                if config_util::allow_analysis_failures(
                    config.expect("output files have a configuration"),
                ) && rule.providers().get(&ANALYSIS_FAILURE_INFO.key()).is_some()
                {
                    return Ok(Some(ConfiguredTarget::Rule(rule)));
                }
                let artifact = rule.find_artifact_by_output_label(output_file.get_label());
                Ok(Some(ConfiguredTarget::OutputFile(Arc::new(
                    OutputFileConfiguredTarget::new(&target_context, artifact, rule),
                ))))
            }
            Target::InputFile(input_file) => {
                let target_context = TargetContext::new(
                    analysis_environment.clone(),
                    target.clone(),
                    config.cloned(),
                    prerequisite_map
                        .get(&DependencyKind::OUTPUT_FILE_RULE_DEPENDENCY)
                        .unwrap_or(&empty),
                    visibility,
                    transitive_visibility,
                );
                let owner_key = ConfiguredTargetKey::builder()
                    .set_label(target.get_label().clone())
                    .set_configuration(config.map(|c| c.as_ref()))
                    .build();
                let artifact = artifact_factory.get_source_artifact_with_owner(
                    &input_file.get_exec_path(
                        analysis_environment
                            .get_starlark_semantics()
                            .experimental_sibling_repository_layout,
                    ),
                    &input_file.get_package_metadata().source_root,
                    ArtifactOwner::ActionLookupKey(ActionLookupKeyRef::new(owner_key)),
                );
                Ok(Some(ConfiguredTarget::InputFile(Arc::new(
                    InputFileConfiguredTarget::new(&target_context, artifact),
                ))))
            }
            Target::PackageGroup(package_group) => {
                let target_context = TargetContext::new(
                    analysis_environment.clone(),
                    target.clone(),
                    config.cloned(),
                    prerequisite_map
                        .get(&DependencyKind::VISIBILITY_DEPENDENCY)
                        .unwrap_or(&empty),
                    visibility,
                    /* transitive_visibility= */ None,
                );
                // No transitive visibility checking on package_groups, in part because transitive
                // visibility groups *are* package_groups, and we want to avoid circular
                // dependencies.
                Ok(Some(ConfiguredTarget::PackageGroup(Arc::new(
                    PackageGroupConfiguredTarget::new(
                        ActionLookupKeyRef::new(configured_target_key.clone()),
                        &target_context,
                        package_group,
                    ),
                ))))
            }
            Target::EnvironmentGroup(_) => Ok(Some(ConfiguredTarget::EnvironmentGroup(Arc::new(
                EnvironmentGroupConfiguredTarget::new(ActionLookupKeyRef::new(
                    configured_target_key.clone(),
                )),
            )))),
            _ => panic!("AssertionError: Unexpected target class: {target}"),
        }
    }

    /// Factory method: constructs a RuleConfiguredTarget of the appropriate class, based on the
    /// rule class. May return `None` if an error occurred.
    ///
    /// PORT: Java computes the required config fragments here with
    /// `RequiredFragmentsUtil.getRuleRequiredFragmentsIfEnabled(...)`; the caller (agent C's
    /// `ConfiguredTargetFunction`) passes the result.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:361 ConfiguredTargetFactory.createRule
    fn create_rule(
        &self,
        env: &Arc<CachingAnalysisEnvironment>,
        rule: &Arc<Rule>,
        package: &Package,
        configuration: &Arc<BuildConfigurationValue>,
        configured_target_key: &ConfiguredTargetKey,
        prerequisite_map: &PrerequisiteMap,
        materializer_targets: Option<&PrerequisiteMap>,
        config_conditions: &ConfigConditions,
        toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
        transitive_packages: Option<
            NestedSet<
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
            >,
        >,
        exec_group_collection_builder: ExecGroupCollectionBuilder,
        required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredTarget>, ConfiguredTargetFactoryError> {
        let rule_class = rule.get_rule_class_object().clone();
        let configuration_fragment_policy = rule_class.get_configuration_fragment_policy().clone();
        let target = Target::Rule(rule.clone());

        // Visibility computation and checking is done for every rule.
        let mut builder = RuleContextBuilder::new(
            env.clone(),
            target.clone(),
            /* aspects= */ Vec::new(),
            configuration.clone(),
        );
        builder
            .set_rule_class_provider(self.rule_class_provider.clone())
            .set_conflict_finder(self.conflict_finder.clone())
            .set_configuration_fragment_policy(configuration_fragment_policy.clone())
            .set_action_owner_symbol(ActionLookupKeyRef::new(configured_target_key.clone()))
            .set_mutability("configured target")
            .set_visibility(Self::convert_visibility(prerequisite_map, env, &target))
            .set_prerequisites(Self::remove_toolchain_deps(prerequisite_map))
            .set_materializer_targets(materializer_targets.cloned())
            .set_config_conditions(config_conditions.clone())
            .set_toolchain_contexts(toolchain_contexts)
            .set_transitive_visibility_imposed_by_this_package(
                Self::get_transitive_visibility_for_current_package(prerequisite_map, env, &target),
            )
            .set_exec_group_collection_builder(exec_group_collection_builder)
            .set_required_config_fragments(required_config_fragments)
            .set_transitive_packages_for_runfile_repo_mapping_manifest(transitive_packages)
            .set_allow_materializer_rule_real_deps(rule_class.materializer_rule_allows_real_deps());
        let rule_context = builder.build()?;

        let analysis_failures = Self::dep_analysis_failures(&rule_context, &[]);
        if !analysis_failures.is_empty() {
            return Self::errored_configured_target_with_failures(&rule_context, analysis_failures);
        }
        if rule_context.has_errors() {
            return Self::errored_configured_target(&rule_context, None);
        }

        if let Err(e) = check_macro_namespace_compliance(package, &target) {
            rule_context.rule_error(&e);
            return Self::errored_configured_target(&rule_context, None);
        }

        let mut missing_fragment_class: Option<String> = None;
        for fragment_class in configuration_fragment_policy.get_required_configuration_fragments() {
            if !config_util::has_fragment_named(configuration, fragment_class) {
                let missing_fragment_policy =
                    configuration_fragment_policy.get_missing_fragment_policy(fragment_class);
                if missing_fragment_policy != MissingFragmentPolicy::Ignore {
                    if missing_fragment_policy == MissingFragmentPolicy::FailAnalysis {
                        rule_context.rule_error(&Self::missing_fragment_error(
                            &rule_context,
                            &configuration_fragment_policy,
                            configuration.checksum(),
                        ));
                        return Ok(None);
                    }
                    // Otherwise missingFragmentPolicy == MissingFragmentPolicy.CREATE_FAIL_ACTIONS:
                    missing_fragment_class = Some(fragment_class.clone());
                }
            }
        }
        if let Some(missing) = missing_fragment_class {
            return Self::create_fail_configured_target_for_missing_fragment_class(
                &rule_context,
                &missing,
            );
        }

        if rule_class.is_starlark() {
            // PORT: Java's initStarlarkRuleContext / StarlarkRuleConfiguredTargetUtil.evalRule /
            // createTarget / close sequence is agent S's `build_rule`, which returns `Ok(None)`
            // where Java's rawProviders or target is null (errors are on the rule context).
            let mut rule_context = rule_context;
            let result = crate::starlark::starlark_rule_configured_target_util::build_rule(
                &mut rule_context,
                &rule_class,
            );
            rule_context.close();
            return match result {
                Ok(outcome) => match outcome.target {
                    Some(target) => Ok(Some(target)),
                    None => Self::errored_configured_target(
                        &rule_context,
                        outcome.required_config_fragments_provider,
                    ),
                },
                Err(CreateConfiguredTargetError::RuleError(_)) => {
                    Self::errored_configured_target(&rule_context, None)
                }
                Err(e) => Err(e.into()),
            };
        }

        let factory_name = rule_class
            .get_configured_target_factory()
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: No configured target factory for {}",
                    rule_class.get_name()
                )
            });
        let factory = self
            .rule_class_provider
            .get_configured_target_factory(factory_name)
            .unwrap_or_else(|| {
                blaze_util::unported!(
                    "RuleConfiguredTargetFactory.create",
                    "no native factory registered for {factory_name}"
                )
            })
            .clone();
        // PORT: the trait takes `&mut RuleContext`; every RuleContext method takes `&self`.
        let mut rule_context = rule_context;
        let created = factory.create(&mut rule_context);
        // close() is required if the native rule created StarlarkRuleContext to perform any
        // Starlark evaluation, i.e. using the @_builtins mechanism.
        rule_context.close();
        match created {
            Ok(target) => {
                if let Some(t) = &target {
                    // If a configured target is created, check that all advertised providers are
                    // returned.
                    Self::validate_rule_advertised_providers(&rule_context, t, &rule_class);
                }
                // TODO(https://github.com/bazelbuild/bazel/issues/17915): genquery and similar
                // native rules may return null without setting a ruleContext error to signal a
                // skyframe restart.
                match target {
                    Some(t) => Ok(Some(t)),
                    None => Self::errored_configured_target(&rule_context, None),
                }
            }
            Err(CreateConfiguredTargetError::RuleError(_)) => {
                Self::errored_configured_target(&rule_context, None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Checks that all the rule advertised providers are returned in the configured target and
    /// adds an error to `rule_context` if not.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:495 ConfiguredTargetFactory.validateRuleAdvertisedProviders
    fn validate_rule_advertised_providers(
        rule_context: &RuleContext,
        configured_target: &ConfiguredTarget,
        rule_class: &blaze_packages::rule_class::RuleClass,
    ) {
        let advertised_providers = rule_class.get_advertised_providers();
        for provider_id in advertised_providers.get_starlark_providers() {
            if crate::configuredtargets::get_info(configured_target, provider_id.get_key())
                .is_none()
            {
                rule_context.rule_error(&format!(
                    "rule advertised the '{provider_id}' provider, but this provider was not among those returned"
                ));
            }
        }
        for klass in advertised_providers.get_builtin_providers() {
            if !crate::configuredtargets::has_builtin_provider_named(configured_target, klass) {
                rule_context.rule_error(&format!(
                    "rule advertised the '{klass}' provider, but this provider was not among those returned"
                ));
            }
        }
    }

    /// If `--allow_analysis_failures` is true, returns the propagated analysis failures of the
    /// target's dependencies and `extra_deps`. Otherwise returns the empty list.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:547 ConfiguredTargetFactory.depAnalysisFailures
    fn dep_analysis_failures(
        rule_context: &RuleContext,
        extra_deps: &[ConfiguredTarget],
    ) -> Vec<starlark::values::OwnedFrozenValue> {
        if config_util::allow_analysis_failures(rule_context.get_configuration()) {
            let mut analysis_failures = Vec::new();
            let info_collections = rule_context
                .get_all_prerequisites()
                .into_iter()
                .chain(extra_deps.iter().cloned());
            for info_collection in info_collections {
                if let Some(failure_info) = crate::configuredtargets::get_info(
                    &info_collection,
                    &ANALYSIS_FAILURE_INFO.key(),
                ) {
                    analysis_failures.push(failure_info);
                }
            }
            return analysis_failures;
        }
        // Analysis failures are only created and propagated if --allow_analysis_failures is
        // enabled, otherwise these result in actual rule errors which are not caught.
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:567 ConfiguredTargetFactory.erroredConfiguredTargetWithFailures
    fn errored_configured_target_with_failures(
        rule_context: &RuleContext,
        analysis_failures: Vec<starlark::values::OwnedFrozenValue>,
    ) -> Result<Option<ConfiguredTarget>, ConfiguredTargetFactoryError> {
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder.add_native_declared_provider(crate::info::freeze_native_info(
            AnalysisFailureInfo::for_analysis_failure_sets(causes_of(&analysis_failures)),
        ));
        builder.add_provider(crate::providers::file_provider::RunfilesProvider::simple(
            blaze_actions::analysis::runfiles::Runfiles::empty(),
        ));
        match builder.build()? {
            Some(configured_target) => Ok(Some(configured_target)),
            // A failure here is a failure in analysis failure testing machinery, not a "normal"
            // analysis failure that some outer analysis failure test may want to capture.
            // Instead, this failure means that the outer test would be unusable. So we throw an
            // exception rather than returning null and allowing it to propagate up in the usual
            // way.
            None => Err(ConfiguredTargetFactoryError::AnalysisFailurePropagation {
                label: rule_context.get_label().clone(),
                messages: rule_context.get_suppressed_error_messages(),
            }),
        }
    }

    /// Returns a `ConfiguredTarget` which indicates that an analysis error occurred in processing
    /// the target. In most cases, this returns `None`, which signals to callers that the target
    /// failed to build and thus the build should fail. However, if analysis failures are allowed
    /// in this build, this returns a stub `ConfiguredTarget` which contains information about the
    /// failure.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:484 ConfiguredTargetFactory.erroredConfiguredTarget
    fn errored_configured_target(
        rule_context: &RuleContext,
        required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredTarget>, ConfiguredTargetFactoryError> {
        if config_util::allow_analysis_failures(rule_context.get_configuration()) {
            let analysis_failures: Vec<AnalysisFailure> = rule_context
                .get_suppressed_error_messages()
                .into_iter()
                .map(|error_message| {
                    AnalysisFailure::create(rule_context.get_label().clone(), error_message)
                })
                .collect();
            let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
            builder.add_native_declared_provider(crate::info::freeze_native_info(
                AnalysisFailureInfo::for_analysis_failures(analysis_failures),
            ));
            builder.add_provider(crate::providers::file_provider::RunfilesProvider::simple(
                blaze_actions::analysis::runfiles::Runfiles::empty(),
            ));
            if let Some(provider) = required_config_fragments_provider {
                builder.add_provider(provider);
            }
            return match builder.build()? {
                Some(configured_target) => Ok(Some(configured_target)),
                // See comment in erroredConfiguredTargetWithFailures.
                None => Err(ConfiguredTargetFactoryError::AnalysisFailurePropagation {
                    label: rule_context.get_label().clone(),
                    messages: rule_context.get_suppressed_error_messages(),
                }),
            };
        } // Returning a null ConfiguredTarget is an indication a rule error occurred. Exceptions
        // are not propagated, as this would show a nasty stack trace to users, and only provide
        // info on one specific failure with poor messaging. By returning null, the caller can
        // inspect ruleContext for multiple errors and output thorough messaging on each.
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:437 ConfiguredTargetFactory.missingFragmentError
    fn missing_fragment_error(
        rule_context: &RuleContext,
        configuration_fragment_policy: &blaze_packages::rule_class::ConfigurationFragmentPolicy,
        configuration_id: &str,
    ) -> String {
        let rule_class = rule_context.get_rule().get_rule_class_object();
        let mut missing_fragments: Vec<String> = Vec::new();
        for fragment in configuration_fragment_policy.get_required_configuration_fragments() {
            if !config_util::has_fragment_named(rule_context.get_configuration(), fragment)
                && !missing_fragments.contains(fragment)
            {
                missing_fragments.push(fragment.clone());
            }
        }
        blaze_util::check_state!(!missing_fragments.is_empty());
        format!(
            "all rules of type {} require the presence of all of [{}], but these were all disabled in configuration {}",
            rule_class.get_name(),
            missing_fragments.join(","),
            configuration_id
        )
    }

    /// Removes the toolchain dependencies from a prerequisite map.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:649 ConfiguredTargetFactory.removeToolchainDeps
    pub fn remove_toolchain_deps(map: &PrerequisiteMap) -> PrerequisiteMap {
        let mut result = PrerequisiteMap::new();
        for (kind, values) in map {
            if kind.is_toolchain() {
                continue;
            }
            result
                .entry(kind.clone())
                .or_default()
                .extend(values.iter().cloned());
        }
        result
    }

    /// Constructs a `ConfiguredAspect`. Returns `Ok(None)` if an error occurs; in that case,
    /// `aspect_factory` should call one of the error reporting methods of `RuleContext`.
    ///
    /// PORT: Java computes the required config fragments here with
    /// `RequiredFragmentsUtil.getAspectRequiredFragmentsIfEnabled(aspect, aspectFactory,
    /// associatedTarget.getAssociatedRule(), aspectConfiguration, universalFragments,
    /// configConditions, deps + configuredTarget, starlarkExecTransition)`; as for
    /// `create_configured_target`, the caller (`AspectFunction`) passes the result in place of
    /// Java's last parameter (`starlarkExecTransition`, only used for that computation).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:668 ConfiguredTargetFactory.createAspect
    pub fn create_aspect(
        &self,
        env: &Arc<CachingAnalysisEnvironment>,
        associated_target: &Target,
        configured_target: &ConfiguredTarget,
        aspect_path: Vec<Aspect>,
        aspect_factory: &dyn ConfiguredAspectFactory,
        aspect: &Aspect,
        prerequisite_map: &PrerequisiteMap,
        config_conditions: &ConfigConditions,
        toolchain_contexts: Option<Arc<ToolchainCollection<ResolvedToolchainContext>>>,
        base_target_toolchain_contexts: Option<
            Arc<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>,
        >,
        exec_group_collection_builder: Option<ExecGroupCollectionBuilder>,
        aspect_configuration: &Arc<BuildConfigurationValue>,
        transitive_packages: Option<
            NestedSet<
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
            >,
        >,
        aspect_key: &AspectKey,
        required_config_fragments: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredAspect>, ConfiguredTargetFactoryError> {
        let mut builder = RuleContextBuilder::new(
            env.clone(),
            associated_target.clone(),
            aspect_path,
            aspect_configuration.clone(),
        );
        builder
            .set_rule_class_provider(self.rule_class_provider.clone())
            .set_conflict_finder(self.conflict_finder.clone())
            .set_configuration_fragment_policy(
                aspect
                    .get_definition()
                    .get_configuration_fragment_policy()
                    .clone(),
            )
            .set_action_owner_symbol(ActionLookupKeyRef::new(aspect_key.clone()))
            .set_mutability("aspect")
            .set_visibility(Self::convert_visibility(
                prerequisite_map,
                env,
                associated_target,
            ))
            .set_prerequisites(Self::remove_toolchain_deps(prerequisite_map))
            .set_config_conditions(config_conditions.clone())
            .set_toolchain_contexts(toolchain_contexts)
            .set_base_target_toolchain_contexts(base_target_toolchain_contexts);
        // Java: setExecGroupCollectionBuilder(execGroupCollectionBuilder) (null is kept, and
        // build() then fails on it like Java's NullPointerException).
        if let Some(exec_group_collection_builder) = exec_group_collection_builder {
            builder.set_exec_group_collection_builder(exec_group_collection_builder);
        }
        // PORT: setConflictFinder is skipped, as in create_rule (Blaze checks conflicts after
        // analysis).
        builder
            .set_exec_properties(indexmap::IndexMap::new())
            .set_required_config_fragments(required_config_fragments)
            .set_transitive_packages_for_runfile_repo_mapping_manifest(transitive_packages);
        let rule_context = builder.build()?;

        // If allowing analysis failures, targets should be created as normal as possible, and
        // errors will be propagated via a hook elsewhere as AnalysisFailureInfo.
        let allow_analysis_failures =
            config_util::allow_analysis_failures(rule_context.get_configuration());

        let analysis_failures =
            Self::dep_analysis_failures(&rule_context, std::slice::from_ref(configured_target));
        if !analysis_failures.is_empty() {
            return Self::errored_configured_aspect_with_failures(&rule_context, analysis_failures);
        }
        if rule_context.has_errors() && !allow_analysis_failures {
            return Self::errored_configured_aspect(&rule_context, None);
        }

        // PORT: the factory takes `&mut RuleContext` (the Starlark factory lends it to the
        // implementation's thread); every RuleContext method takes `&self`.
        let mut rule_context = rule_context;
        // Java declares RuleErrorException on createAspect: it propagates to the caller.
        let configured_aspect = aspect_factory.create(
            associated_target.get_label(),
            configured_target,
            &mut rule_context,
            aspect.get_parameters(),
            // PORT: Java `ruleClassProvider.getToolsRepository()`; Bazel's provider sets
            // RepositoryName.BAZEL_TOOLS (BazelRuleClassProvider.java:214) and
            // ConfiguredRuleClassProvider (lead) has no accessor yet.
            &blaze_cmdline::repository_name::BAZEL_TOOLS,
        )?;

        // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:741 ConfiguredTargetFactory.createAspect
        if let (Some(conflict_finder), Some(configured_aspect)) =
            (rule_context.get_conflict_finder(), &configured_aspect)
        {
            for action in configured_aspect.get_actions() {
                conflict_finder
                    .conflict_check_per_action(action)
                    .map_err(|e| match e {
                        crate::rule::action_conflict_exception::ConflictCheckPerActionError::ActionConflict(e) => {
                            ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::ActionConflict(e))
                        }
                        crate::rule::action_conflict_exception::ConflictCheckPerActionError::Interrupted => {
                            ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::Interrupted)
                        }
                    })?;
            }
        }
        let Some(configured_aspect) = configured_aspect else {
            return Self::errored_configured_aspect(&rule_context, None);
        };
        if configured_aspect
            .get(&ANALYSIS_FAILURE_INFO.key())
            .is_some()
        {
            // this was created by #erroredConfiguredAspect, return early to skip validating
            // advertised providers
            return Ok(Some(configured_aspect));
        }

        Self::validate_advertised_providers(
            &configured_aspect,
            aspect_key,
            aspect.get_definition().get_advertised_providers(),
            associated_target,
            env,
        );
        Ok(Some(configured_aspect))
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:763 ConfiguredTargetFactory.erroredConfiguredAspectWithFailures
    fn errored_configured_aspect_with_failures(
        rule_context: &RuleContext,
        analysis_failures: Vec<starlark::values::OwnedFrozenValue>,
    ) -> Result<Option<ConfiguredAspect>, ConfiguredTargetFactoryError> {
        let mut builder = ConfiguredAspect::builder(rule_context);
        builder.add_native_declared_provider(crate::info::freeze_native_info(
            AnalysisFailureInfo::for_analysis_failure_sets(causes_of(&analysis_failures)),
        ));
        // Unlike erroredConfiguredTargetAspectWithFailures, we do not add a RunfilesProvider;
        // that would result in a RunfilesProvider being provided twice in the merged configured
        // target.
        Ok(builder.build()?)
    }

    /// Returns a `ConfiguredAspect` which indicates that an analysis error occurred in
    /// processing the aspect. In most cases, this returns `None`, which signals to callers that
    /// the target failed to build and thus the build should fail. However, if analysis failures
    /// are allowed in this build, this returns a stub `ConfiguredAspect` which contains
    /// information about the failure.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:785 ConfiguredTargetFactory.erroredConfiguredAspect
    pub fn errored_configured_aspect(
        rule_context: &RuleContext,
        required_config_fragments_provider: Option<RequiredConfigFragmentsProvider>,
    ) -> Result<Option<ConfiguredAspect>, ConfiguredTargetFactoryError> {
        if config_util::allow_analysis_failures(rule_context.get_configuration()) {
            let analysis_failures: Vec<AnalysisFailure> = rule_context
                .get_suppressed_error_messages()
                .into_iter()
                .map(|error_message| {
                    AnalysisFailure::create(rule_context.get_label().clone(), error_message)
                })
                .collect();
            let mut builder = ConfiguredAspect::builder(rule_context);
            builder.add_native_declared_provider(crate::info::freeze_native_info(
                AnalysisFailureInfo::for_analysis_failures(analysis_failures),
            ));
            if let Some(provider) = required_config_fragments_provider {
                builder.add_provider(provider);
            }
            // Unlike erroredConfiguredTarget, we do not add a RunfilesProvider; that would
            // result in a RunfilesProvider being provided twice in the merged configured target.
            return Ok(builder.build()?);
        } // Returning a null ConfiguredAspect is an indication a rule error occurred. Exceptions
        // are not propagated, as this would show a nasty stack trace to users, and only provide
        // info on one specific failure with poor messaging. By returning null, the caller can
        // inspect ruleContext for multiple errors and output thorough messaging on each.
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:819 ConfiguredTargetFactory.validateAdvertisedProviders
    fn validate_advertised_providers(
        configured_aspect: &ConfiguredAspect,
        aspect_key: &AspectKey,
        advertised_providers: &blaze_packages::rule_class::AdvertisedProviderSet,
        target: &Target,
        event_handler: &CachingAnalysisEnvironment,
    ) {
        if advertised_providers.can_have_any_provider() {
            return;
        }
        if let Some(a_class) = advertised_providers.get_builtin_providers().iter().next() {
            // Java: for each class, configuredAspect.getProvider(aClass) == null. Native
            // providers are keyed by Rust type in TransitiveInfoProviderMap; a lookup by Java
            // class name is not ported (Starlark aspects advertise no builtin providers).
            blaze_util::unported!(
                "ConfiguredTargetFactory.validateAdvertisedProviders",
                "builtin advertised provider {}",
                a_class
            );
        }

        for provider_id in advertised_providers.get_starlark_providers() {
            if configured_aspect.get(provider_id.get_key()).is_none() {
                event_handler.get_event_handler().handle(&Event::error_at(
                    Some(to_event_location(&target.get_location())),
                    format!(
                        "Aspect '{}', applied to '{}', does not provide advertised provider '{}'",
                        aspect_key.get_aspect_class().get_name(),
                        target.get_label(),
                        provider_id
                    ),
                ));
            }
        }
    }

    /// A pseudo-implementation for configured targets that creates fail actions for all
    /// declared outputs, both implicit and explicit, due to a missing fragment class.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredTargetFactory.java:857 ConfiguredTargetFactory.createFailConfiguredTargetForMissingFragmentClass
    fn create_fail_configured_target_for_missing_fragment_class(
        rule_context: &RuleContext,
        missing_fragment_class: &str,
    ) -> Result<Option<ConfiguredTarget>, ConfiguredTargetFactoryError> {
        if !rule_context.get_output_artifacts().is_empty() {
            // missingFragmentClass.getName(): the policy holds a fragment by the name its rule
            // class declared, which is the binary class name only when it has a package.
            // PORT: no rule class at the pin sets MissingFragmentPolicy.CREATE_FAIL_ACTIONS
            // (only IGNORE: TestSuiteRule, StarlarkRuleClassFunctions), so this is unreachable.
            if !missing_fragment_class.contains('.') {
                blaze_util::unported!(
                    "Class.getName",
                    "binary name of the fragment class {missing_fragment_class}"
                );
            }
            rule_context.register_action(Arc::new(
                blaze_actions::actions::fail_action::FailAction::new(
                    rule_context.get_action_owner(),
                    rule_context.get_output_artifacts(),
                    &format!("Missing fragment class: {missing_fragment_class}"),
                    blaze_protos::failure_details::fail_action::Code::FragmentClassMissing,
                ),
            ));
        }
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder.add_provider(RunfilesProvider::simple(Runfiles::empty()));
        match builder.build() {
            Ok(t) => Ok(t),
            Err(RuleConfiguredTargetBuildError::ActionConflict(_)) => panic!(
                "IllegalStateException: Can't have an action conflict with one action: {}",
                rule_context.get_label()
            ),
            Err(RuleConfiguredTargetBuildError::Interrupted) => Err(
                ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::Interrupted),
            ),
        }
    }
}

/// Java `target.getPackageoid().checkMacroNamespaceCompliance(target)`: `Err` is the
/// `MacroNamespaceViolationException` message. `package` is the target's package (Java
/// `getPackageoid()`; Blaze targets do not point back to their package).
// Java: com/google/devtools/build/lib/packages/Package.java:319 Package.checkMacroNamespaceCompliance
fn check_macro_namespace_compliance(package: &Package, target: &Target) -> Result<(), String> {
    package
        .check_macro_namespace_compliance(target)
        .map_err(|e| e.message)
}

/// Re-export for callers matching on rule errors.
pub type FactoryRuleError = RuleErrorException;

/// `infoCollection.get(AnalysisFailureInfo.STARLARK_CONSTRUCTOR).getCausesNestedSet()` of each
/// collected provider (`depAnalysisFailures` collects the providers).
fn causes_of(
    analysis_failures: &[starlark::values::OwnedFrozenValue],
) -> Vec<blaze_collect::NestedSet<AnalysisFailure>> {
    analysis_failures
        .iter()
        .map(|info| {
            crate::info::downcast_info::<AnalysisFailureInfo>(info)
                .expect("ClassCastException: AnalysisFailureInfo")
                .get_causes_nested_set()
                .clone()
        })
        .collect()
}
