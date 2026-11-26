//! Port of `skyframe/AspectFunction.java`: the Skyframe function that generates aspects.
//!
//! This class, together with `ConfiguredTargetFunction`, drives the analysis phase.
//! `AspectFunction` takes an `AspectKey` (a tuple of target label, configuration, aspect class
//! and aspect parameters), loads an `Aspect` from the aspect class and parameters, gets the
//! `ConfiguredTarget` for the label and configuration, and then creates a `ConfiguredAspect` for
//! the key.
//!
//! PORT: Java's `State` (a `SkyKeyComputeState` memoizing work across restarts) lives for one
//! `compute` call here: the evaluator blocks instead of restarting. Remote analysis caching
//! (`RemoteAnalysisCachingDependenciesProvider`, `retrieveRemoteSkyValue`) is not ported:
//! retrieval is never enabled, as in `configured_target_function.rs`.

use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::aspect_key_creator::{ASPECT, AspectKey, AspectKeyCreator};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::base_target_prerequisites_supplier::BaseTargetPrerequisitesSupplier;
use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::incompatible_platform_provider::INCOMPATIBLE_PLATFORM_PROVIDER;
use blaze_analysis::config::ordered_set_multimap::PrerequisiteMap;
use blaze_analysis::config::transitions::StarlarkAttributeTransitionProvider;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_aspect_factory::ConfiguredAspectFactory;
use blaze_analysis::configured_rule_class_provider::{
    ConfiguredRuleClassProvider, CreateConfiguredTargetError,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configuredtargets::MergedConfiguredTarget;
use blaze_analysis::dependency_kind::DependencyKind;
use blaze_analysis::platform::toolchain_type_info::ToolchainTypeInfo;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::rule::action_conflict_exception::ActionConflictException;
use blaze_analysis::rule::analysis_environment::CachingAnalysisEnvironment;
use blaze_analysis::rule::configured_target_factory::{
    ConfiguredTargetFactory, ConfiguredTargetFactoryError,
};
use blaze_analysis::starlark::starlark_aspect_factory::StarlarkAspectFactory;
use blaze_analysis::toolchains::aspect_base_target_resolved_toolchain_context::AspectBaseTargetResolvedToolchainContext;
use blaze_analysis::toolchains::{
    ResolvedToolchainContext, ToolchainCollection, ToolchainContextKey, ToolchainException,
    UnloadedToolchainContext, UnloadedToolchainContextsInputs,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::{Event, StoredEventHandler};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_packages::aspect::{
    Aspect, AspectClass, AspectDefinition, NativeAspectClass, StarlarkAspectClass,
};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_packages::package::Package;
use blaze_packages::starlark_defs::aspect::{aspect_data, is_starlark_aspect};
use blaze_packages::target::Target;
use blaze_packages::types::AttrValue;
use blaze_sky_loading::bzl_load::{BzlLoadFailedException, BzlLoadKey, BzlLoadValue};
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::repos::is_builtins_repo;
use blaze_sky_loading::starlark_builtins::StarlarkBuiltinsKey;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, SkyFunction, SkyFunctionException, SkyKey,
    Transience,
};
use indexmap::{IndexMap, IndexSet};
use starlark::values::OwnedFrozenValue;

use crate::analysis_progress_receiver::ANALYSIS_PROGRESS;
use crate::dependency_resolver::{
    ComputeDependenciesException, DependencyResolverState, compute_dependencies,
    get_prioritized_detailed_exit_code, get_unloaded_toolchain_contexts_inputs,
    load_starlark_exec_transition_provider, toolchain_exception_detailed_exit_code,
};
use crate::producers::configured_aspect_producer::{
    configured_target_key_from_configured_target, to_configured_aspect,
};
use crate::producers::dependency_context_producer::{
    DependencyContext, produce_dependency_context,
};
use crate::producers::errors::{
    AspectCreationException, Cause, ConfiguredTargetFunctionException,
    ConfiguredValueCreationException, DependencyContextError, DependencyEvaluationCause,
    DependencyEvaluationException, TargetAndConfiguration, TransitiveDependencyState,
    create_aspect_detailed_exit_code, event_location,
};
use crate::toolchains::toolchain_context_util::{
    create_default_toolchain_context_key, process_declared_exec_groups,
};
use crate::toolchains::unloaded_toolchain_contexts_producer::produce_unloaded_toolchain_contexts_with_supplier;

/// The Skyframe function that generates aspects (Java `AspectFunction`).
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:131 AspectFunction
pub struct AspectFunction {
    services: Arc<AnalysisServices>,
    /// Indicates whether the set of packages transitively loaded for a given `AspectValue` will
    /// be needed later. If not, they are not collected and stored.
    store_transitive_packages: bool,
}

/// Registers `ASPECT`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:857 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder, services: Arc<AnalysisServices>) -> EvaluatorBuilder {
    builder.function(
        ASPECT,
        AspectFunction {
            services,
            // Java: SkyframeExecutor.shouldStoreTransitivePackagesInLoadingAndAnalysis()
            // (`allowExternalRepositories`, true in Bazel).
            store_transitive_packages: true,
        },
    )
}

/// Used to indicate errors during the computation of an `AspectValue` (the cause of the
/// function's `SkyFunctionException`).
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:1059 AspectFunction.AspectFunctionException
#[derive(Clone, Debug, thiserror::Error)]
pub enum AspectFunctionException {
    #[error("{0}")]
    NoSuchThing(NoSuchThingException),
    #[error("{0}")]
    AspectCreation(AspectCreationException),
    #[error("{0}")]
    ConfiguredValueCreation(ConfiguredValueCreationException),
    /// `InvalidExecGroupException`, carried by its message.
    #[error("{0}")]
    InvalidExecGroup(String),
    #[error("{0}")]
    ActionConflict(ActionConflictException),
    /// `RuleErrorException`, carried by its message.
    #[error("{0}")]
    RuleError(String),
}

fn throw(e: AspectFunctionException) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

/// The values `getInitialValues` computes (Java `AspectFunction.InitialValues`).
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:240 AspectFunction.InitialValues
struct InitialValues {
    aspect: Aspect,
    aspect_factory: Arc<dyn ConfiguredAspectFactory>,
    base_configured_target: ConfiguredTarget,
    /// PORT: Java's `target.getPackage()`; Blaze targets hold only the package metadata, so
    /// the base package (requested here) is kept for `createAspect`.
    base_package: Arc<Package>,
}

/// The exceptions the body of `compute` catches (and those it throws directly).
enum Caught {
    DependencyEvaluation(DependencyEvaluationException),
    AspectCreation(AspectCreationException),
    ConfiguredValueCreation(ConfiguredValueCreationException),
    Toolchain(ToolchainException),
    /// `ExecGroupCollection.InvalidExecGroupException`, carried by its message.
    InvalidExecGroup(String),
    /// `throw new AspectFunctionException(...)` inside the `try`: not caught.
    Thrown(AspectFunctionException),
    Compute(ComputeError),
}

impl From<blaze_skyframe::Interrupted> for Caught {
    fn from(e: blaze_skyframe::Interrupted) -> Self {
        Caught::Compute(e.into())
    }
}

impl From<ComputeDependenciesException> for Caught {
    fn from(e: ComputeDependenciesException) -> Self {
        match e {
            ComputeDependenciesException::DependencyEvaluation(e) => {
                Caught::DependencyEvaluation(e)
            }
            ComputeDependenciesException::ConfiguredValueCreation(e) => {
                Caught::ConfiguredValueCreation(e)
            }
            ComputeDependenciesException::AspectCreation(e) => Caught::AspectCreation(e),
            ComputeDependenciesException::Interrupted(e) => Caught::Compute(e.into()),
        }
    }
}

impl SkyFunction for AspectFunction {
    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:258 AspectFunction.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> blaze_skyframe::ComputeResult {
        let aspect_key = key.downcast::<AspectKey>().expect("ASPECT key");
        Ok(self
            .compute_typed(aspect_key, env)?
            .map(|v| v as blaze_skyframe::SkyValue))
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:1052 AspectFunction.extractTag
    fn extract_tag(&self, key: &SkyKey) -> Option<String> {
        // Label.print(aspectKey.getLabel())
        key.downcast::<AspectKey>()
            .map(|k| k.get_label().to_string())
    }
}

impl AspectFunction {
    /// Used to look up configured targets and configurations of the underlying target
    /// dependencies without adding dependency edges to them.
    ///
    /// A regular Skyframe lookup of the target's dependencies while evaluating the aspect
    /// propagation logic adds unnecessary dependency edges between the aspect and its target's
    /// dependencies. Instead the supplier is used to directly look up the dependency values.
    ///
    /// Regular Skyframe lookup is used to get the `ConfiguredTargetValue`s of the aspect's
    /// implicit dependencies and its underlying target to establish a dependency from the
    /// `AspectValue` to them.
    fn base_target_prerequisites_supplier(&self) -> &BaseTargetPrerequisitesSupplier {
        self.services.get_base_target_prerequisites_supplier()
    }

    fn compute_typed(
        &self,
        key: &AspectKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<AspectValue>>, ComputeError> {
        // PORT: remote analysis caching is not ported (`isRetrievalEnabled()` is false).
        let mut state = DependencyResolverState::new(self.store_transitive_packages);
        let Some(initial_values) = get_initial_values(&mut state, key, env, &self.services.rule_class_provider)? else {
            return Ok(None);
        };
        let InitialValues {
            aspect,
            aspect_factory,
            base_configured_target,
            base_package,
        } = initial_values;
        let mut associated_target = base_configured_target;
        let target_and_configuration = state
            .target_and_configuration
            .clone()
            .expect("targetAndConfiguration");
        let target = target_and_configuration.get_target();
        let configuration = target_and_configuration.get_configuration();

        // If the target is incompatible, then there's not much to do. The intent here is to
        // create an AspectValue that doesn't trigger any of the associated target's dependencies
        // to be evaluated against this aspect.
        if associated_target
            .get(&INCOMPATIBLE_PLATFORM_PROVIDER.key())
            .is_some()
            // Similarly, aspects that propagate into post-NoConfigTransition targets can't
            // access most flags or dependencies and are likely to be unsound. So make aspects
            // propagating to these configurations no-ops.
            || configuration.is_some_and(|c| c.get_options().has_no_config())
        {
            return Ok(Some(Arc::new(AspectValue::create(
                key.clone(),
                aspect,
                &ConfiguredAspect::NonApplicable,
                state.transitive_state.transitive_packages(),
            ))));
        }

        if AliasProvider::is_alias(&associated_target) {
            return self.create_alias_aspect_of_target(
                env,
                &target_and_configuration,
                &aspect,
                key,
                &associated_target,
                &state.transitive_state,
            );
        }
        // If we get here, label should match original label, and therefore the target we looked
        // up above indeed corresponds to associatedTarget.getLabel().
        blaze_util::check_state!(
            associated_target.get_original_label() == associated_target.get_label(),
            "Non-alias {} should have matching label but found {}",
            associated_target.get_original_label(),
            associated_target.get_label()
        );

        if associated_target.get_configuration_key().is_none()
            || !target_satisfies_aspect(target, &aspect)
        {
            // Aspects cannot apply to PackageGroups or InputFiles, the only cases where
            // configuration key is null. They also cannot apply to targets that don't satisfy the
            // aspect's requirements.
            return Ok(Some(Arc::new(AspectValue::create(
                key.clone(),
                aspect,
                &ConfiguredAspect::NonApplicable,
                state.transitive_state.transitive_packages(),
            ))));
        }

        let topological_aspect_path: Vec<Aspect>;
        if key.get_base_keys().is_empty() {
            topological_aspect_path = vec![aspect.clone()];
        } else {
            let mut ordered_keys: IndexSet<AspectKey> = IndexSet::new();
            collect_aspect_keys_in_topological_order(key.get_base_keys(), &mut ordered_keys);
            let sky_keys: Vec<SkyKey> = ordered_keys
                .iter()
                .map(|k| SkyKey::new(k.clone()))
                .collect();
            let aspect_values = env.get_values_and_exceptions(&sky_keys)?;
            if env.values_missing() {
                return Ok(None);
            }
            let mut topological_aspect_path_builder: Vec<Aspect> =
                Vec::with_capacity(ordered_keys.len() + 1);
            for aspect_key in &ordered_keys {
                let Some(aspect_value) = aspect_values.get_typed(aspect_key) else {
                    // PORT: BugReport.logUnexpected("aspectValue for: '%s' was missing, this
                    // should never happen", aspectKey) is not ported.
                    return Ok(None);
                };
                topological_aspect_path_builder.push(aspect_value.get_aspect().clone());
            }
            topological_aspect_path_builder.push(aspect.clone());
            topological_aspect_path = topological_aspect_path_builder;

            let directly_required_aspects: Vec<ConfiguredAspect> = key
                .get_base_keys()
                .iter()
                .map(|k| {
                    to_configured_aspect(
                        &aspect_values
                            .get_typed(k)
                            .expect("NullPointerException: base aspect value"),
                    )
                })
                .collect();
            match MergedConfiguredTarget::of(associated_target.clone(), &directly_required_aspects)
            {
                Ok(merged) => associated_target = merged,
                Err(e) => {
                    env.listener().handle(&Event::error_at(
                        event_location(Some(&target.get_location())),
                        e.get_message(),
                    ));
                    return Err(throw(AspectFunctionException::AspectCreation(
                        AspectCreationException::for_target(
                            e.get_message(),
                            target.get_label(),
                            configuration.map(|c| &**c),
                        ),
                    )));
                }
            }
        }

        let result = self.compute_after_aspect_path(
            env,
            &mut state,
            key,
            &aspect,
            &aspect_factory,
            &target_and_configuration,
            &base_package,
            &associated_target,
            &topological_aspect_path,
        );
        match result {
            Ok(v) => Ok(v),
            Err(Caught::DependencyEvaluation(e)) => {
                // TODO(bazel-team): consolidate all env.getListener().handle() calls in this
                // method, like in ConfiguredTargetFunction. This encourages clear, consistent
                // user messages (ideally without the programmer having to think about it).
                if !e.dep_reported_own_error() {
                    env.listener().handle(&Event::error_at(
                        event_location(e.get_location()),
                        e.get_message(),
                    ));
                }
                Err(throw(AspectFunctionException::AspectCreation(
                    match e.get_cause() {
                        DependencyEvaluationCause::Creation(cause) => AspectCreationException::new(
                            cause.get_message(),
                            cause.get_root_causes().clone(),
                            cause.get_detailed_exit_code().clone(),
                        ),
                        // Exception while evaluating the aspect `attr_aspects` and
                        // `toolchains_aspects` functions.
                        DependencyEvaluationCause::Eval(message) => {
                            AspectCreationException::for_target(
                                message.clone(),
                                key.get_label(),
                                configuration.map(|c| &**c),
                            )
                        }
                        DependencyEvaluationCause::InconsistentAspectOrder(cause) => {
                            env.listener().handle(&Event::error_at(
                                event_location(Some(cause.get_location())),
                                cause.get_message(),
                            ));
                            AspectCreationException::for_target(
                                cause.get_message(),
                                key.get_label(),
                                configuration.map(|c| &**c),
                            )
                        }
                    },
                )))
            }
            Err(Caught::AspectCreation(e)) => {
                Err(throw(AspectFunctionException::AspectCreation(e)))
            }
            Err(Caught::ConfiguredValueCreation(e)) => {
                Err(throw(AspectFunctionException::ConfiguredValueCreation(e)))
            }
            Err(Caught::Toolchain(e)) => Err(throw(AspectFunctionException::AspectCreation(
                AspectCreationException::of_label_cause(
                    e.get_message(),
                    Cause::Label {
                        label: key.get_label().clone(),
                        detailed_exit_code: toolchain_exception_detailed_exit_code(&e),
                    },
                ),
            ))),
            Err(Caught::InvalidExecGroup(message)) => {
                Err(throw(AspectFunctionException::AspectCreation(
                    AspectCreationException::of_label_cause(
                        message.clone(),
                        Cause::Label {
                            label: key.get_label().clone(),
                            // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:240 ExecGroupCollection.InvalidExecGroupException.getDetailedExitCode
                            detailed_exit_code: create_aspect_detailed_exit_code(
                                &message,
                                "EXEC_GROUP_MISSING",
                            ),
                        },
                    ),
                )))
            }
            Err(Caught::Thrown(e)) => Err(throw(e)),
            Err(Caught::Compute(e)) => Err(e),
        }
    }

    /// The `try` block of `compute`.
    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:385 AspectFunction.compute
    fn compute_after_aspect_path(
        &self,
        env: &mut Environment<'_>,
        state: &mut DependencyResolverState,
        key: &AspectKey,
        aspect: &Aspect,
        aspect_factory: &Arc<dyn ConfiguredAspectFactory>,
        target_and_configuration: &TargetAndConfiguration,
        base_package: &Arc<Package>,
        associated_target: &ConfiguredTarget,
        topological_aspect_path: &[Aspect],
    ) -> Result<Option<Arc<AspectValue>>, Caught> {
        let target = target_and_configuration.get_target();
        let configuration = target_and_configuration.get_configuration();

        let Some(dependency_context) = get_dependency_context(state, key, aspect, env)? else {
            return Ok(None);
        };

        let mut base_target_unloaded_toolchain_contexts: Option<
            Arc<ToolchainCollection<UnloadedToolchainContext>>,
        > = None;
        if target.is_rule() {
            match self.get_base_target_unloaded_toolchain_contexts(
                target_and_configuration,
                key.get_base_configured_target_key(),
                env,
            )? {
                // Need Skyframe deps.
                None => return Ok(None),
                Some(contexts) => base_target_unloaded_toolchain_contexts = contexts,
            }
        }

        // Java requests the exec transition's .bzl through BZL_LOAD and returns null for a
        // restart (see `load_starlark_exec_transition_provider`).
        let starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>> =
            match load_starlark_exec_transition_provider(
                configuration.map(|c| &**c.get_options()),
                &self.services,
                env,
            ) {
                Ok(Some(t)) => t,
                Ok(None) => return Ok(None),
                Err(e) => {
                    return Err(Caught::AspectCreation(AspectCreationException::for_target(
                        e.0.clone(),
                        key.get_label(),
                        configuration.map(|c| &**c),
                    )));
                }
            };

        let dep_value_map = compute_dependencies(
            state,
            &configured_target_key_from_configured_target(associated_target),
            topological_aspect_path,
            // Relevant for exec-config deps of the aspect itself. May need to pass the actual
            // key if there's a use case for an aspect to be applied to exec-config deps of an
            // aspect.
            /* load_exec_aspects_key= */
            None,
            &self.services,
            starlark_exec_transition.clone(),
            env,
            Some(self.base_target_prerequisites_supplier()),
            base_target_unloaded_toolchain_contexts.as_deref(),
        )?;
        if state.transitive_state.has_root_cause() {
            let causes: NestedSet<Cause> = state.transitive_root_causes();
            let detailed_exit_code = get_prioritized_detailed_exit_code(&causes)
                .expect("NullPointerException: detailed exit code of root causes");
            return Err(Caught::Thrown(AspectFunctionException::AspectCreation(
                AspectCreationException::new("Loading failed", causes, detailed_exit_code),
            )));
        }
        let Some(dep_value_map) = dep_value_map else {
            return Ok(None);
        };

        // Load the requested toolchains into the ToolchainContext, now that we have dependencies.
        let unloaded_toolchain_contexts = dependency_context.unloaded_toolchain_contexts.clone();
        let mut toolchain_contexts: Option<ToolchainCollection<ResolvedToolchainContext>> = None;
        if let Some(unloaded_toolchain_contexts) = &unloaded_toolchain_contexts {
            let target_description = format!(
                "aspect {} applied to {}",
                aspect.get_descriptor().get_description(),
                target
            );
            let mut contexts_builder = ToolchainCollection::builder();
            for (exec_group, unloaded_context) in unloaded_toolchain_contexts.context_map() {
                // ImmutableSet.copyOf: identity-distinct values, in order.
                let toolchain_dependencies: Vec<ConfiguredTargetAndData> = dep_value_map
                    .get(&DependencyKind::for_exec_group(exec_group))
                    .cloned()
                    .unwrap_or_default();
                contexts_builder.add_context(
                    exec_group,
                    ResolvedToolchainContext::load(
                        unloaded_context,
                        &target_description,
                        &toolchain_dependencies,
                    )
                    .map_err(Caught::Toolchain)?,
                );
            }
            toolchain_contexts = Some(contexts_builder.build());
        }
        let base_target_toolchain_contexts = match get_base_target_toolchain_contexts(
            base_target_unloaded_toolchain_contexts.as_deref(),
            aspect,
            target,
            &dep_value_map,
        ) {
            Ok(c) => c,
            Err(message) => {
                env.listener().handle(&Event::error_at(
                    event_location(Some(&target.get_location())),
                    message.clone(),
                ));
                return Err(Caught::Thrown(AspectFunctionException::AspectCreation(
                    AspectCreationException::for_target(
                        message,
                        target.get_label(),
                        configuration.map(|c| &**c),
                    ),
                )));
            }
        };
        let exec_group_collection_builder = state
            .exec_group_collection_builder
            .clone()
            .unwrap_or_else(UnloadedToolchainContextsInputs::empty);
        self.create_aspect(
            env,
            key,
            topological_aspect_path,
            aspect,
            aspect_factory,
            target,
            base_package,
            associated_target,
            configuration,
            &dependency_context.config_conditions,
            toolchain_contexts,
            base_target_toolchain_contexts,
            &exec_group_collection_builder,
            &dep_value_map,
            &state.transitive_state,
            starlark_exec_transition,
        )
    }

    /// Returns the `ToolchainCollection` of `UnloadedToolchainContext`s for the base target:
    /// `None` when a Skyframe restart is needed, `Some(None)` when the base target doesn't
    /// require toolchain resolution.
    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:576 AspectFunction.getBaseTargetUnloadedToolchainContexts
    fn get_base_target_unloaded_toolchain_contexts(
        &self,
        target_and_configuration: &TargetAndConfiguration,
        configured_target_key: &ConfiguredTargetKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<Option<Arc<ToolchainCollection<UnloadedToolchainContext>>>>, Caught> {
        // initiate evaluating the base target's toolchain contexts.
        let unloaded_toolchain_contexts_inputs = get_unloaded_toolchain_contexts_inputs(
            target_and_configuration,
            configured_target_key.get_execution_platform_label(),
            &self.services,
            env,
        )
        .map_err(Caught::InvalidExecGroup)?;
        let supplier = self.base_target_prerequisites_supplier();
        let lookup = |key: &ToolchainContextKey| supplier.get_unloaded_toolchain_context(key);
        match produce_unloaded_toolchain_contexts_with_supplier(
            env,
            &unloaded_toolchain_contexts_inputs,
            Some(&lookup),
        )? {
            // Skyframe restart is needed
            None => Ok(None),
            Some(Err(error)) => Err(Caught::Toolchain(error)),
            // base target's toolchain contexts are evaluated in this iteration without requiring
            // a Skyframe restart.
            Some(Ok(contexts)) => Ok(Some(contexts)),
        }
    }

    /// Computes the given aspectKey of an alias-like target, by depending on the corresponding
    /// key of the next target in the alias chain (if there are more), or the "real" configured
    /// target.
    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:810 AspectFunction.createAliasAspect
    fn create_alias_aspect_of_target(
        &self,
        env: &mut Environment<'_>,
        target_and_configuration: &TargetAndConfiguration,
        aspect: &Aspect,
        original_key: &AspectKey,
        base_configured_target: &ConfiguredTarget,
        transitive_state: &TransitiveDependencyState,
    ) -> Result<Option<Arc<AspectValue>>, ComputeError> {
        let alias_chain = base_configured_target
            .get_provider::<AliasProvider>()
            .expect("NullPointerException: AliasProvider")
            .get_alias_chain()
            .to_vec();

        // Java: ConfiguredTarget.getActualNoFollow()
        let next_target = match base_configured_target {
            ConfiguredTarget::Alias(a) => a.get_actual_no_follow().clone(),
            other => other.clone(),
        };

        if alias_chain.len() > 1 {
            blaze_util::check_state!(alias_chain[1] == next_target.get_original_label());
        }

        let actual_key = build_alias_aspect_key(
            original_key,
            &next_target.get_original_label(),
            next_target.get_configuration_key(),
        );

        self.create_alias_aspect(
            env,
            target_and_configuration.get_target(),
            original_key,
            aspect,
            actual_key,
            transitive_state,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:836 AspectFunction.createAliasAspect
    fn create_alias_aspect(
        &self,
        env: &mut Environment<'_>,
        original_target: &Target,
        original_key: &AspectKey,
        aspect: &Aspect,
        dep_key: AspectKey,
        transitive_state: &TransitiveDependencyState,
    ) -> Result<Option<Arc<AspectValue>>, ComputeError> {
        // Compute the AspectValue of the target the alias refers to (which can itself be either
        // an alias or a real target)
        let real = env.get_value(dep_key)?;
        if env.values_missing() {
            return Ok(None);
        }
        let real = real.expect("values not missing");

        let transitive_packages = if self.store_transitive_packages {
            let mut builder = NestedSetBuilder::stable_order();
            let metadata = original_target.get_package_metadata();
            builder.add(
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata::new(
                    metadata.package_identifier.clone(),
                    metadata.repository_mapping.clone(),
                ),
            );
            if let Some(p) = transitive_state.transitive_packages() {
                builder.add_transitive(&p);
            }
            if let Some(p) = real.get_transitive_packages() {
                builder.add_transitive(p);
            }
            Some(builder.build())
        } else {
            None
        };

        ANALYSIS_PROGRESS.done_configure_aspect();
        Ok(Some(Arc::new(AspectValue::create_for_alias(
            original_key.clone(),
            aspect.clone(),
            &ConfiguredAspect::for_alias(&real.get_configured_object()),
            transitive_packages,
        ))))
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:883 AspectFunction.createAspect
    fn create_aspect(
        &self,
        env: &mut Environment<'_>,
        key: &AspectKey,
        topological_aspect_path: &[Aspect],
        aspect: &Aspect,
        aspect_factory: &Arc<dyn ConfiguredAspectFactory>,
        associated_target: &Target,
        package: &Arc<Package>,
        associated_configured_target: &ConfiguredTarget,
        configuration: Option<&Arc<BuildConfigurationValue>>,
        config_conditions: &ConfigConditions,
        toolchain_contexts: Option<ToolchainCollection<ResolvedToolchainContext>>,
        base_target_toolchain_contexts: Option<
            ToolchainCollection<AspectBaseTargetResolvedToolchainContext>,
        >,
        exec_group_collection_builder: &UnloadedToolchainContextsInputs,
        direct_deps: &PrerequisiteMap,
        transitive_state: &TransitiveDependencyState,
        starlark_exec_transition: Option<Arc<StarlarkAttributeTransitionProvider>>,
    ) -> Result<Option<Arc<AspectValue>>, Caught> {
        let services = &*self.services;
        // Should be successfully evaluated and cached from the loading phase.
        let starlark_builtins_value = env.get_value(StarlarkBuiltinsKey::key(false))?;
        if env.values_missing() {
            return Ok(None);
        }
        let starlark_builtins_value = starlark_builtins_value.expect("values not missing");

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
            ActionLookupKeyRef::new(key.clone()),
            extended_sanity_checks,
            allow_analysis_failures,
            events.clone(),
            starlark_builtins_value.starlark_semantics.clone(),
            Some(Arc::new(starlark_defined_builtins)),
            services.main_repo_mapping.read().clone(),
            // PORT: Java requests WorkspaceStatusValue.BUILD_INFO_KEY on demand.
            Some(services.get_workspace_status_artifacts().clone()),
            Some(package.clone()),
        ));

        let configured_aspect: Option<ConfiguredAspect>;
        if aspect.get_definition().apply_to_generating_rules() {
            if let Target::OutputFile(output_file) = associated_target {
                let label = output_file.get_generating_rule().get_label().clone();
                return self
                    .create_alias_aspect(
                        env,
                        associated_target,
                        key,
                        aspect,
                        key.with_label(&label),
                        transitive_state,
                    )
                    .map_err(Caught::Compute);
            }
        }
        {
            // PORT: CurrentRuleTracker (memory instrumentation) is skipped.
            let factory = ConfiguredTargetFactory::new(services.rule_class_provider.clone());
            // Java: RequiredFragmentsUtil.getAspectRequiredFragmentsIfEnabled (called by
            // ConfiguredTargetFactory.createAspect): null unless
            // --include_config_fragments_provider.
            let required_config_fragments = match (core_options, configuration) {
                (Some(o), Some(aspect_configuration))
                    if o.include_required_config_fragments_provider
                        != blaze_optdefs::fragment::core_options::CoreOptionsIncludeConfigFragmentsEnum::OFF =>
                {
                    // Java: ruleClassProvider.getFragmentRegistry().getUniversalFragments()
                    let universal_fragments = blaze_build_config::fragment::FragmentClassSet::of(
                        blaze_build_config::fragments::bazel_fragments().1,
                    );
                    let universal_fragments: Vec<_> = universal_fragments.iter().collect();
                    blaze_analysis::required_fragments_util::get_aspect_required_fragments_if_enabled(
                        aspect,
                        &**aspect_factory,
                        associated_target
                            .get_associated_rule()
                            .expect("NullPointerException: associatedTarget.getAssociatedRule()"),
                        aspect_configuration,
                        &universal_fragments,
                        config_conditions,
                        direct_deps
                            .values()
                            .flatten()
                            .map(|d| d.get_configured_target())
                            .chain(std::iter::once(associated_configured_target)),
                        starlark_exec_transition.clone(),
                    )
                }
                _ => None,
            };
            // PORT: Java's createAspect also takes the Starlark exec transition; W4's port does
            // not (reported), so it is not passed.
            let _ = &starlark_exec_transition;
            match factory.create_aspect(
                &analysis_environment,
                associated_target,
                associated_configured_target,
                topological_aspect_path.to_vec(),
                &**aspect_factory,
                aspect,
                direct_deps,
                config_conditions,
                toolchain_contexts.map(Arc::new),
                base_target_toolchain_contexts.map(Arc::new),
                Some(
                    exec_group_collection_builder
                        .as_exec_group_collection_builder()
                        .clone(),
                ),
                configuration.expect("NullPointerException: aspect configuration"),
                transitive_state.transitive_packages(),
                key,
                required_config_fragments,
            ) {
                Ok(ca) => {
                    // Java: CachingAnalysisEnvironment.getWorkspaceStatusValue's
                    // env.getValue(WorkspaceStatusValue.BUILD_INFO_KEY) (see
                    // configured_target_function.rs).
                    if analysis_environment.was_workspace_status_requested()
                        && env
                            .get_value(blaze_analysis::analysis_services::WorkspaceStatusKey)?
                            .is_none()
                    {
                        return Ok(None);
                    }
                    configured_aspect = ca;
                }
                Err(ConfiguredTargetFactoryError::Create(
                    CreateConfiguredTargetError::ActionConflict(e),
                )) => {
                    return Err(Caught::Thrown(AspectFunctionException::ActionConflict(e)));
                }
                Err(ConfiguredTargetFactoryError::Create(
                    CreateConfiguredTargetError::RuleError(e),
                )) => {
                    return Err(Caught::Thrown(AspectFunctionException::RuleError(
                        e.to_string(),
                    )));
                }
                Err(ConfiguredTargetFactoryError::InvalidExecGroup(e)) => {
                    return Err(Caught::Thrown(AspectFunctionException::InvalidExecGroup(
                        e.get_message().to_owned(),
                    )));
                }
                Err(ConfiguredTargetFactoryError::Create(
                    CreateConfiguredTargetError::Interrupted,
                )) => {
                    return Err(Caught::Compute(blaze_skyframe::Interrupted.into()));
                }
                Err(ConfiguredTargetFactoryError::AnalysisFailurePropagation { .. }) => {
                    panic!(
                        "IllegalStateException: AnalysisFailurePropagationException from \
                         ConfiguredTargetFactory.createAspect"
                    )
                }
            }
        }

        events.replay_on(env.listener());
        if events.has_errors() {
            analysis_environment.disable(associated_target);
            let msg = format!(
                "Analysis of target '{}' (config: {}) failed",
                associated_target.get_label(),
                match configuration {
                    Some(c) => c.get_options().short_id(),
                    None => "none".to_owned(),
                }
            );
            return Err(Caught::Thrown(AspectFunctionException::AspectCreation(
                AspectCreationException::for_target(
                    msg,
                    key.get_label(),
                    configuration.map(|c| &**c),
                ),
            )));
        }
        blaze_util::check_state!(
            !analysis_environment.has_errors(),
            "Analysis environment hasError() but no errors reported"
        );

        if env.values_missing() {
            return Ok(None);
        }

        analysis_environment.disable(associated_target);
        let configured_aspect =
            configured_aspect.unwrap_or_else(|| panic!("NullPointerException: configuredAspect"));

        if !configured_aspect.is_non_applicable() {
            ANALYSIS_PROGRESS.done_configure_aspect();
        }
        Ok(Some(Arc::new(AspectValue::create(
            key.clone(),
            aspect.clone(),
            &configured_aspect,
            transitive_state.transitive_packages(),
        ))))
    }
}

/// Returns the `ToolchainCollection` of `AspectBaseTargetResolvedToolchainContext`s for the
/// base target. `Err` carries a `MergingException`'s message.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:519 AspectFunction.getBaseTargetToolchainContexts
fn get_base_target_toolchain_contexts(
    base_target_unloaded_toolchain_contexts: Option<&ToolchainCollection<UnloadedToolchainContext>>,
    aspect: &Aspect,
    target: &Target,
    dep_value_map: &PrerequisiteMap,
) -> Result<Option<ToolchainCollection<AspectBaseTargetResolvedToolchainContext>>, String> {
    let Some(base_target_unloaded_toolchain_contexts) = base_target_unloaded_toolchain_contexts
    else {
        return Ok(None);
    };
    let description = format!(
        "aspect {} applied to {}",
        aspect.get_descriptor().get_description(),
        target
    );

    let mut target_contexts_builder = ToolchainCollection::builder();

    for (exec_group, unloaded_context) in base_target_unloaded_toolchain_contexts.context_map() {
        // For each requested toolchain type, collect the targets of its resolved toolchains. If
        // multiple types are resolved to the same toolchain, the `ConfiguredTargetAndData` of
        // the toolchain can be different for each of them depending on the aspects propagating
        // to each toolchain type.
        let mut toolchains_deps: IndexMap<ToolchainTypeInfo, Vec<ConfiguredTargetAndData>> =
            IndexMap::new();

        // `toolchainTypeToResolved().keySet()`: the distinct types in first-insertion order.
        let mut toolchain_type_infos: Vec<&ToolchainTypeInfo> = Vec::new();
        for (toolchain_type_info, _) in unloaded_context.toolchain_type_to_resolved() {
            if !toolchain_type_infos.contains(&toolchain_type_info) {
                toolchain_type_infos.push(toolchain_type_info);
            }
        }
        for toolchain_type_info in toolchain_type_infos {
            // ImmutableMultimap.Builder.putAll(key, emptySet) adds no entry.
            if let Some(deps) = dep_value_map.get(&DependencyKind::for_base_target_exec_group(
                exec_group,
                toolchain_type_info.type_label().clone(),
            )) {
                if !deps.is_empty() {
                    toolchains_deps
                        .entry(toolchain_type_info.clone())
                        .or_default()
                        .extend(deps.iter().cloned());
                }
            }
        }

        target_contexts_builder.add_context(
            exec_group,
            AspectBaseTargetResolvedToolchainContext::load(
                unloaded_context,
                &description,
                &toolchains_deps,
            )
            .map_err(|e| e.get_message().to_owned())?,
        );
    }

    Ok(Some(target_contexts_builder.build()))
}

/// Populates `state.exec_group_collection_builder` as a side effect. `None` if a Skyframe
/// restart is needed.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:623 AspectFunction.getDependencyContext
fn get_dependency_context(
    state: &mut DependencyResolverState,
    key: &AspectKey,
    aspect: &Aspect,
    env: &mut Environment<'_>,
) -> Result<Option<DependencyContext>, Caught> {
    if let Some(c) = &state.dependency_context {
        return Ok(Some(c.clone()));
    }
    let target_and_configuration = state
        .target_and_configuration
        .clone()
        .expect("targetAndConfiguration");
    let unloaded_toolchain_contexts_inputs = get_unloaded_toolchain_contexts_inputs_of_aspect(
        aspect.get_definition(),
        key.get_configuration_key(),
        target_and_configuration.get_configuration().map(|c| &**c),
    );
    state.exec_group_collection_builder = Some(unloaded_toolchain_contexts_inputs.clone());
    let result = produce_dependency_context(
        env,
        &unloaded_toolchain_contexts_inputs,
        &target_and_configuration,
        key.get_configuration_key(),
        &mut state.transitive_state,
    )?;

    // During error bubbling, the state machine might not be done, but still emit an error.
    match result {
        // Not yet done.
        None => Ok(None),
        Some(Ok(c)) => {
            state.dependency_context = Some(c.clone());
            Ok(Some(c))
        }
        Some(Err(error)) => match error {
            DependencyContextError::Toolchain(e) => Err(Caught::Toolchain(e)),
            DependencyContextError::ConfiguredValueCreation(e) => {
                Err(Caught::ConfiguredValueCreation(e))
            }
            DependencyContextError::IncompatibleTarget(t) => {
                panic!("IllegalStateException: Unexpected error: {:?}", t)
            }
            DependencyContextError::Validation(v) => {
                panic!("IllegalStateException: Unexpected error: {:?}", v)
            }
        },
    }
}

/// The `BzlLoadValue.Key` of the .bzl defining a Starlark aspect.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:672 AspectFunction.bzlLoadKeyForStarlarkAspect
pub fn bzl_load_key_for_starlark_aspect(starlark_aspect_class: &StarlarkAspectClass) -> BzlLoadKey {
    let extension_label = starlark_aspect_class.get_extension_label();
    if is_builtins_repo(extension_label.get_repository()) {
        BzlLoadKey::key_for_builtins(extension_label.clone())
    } else {
        BzlLoadKey::key_for_build(extension_label.clone())
    }
}

/// `None` if a Skyframe restart is needed.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:680 AspectFunction.getInitialValues
fn get_initial_values(
    state: &mut DependencyResolverState,
    key: &AspectKey,
    env: &mut Environment<'_>,
    rule_class_provider: &ConfiguredRuleClassProvider,
) -> Result<Option<InitialValues>, ComputeError> {
    let configured_target_lookup_key = key.get_base_configured_target_key().clone();
    let base_package_key = PackageKey(
        key.get_base_configured_target_key()
            .get_label()
            .get_package_identifier()
            .clone(),
    );
    let mut initial_keys: Vec<SkyKey> = vec![
        SkyKey::new(configured_target_lookup_key.clone()),
        SkyKey::new(base_package_key.clone()),
    ];

    let configuration_key: Option<BuildConfigurationKey> = key.get_configuration_key().cloned();
    if let Some(configuration_key) = &configuration_key {
        initial_keys.push(SkyKey::new(configuration_key.clone()));
    }

    let starlark_aspect_class: Option<StarlarkAspectClass>;
    let bzl_load_key: Option<BzlLoadKey>;
    match key.get_aspect_class() {
        AspectClass::Native(_) => {
            starlark_aspect_class = None;
            bzl_load_key = None;
        }
        AspectClass::Starlark(c) => {
            let k = bzl_load_key_for_starlark_aspect(c);
            initial_keys.push(SkyKey::new(k.clone()));
            starlark_aspect_class = Some(c.clone());
            bzl_load_key = Some(k);
        }
    }

    let initial_values = env.get_values_and_exceptions(&initial_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let base_configured_target =
        match initial_values.get_or_throw_with(&configured_target_lookup_key, |e| {
            e.downcast_ref::<ConfiguredTargetFunctionException>()
                .and_then(|e| e.as_configured_value_creation().cloned())
        }) {
            // Assigned target might not be configured yet, in which case Skyframe restart is needed.
            Ok(None) => return Ok(None),
            Ok(Some(v)) => v.get_configured_target().clone(),
            Err(e) => {
                return Err(throw(AspectFunctionException::AspectCreation(
                    AspectCreationException::new(
                        e.get_message(),
                        e.get_root_causes().clone(),
                        e.get_detailed_exit_code().clone(),
                    ),
                )));
            }
        };
    blaze_util::check_state!(
        key.get_configuration_key().cloned() == base_configured_target.get_configuration_key(),
        "Aspect not in same configuration as base configured target: {:?}, {:?}",
        key,
        base_configured_target
    );

    // Keep this in sync with the same code in ConfiguredTargetFunction.
    let base_package = initial_values
        .get_typed(&base_package_key)
        .expect("NullPointerException: base package")
        .get_package()
        .clone();
    if base_package.contains_errors() {
        return Err(throw(AspectFunctionException::NoSuchThing(
            NoSuchThingException::Package(NoSuchPackageException::build_file_contains_errors(
                key.get_label().get_package_identifier().clone(),
            )),
        )));
    }
    let target =
        match base_package.get_target(base_configured_target.get_original_label().get_name()) {
            Ok(t) => t.clone(),
            Err(e) => panic!("IllegalStateException: Name already verified: {}", e),
        };

    let configuration: Option<Arc<BuildConfigurationValue>> = match &configuration_key {
        None => None,
        Some(k) => initial_values.get_typed(k),
    };

    state.target_and_configuration = Some(TargetAndConfiguration::new(target, configuration));

    let aspect_factory: Arc<dyn ConfiguredAspectFactory>;
    let aspect: Aspect;
    match bzl_load_key {
        None => {
            let AspectClass::Native(native_aspect_class) = key.get_aspect_class() else {
                unreachable!("native aspect class");
            };
            aspect_factory = native_aspect_factory(rule_class_provider, native_aspect_class);
            aspect = Aspect::for_native(native_aspect_class, key.get_parameters().clone());
        }
        Some(bzl_load_key) => {
            let starlark_aspect_class = starlark_aspect_class.expect("Starlark aspect class");
            let starlark_aspect =
                (|| -> Result<Option<OwnedFrozenValue>, AspectCreationException> {
                    let bzl_load_value = match initial_values
                        .get_or_throw_typed::<BzlLoadKey, BzlLoadFailedException>(&bzl_load_key)
                    {
                        Ok(Some(v)) => v,
                        Ok(None) => {
                            // PORT: BugReport.logUnexpected("Unexpected exception with %s and
                            // AspectKey %s", bzlLoadKey, key) is not ported.
                            return Ok(None);
                        }
                        Err(e) => {
                            return Err(AspectCreationException::for_label_with_exit_code(
                                e.get_message(),
                                starlark_aspect_class.get_extension_label(),
                                e.get_detailed_exit_code().clone(),
                            ));
                        }
                    };
                    load_aspect_from_bzl(&starlark_aspect_class, &bzl_load_value).map(Some)
                })();
            let starlark_aspect = match starlark_aspect {
                Ok(Some(a)) => a,
                Ok(None) => return Ok(None),
                Err(e) => {
                    env.listener().handle(&Event::error(e.get_message()));
                    return Err(throw(AspectFunctionException::AspectCreation(e)));
                }
            };
            let starlark_aspect_data =
                aspect_data(starlark_aspect.value()).expect("checked by loadAspectFromBzl");
            aspect = Aspect::for_starlark(
                starlark_aspect_data.get_aspect_class(),
                starlark_aspect_data.get_definition(key.get_parameters()),
                key.get_parameters().clone(),
            );
            aspect_factory = Arc::new(StarlarkAspectFactory::new(starlark_aspect_data));
        }
    }

    Ok(Some(InitialValues {
        aspect,
        aspect_factory,
        base_configured_target,
        base_package,
    }))
}

/// Java `(ConfiguredAspectFactory) nativeAspectClass`: a native aspect class is its own factory.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:744 AspectFunction.getInitialValues
fn native_aspect_factory(
    rule_class_provider: &ConfiguredRuleClassProvider,
    native_aspect_class: &Arc<dyn NativeAspectClass>,
) -> Arc<dyn ConfiguredAspectFactory> {
    // Java: aspectFactory = (ConfiguredAspectFactory) nativeAspectClass
    rule_class_provider
        .get_native_aspect_factory(native_aspect_class.as_ref())
        .unwrap_or_else(|| {
            panic!(
                "ClassCastException: {} is not a registered ConfiguredAspectFactory",
                native_aspect_class.get_name()
            )
        })
}

/// Loads a Starlark-defined aspect from an extension file: the frozen `StarlarkDefinedAspect`
/// value.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:767 AspectFunction.loadAspectFromBzl
pub fn load_aspect_from_bzl(
    starlark_aspect_class: &StarlarkAspectClass,
    bzl_load_value: &BzlLoadValue,
) -> Result<OwnedFrozenValue, AspectCreationException> {
    let extension_label = starlark_aspect_class.get_extension_label();
    let starlark_value_name = starlark_aspect_class.get_exported_name();
    // Java `Module.getGlobal(name)`: any global, exported or not; a name the file only binds by
    // `load` is not a global.
    let starlark_value: Option<OwnedFrozenValue> = bzl_load_value
        .get_module()
        .java_get_global(starlark_value_name);
    let found = starlark_value.is_some();
    match starlark_value.filter(|v| is_starlark_aspect(v.value())) {
        Some(aspect) => Ok(aspect),
        None => Err(AspectCreationException::for_label(
            if !found {
                format!(
                    "{} is not exported from {}",
                    starlark_value_name, extension_label
                )
            } else {
                format!(
                    "{} from {} is not an aspect",
                    starlark_value_name, extension_label
                )
            },
            extension_label,
        )),
    }
}

// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:785 AspectFunction.getUnloadedToolchainContextsInputs
fn get_unloaded_toolchain_contexts_inputs_of_aspect(
    aspect_definition: &AspectDefinition,
    configuration_key: Option<&BuildConfigurationKey>,
    configuration: Option<&BuildConfigurationValue>,
) -> UnloadedToolchainContextsInputs {
    let Some(configuration) = configuration else {
        // Configuration can be null in the case of aspects applied to input files. In this
        // case, there are no toolchains being used.
        return UnloadedToolchainContextsInputs::empty();
    };

    let use_auto_exec_groups = should_use_auto_exec_groups(aspect_definition, configuration);
    let exec_compatible_with: Vec<Label> = aspect_definition
        .exec_compatible_with()
        .iter()
        .cloned()
        .collect();
    let toolchain_types: Vec<_> = aspect_definition
        .get_toolchain_types()
        .iter()
        .cloned()
        .collect();
    let processed_exec_groups = process_declared_exec_groups(
        aspect_definition.exec_groups(),
        &exec_compatible_with,
        /* exec_group_exec_with= */ &IndexMap::new(),
        &toolchain_types,
        use_auto_exec_groups,
    );
    // Note: `configuration.getOptions().hasNoConfig()` is handled early in #compute.
    UnloadedToolchainContextsInputs::create(
        processed_exec_groups,
        Some(create_default_toolchain_context_key(
            configuration_key
                .expect("NullPointerException: configuration key")
                .clone(),
            exec_compatible_with,
            /* debug_target= */ false,
            /* use_auto_exec_groups= */ use_auto_exec_groups,
            toolchain_types,
            /* parent_execution_platform_label= */ None,
        )),
    )
}

// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:814 AspectFunction.shouldUseAutoExecGroups
fn should_use_auto_exec_groups(
    aspect_definition: &AspectDefinition,
    configuration: &BuildConfigurationValue,
) -> bool {
    // TODO: b/370558813 - Use AutoExecGroupsMode for aspects, as well.
    let aspect_attributes = aspect_definition.get_attributes();
    if let Some(attribute) = aspect_attributes.get("$use_auto_exec_groups") {
        return match attribute.get_default_value_unchecked() {
            Some(AttrValue::Bool(b)) => *b,
            other => panic!(
                "ClassCastException: $use_auto_exec_groups default {:?}",
                other
            ),
        };
    }
    blaze_analysis::rule::config_util::use_auto_exec_groups(configuration)
}

/// Collects `AspectKey` dependencies by performing a postorder traversal over
/// `AspectKey::get_base_keys`.
///
/// The resulting set of `ordered_keys` is topologically ordered: each aspect key appears after
/// all of its dependencies.
// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:830 AspectFunction.collectAspectKeysInTopologicalOrder
fn collect_aspect_keys_in_topological_order(
    base_keys: &[AspectKey],
    ordered_keys: &mut IndexSet<AspectKey>,
) {
    for key in base_keys {
        if !ordered_keys.contains(key) {
            collect_aspect_keys_in_topological_order(key.get_base_keys(), ordered_keys);
            ordered_keys.insert(key.clone());
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:868 AspectFunction.buildAliasAspectKey
fn build_alias_aspect_key(
    original_key: &AspectKey,
    alias_label: &Label,
    configuration_key: Option<BuildConfigurationKey>,
) -> AspectKey {
    let aliased_base_keys: Vec<AspectKey> = original_key
        .get_base_keys()
        .iter()
        .map(|base_key| build_alias_aspect_key(base_key, alias_label, configuration_key.clone()))
        .collect();
    AspectKeyCreator::create_aspect_key(
        original_key.get_aspect_descriptor().clone(),
        aliased_base_keys,
        ConfiguredTargetKey::builder()
            .set_label(alias_label.clone())
            .set_configuration_key(configuration_key)
            .build(),
    )
}

// Java: com/google/devtools/build/lib/skyframe/AspectFunction.java:1040 AspectFunction.targetSatisfiesAspect
fn target_satisfies_aspect(target: &Target, aspect: &Aspect) -> bool {
    if let Some(rule) = target.as_rule() {
        return rule.satisfies(aspect.get_definition().get_required_providers());
    }

    if aspect.get_definition().apply_to_files()
        || aspect.get_definition().apply_to_generating_rules()
    {
        return true;
    }

    false
}
