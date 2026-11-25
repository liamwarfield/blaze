//! Port of `skyframe/DependencyResolver.java`: performs the analysis phase of a configured
//! target through computation of its prerequisites.
//!
//! For `ConfiguredTargetFunction` this includes getting the target's `Target` and configuration,
//! its `select()` keys (`ConfigConditions`), the toolchains it needs and the configured targets
//! of its prerequisites.
//!
//! PORT: Java memoizes each step in `State` across Skyframe restarts. Blaze's evaluator blocks
//! instead of restarting, so `State` lives for one `compute` call; it is kept because its parts
//! (stored events, transitive state) are shared by the steps exactly as in Java.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::aspect_key_creator::LoadAspectsKey;
use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::ordered_set_multimap::PrerequisiteMap;
use blaze_analysis::config::transitions::StarlarkAttributeTransitionProvider;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent;
use blaze_analysis::toolchains::{
    ToolchainCollection, ToolchainException, UnloadedToolchainContext,
    UnloadedToolchainContextsInputs,
};
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_build_config::starlark_transition::StarlarkExecTransitionLoadingException;
use blaze_build_config::transitions::EMPTY_OPTIONS;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, ExtendedEventHandler, StoredEventHandler};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_packages::aspect::{AspectClass, StarlarkAspectClass};
use blaze_packages::exceptions::DetailedExitCode;
use blaze_packages::target::Target;
use blaze_skyframe::{Environment, Interrupted};
use blaze_starlark::Location;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::producers::dependency_context_producer::{
    DependencyContext, produce_dependency_context_with_compatibility_check,
};
use crate::producers::dependency_map_producer::{PrerequisiteParameters, produce_dependency_map};
use crate::producers::dependency_resolution_helpers::{Aspect, compute_dependency_labels};
use crate::producers::errors::{
    AspectCreationException, Cause, ConfiguredValueCreationException, DependencyContextError,
    DependencyError, DependencyEvaluationException, TargetAndConfiguration,
    TransitiveDependencyState, choose_more_important_with_first_if_tie, configuration_id,
    configuration_id_message, create_detailed_exit_code, event_location,
};
use crate::producers::target_and_configuration_producer::compute_toolchain_configuration_key;
use blaze_analysis::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier;

/// Memoizes computation steps of [`DependencyResolver::evaluate`].
// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:139 DependencyResolver.State
pub struct DependencyResolverState {
    /// Must be set before calling [`DependencyResolver::evaluate`].
    pub target_and_configuration: Option<TargetAndConfiguration>,
    /// Set once the dependency context producer starts.
    pub exec_group_collection_builder: Option<UnloadedToolchainContextsInputs>,
    pub dependency_context: Option<DependencyContext>,
    dependency_map: Option<PrerequisiteMap>,
    materializer_targets: Option<PrerequisiteMap>,
    pub transitive_state: TransitiveDependencyState,
    /// Stores events emitted by memoized computations (the target-and-configuration producer
    /// and dependency resolution may perform Starlark transitions that emit events).
    pub stored_events: StoredEventHandler,
}

impl DependencyResolverState {
    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:211 DependencyResolver.State.<init>
    pub fn new(store_transitive_packages: bool) -> Self {
        DependencyResolverState {
            target_and_configuration: None,
            exec_group_collection_builder: None,
            dependency_context: None,
            dependency_map: None,
            materializer_targets: None,
            transitive_state: TransitiveDependencyState::new(store_transitive_packages),
            stored_events: StoredEventHandler::new(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:224 DependencyResolver.State.transitiveRootCauses
    pub fn transitive_root_causes(&self) -> NestedSet<Cause> {
        self.transitive_state.transitive_root_causes().build()
    }
}

/// The exceptions [`DependencyResolver::evaluate`] throws.
#[derive(Debug)]
pub enum DependencyResolverException {
    /// `ReportedException`.
    Reported(ConfiguredValueCreationException),
    /// `UnreportedException`.
    Unreported(ConfiguredValueCreationException),
    /// `IncompatibleTargetChecker.IncompatibleTargetException`.
    IncompatibleTarget(Arc<ConfiguredTargetValue>),
    Interrupted(Interrupted),
}

impl From<Interrupted> for DependencyResolverException {
    fn from(e: Interrupted) -> Self {
        DependencyResolverException::Interrupted(e)
    }
}

/// The exceptions `evaluate` catches and hands to `handleException`.
///
/// Public for the test SkyFunction of `ToolchainsForTargetsTest` (blaze_sky_top's tests), which
/// calls [`get_dependency_context`].
pub enum CaughtException {
    DependencyEvaluation(DependencyEvaluationException),
    ConfiguredValueCreation(ConfiguredValueCreationException),
    Toolchain(ToolchainException),
    AspectCreation(AspectCreationException),
    StarlarkExecTransitionLoading(StarlarkExecTransitionLoadingException),
    InvalidExecGroup(String),
    IncompatibleTarget(Arc<ConfiguredTargetValue>),
    /// `evaluate` throws a `ReportedException` itself (not through `handleException`).
    DirectReported(ConfiguredValueCreationException),
    /// `evaluate` throws an `UnreportedException` itself.
    DirectUnreported(ConfiguredValueCreationException),
    Interrupted(Interrupted),
}

impl From<Interrupted> for CaughtException {
    fn from(e: Interrupted) -> Self {
        CaughtException::Interrupted(e)
    }
}

/// The exceptions [`compute_dependencies`] throws.
#[derive(Debug)]
pub enum ComputeDependenciesException {
    DependencyEvaluation(DependencyEvaluationException),
    ConfiguredValueCreation(ConfiguredValueCreationException),
    AspectCreation(AspectCreationException),
    Interrupted(Interrupted),
}

impl From<Interrupted> for ComputeDependenciesException {
    fn from(e: Interrupted) -> Self {
        ComputeDependenciesException::Interrupted(e)
    }
}

impl From<ComputeDependenciesException> for CaughtException {
    fn from(e: ComputeDependenciesException) -> Self {
        match e {
            ComputeDependenciesException::DependencyEvaluation(e) => {
                CaughtException::DependencyEvaluation(e)
            }
            ComputeDependenciesException::ConfiguredValueCreation(e) => {
                CaughtException::ConfiguredValueCreation(e)
            }
            ComputeDependenciesException::AspectCreation(e) => CaughtException::AspectCreation(e),
            ComputeDependenciesException::Interrupted(e) => CaughtException::Interrupted(e),
        }
    }
}

/// Helper logic for `ConfiguredTargetFunction`: performs the analysis phase through
/// computation of prerequisites.
// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:132 DependencyResolver
pub struct DependencyResolver {
    target_and_configuration: TargetAndConfiguration,
    dep_value_map: Option<PrerequisiteMap>,
    materializer_targets: Option<PrerequisiteMap>,
    config_conditions: Option<ConfigConditions>,
    platform_info: Option<Arc<PlatformInfo>>,
    unloaded_toolchain_contexts: Option<Arc<ToolchainCollection<UnloadedToolchainContext>>>,
}

impl DependencyResolver {
    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:286 DependencyResolver.<init>
    pub fn new(target_and_configuration: TargetAndConfiguration) -> Self {
        DependencyResolver {
            target_and_configuration,
            dep_value_map: None,
            materializer_targets: None,
            config_conditions: None,
            platform_info: None,
            unloaded_toolchain_contexts: None,
        }
    }

    /// Return this target's `TargetAndConfiguration`.
    pub fn get_target_and_configuration(&self) -> &TargetAndConfiguration {
        &self.target_and_configuration
    }

    /// Return this target's fully resolved dependencies.
    pub fn get_dep_value_map(&self) -> &PrerequisiteMap {
        self.dep_value_map
            .as_ref()
            .expect("NullPointerException: depValueMap")
    }

    pub fn get_materializer_targets(&self) -> Option<&PrerequisiteMap> {
        self.materializer_targets.as_ref()
    }

    /// Return the keys in this target's `select()`s.
    pub fn get_config_conditions(&self) -> &ConfigConditions {
        self.config_conditions
            .as_ref()
            .expect("NullPointerException: configConditions")
    }

    /// Return this target's platform metadata, or `None` if it doesn't use platforms.
    pub fn get_platform_info(&self) -> Option<&Arc<PlatformInfo>> {
        self.platform_info.as_ref()
    }

    /// Return this target's toolchain requirements, or `None` if it doesn't use toolchains.
    pub fn get_unloaded_toolchain_contexts(
        &self,
    ) -> Option<&Arc<ToolchainCollection<UnloadedToolchainContext>>> {
        self.unloaded_toolchain_contexts.as_ref()
    }

    /// Runs the analysis phase for this target through prerequisite evaluation. Returns
    /// `Ok(false)` when any Skyframe dependencies need to be evaluated, else `Ok(true)`.
    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:351 DependencyResolver.evaluate
    pub fn evaluate(
        &mut self,
        state: &mut DependencyResolverState,
        configured_target_key: &ConfiguredTargetKey,
        services: &AnalysisServices,
        env: &mut Environment<'_>,
    ) -> Result<bool, DependencyResolverException> {
        match self.evaluate_internal(state, configured_target_key, services, env) {
            Ok(done) => Ok(done),
            Err(CaughtException::Interrupted(e)) => {
                Err(DependencyResolverException::Interrupted(e))
            }
            Err(CaughtException::IncompatibleTarget(t)) => {
                Err(DependencyResolverException::IncompatibleTarget(t))
            }
            Err(CaughtException::DirectReported(e)) => {
                Err(DependencyResolverException::Reported(e))
            }
            Err(CaughtException::DirectUnreported(e)) => {
                Err(DependencyResolverException::Unreported(e))
            }
            // We handle exceptions in a dedicated method to keep this method concise and
            // readable.
            Err(e) => Err(self.handle_exception(env, e)),
        }
    }

    fn evaluate_internal(
        &mut self,
        state: &mut DependencyResolverState,
        configured_target_key: &ConfiguredTargetKey,
        services: &AnalysisServices,
        env: &mut Environment<'_>,
    ) -> Result<bool, CaughtException> {
        let Some(dependency_context) =
            get_dependency_context(state, configured_target_key, services, env)?
        else {
            return Ok(false);
        };
        self.unloaded_toolchain_contexts = dependency_context.unloaded_toolchain_contexts.clone();
        self.platform_info = self
            .unloaded_toolchain_contexts
            .as_ref()
            .map(|u| u.get_target_platform().clone());
        let config_conditions = dependency_context.config_conditions.clone();
        self.config_conditions = Some(config_conditions.clone());

        // TODO(ulfjack): ConfiguredAttributeMapper (indirectly used from computeDependencies)
        // isn't safe to use if there are missing config conditions, so we stop here, but only if
        // there are config conditions - though note that we can't check if configConditions is
        // non-empty - it may be empty for other reasons. It would be better to continue here so
        // that we can collect more root causes during computeDependencies.
        if state.transitive_state.has_root_cause() && !config_conditions.is_empty_instance() {
            let causes = state.transitive_root_causes();
            env.listener().handle(&Event::error_at(
                event_location(Some(
                    &self.target_and_configuration.get_target().get_location(),
                )),
                "Cannot compute config conditions",
            ));
            let detailed_exit_code = get_prioritized_detailed_exit_code(&causes);
            return Err(CaughtException::DirectReported(
                ConfiguredValueCreationException::for_target_full(
                    self.target_and_configuration.get_target(),
                    configuration_id(
                        self.target_and_configuration
                            .get_configuration()
                            .map(|c| &**c),
                    ),
                    "Cannot compute config conditions",
                    Some(causes),
                    detailed_exit_code,
                ),
            ));
        }

        let Some(starlark_exec_transition) = load_starlark_exec_transition_provider(
            self.target_and_configuration
                .get_configuration()
                .map(|c| &**c.get_options()),
            services,
            env,
        )
        .map_err(CaughtException::StarlarkExecTransitionLoading)?
        else {
            return Ok(false);
        };

        let mut load_exec_aspects_key: Option<LoadAspectsKey> = None;
        if let Some(configuration_key) = configured_target_key.get_configuration_key() {
            let exec_aspects = &configuration_key
                .get_options()
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .exec_aspects;
            if !exec_aspects.is_empty() {
                let aspect_classes = self
                    .create_aspect_classes(exec_aspects)
                    .map_err(CaughtException::AspectCreation)?;
                if !aspect_classes.is_empty() {
                    load_exec_aspects_key = Some(LoadAspectsKey::create(
                        aspect_classes,
                        /* top_level_aspects_parameters= */ IndexMap::new(),
                    ));
                }
            }
        }
        // Calculate the dependencies of this target.
        let dep_value_map = compute_dependencies(
            state,
            configured_target_key,
            /* aspects= */ &[],
            load_exec_aspects_key.as_ref(),
            services,
            starlark_exec_transition,
            env,
            /* base_target_prerequisites_supplier= */ None,
            /* base_target_unloaded_toolchain_contexts= */ None,
        )?;
        if state.transitive_state.has_root_cause() {
            let causes = state.transitive_root_causes();
            // TODO(bazel-team): consider reporting the error in this class vs. exporting it for
            // BuildTool to handle.
            let detailed_exit_code = get_prioritized_detailed_exit_code(&causes);
            return Err(CaughtException::DirectUnreported(
                ConfiguredValueCreationException::for_target_full(
                    self.target_and_configuration.get_target(),
                    configuration_id(
                        self.target_and_configuration
                            .get_configuration()
                            .map(|c| &**c),
                    ),
                    "Analysis failed",
                    Some(causes),
                    detailed_exit_code,
                ),
            ));
        }
        let Some(dep_value_map) = dep_value_map else {
            return Ok(false);
        };
        self.dep_value_map = Some(dep_value_map);
        self.materializer_targets = state.materializer_targets.clone();
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:906 DependencyResolver.createAspectClasses
    fn create_aspect_classes(
        &self,
        aspect_names: &[String],
    ) -> Result<Vec<AspectClass>, AspectCreationException> {
        let mut aspect_classes_builder = Vec::new();
        for aspect in aspect_names {
            match StarlarkAspectClass::get_aspect_class_from_name(aspect) {
                Ok(c) => aspect_classes_builder.push(AspectClass::Starlark(c)),
                Err(e) => {
                    return Err(AspectCreationException::for_label(
                        e.0,
                        self.get_target_and_configuration().get_label(),
                    ));
                }
            }
        }
        Ok(aspect_classes_builder)
    }

    /// Handles all exceptions that [`DependencyResolver::evaluate`] may throw.
    // Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:548 DependencyResolver.handleException
    fn handle_exception(
        &self,
        env: &mut Environment<'_>,
        untyped: CaughtException,
    ) -> DependencyResolverException {
        let target = self.target_and_configuration.get_target();
        let configuration = self
            .target_and_configuration
            .get_configuration()
            .map(|c| &**c);
        match untyped {
            CaughtException::DependencyEvaluation(e) => {
                let mut error_message = e.get_message().to_owned();
                if !e.dep_reported_own_error() {
                    env.listener().handle(&Event::error_at(
                        event_location(e.get_location()),
                        e.get_message(),
                    ));
                }

                let mut cvce = None;
                if let Some(cause) = e.get_creation_cause() {
                    // Check if this is caused by an unresolved toolchain, and report it as such.
                    if let Some(unloaded) = &self.unloaded_toolchain_contexts {
                        let required_toolchains = unloaded.get_resolved_toolchains();
                        let any_toolchain_dependency_error = cause
                            .get_root_causes()
                            .to_list()
                            .iter()
                            .any(|c| required_toolchains.contains(c.get_label()));

                        if any_toolchain_dependency_error {
                            error_message = format!(
                                "errors encountered resolving toolchains for {}",
                                target.get_label()
                            );
                            env.listener().handle(&Event::error_at(
                                event_location(Some(&target.get_location())),
                                error_message.clone(),
                            ));
                        }
                    }
                    cvce = Some(cause.clone());
                }

                DependencyResolverException::Reported(cvce.unwrap_or_else(|| {
                    ConfiguredValueCreationException::for_target_full(
                        target,
                        configuration_id(configuration),
                        error_message,
                        None,
                        e.get_detailed_exit_code().cloned(),
                    )
                }))
            }
            CaughtException::ConfiguredValueCreation(e) => {
                if !e.get_message().is_empty() {
                    // Report the error to the user.
                    env.listener().handle(&Event::error_at(
                        event_location(e.get_location()),
                        e.get_message(),
                    ));
                }
                DependencyResolverException::Reported(e)
            }
            CaughtException::AspectCreation(e) => {
                if !e.get_message().is_empty() {
                    // Report the error to the user.
                    env.listener().handle(&Event::error(e.get_message()));
                }
                DependencyResolverException::Reported(
                    ConfiguredValueCreationException::for_target_full(
                        target,
                        configuration_id(configuration),
                        e.get_message(),
                        Some(e.get_causes().clone()),
                        Some(e.get_detailed_exit_code().clone()),
                    ),
                )
            }
            CaughtException::Toolchain(e) => {
                let cvce = toolchain_exception_as_configured_value_creation_exception(
                    &e,
                    &self.target_and_configuration,
                );
                env.listener().handle(&Event::error_at(
                    event_location(Some(&target.get_location())),
                    cvce.get_message(),
                ));
                DependencyResolverException::Reported(cvce)
            }
            CaughtException::StarlarkExecTransitionLoading(e) => {
                if !e.0.is_empty() {
                    // Report the error to the user.
                    env.listener().handle(&Event::error(e.0.clone()));
                }
                DependencyResolverException::Reported(
                    ConfiguredValueCreationException::for_target_full(
                        target,
                        configuration_id(configuration),
                        e.0.clone(),
                        /* root_causes= */ None,
                        /* detailed_exit_code= */ None,
                    ),
                )
            }
            CaughtException::InvalidExecGroup(message) => {
                env.listener().handle(&Event::error_at(
                    event_location(Some(&target.get_location())),
                    message.clone(),
                ));
                DependencyResolverException::Reported(
                    ConfiguredValueCreationException::for_target_full(
                        target,
                        configuration_id(configuration),
                        message,
                        /* root_causes= */ None,
                        /* detailed_exit_code= */ None,
                    ),
                )
            }
            CaughtException::IncompatibleTarget(_)
            | CaughtException::DirectReported(_)
            | CaughtException::DirectUnreported(_)
            | CaughtException::Interrupted(_) => {
                panic!("IllegalStateException: unexpected exception with no appropriate handler")
            }
        }
    }
}

/// Java `ToolchainException.asConfiguredValueCreationException(targetAndConfiguration)`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:69 ToolchainException.asConfiguredValueCreationException
pub fn toolchain_exception_as_configured_value_creation_exception(
    e: &ToolchainException,
    target_and_configuration: &TargetAndConfiguration,
) -> ConfiguredValueCreationException {
    // Attempt to find a ConfiguredValueCreationException in the ToolchainException's causes.
    let mut cause: Option<&(dyn std::error::Error + 'static)> = e.get_cause().map(|c| {
        let c: &(dyn std::error::Error + 'static) = &**c;
        c
    });
    while let Some(c) = cause {
        if let Some(configured_value_creation_exception) =
            c.downcast_ref::<ConfiguredValueCreationException>()
        {
            return configured_value_creation_exception.clone();
        }
        cause = c.source();
    }
    let configuration = target_and_configuration.get_configuration();
    let cause = Cause::AnalysisFailed {
        label: target_and_configuration.get_label().clone(),
        configuration: configuration_id_message(configuration.map(|c| c.checksum())),
        detailed_exit_code: create_detailed_exit_code(&format!(
            "While resolving toolchains for target {}: {}",
            target_and_configuration.get_label(),
            e.get_message()
        )),
    };
    ConfiguredValueCreationException::for_target_full(
        target_and_configuration.get_target(),
        configuration_id(configuration.map(|c| &**c)),
        format!(
            "While resolving toolchains for target {} ({}): {}",
            target_and_configuration.get_label(),
            configuration
                .map(|c| c.short_id())
                .unwrap_or_else(|| "null".to_owned()),
            e.get_message()
        ),
        Some(NestedSetBuilder::create(Order::STABLE_ORDER, [cause])),
        Some(toolchain_exception_detailed_exit_code(e)),
    )
}

/// Java `ToolchainException.getDetailedExitCode`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:51 ToolchainException.getDetailedExitCode
pub fn toolchain_exception_detailed_exit_code(
    e: &blaze_analysis::toolchains::ToolchainException,
) -> DetailedExitCode {
    if let Some(cause) = e.get_cause() {
        // `getCause() instanceof DetailedException`.
        let cause: &(dyn std::error::Error + 'static) = &**cause;
        if let Some(c) = cause.downcast_ref::<blaze_packages::exceptions::NoSuchThingException>() {
            return c.get_detailed_exit_code();
        }
        if let Some(c) = cause.downcast_ref::<blaze_packages::exceptions::NoSuchPackageException>()
        {
            return c.get_detailed_exit_code();
        }
        if let Some(c) = cause.downcast_ref::<blaze_packages::exceptions::NoSuchTargetException>() {
            return c.get_detailed_exit_code();
        }
        if let Some(c) = cause.downcast_ref::<ConfiguredValueCreationException>() {
            return c.get_detailed_exit_code().clone();
        }
    }
    DetailedExitCode {
        message: e.get_message().to_owned(),
        code: blaze_packages::exceptions::FailureCode::Other(format!(
            "Toolchain.{}",
            e.get_detailed_code()
        )),
    }
}

/// Null when a Skyframe restart is needed.
///
/// Public for the test SkyFunction of `ToolchainsForTargetsTest` (blaze_sky_top's tests).
// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:479 DependencyResolver.getDependencyContext
pub fn get_dependency_context(
    state: &mut DependencyResolverState,
    configured_target_key: &ConfiguredTargetKey,
    services: &AnalysisServices,
    env: &mut Environment<'_>,
) -> Result<Option<DependencyContext>, CaughtException> {
    if let Some(c) = &state.dependency_context {
        return Ok(Some(c.clone()));
    }
    let target_and_configuration = state
        .target_and_configuration
        .clone()
        .expect("targetAndConfiguration");
    let unloaded_toolchain_contexts_inputs = get_unloaded_toolchain_contexts_inputs(
        &target_and_configuration,
        configured_target_key.get_execution_platform_label(),
        services,
        env,
    )
    .map_err(CaughtException::InvalidExecGroup)?;
    state.exec_group_collection_builder = Some(unloaded_toolchain_contexts_inputs.clone());
    let result = produce_dependency_context_with_compatibility_check(
        env,
        &target_and_configuration,
        configured_target_key,
        &unloaded_toolchain_contexts_inputs,
        &mut state.transitive_state,
    )?;

    match result {
        None => Ok(None), // Not yet done.
        Some(Ok(c)) => {
            state.dependency_context = Some(c.clone());
            Ok(Some(c))
        }
        Some(Err(error)) => Err(match error {
            DependencyContextError::Toolchain(e) => CaughtException::Toolchain(e),
            DependencyContextError::ConfiguredValueCreation(e) => {
                CaughtException::ConfiguredValueCreation(e)
            }
            DependencyContextError::IncompatibleTarget(t) => CaughtException::IncompatibleTarget(t),
            DependencyContextError::Validation(validation_exception) => {
                CaughtException::DependencyEvaluation(handle_dependency_root_cause_error(
                    &target_and_configuration,
                    Some(target_and_configuration.get_target().get_location()),
                    &validation_exception.0,
                    env.listener(),
                ))
            }
        }),
    }
}

/// The Starlark exec transition (as a `StarlarkAttributeTransitionProvider`).
///
/// Returns `Ok(None)` where Java returns null (Skyframe deps missing), `Ok(Some(None))` when the
/// build has no Starlark exec transition.
///
/// PORT: a file in `@_builtins` is evaluated by the bundled loader of `blaze_build_config`
/// (Java: BZL_LOAD with `keyForBuiltins`), whose result is memoized here per
/// `--experimental_exec_config` value, for the life of the build arena
/// (`blaze_util::build_arena`, decision D7): one command in batch mode and in the in-process
/// server, and also one command in the persistent server, which releases the arena after every
/// command (its graph is discarded after every command, decision D2, so Java's BZL_LOAD node
/// would be gone too); another file is requested through BZL_LOAD
/// (`keyForBuild`), as in Java.
// Java: com/google/devtools/build/lib/analysis/config/StarlarkExecTransitionLoader.java:78 StarlarkExecTransitionLoader.loadStarlarkExecTransition
pub fn load_starlark_exec_transition_provider(
    options: Option<&BuildOptions>,
    services: &AnalysisServices,
    env: &mut Environment<'_>,
) -> Result<
    Option<Option<Arc<StarlarkAttributeTransitionProvider>>>,
    StarlarkExecTransitionLoadingException,
> {
    #[derive(Default)]
    struct Loaded(Mutex<HashMap<String, Option<Arc<StarlarkAttributeTransitionProvider>>>>);
    impl blaze_util::build_arena::ArenaSlot for Loaded {
        // The providers own their modules' heaps (`StarlarkDefinedConfigTransition::from_frozen`
        // of an `OwnedFrozenValue`).
        const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
            blaze_util::build_arena::ReleaseOrder::State;
    }
    let loaded = blaze_util::build_arena::slot::<Loaded>();
    let Some(options) = options else {
        return Ok(Some(None));
    };
    if *options == *EMPTY_OPTIONS {
        return Ok(Some(None));
    }
    let user_ref = options
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .starlark_exec_config
        .clone();
    if let Some(p) = loaded.0.lock().get(&user_ref) {
        return Ok(Some(p.clone()));
    }
    let loader = EnvironmentBzlFileLoader {
        env: std::cell::RefCell::new(env),
        semantics: &services.starlark_semantics,
        loaded_user_file: std::cell::Cell::new(false),
    };
    let Some(transition) =
        blaze_build_config::starlark_transition::load_starlark_exec_transition_with_loader(
            Some(options),
            &loader,
        )?
    else {
        return Ok(None);
    };
    let provider = transition.map(|t| Arc::new(StarlarkAttributeTransitionProvider::new_exec(t)));
    if loader.loaded_user_file.get() {
        return Ok(Some(provider));
    }
    Ok(Some(
        loaded.0.lock().entry(user_ref).or_insert(provider).clone(),
    ))
}

/// `(bzlKey) -> (BzlLoadValue) env.getValueOrThrow(bzlKey, BzlLoadFailedException.class)`.
// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:404 DependencyResolver.evaluate
struct EnvironmentBzlFileLoader<'e, 'a, 's> {
    env: std::cell::RefCell<&'e mut Environment<'a>>,
    semantics: &'s Arc<blaze_starlark::StarlarkSemantics>,
    loaded_user_file: std::cell::Cell<bool>,
}

impl blaze_build_config::starlark_transition::BzlFileLoader
    for EnvironmentBzlFileLoader<'_, '_, '_>
{
    fn convert_transition(
        &self,
        value: &starlark::values::OwnedFrozenValue,
    ) -> Option<Arc<blaze_build_config::starlark_transition::StarlarkDefinedConfigTransition>> {
        let frozen = value.value().unpack_frozen()?;
        blaze_config::transitions::StarlarkDefinedConfigTransition::from_value(frozen.to_value())?;
        // The module's heap stays alive in the build arena (BZL_LOAD's keep-alive), as
        // StarlarkCallable requires.
        Some(
            blaze_analysis::config::transitions::starlark_defined_config_transition_of(
                &blaze_packages::rule_class::StarlarkCallable::new(frozen),
            ),
        )
    }
    fn get_value(
        &self,
        bzl_file: &blaze_cmdline::Label,
        for_builtins: bool,
    ) -> Result<Option<starlark::environment::FrozenModule>, String> {
        if for_builtins {
            return blaze_build_config::starlark_transition::load_builtins_bzl(
                bzl_file,
                self.semantics,
            )
            .map(Some);
        }
        self.loaded_user_file.set(true);
        let key = blaze_sky_loading::bzl_load::BzlLoadKey::key_for_build(bzl_file.clone());
        let mut env = self.env.borrow_mut();
        match env.get_value_or_throw::<
            blaze_sky_loading::bzl_load::BzlLoadKey,
            blaze_sky_loading::bzl_load::BzlLoadFailedException,
        >(key)
        {
            Ok(Ok(Some(value))) => Ok(Some(value.get_module().clone())),
            Ok(Ok(None)) => Ok(None),
            Ok(Err(e)) => Err(e.to_string()),
            // PORT: an interrupt while loading the exec transition's .bzl; the loader interface
            // has no interrupted result.
            Err(_) => blaze_util::unported!(
                "StarlarkExecTransitionLoader.loadStarlarkExecTransition",
                "interrupted while loading {}",
                bzl_file
            ),
        }
    }
}

/// Computes the direct dependencies of a node in the configured target graph (a configured
/// target or an aspect). Returns `None` if Skyframe hasn't evaluated the required dependencies
/// yet.
///
/// REQUIRES: `state.dependency_context` is populated.
///
/// `load_exec_aspects_key`: the key of the aspects passed to the --exec_aspects flag that are
/// attached to exec-configured targets. `base_target_prerequisites_supplier` and
/// `base_target_unloaded_toolchain_contexts` are non-null only in case of aspect evaluation (the
/// latter is the `UnloadedToolchainContext`s of the underlying target to support aspects
/// toolchains propagation).
// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:648 DependencyResolver.computeDependencies
pub fn compute_dependencies(
    state: &mut DependencyResolverState,
    configured_target_key: &ConfiguredTargetKey,
    aspects: &[Aspect],
    load_exec_aspects_key: Option<&LoadAspectsKey>,
    services: &AnalysisServices,
    starlark_transition_provider: Option<Arc<StarlarkAttributeTransitionProvider>>,
    env: &mut Environment<'_>,
    base_target_prerequisites_supplier: Option<&BaseTargetPrerequisitesSupplier>,
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
) -> Result<Option<PrerequisiteMap>, ComputeDependenciesException> {
    // Replays stored events unless a Skyframe restart is immediately needed and the events would
    // be unused anyway.
    let mut should_replay_stored_events = true;
    let result = compute_dependencies_internal(
        state,
        configured_target_key,
        aspects,
        load_exec_aspects_key,
        services,
        starlark_transition_provider,
        env,
        base_target_prerequisites_supplier,
        base_target_unloaded_toolchain_contexts,
        &mut should_replay_stored_events,
    );
    if should_replay_stored_events {
        state.stored_events.replay_on(env.listener());
    }
    result
}

fn compute_dependencies_internal(
    state: &mut DependencyResolverState,
    configured_target_key: &ConfiguredTargetKey,
    aspects: &[Aspect],
    load_exec_aspects_key: Option<&LoadAspectsKey>,
    services: &AnalysisServices,
    starlark_transition_provider: Option<Arc<StarlarkAttributeTransitionProvider>>,
    env: &mut Environment<'_>,
    base_target_prerequisites_supplier: Option<&BaseTargetPrerequisitesSupplier>,
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    should_replay_stored_events: &mut bool,
) -> Result<Option<PrerequisiteMap>, ComputeDependenciesException> {
    if let Some(m) = &state.dependency_map {
        return Ok(Some(m.clone()));
    }
    let ctg_value = state
        .target_and_configuration
        .clone()
        .expect("targetAndConfiguration");
    let dependency_context = state.dependency_context.clone().expect("dependencyContext");
    let toolchain_contexts = dependency_context.toolchain_contexts();
    let config_conditions = Arc::new(dependency_context.config_conditions.as_providers().clone());
    let dependency_labels = match compute_dependency_labels(
        &ctg_value,
        aspects,
        &config_conditions,
        toolchain_contexts,
        base_target_unloaded_toolchain_contexts,
    ) {
        Ok(l) => l,
        Err(e) => {
            return Err(ComputeDependenciesException::DependencyEvaluation(
                handle_dependency_root_cause_error(
                    &ctg_value,
                    e.location,
                    &e.message,
                    env.listener(),
                ),
            ));
        }
    };
    let outcome = {
        let mut parameters = PrerequisiteParameters {
            services,
            configured_target_key,
            target_and_configuration: &ctg_value,
            aspects,
            load_exec_aspects_key,
            starlark_transition_provider,
            toolchain_contexts,
            attribute_map: dependency_labels.attribute_map.as_ref(),
            transitive_state: &mut state.transitive_state,
            event_handler: &state.stored_events,
            base_target_prerequisites_supplier,
            base_target_toolchain_contexts: base_target_unloaded_toolchain_contexts,
        };
        produce_dependency_map(env, &mut parameters, &dependency_labels.labels)?
    };

    let Some(outcome) = outcome else {
        if !state.transitive_state.has_root_cause() {
            *should_replay_stored_events = false; // Skyframe restart is needed.
        }
        return Ok(None);
    };
    state.materializer_targets = outcome.materializer_targets;
    if let Some(error) = outcome.dependency_map_error {
        let target = ctg_value.get_target();
        return Err(match error {
            DependencyError::DependencyTransition(e) => {
                ComputeDependenciesException::ConfiguredValueCreation(
                    ConfiguredValueCreationException::for_target(target, e.0),
                )
            }
            DependencyError::DependencyOptionsParsing(m) => {
                ComputeDependenciesException::ConfiguredValueCreation(
                    ConfiguredValueCreationException::for_target(target, m),
                )
            }
            DependencyError::Materializer(m) => {
                ComputeDependenciesException::ConfiguredValueCreation(
                    ConfiguredValueCreationException::for_target(target, m),
                )
            }
            DependencyError::InvalidVisibility(label) => {
                ComputeDependenciesException::DependencyEvaluation(
                    handle_dependency_root_cause_error(
                        &ctg_value,
                        Some(target.get_location()),
                        &format!("Label '{}' does not refer to a package group.", label),
                        env.listener(),
                    ),
                )
            }
            DependencyError::AspectEvaluation(e) => {
                ComputeDependenciesException::DependencyEvaluation(e)
            }
            DependencyError::AspectCreation(e) => ComputeDependenciesException::AspectCreation(e),
            DependencyError::PlatformMapping(m)
            | DependencyError::InvalidPlatform(m)
            | DependencyError::BuildOptionsScope(m) => {
                ComputeDependenciesException::ConfiguredValueCreation(
                    ConfiguredValueCreationException::for_target(target, m),
                )
            }
            DependencyError::TransitionCreation(e) => {
                ComputeDependenciesException::ConfiguredValueCreation(
                    ConfiguredValueCreationException::for_target(target, e.0),
                )
            }
        });
    }
    if !state.transitive_state.has_root_cause() && outcome.dependency_map.is_none() {
        *should_replay_stored_events = false; // Skyframe restart is needed.
    }
    state.dependency_map = outcome.dependency_map.clone();
    Ok(outcome.dependency_map)
}

// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:782 DependencyResolver.getUnloadedToolchainContextsInputs
pub fn get_unloaded_toolchain_contexts_inputs(
    target_and_configuration: &TargetAndConfiguration,
    parent_execution_platform_label: Option<&Label>,
    services: &AnalysisServices,
    env: &mut Environment<'_>,
) -> Result<UnloadedToolchainContextsInputs, String> {
    let Some(configuration) = target_and_configuration.get_configuration() else {
        return Ok(UnloadedToolchainContextsInputs::empty());
    };
    let toolchain_configuration_key = {
        let events = StoredEventHandler::new();
        let key =
            compute_toolchain_configuration_key(configuration.get_options(), services, &events);
        events.replay_on(env.listener());
        key
    };
    crate::toolchains::toolchain_context_util::get_unloaded_toolchain_contexts_inputs(
        target_and_configuration.get_target(),
        configuration
            .get_options()
            .get::<CoreOptions>()
            .expect("CoreOptions"),
        configuration.get_fragment::<PlatformConfiguration>(),
        parent_execution_platform_label,
        toolchain_configuration_key,
    )
    .map_err(|e| e.to_string())
}

// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:870 DependencyResolver.handleDependencyRootCauseError
fn handle_dependency_root_cause_error(
    target_and_configuration: &TargetAndConfiguration,
    location: Option<Location>,
    message: &str,
    listener: &dyn ExtendedEventHandler,
) -> DependencyEvaluationException {
    let configuration = target_and_configuration.get_configuration();
    let label = target_and_configuration.get_label();
    // AnalysisRootCauseEvent.withConfigurationValue(configuration, label, message)
    listener.post(Arc::new(AnalysisRootCauseEvent {
        configuration_checksum: configuration.map(|c| c.checksum().to_owned()),
        label: label.clone(),
        error_message: message.to_owned(),
    }));
    let cause = Cause::AnalysisFailed {
        label: label.clone(),
        configuration: configuration_id_message(configuration.map(|c| c.checksum())),
        detailed_exit_code: create_detailed_exit_code(message),
    };
    let exit_code = cause.get_detailed_exit_code().clone();
    DependencyEvaluationException::of_creation(
        ConfiguredValueCreationException::new(
            location,
            message,
            label,
            configuration_id(configuration.map(|c| &**c)),
            Some(NestedSetBuilder::create(Order::STABLE_ORDER, [cause])),
            Some(exit_code),
        ),
        // These errors occur in dependency resolution, which is attached to the current target.
        // i.e. no dependent ConfiguredTargetFunction call happens to report its own error.
        /* dep_reported_own_error= */
        false,
    )
}

// Java: com/google/devtools/build/lib/skyframe/DependencyResolver.java:864 DependencyResolver.getPrioritizedDetailedExitCode
pub fn get_prioritized_detailed_exit_code(causes: &NestedSet<Cause>) -> Option<DetailedExitCode> {
    let mut prioritized_detailed_exit_code = None;
    for c in causes.to_list().iter() {
        prioritized_detailed_exit_code = choose_more_important_with_first_if_tie(
            prioritized_detailed_exit_code,
            Some(c.get_detailed_exit_code().clone()),
        );
    }
    prioritized_detailed_exit_code
}

/// Java `Target` of a node (helper for callers).
pub fn target_of(state: &DependencyResolverState) -> Option<&Target> {
    state
        .target_and_configuration
        .as_ref()
        .map(|t| t.get_target())
}
