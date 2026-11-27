//! Port of `skyframe/ConfiguredTargetFunction.java`: the `CONFIGURED_TARGET` SkyFunction.
//!
//! This function computes a target's complete analysis: its input is a target label and
//! configuration and its output is the target's actions. This implicitly constructs the build's
//! configured target and action graphs because a target's dependencies must be evaluated before
//! the target itself.
//!
//! Helpers, all called directly or indirectly from here: `DependencyResolver` (computes the
//! target's prerequisites, `config_setting`s and toolchains), `DependencyResolutionHelpers`
//! (what the dependencies are and what their configurations should be), `DependencyKind`
//! (structured dependency type), `AspectFunction` (aspects on dependencies) and
//! `ConfiguredTargetFactory` (runs the rule logic once all prerequisites are ready).

use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::ordered_set_multimap::{PrerequisiteMap, prerequisite_values};
use blaze_analysis::configured_rule_class_provider::CreateConfiguredTargetError;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_analysis::rule::analysis_environment::CachingAnalysisEnvironment;
use blaze_analysis::rule::configured_target_factory::{
    ConfiguredTargetFactory, ConfiguredTargetFactoryError,
};
use blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent;
use blaze_analysis::toolchains::{
    ResolvedToolchainContext, ToolchainCollection, UnloadedToolchainContextsInputs,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, StoredEventHandler};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::core_options::CoreOptionsIncludeConfigFragmentsEnum;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_packages::exceptions::{DetailedExitCode, FailureCode};
use blaze_packages::target::Target;
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::starlark_builtins::StarlarkBuiltinsKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, SkyFunction, SkyFunctionException, SkyKey,
    Transience,
};
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::dependency_resolver::{
    DependencyResolver, DependencyResolverException, DependencyResolverState,
    toolchain_exception_as_configured_value_creation_exception,
};
use crate::incompatible_target_checker::create_indirectly_incompatible_target;
use crate::producers::configured_target_and_data_producer::configured_target_label;
use crate::producers::errors::{
    Cause, ConfiguredTargetFunctionException, ConfiguredValueCreationException,
    TargetAndConfiguration, configuration_id_message, create_detailed_exit_code, event_location,
};
use crate::producers::target_and_configuration_producer::{
    TargetAndConfigurationError, TargetAndConfigurationResult, produce_target_and_configuration,
};

/// SkyFunction for `ConfiguredTargetValue`s (Java `ConfiguredTargetFunction`).
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:122 ConfiguredTargetFunction
pub struct ConfiguredTargetFunction {
    services: Arc<AnalysisServices>,
    /// Whether the set of packages transitively loaded for a value will be needed later.
    store_transitive_packages: bool,
}

/// Registers `CONFIGURED_TARGET` (and `STARLARK_BUILD_SETTINGS_DETAILS`, which only the
/// configured target producers request).
pub fn register(builder: EvaluatorBuilder, services: Arc<AnalysisServices>) -> EvaluatorBuilder {
    let function = ConfiguredTargetFunction {
        services,
        // Java: SkyframeExecutor.shouldStoreTransitivePackagesInLoadingAndAnalysis()
        // (`allowExternalRepositories`, true in Bazel).
        store_transitive_packages: true,
    };
    let builder = builder.function(
        blaze_analysis::configured_target_key::CONFIGURED_TARGET,
        function,
    );
    crate::producers::starlark_build_settings_details_function::register(builder)
}

fn throw(e: ConfiguredTargetFunctionException) -> ComputeError {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetEvaluationExceptions.java:29 ReportedException.<init>
    // A ReportedException carries the exception without its message.
    let e = match e {
        ConfiguredTargetFunctionException::Reported(cvce) => {
            ConfiguredTargetFunctionException::Reported(cvce.without_message())
        }
        other => other,
    };
    SkyFunctionException::new(e, Transience::Persistent).into()
}

impl SkyFunction for ConfiguredTargetFunction {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:268 ConfiguredTargetFunction.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> blaze_skyframe::ComputeResult {
        let configured_target_key = key
            .downcast::<ConfiguredTargetKey>()
            .expect("CONFIGURED_TARGET key");
        Ok(self
            .compute_typed(configured_target_key, env)?
            .map(|v| v as blaze_skyframe::SkyValue))
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:425 ConfiguredTargetFunction.extractTag
    fn extract_tag(&self, key: &SkyKey) -> Option<String> {
        key.downcast::<ConfiguredTargetKey>()
            .map(|k| k.get_label().to_string())
    }
}

impl ConfiguredTargetFunction {
    fn compute_typed(
        &self,
        configured_target_key: &ConfiguredTargetKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<ConfiguredTargetValue>>, ComputeError> {
        let services = &*self.services;
        // PORT: remote analysis caching (RemoteAnalysisCachingDependenciesProvider) and the
        // CPU-bound semaphore are not ported.
        // PORT: skipped, the environment wrapper only releases and reacquires that semaphore around dependency fetches (scheduling, nothing a build shows): StateInformingSkyFunctionEnvironment.StateInformingSkyFunctionEnvironment StateInformingSkyFunctionEnvironment.getValue StateInformingSkyFunctionEnvironment.getValueOrThrow StateInformingSkyFunctionEnvironment.valuesMissing StateInformingSkyFunctionEnvironment.getValuesAndExceptions StateInformingSkyFunctionEnvironment.getListener StateInformingSkyFunctionEnvironment.inErrorBubbling
        // PORT: skipped, same wrapper: StateInformingSkyFunctionEnvironment.getTemporaryDirectDeps StateInformingSkyFunctionEnvironment.injectVersionForNonHermeticFunction StateInformingSkyFunctionEnvironment.registerDependencies StateInformingSkyFunctionEnvironment.dependOnFuture StateInformingSkyFunctionEnvironment.getLookupHandleForPreviouslyRequestedDeps StateInformingSkyFunctionEnvironment.getState StateInformingSkyFunctionEnvironment.getMaxTransitiveSourceVersionSoFar Informee.inform
        let mut state = DependencyResolverState::new(self.store_transitive_packages);

        // computeTargetAndConfiguration
        let configured_target_key =
            match self.compute_target_and_configuration(env, &mut state, configured_target_key)? {
                // Null or an immediate `ConfiguredTargetValue`.
                ComputedTargetAndConfiguration::Restart => return Ok(None),
                ComputedTargetAndConfiguration::Value(v) => return Ok(Some(v)),
                // Otherwise, the result contains a `ConfiguredTargetKey`.
                ComputedTargetAndConfiguration::Key(k) => k,
            };

        let target_and_configuration = state
            .target_and_configuration
            .clone()
            .expect("targetAndConfiguration");
        let mut prereqs = DependencyResolver::new(target_and_configuration.clone());
        // Perform all analysis through dependency evaluation.
        match prereqs.evaluate(&mut state, &configured_target_key, services, env) {
            Ok(false) => return Ok(None),
            Ok(true) => {}
            Err(DependencyResolverException::Interrupted(e)) => return Err(e.into()),
            // Java: `catch (IncompatibleTargetChecker.IncompatibleTargetException e)`
            Err(DependencyResolverException::IncompatibleTarget(t)) => return Ok(Some(t)),
            Err(DependencyResolverException::Reported(e)) => {
                return Err(throw(ConfiguredTargetFunctionException::Reported(e)));
            }
            Err(DependencyResolverException::Unreported(e)) => {
                return Err(throw(ConfiguredTargetFunctionException::Unreported(e)));
            }
        }

        // If one of our dependencies is platform-incompatible with this build, so are we.
        if let Some(incompatible_target) = create_indirectly_incompatible_target(
            prereqs.get_target_and_configuration(),
            &configured_target_key,
            prereqs.get_dep_value_map(),
            prereqs.get_config_conditions(),
            prereqs.get_platform_info().map(|p| &**p),
            &state.transitive_state,
        ) {
            return Ok(Some(incompatible_target));
        }

        match self.compute_after_dependencies(
            env,
            &prereqs,
            &mut state,
            &target_and_configuration,
            &configured_target_key,
        ) {
            Ok(v) => Ok(v),
            Err(CaughtException::ConfiguredValueCreation(e)) => {
                if !e.get_message().is_empty() {
                    // Report the error to the user.
                    env.listener().handle(&Event::error_at(
                        event_location(e.get_location()),
                        e.get_message(),
                    ));
                }
                Err(throw(ConfiguredTargetFunctionException::Reported(e)))
            }
            Err(CaughtException::Toolchain(e)) => {
                let cvce = toolchain_exception_as_configured_value_creation_exception(
                    &e,
                    prereqs.get_target_and_configuration(),
                );
                env.listener().handle(&Event::error_at(
                    event_location(Some(
                        &prereqs
                            .get_target_and_configuration()
                            .get_target()
                            .get_location(),
                    )),
                    cvce.get_message(),
                ));
                Err(throw(ConfiguredTargetFunctionException::Reported(cvce)))
            }
            // The reporting will be done when going through errors in the build.
            Err(CaughtException::ActionConflict(e)) => {
                Err(throw(ConfiguredTargetFunctionException::ActionConflict(e)))
            }
            Err(CaughtException::Compute(e)) => Err(e),
        }
    }

    fn compute_after_dependencies(
        &self,
        env: &mut Environment<'_>,
        prereqs: &DependencyResolver,
        state: &mut DependencyResolverState,
        target_and_configuration: &TargetAndConfiguration,
        configured_target_key: &ConfiguredTargetKey,
    ) -> Result<Option<Arc<ConfiguredTargetValue>>, CaughtException> {
        // IF this build has a --run_under target, check it's an executable. We have to check this
        // at the parent: --run_under targets are configured in the exec configuration, but the
        // --run_under build option doesn't pass to the exec config.
        let config = prereqs.get_target_and_configuration().get_configuration();
        if let Some(config) = config {
            if let Some(RunUnder::LabelRunUnder { label, .. }) = &config
                .get_options()
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .run_under
            {
                let run_under_target = prerequisite_values(prereqs.get_dep_value_map())
                    .map(ConfiguredTargetAndData::get_configured_target)
                    .find(|d| configured_target_label(d) == *label);
                if let Some(run_under_target) = run_under_target {
                    let executable = run_under_target
                        .get_provider::<blaze_analysis::providers::file_provider::FilesToRunProvider>()
                        .expect("NullPointerException: FilesToRunProvider")
                        .get_executable();
                    if executable.is_none() {
                        return Err(CaughtException::ConfiguredValueCreation(
                            ConfiguredValueCreationException::for_target(
                                prereqs.get_target_and_configuration().get_target(),
                                format!("run_under target {} is not executable", label),
                            ),
                        ));
                    }
                }
            }
        }

        // Load the requested toolchains into the ToolchainContext, now that we have dependencies.
        let mut toolchain_contexts: Option<ToolchainCollection<ResolvedToolchainContext>> = None;
        if let Some(unloaded) = prereqs.get_unloaded_toolchain_contexts() {
            let target_description = prereqs
                .get_target_and_configuration()
                .get_target()
                .to_string();
            let mut contexts_builder = ToolchainCollection::builder();
            for (exec_group, unloaded_context) in unloaded.context_map() {
                // ImmutableSet.copyOf: identity-distinct values, in order.
                let toolchain_dependencies: Vec<ConfiguredTargetAndData> = prereqs
                    .get_dep_value_map()
                    .get(&DependencyKind::for_exec_group(exec_group))
                    .cloned()
                    .unwrap_or_default();
                let resolved = ResolvedToolchainContext::load(
                    unloaded_context,
                    &target_description,
                    &toolchain_dependencies,
                )
                .map_err(CaughtException::Toolchain)?;
                contexts_builder.add_context(exec_group, resolved);
            }
            toolchain_contexts = Some(contexts_builder.build());
        }

        // Run this target's rule logic to create its actions and return its value.
        let exec_group_collection_builder = state
            .exec_group_collection_builder
            .clone()
            .unwrap_or_else(UnloadedToolchainContextsInputs::empty);
        let ans = create_configured_target(
            &self.services,
            env,
            target_and_configuration,
            configured_target_key,
            prereqs.get_dep_value_map(),
            prereqs.get_materializer_targets(),
            prereqs.get_config_conditions(),
            toolchain_contexts.as_ref(),
            &exec_group_collection_builder,
            state.transitive_state.transitive_packages(),
        );
        if let Ok(Some(_)) = &ans {
            crate::analysis_progress_receiver::ANALYSIS_PROGRESS.done_configure_target();
        }
        ans
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:537 ConfiguredTargetFunction.computeTargetAndConfiguration
    fn compute_target_and_configuration(
        &self,
        env: &mut Environment<'_>,
        state: &mut DependencyResolverState,
        configured_target_key: &ConfiguredTargetKey,
    ) -> Result<ComputedTargetAndConfiguration, ComputeError> {
        let result = produce_target_and_configuration(
            env,
            &self.services,
            configured_target_key,
            &mut state.transitive_state,
            &state.stored_events,
        )?;
        let stored_events = &state.stored_events;
        let outcome = match result {
            // The result is null for a restart, so replayed events would not be used anyway.
            None => return Ok(ComputedTargetAndConfiguration::Restart),
            Some(TargetAndConfigurationResult::TargetAndConfiguration(value, full_key)) => {
                // `DependencyResolver.computeDependencies` takes ownership of stored events.
                state.target_and_configuration = Some(value);
                return Ok(ComputedTargetAndConfiguration::Key(full_key));
            }
            Some(TargetAndConfigurationResult::DelegatedValue(v)) => {
                Ok(ComputedTargetAndConfiguration::Value(v))
            }
            Some(TargetAndConfigurationResult::Error(error)) => Err(match error {
                TargetAndConfigurationError::ConfiguredValueCreation(e) => {
                    if !e.get_message().is_empty() {
                        // Reports the error to the user on storedEvents to preserve ordering.
                        // These will be immediately replayed below.
                        stored_events.post(Arc::new(
                            // Even without an error here, the configuration key might not be
                            // turned into a configuration value by the build because it does
                            // not include the rule transition. It's therefore marked
                            // unavailable.
                            AnalysisRootCauseEvent::with_unavailable_configuration(
                                // configurationIdMessage(configuredTargetKey.getConfigurationKey())
                                configured_target_key
                                    .get_configuration_key()
                                    .map(|k| k.get_options_checksum().to_owned()),
                                configured_target_key.get_label().clone(),
                                e.get_message().to_owned(),
                            ),
                        ));
                        stored_events.handle(&Event::error_at(
                            event_location(e.get_location()),
                            e.get_message(),
                        ));
                    }
                    throw(ConfiguredTargetFunctionException::Reported(e))
                }
                TargetAndConfigurationError::NoSuchThing(e) => {
                    throw(ConfiguredTargetFunctionException::NoSuchThing(e))
                }
                TargetAndConfigurationError::InconsistentNullConfig(e) => {
                    throw(ConfiguredTargetFunctionException::InconsistentNullConfig(e))
                }
            }),
        };
        // If there is exception or an immediate value, replays events because
        // `ConfiguredTargetFunction.compute` will promptly end.
        stored_events.replay_on(env.listener());
        outcome
    }
}

enum ComputedTargetAndConfiguration {
    Restart,
    Value(Arc<ConfiguredTargetValue>),
    Key(ConfiguredTargetKey),
}

/// The exceptions `compute` catches after dependency resolution.
enum CaughtException {
    ConfiguredValueCreation(ConfiguredValueCreationException),
    Toolchain(blaze_analysis::toolchains::ToolchainException),
    ActionConflict(blaze_analysis::rule::action_conflict_exception::ActionConflictException),
    Compute(ComputeError),
}

impl From<blaze_skyframe::Interrupted> for CaughtException {
    fn from(e: blaze_skyframe::Interrupted) -> Self {
        CaughtException::Compute(e.into())
    }
}

/// Runs the target's rule logic (Java `createConfiguredTarget`).
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:430 ConfiguredTargetFunction.createConfiguredTarget
fn create_configured_target(
    services: &Arc<AnalysisServices>,
    env: &mut Environment<'_>,
    ctg_value: &TargetAndConfiguration,
    configured_target_key: &ConfiguredTargetKey,
    dep_value_map: &PrerequisiteMap,
    materializer_targets: Option<&PrerequisiteMap>,
    config_conditions: &ConfigConditions,
    toolchain_contexts: Option<&ToolchainCollection<ResolvedToolchainContext>>,
    exec_group_collection_builder: &UnloadedToolchainContextsInputs,
    transitive_packages: Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    >,
) -> Result<Option<Arc<ConfiguredTargetValue>>, CaughtException> {
    let target = ctg_value.get_target();
    let configuration: Option<&Arc<BuildConfigurationValue>> = ctg_value.get_configuration();

    // Should be successfully evaluated and cached from the loading phase.
    let Some(starlark_builtins_value) = env.get_value(StarlarkBuiltinsKey::key(false))? else {
        return Ok(None);
    };

    // PORT: Java's `target.getPackage()`; Blaze targets hold only the package metadata, so the
    // (already computed) package is requested again.
    let Some(package_value) = env.get_value(PackageKey(
        target.get_label().get_package_identifier().clone(),
    ))?
    else {
        return Ok(None);
    };

    let events = Arc::new(StoredEventHandler::new());
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1336 SkyframeBuildView.createAnalysisEnvironment
    let core_options =
        configuration.map(|c| c.get_options().get::<CoreOptions>().expect("CoreOptions"));
    let extended_sanity_checks = core_options.is_some_and(|o| o.extended_sanity_checks);
    let allow_analysis_failures = core_options.is_some_and(|o| o.allow_analysis_failures);
    let starlark_defined_builtins: IndexMap<String, OwnedFrozenValue> = starlark_builtins_value
        .exported_to_java
        .iter()
        .map(|(name, entry)| (name.clone(), entry.value.clone()))
        .collect();
    let analysis_environment = Arc::new(CachingAnalysisEnvironment::new(
        services.artifact_factory.clone(),
        services.action_key_context.clone(),
        ActionLookupKeyRef::new(configured_target_key.clone()),
        extended_sanity_checks,
        allow_analysis_failures,
        events.clone(),
        starlark_builtins_value.starlark_semantics.clone(),
        Some(Arc::new(starlark_defined_builtins)),
        services.main_repo_mapping.read().clone(),
        // PORT: Java requests WorkspaceStatusValue.BUILD_INFO_KEY on demand.
        Some(services.get_workspace_status_artifacts().clone()),
        Some(package_value.get_package().clone()),
    ));

    // Java: RequiredFragmentsUtil.getRuleRequiredFragmentsIfEnabled (called by
    // ConfiguredTargetFactory.createRule): null unless --include_config_fragments_provider.
    let required_config_fragments = match (core_options, configuration, target.as_rule()) {
        (Some(o), Some(configuration), Some(rule))
            if o.include_required_config_fragments_provider
                != CoreOptionsIncludeConfigFragmentsEnum::OFF =>
        {
            // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1386 SkyframeBuildView.createConfiguredTarget
            // (StarlarkExecTransitionLoader.loadStarlarkExecTransition; null means a restart).
            // PORT: only RequiredFragmentsUtil reads it here (see the factory call below).
            let starlark_exec_transition =
                match crate::dependency_resolver::load_starlark_exec_transition_provider(
                    Some(&**configuration.get_options()),
                    services,
                    env,
                ) {
                    Ok(Some(t)) => t,
                    Ok(None) => return Ok(None),
                    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetFunction.java:468 ConfiguredTargetFunction.createConfiguredTarget
                    Err(e) => {
                        return Err(CaughtException::ConfiguredValueCreation(
                            ConfiguredValueCreationException::for_target(
                                ctg_value.get_target(),
                                &e.0,
                            ),
                        ));
                    }
                };
            let universal_fragments = blaze_build_config::fragment::FragmentClassSet::of(
                blaze_build_config::fragments::bazel_fragments().1,
            );
            let universal_fragments: Vec<_> = universal_fragments.iter().collect();
            blaze_analysis::required_fragments_util::get_rule_required_fragments_if_enabled(
                rule,
                configuration,
                &universal_fragments,
                config_conditions,
                dep_value_map
                    .values()
                    .flatten()
                    .map(|d| d.get_configured_target()),
                starlark_exec_transition,
                &services.rule_class_provider,
            )
        }
        _ => None,
    };

    let factory_transitive_packages = transitive_packages.clone();

    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1365 SkyframeBuildView.createConfiguredTarget
    // PORT: the Starlark exec transition is not loaded here (the bundled exec transition is
    // applied by function_transition_util), so StarlarkExecTransitionLoadingException cannot
    // occur.
    let factory = ConfiguredTargetFactory::new(services.rule_class_provider.clone());
    // Java: the CachingAnalysisEnvironment holds `env` (getSkyframeEnv()); it is lent to the
    // factory call (see blaze_analysis::skyframe_env).
    let created = blaze_analysis::skyframe_env::with_skyframe_env(env, || {
        factory.create_configured_target(
            &analysis_environment,
            &services.artifact_factory,
            target,
            package_value.get_package(),
            configuration,
            configured_target_key,
            dep_value_map,
            materializer_targets,
            config_conditions,
            toolchain_contexts.cloned().map(Arc::new),
            factory_transitive_packages,
            exec_group_collection_builder
                .as_exec_group_collection_builder()
                .clone(),
            required_config_fragments,
        )
    });
    let configured_target = match created {
        Ok(ct) => {
            // Java: CachingAnalysisEnvironment.getWorkspaceStatusValue's
            // env.getValue(WorkspaceStatusValue.BUILD_INFO_KEY) (see there).
            if analysis_environment.was_workspace_status_requested()
                && env
                    .get_value(blaze_analysis::analysis_services::WorkspaceStatusKey)?
                    .is_none()
            {
                return Ok(None);
            }
            ct
        }
        Err(ConfiguredTargetFactoryError::InvalidExecGroup(e)) => {
            return Err(CaughtException::ConfiguredValueCreation(
                ConfiguredValueCreationException::for_target(
                    ctg_value.get_target(),
                    e.get_message(),
                ),
            ));
        }
        Err(ConfiguredTargetFactoryError::AnalysisFailurePropagation { label, messages }) => {
            // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailurePropagationException.java:37 AnalysisFailurePropagationException.<init>
            let message = format!(
                "Error while collecting analysis-phase failure information for '{}': {}",
                label,
                messages.join("; ")
            );
            // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailurePropagationException.java:45 AnalysisFailurePropagationException.getDetailedExitCode
            let detailed_exit_code = DetailedExitCode {
                message: message.clone(),
                code: FailureCode::Other("Analysis.ANALYSIS_FAILURE_PROPAGATION_FAILED".to_owned()),
            };
            return Err(CaughtException::ConfiguredValueCreation(
                ConfiguredValueCreationException::for_target_full(
                    ctg_value.get_target(),
                    /* build_event_id= */ None,
                    message,
                    /* root_causes= */ None,
                    Some(detailed_exit_code),
                ),
            ));
        }
        Err(ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::ActionConflict(
            e,
        ))) => {
            return Err(CaughtException::ActionConflict(e));
        }
        Err(ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::Interrupted)) => {
            return Err(CaughtException::Compute(blaze_skyframe::Interrupted.into()));
        }
        Err(ConfiguredTargetFactoryError::Create(CreateConfiguredTargetError::RuleError(e))) => {
            // Java's ConfiguredTargetFactory.createRule catches RuleErrorException.
            panic!("RuleErrorException escaped ConfiguredTargetFactory: {}", e)
        }
    };

    events.replay_on(env.listener());
    if events.has_errors() {
        analysis_environment.disable(target);
        let root_causes: NestedSet<Cause> = NestedSetBuilder::wrap(
            Order::STABLE_ORDER,
            events
                .get_events()
                .iter()
                .filter(|event| event.get_kind() == EventKind::Error)
                .map(|event| Cause::AnalysisFailed {
                    label: target.get_label().clone(),
                    configuration: configuration_id_message(configuration.map(|c| c.checksum())),
                    detailed_exit_code: create_detailed_exit_code(&event.get_message()),
                }),
        );
        return Err(CaughtException::ConfiguredValueCreation(
            ConfiguredValueCreationException::for_target_full(
                ctg_value.get_target(),
                None,
                format!(
                    "Analysis of target '{}' (config: {}) failed",
                    target.get_label(),
                    match configuration {
                        Some(c) => c.get_options().short_id(),
                        None => "none".to_owned(),
                    }
                ),
                Some(root_causes),
                None,
            ),
        ));
    }
    blaze_util::check_state!(
        !analysis_environment.has_errors(),
        "Analysis environment hasError() but no errors reported"
    );
    if env.values_missing() {
        return Ok(None);
    }

    analysis_environment.disable(target);
    let configured_target: ConfiguredTarget =
        configured_target.unwrap_or_else(|| panic!("NullPointerException: {}", target.get_label()));

    let value = match &configured_target {
        ConfiguredTarget::Rule(rule_configured_target) => ConfiguredTargetValue::new(
            configured_target.clone(),
            rule_configured_target.get_actions().to_vec(),
            transitive_packages,
            Some(target.clone()),
        ),
        _ => {
            blaze_util::check_state!(
                analysis_environment.get_registered_actions().is_empty(),
                "Non-rule can't have actions: {:?} {:?} {:?}",
                configured_target_key,
                analysis_environment.get_registered_actions(),
                configured_target
            );
            // PORT: the remote analysis cache download path (RemoteConfiguredTargetValue) is
            // not ported.
            ConfiguredTargetValue::new(
                configured_target.clone(),
                Vec::new(),
                transitive_packages,
                Some(target.clone()),
            )
        }
    };
    Ok(Some(Arc::new(value)))
}

/// Java `ConfiguredTargetAndData` constructor helper for callers that need the `Target` of a
/// configured target key's value (cquery).
pub fn target_of_value(value: &ConfiguredTargetValue) -> Option<&Target> {
    value.get_target()
}
