//! Port of `skyframe/ToplevelStarlarkAspectFunction.java`: the `TOP_LEVEL_ASPECTS` SkyFunction,
//! which runs the aspects path obtained from top-level aspects on a top-level target.
//!
//! Used for loading top-level aspects, filtering them based on their required providers, and
//! computing the relationship between top-level aspects. At top level, in `BuildView`, two
//! SkyFunctions cannot be invoked one after another, so `BuildView` calls this function to do the
//! work.
//!
//! PORT: Java's `State` (a `SkyKeyComputeState`) lives for one `compute` call here.

use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::aspect_key_creator::{
    AspectKey, LoadAspectsKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey, TopLevelAspectsValue,
};
use blaze_analysis::aspect_resolution_helpers::{
    ComputeAspectCollectionError, compute_aspect_collection,
    compute_aspect_collection_no_aspects_filtering,
};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::rule::action_conflict_exception::ActionConflictException;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Event, StoredEventHandler};
use blaze_packages::aspect::Aspect;
use blaze_packages::exceptions::{DetailedExitCode, NoSuchThingException};
use blaze_packages::target::Target;
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, SkyFunction, SkyFunctionException, SkyKey,
    Transience,
};
use blaze_starlark::Location;
use indexmap::IndexMap;

use crate::load_aspects_function::TopLevelAspectsDetailsBuildFailedException;
use crate::producers::errors::{
    Cause, ConfiguredTargetFunctionException, ConfiguredValueCreationException,
    TransitiveDependencyState, configuration_id_message, configuration_id_of_key,
    create_detailed_exit_code, event_location,
};
use crate::producers::target_and_configuration_producer::{
    IdempotencyState, apply_rule_transition,
};

/// SkyFunction to run the aspects path obtained from top-level aspects on the list of top-level
/// targets.
// Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:83 ToplevelStarlarkAspectFunction
pub struct ToplevelStarlarkAspectFunction {
    services: Arc<AnalysisServices>,
    store_transitive_packages: bool,
}

/// Registers `TOP_LEVEL_ASPECTS`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:867 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder, services: Arc<AnalysisServices>) -> EvaluatorBuilder {
    builder.function(
        TOP_LEVEL_ASPECTS,
        ToplevelStarlarkAspectFunction {
            services,
            // Java: SkyframeExecutor.shouldStoreTransitivePackagesInLoadingAndAnalysis()
            // (`allowExternalRepositories`, true in Bazel).
            store_transitive_packages: true,
        },
    )
}

/// The exceptions `ToplevelStarlarkAspectFunction` throws that are its own (Java
/// `TopLevelStarlarkAspectFunctionException`); `DependencyException` and `ReportedException`
/// are thrown as [`ConfiguredTargetFunctionException`]s, as `ConfiguredTargetFunction` does.
// Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:347 ToplevelStarlarkAspectFunction.TopLevelStarlarkAspectFunctionException
#[derive(Clone, Debug, thiserror::Error)]
pub enum TopLevelStarlarkAspectFunctionException {
    /// `ActionConflictException.withAspectKeyInfo(e, aspectKey)`.
    // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:89 ActionConflictException.withAspectKeyInfo
    #[error("{cause}")]
    ActionConflictWithAspectKeyInfo {
        cause: ActionConflictException,
        aspect_key: AspectKey,
    },
    #[error("{0}")]
    TopLevelAspectsDetailsBuildFailed(TopLevelAspectsDetailsBuildFailedException),
}

fn throw<E: std::error::Error + Send + Sync + 'static>(e: E) -> ComputeError {
    SkyFunctionException::new(e, Transience::Persistent).into()
}

/// State which drives the rule transition of the base target and accepts the configuration when
/// complete (Java `ToplevelStarlarkAspectFunction.State`).
// Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:426 ToplevelStarlarkAspectFunction.State
struct State {
    transitive_state: TransitiveDependencyState,
    stored_events: StoredEventHandler,
    // --------------- Configuration fields ------------------
    configuration_key: Option<BuildConfigurationKey>,
    idempotency_state: Option<IdempotencyState>,
    // --------------- Error handling fields ------------------
    message: Option<String>,
    location: Option<Location>,
    exit_code: Option<DetailedExitCode>,
}

impl State {
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:446 ToplevelStarlarkAspectFunction.State.<init>
    fn new(store_transitive_packages: bool) -> Self {
        State {
            transitive_state: TransitiveDependencyState::new(store_transitive_packages),
            stored_events: StoredEventHandler::new(),
            configuration_key: None,
            idempotency_state: None,
            message: None,
            location: None,
            exit_code: None,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:476 ToplevelStarlarkAspectFunction.State.hasError
    fn has_error(&self) -> bool {
        self.message.is_some() || self.location.is_some() || self.exit_code.is_some()
    }

    /// Handles an exception thrown during the rule transition application in
    /// `RuleTransitionApplier`.
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:484 ToplevelStarlarkAspectFunction.State.createException
    fn create_exception(
        &self,
        base_configured_target_key: &ConfiguredTargetKey,
        target: &Target,
    ) -> ConfiguredValueCreationException {
        let message = self.message.clone().unwrap_or_else(|| "null".to_owned());
        let exit_code = self
            .exit_code
            .clone()
            .unwrap_or_else(|| create_detailed_exit_code(&message));
        let cause = Cause::AnalysisFailed {
            label: base_configured_target_key.get_label().clone(),
            configuration: configuration_id_message(Some(
                base_configured_target_key
                    .get_configuration_key()
                    .expect("NullPointerException: configuration key")
                    .get_options_checksum(),
            )),
            detailed_exit_code: exit_code.clone(),
        };
        ConfiguredValueCreationException::new(
            self.location.clone(),
            message,
            target.get_label(),
            configuration_id_of_key(base_configured_target_key.get_configuration_key()),
            Some(NestedSetBuilder::create(Order::STABLE_ORDER, [cause])),
            Some(exit_code),
        )
    }
}

impl SkyFunction for ToplevelStarlarkAspectFunction {
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:105 ToplevelStarlarkAspectFunction.compute
    fn compute(&self, key: &SkyKey, env: &mut Environment<'_>) -> blaze_skyframe::ComputeResult {
        let top_level_aspects_key = key
            .downcast::<TopLevelAspectsKey>()
            .expect("TOP_LEVEL_ASPECTS key");

        let load_aspects_key = LoadAspectsKey::create(
            top_level_aspects_key
                .get_top_level_aspects_classes()
                .to_vec(),
            top_level_aspects_key
                .get_top_level_aspects_parameters()
                .clone(),
        );
        let package_identifier = PackageKey(
            top_level_aspects_key
                .get_base_configured_target_key()
                .get_label()
                .get_package_identifier()
                .clone(),
        );

        let initial_lookup_result = env.get_values_and_exceptions(&[
            SkyKey::new(load_aspects_key.clone()),
            SkyKey::new(package_identifier.clone()),
        ])?;

        let Some(load_aspects_value) = initial_lookup_result.get_typed(&load_aspects_key) else {
            return Ok(None); // aspects are not ready
        };

        let Some(package_value) = initial_lookup_result.get_typed(&package_identifier) else {
            return Ok(None); // package is not ready
        };
        let target = get_target(
            &package_value,
            top_level_aspects_key
                .get_base_configured_target_key()
                .get_label(),
        )?;

        let mut state = State::new(self.store_transitive_packages);

        // Configuration of top level target could change during the analysis phase with rule
        // transitions. In order not to wait for the complete configuration of the assigned
        // target, `RuleTransitionApplier` is used to apply potentially requested rule
        // transitions upfront. Configuration can be `null` if the target is not configurable,
        // in which case the Skyframe restart is needed.
        let Some(base_configured_target_key) = self.get_configured_target_key(
            &mut state,
            top_level_aspects_key.get_base_configured_target_key(),
            &target,
            env,
        )?
        else {
            return Ok(None);
        };

        let Some(aspects_keys) = create_aspects_keys(
            target,
            load_aspects_value.get_aspects(),
            &base_configured_target_key,
            env,
        )?
        else {
            return Ok(None); // alias target needs to be resolved
        };

        let sky_keys: Vec<SkyKey> = aspects_keys
            .iter()
            .map(|k| SkyKey::new(k.clone()))
            .collect();
        let result = env.get_values_and_exceptions(&sky_keys)?;
        if env.values_missing() {
            return Ok(None); // some aspects keys are not evaluated
        }
        let mut values_map: IndexMap<AspectKey, Arc<AspectValue>> =
            IndexMap::with_capacity(aspects_keys.len());
        for aspect_key in aspects_keys {
            match result.get_or_throw_with(&aspect_key, |e| {
                match e.downcast_ref::<crate::aspect_function::AspectFunctionException>()? {
                    crate::aspect_function::AspectFunctionException::ActionConflict(c) => {
                        Some(c.clone())
                    }
                    _ => None,
                }
            }) {
                Ok(None) => return Ok(None),
                Ok(Some(value)) => {
                    // ImmutableMap.Builder.buildOrThrow on a duplicate key.
                    if values_map.insert(aspect_key.clone(), value).is_some() {
                        panic!(
                            "IllegalArgumentException: Multiple entries with same key: {:?}",
                            aspect_key
                        );
                    }
                }
                Err(e) => {
                    // Required in case of skymeld: the AspectKey isn't accessible from the
                    // BuildDriverKey.
                    return Err(throw(
                        TopLevelStarlarkAspectFunctionException::ActionConflictWithAspectKeyInfo {
                            cause: e,
                            aspect_key,
                        },
                    ));
                }
            }
        }
        Ok(Some(
            Arc::new(TopLevelAspectsValue::new(values_map)) as blaze_skyframe::SkyValue
        ))
    }
}

// Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:179 ToplevelStarlarkAspectFunction.getTarget
fn get_target(package_value: &PackageValue, target_label: &Label) -> Result<Target, ComputeError> {
    match package_value
        .get_package()
        .get_target(target_label.get_name())
    {
        Ok(t) => Ok(t.clone()),
        // Java: `throw new DependencyException(e)`.
        Err(e) => Err(throw(ConfiguredTargetFunctionException::NoSuchThing(
            NoSuchThingException::Target(e),
        ))),
    }
}

/// `None` if the alias target needs to be resolved.
// Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:189 ToplevelStarlarkAspectFunction.createAspectsKeys
fn create_aspects_keys(
    mut target: Target,
    aspects: &[Aspect],
    base_configured_target_key: &ConfiguredTargetKey,
    env: &mut Environment<'_>,
) -> Result<Option<Vec<AspectKey>>, ComputeError> {
    // In case the target is an alias, we need to resolve its actual target.
    if AliasProvider::may_be_alias(&target) {
        let alias_configured_value = env.get_value(base_configured_target_key.clone())?;
        if env.values_missing() {
            return Ok(None);
        }
        let alias_configured_value = alias_configured_value.expect("values not missing");

        let actual_label = alias_configured_value
            .get_configured_target()
            .get_actual()
            .get_label();
        let package_value =
            env.get_value(PackageKey(actual_label.get_package_identifier().clone()))?;
        if env.values_missing() {
            return Ok(None);
        }
        target = get_target(&package_value.expect("values not missing"), &actual_label)?;
    }

    // TODO(bazel-team): Filter aspects more based on rule type. For example, aspect key should
    // not be created for a file target if the aspect does not apply to files or their generating
    // rules. Currently, some tests depend on such keys being created, so they need to be modified
    // first.
    let aspect_collection = match target.as_rule() {
        Some(rule_target) => compute_aspect_collection(
            aspects,
            rule_target.get_advertised_providers(),
            rule_target.get_label(),
            rule_target.get_rule_class_object(),
            &rule_target.get_only_tags_attribute(),
            rule_target.get_location(),
            env.listener(),
        ),
        None => compute_aspect_collection_no_aspects_filtering(
            aspects,
            target.get_label(),
            &target.get_location(),
        )
        .map_err(ComputeAspectCollectionError::InconsistentAspectOrder),
    };
    let aspect_collection = match aspect_collection {
        Ok(c) => c,
        Err(ComputeAspectCollectionError::InconsistentAspectOrder(e)) => {
            // This is very unlikely, because AspectCollection should have deduplicated top level
            // aspects.
            env.listener().handle(&Event::error(e.get_message()));
            return Err(throw(
                TopLevelStarlarkAspectFunctionException::TopLevelAspectsDetailsBuildFailed(
                    TopLevelAspectsDetailsBuildFailedException::new(
                        e.get_message(),
                        "ASPECT_CREATION_FAILED",
                    ),
                ),
            ));
        }
        Err(ComputeAspectCollectionError::Eval(e)) => {
            env.listener().handle(&Event::error(e.message_with_stack()));
            return Err(throw(
                TopLevelStarlarkAspectFunctionException::TopLevelAspectsDetailsBuildFailed(
                    TopLevelAspectsDetailsBuildFailedException::new(
                        e.message.clone(),
                        "ASPECT_CREATION_FAILED",
                    ),
                ),
            ));
        }
    };

    Ok(Some(
        aspect_collection.create_aspect_keys(base_configured_target_key),
    ))
}

impl ToplevelStarlarkAspectFunction {
    /// Returns `base_configured_target_key` if the configuration didn't change with potential
    /// transitions (`IdempotencyState::Identity`). Otherwise, returns a new `ConfiguredTargetKey`
    /// with the new configuration.
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:252 ToplevelStarlarkAspectFunction.createConfiguredTargetKey
    fn create_configured_target_key(
        build_configuration_key: BuildConfigurationKey,
        base_configured_target_key: &ConfiguredTargetKey,
        idempotency_state: IdempotencyState,
    ) -> ConfiguredTargetKey {
        if idempotency_state == IdempotencyState::Identity {
            return base_configured_target_key.clone();
        }
        let mut key_builder = ConfiguredTargetKey::builder()
            .set_label(base_configured_target_key.get_label().clone())
            .set_configuration_key(Some(build_configuration_key));

        if idempotency_state == IdempotencyState::NonIdempotent {
            // The transition was not idempotent. Explicitly informs the delegate to avoid
            // applying a rule transition.
            key_builder = key_builder.set_should_apply_rule_transition(false);
        }
        key_builder.build()
    }

    /// Computes the configuration of the target by driving `RuleTransitionApplier` (Java
    /// `computeConfiguration` with `TransitionedBaseConfigurationProducer`).
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:277 ToplevelStarlarkAspectFunction.computeConfiguration
    fn compute_configuration(
        &self,
        env: &mut Environment<'_>,
        state: &mut State,
        base_configured_target_key: &ConfiguredTargetKey,
        target: &Target,
    ) -> Result<(), ComputeError> {
        match apply_rule_transition(
            env,
            &self.services,
            target,
            base_configured_target_key,
            &mut state.transitive_state,
            &state.stored_events,
        )? {
            None => {}
            // Java: State.acceptConfiguration
            Some(Ok((configuration_key, idempotency_state))) => {
                state.configuration_key = Some(configuration_key);
                state.idempotency_state = Some(idempotency_state);
            }
            // Java: State.acceptErrorMessage
            Some(Err((message, location, exit_code))) => {
                state.message = Some(message);
                state.location = location;
                state.exit_code = exit_code;
            }
        }
        Ok(())
    }

    /// Computes the `BuildConfigurationKey` by driving `RuleTransitionApplier` and returns the
    /// new `ConfiguredTargetKey` with the obtained build configuration. `None` when the
    /// configuration key is not ready (Skyframe restart).
    // Java: com/google/devtools/build/lib/skyframe/ToplevelStarlarkAspectFunction.java:302 ToplevelStarlarkAspectFunction.getConfiguredTargetKey
    fn get_configured_target_key(
        &self,
        state: &mut State,
        base_configured_target_key: &ConfiguredTargetKey,
        target: &Target,
        env: &mut Environment<'_>,
    ) -> Result<Option<ConfiguredTargetKey>, ComputeError> {
        if !target.is_configurable() {
            return Ok(Some(
                base_configured_target_key
                    .to_builder()
                    .set_configuration_key(None)
                    .build(),
            ));
        }

        self.compute_configuration(env, state, base_configured_target_key, target)?;

        if state.has_error() {
            let exception = state.create_exception(base_configured_target_key, target);
            if !exception.get_message().is_empty() {
                // Report the error to the user.
                env.listener().handle(&Event::error_at(
                    event_location(exception.get_location()),
                    exception.get_message(),
                ));
            }
            // Java: `throw new ReportedException(exception)` (the exception without its message).
            return Err(throw(ConfiguredTargetFunctionException::Reported(
                exception.without_message(),
            )));
        }

        let Some(configuration_key) = state.configuration_key.clone() else {
            // Skyframe restart is needed since configuration is still not ready.
            return Ok(None);
        };
        Ok(Some(Self::create_configured_target_key(
            configuration_key,
            base_configured_target_key,
            state
                .idempotency_state
                .expect("idempotency state accepted with the configuration"),
        )))
    }
}
