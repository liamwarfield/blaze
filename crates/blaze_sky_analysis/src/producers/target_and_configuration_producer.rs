//! Ports of `analysis/producers/TargetAndConfigurationProducer.java`,
//! `analysis/producers/TargetProducer.java` and `analysis/producers/RuleTransitionApplier.java`.

use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::config::transitions::{
    ConfigurationTransition, PATCH_TRANSITION_KEY, RuleTransitionData,
    create_rule_transition_with_trimming, patch_toolchain_tagged_trimming,
};
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_analysis::toolchains::UnloadedToolchainContextsInputs;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, StoredEventHandler};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_packages::exceptions::{
    DetailedExitCode, FailureCode, NoSuchPackageException, NoSuchThingException, PackageLoadingCode,
};
use blaze_packages::target::Target;
use blaze_packages::target::target_utils::get_location_maybe;
use blaze_sky_loading::package_function::PackageKey;
use blaze_skyframe::Environment;
use blaze_starlark::Location;

use crate::producers::config_conditions_producer::produce_config_conditions;
use crate::producers::errors::{
    Cause, ConfiguredValueCreationException, InconsistentNullConfigException,
    TargetAndConfiguration, TransitiveDependencyState, configuration_id_message,
    configuration_id_of_key, create_detailed_exit_code, event_location,
};
use crate::producers::transition_applier::{Produced, apply_transition};

/// Tagged union of possible errors (Java `TargetAndConfigurationProducer.TargetAndConfigurationError`).
// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:76 TargetAndConfigurationProducer.TargetAndConfigurationError
#[derive(Clone, Debug)]
pub enum TargetAndConfigurationError {
    ConfiguredValueCreation(ConfiguredValueCreationException),
    NoSuchThing(NoSuchThingException),
    InconsistentNullConfig(InconsistentNullConfigException),
}

/// What `TargetAndConfigurationProducer.ResultSink` receives.
#[derive(Clone, Debug)]
pub enum TargetAndConfigurationResult {
    /// `acceptTargetAndConfiguration(value, fullKey)`.
    TargetAndConfiguration(TargetAndConfiguration, ConfiguredTargetKey),
    /// `acceptTargetAndConfigurationDelegatedValue(value)`.
    DelegatedValue(Arc<ConfiguredTargetValue>),
    /// `acceptTargetAndConfigurationError(error)`.
    Error(TargetAndConfigurationError),
}

/// Classifies a transition based on its behavior (no-op, idempotent, or non-idempotent).
///
/// During analysis, when a rule transition modifies the configuration, it triggers a new
/// evaluation of the target with the new configuration. This re-evaluation can inadvertently
/// lead to the rule transition being applied twice. To prevent this, the `ConfiguredTargetKey`
/// includes a `shouldApplyRuleTransition` flag, set false when the result is `NON_IDEMPOTENT`.
// Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:105 RuleTransitionApplier.IdempotencyState
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdempotencyState {
    /// The transition was a no-op.
    Identity,
    /// The rule transition is idempotent.
    Idempotent,
    /// The rule transition is non-idempotent.
    NonIdempotent,
}

/// Computes the `Target` and configuration of a `ConfiguredTargetKey`, applying the rule
/// transition (Java `TargetAndConfigurationProducer`, driven to completion).
// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:147 TargetAndConfigurationProducer.step
pub fn produce_target_and_configuration(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    pre_rule_transition_key: &ConfiguredTargetKey,
    transitive_state: &mut TransitiveDependencyState,
    event_handler: &StoredEventHandler,
) -> Produced<TargetAndConfigurationResult> {
    let target = match produce_target(
        env,
        pre_rule_transition_key.get_label(),
        transitive_state,
        event_handler,
    )? {
        None => return Ok(None),
        Some(Err(e)) => return Ok(Some(TargetAndConfigurationResult::Error(e))),
        Some(Ok(t)) => t,
    };
    determine_configuration(
        env,
        services,
        pre_rule_transition_key,
        target,
        transitive_state,
        event_handler,
    )
}

// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:193 TargetAndConfigurationProducer.determineConfiguration
fn determine_configuration(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    pre_rule_transition_key: &ConfiguredTargetKey,
    target: Target,
    transitive_state: &mut TransitiveDependencyState,
    event_handler: &StoredEventHandler,
) -> Produced<TargetAndConfigurationResult> {
    let Some(configuration_key) = pre_rule_transition_key.get_configuration_key() else {
        if target.is_configurable() {
            // We somehow ended up in a target that requires a non-null configuration but with a
            // key that doesn't have a configuration. This is always an error, but we need to
            // bubble this up to the parent to provide more context.
            return Ok(Some(TargetAndConfigurationResult::Error(
                TargetAndConfigurationError::InconsistentNullConfig(
                    InconsistentNullConfigException,
                ),
            )));
        }
        return Ok(Some(TargetAndConfigurationResult::TargetAndConfiguration(
            TargetAndConfiguration::new(target, /* configuration= */ None),
            pre_rule_transition_key.clone(),
        )));
    };

    if !target.is_configurable() {
        // If target is not configurable, but requested with a configuration. Delegates to a key
        // with the null configuration. This is expected to be uncommon. The common case of a
        // non-configurable target is an input file, but those are usually package local and
        // requested correctly with the null configuration.
        let delegate = ConfiguredTargetKey::builder()
            .set_label(pre_rule_transition_key.get_label().clone())
            .set_execution_platform_label(
                pre_rule_transition_key
                    .get_execution_platform_label()
                    .cloned(),
            )
            .build();
        return delegate_to(env, delegate);
    }

    if !pre_rule_transition_key.should_apply_rule_transition() {
        return look_up_configuration_value(
            env,
            pre_rule_transition_key,
            configuration_key,
            target,
            pre_rule_transition_key,
        );
    }

    // RuleTransitionApplier, then computeTargetAndConfigurationOrDelegatedValue.
    let (configuration_key, idempotency_state) = match apply_rule_transition(
        env,
        services,
        &target,
        pre_rule_transition_key,
        transitive_state,
        event_handler,
    )? {
        None => return Ok(None),
        Some(Err((message, location, exit_code))) => {
            return Ok(Some(TargetAndConfigurationResult::Error(emit_error(
                pre_rule_transition_key,
                &target,
                &message,
                location,
                exit_code,
            ))));
        }
        Some(Ok(r)) => r,
    };

    // Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:291 TargetAndConfigurationProducer.computeTargetAndConfigurationOrDelegatedValue
    if idempotency_state == IdempotencyState::Identity {
        return look_up_configuration_value(
            env,
            pre_rule_transition_key,
            &configuration_key,
            target,
            pre_rule_transition_key,
        );
    }

    let mut key_builder = ConfiguredTargetKey::builder()
        .set_label(pre_rule_transition_key.get_label().clone())
        .set_execution_platform_label(
            pre_rule_transition_key
                .get_execution_platform_label()
                .cloned(),
        )
        .set_configuration_key(Some(configuration_key));

    if idempotency_state == IdempotencyState::NonIdempotent {
        // The transition was not idempotent. Explicitly informs the delegate to avoid applying a
        // rule transition.
        key_builder = key_builder.set_should_apply_rule_transition(false);
    }

    delegate_to(env, key_builder.build())
}

// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:241 TargetAndConfigurationProducer.delegateTo
fn delegate_to(
    env: &mut Environment<'_>,
    delegate: ConfiguredTargetKey,
) -> Produced<TargetAndConfigurationResult> {
    match env.get_value(delegate)? {
        None => Ok(None),
        Some(value) => Ok(Some(TargetAndConfigurationResult::DelegatedValue(value))),
    }
}

// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:250 TargetAndConfigurationProducer.lookUpConfigurationValue
fn look_up_configuration_value(
    env: &mut Environment<'_>,
    pre_rule_transition_key: &ConfiguredTargetKey,
    configuration_key: &BuildConfigurationKey,
    target: Target,
    full_key: &ConfiguredTargetKey,
) -> Produced<TargetAndConfigurationResult> {
    // Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:258 TargetAndConfigurationProducer.acceptValueOrException
    let result = env.get_value_or_throw_with(configuration_key.clone(), |e| {
        e.downcast_ref::<blaze_build_config::skyframe::ConfigurationError>()
            .and_then(|c| match c {
                blaze_build_config::skyframe::ConfigurationError::InvalidConfiguration(i) => {
                    Some(i.clone())
                }
                _ => None,
            })
    })?;
    match result {
        Ok(Some(value)) => Ok(Some(TargetAndConfigurationResult::TargetAndConfiguration(
            TargetAndConfiguration::new(target, Some(value)),
            full_key.clone(),
        ))),
        Ok(None) => Ok(None),
        Err(error) => {
            let error: InvalidConfigurationException = error;
            let location = get_location_maybe(&target);
            Ok(Some(TargetAndConfigurationResult::Error(emit_error(
                pre_rule_transition_key,
                &target,
                error.get_message(),
                location,
                // Java: com/google/devtools/build/lib/analysis/config/InvalidConfigurationException.java:68 InvalidConfigurationException.getDetailedExitCode
                Some(DetailedExitCode {
                    message: error.get_message().to_owned(),
                    code: FailureCode::Other(format!("BuildConfiguration.{}", error.code)),
                }),
            ))))
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/producers/TargetAndConfigurationProducer.java:313 TargetAndConfigurationProducer.emitError
fn emit_error(
    pre_rule_transition_key: &ConfiguredTargetKey,
    target: &Target,
    message: &str,
    location: Option<Location>,
    exit_code: Option<DetailedExitCode>,
) -> TargetAndConfigurationError {
    let exit_code = exit_code.unwrap_or_else(|| create_detailed_exit_code(message));
    let cause = Cause::AnalysisFailed {
        label: pre_rule_transition_key.get_label().clone(),
        configuration: configuration_id_message(
            pre_rule_transition_key
                .get_configuration_key()
                .map(|k| k.get_options_checksum()),
        ),
        detailed_exit_code: exit_code.clone(),
    };
    TargetAndConfigurationError::ConfiguredValueCreation(ConfiguredValueCreationException::new(
        location,
        message,
        target.get_label(),
        configuration_id_of_key(pre_rule_transition_key.get_configuration_key()),
        Some(NestedSetBuilder::create(Order::STABLE_ORDER, [cause])),
        Some(exit_code),
    ))
}

// -------------------------------------------------------------------------------------------
// TargetProducer.

/// Loads the target of `label` (Java `TargetProducer`). `Ok(Some(Ok(None)))` is never
/// produced; a target error is reported to `event_handler` and returned.
// Java: com/google/devtools/build/lib/analysis/producers/TargetProducer.java:67 TargetProducer.step
pub fn produce_target(
    env: &mut Environment<'_>,
    label: &Label,
    transitive_state: &mut TransitiveDependencyState,
    event_handler: &StoredEventHandler,
) -> Produced<Result<Target, TargetAndConfigurationError>> {
    let pkg = match env.get_value_or_throw::<_, NoSuchPackageException>(PackageKey(
        label.get_package_identifier().clone(),
    ))? {
        Ok(Some(v)) => v.get_package().clone(),
        Ok(None) => return Ok(None),
        Err(error) => {
            // Java: TargetAndConfigurationProducer.acceptTargetError(NoSuchPackageException)
            event_handler.handle(&Event::error(error.get_message()));
            return Ok(Some(Err(TargetAndConfigurationError::NoSuchThing(
                NoSuchThingException::Package(error),
            ))));
        }
    };

    // Java: com/google/devtools/build/lib/analysis/producers/TargetProducer.java:89 TargetProducer.unwrapTarget
    let target = match pkg.get_target(label.get_name()) {
        Ok(t) => t.clone(),
        Err(e) => {
            transitive_state.add_transitive_cause(Cause::LoadingFailed {
                label: label.clone(),
                detailed_exit_code: e.get_detailed_exit_code(),
            });
            // Java: TargetAndConfigurationProducer.acceptTargetError(NoSuchTargetException, Location)
            event_handler.handle(&Event::error_at(
                event_location(Some(&pkg.get_metadata().get_build_file_location())),
                e.get_message(),
            ));
            return Ok(Some(Err(TargetAndConfigurationError::NoSuchThing(
                NoSuchThingException::Target(e),
            ))));
        }
    };

    if pkg.contains_errors() {
        let failure_detail =
            contextualize_failure_detail_for_target(pkg.get_failure_detail(), &target);
        // The target can be loaded but may have associated errors, for example, a missing
        // required attribute. In these cases, instead of failing fast, it's possible to perform
        // dependency resolution using the target-in-error to uncover any other errors that could
        // be present in its dependencies. This error is turned into an exception when the
        // transitive causes are examined after dependency resolution.
        transitive_state.add_transitive_cause(Cause::LoadingFailed {
            label: label.clone(),
            detailed_exit_code: failure_detail,
        });
    }
    transitive_state.update_transitive_packages(pkg.get_metadata());
    Ok(Some(Ok(target)))
}

// Java: com/google/devtools/build/lib/packages/Package.java:554 Package.contextualizeFailureDetailForTarget
fn contextualize_failure_detail_for_target(
    failure_detail: Option<&DetailedExitCode>,
    target: &Target,
) -> DetailedExitCode {
    let prefix = format!(
        "Target '{}' contains an error and its package is in error",
        target.get_label()
    );
    match failure_detail {
        None => DetailedExitCode {
            message: prefix,
            code: FailureCode::PackageLoading(PackageLoadingCode::TargetMissing),
        },
        Some(d) => DetailedExitCode {
            message: format!("{}: {}", prefix, d.message),
            code: d.code.clone(),
        },
    }
}

// -------------------------------------------------------------------------------------------
// RuleTransitionApplier.

/// `acceptErrorMessage(message, location, exitCode)`.
pub type ErrorMessage = (String, Option<Location>, Option<DetailedExitCode>);

/// Applies any requested rule transition before producing the final configuration (Java
/// `RuleTransitionApplier`).
// Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:150 RuleTransitionApplier.step
pub fn apply_rule_transition(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    target: &Target,
    pre_rule_transition_key: &ConfiguredTargetKey,
    transitive_state: &mut TransitiveDependencyState,
    event_handler: &StoredEventHandler,
) -> Produced<Result<(BuildConfigurationKey, IdempotencyState), ErrorMessage>> {
    let emit_error_message = |message: String| -> ErrorMessage {
        (
            message,
            get_location_maybe(target),
            /* exit_code= */ None,
        )
    };
    let configuration_key = pre_rule_transition_key
        .get_configuration_key()
        .expect("configured key");
    let options = configuration_key.get_options();
    let platform_options = options.get::<PlatformOptions>();
    let mut platform_configuration = None;
    let unloaded_toolchain_contexts_inputs = match platform_options {
        None => UnloadedToolchainContextsInputs::empty(),
        Some(platform_options) => {
            let pc = PlatformConfiguration::from_platform_options(platform_options);
            let result =
                crate::toolchains::toolchain_context_util::get_unloaded_toolchain_contexts_inputs(
                    target,
                    options.get::<CoreOptions>().expect("CoreOptions"),
                    Some(&pc),
                    pre_rule_transition_key.get_execution_platform_label(),
                    compute_toolchain_configuration_key(options, services, event_handler),
                );
            platform_configuration = Some(pc);
            match result {
                Ok(i) => i,
                Err(e) => return Ok(Some(Err(emit_error_message(e.to_string())))),
            }
        }
    };

    let platform_info: Option<Arc<PlatformInfo>> = if unloaded_toolchain_contexts_inputs
        .target_toolchain_context_key()
        .is_some()
    {
        let platform_configuration = platform_configuration.expect("platform configuration");
        match crate::platform::platform_producer::produce_platform(
            env,
            platform_configuration.get_target_platform(),
            &options
                .get::<CoreOptions>()
                .expect("CoreOptions")
                .get_command_line_flag_aliases(),
        )? {
            None => return Ok(None),
            Some(Ok(info)) => Some(info),
            Some(Err(e)) => return Ok(Some(Err(emit_error_message(e.to_string())))),
        }
    } else {
        None
    };

    // Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:217 RuleTransitionApplier.computeConfigConditions
    // TODO @aranguyen b/297077082
    let config_conditions: ConfigConditions = match produce_config_conditions(
        env,
        target,
        pre_rule_transition_key.get_label(),
        Some(configuration_key),
        platform_info.as_deref(),
        transitive_state,
    )? {
        None => return Ok(None),
        Some(Err(e)) => return Ok(Some(Err(emit_error_message(e.get_message().to_owned())))),
        Some(Ok(c)) => c,
    };

    // Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:243 RuleTransitionApplier.computeTransition
    // Keep in sync with CqueryTransitionResolver.getRuleTransition.
    let rule = target
        .get_associated_rule()
        .expect("configurable target with a rule transition has a rule")
        .clone();
    let transition_data = RuleTransitionData::create(
        rule.clone(),
        Arc::new(config_conditions.as_providers().clone()),
        configuration_key.get_options_checksum().to_owned(),
    );
    let rule_transition: ConfigurationTransition = create_rule_transition_with_trimming(
        rule.get_rule_class_object().get_transition_factory(),
        &transition_data,
    );
    let configuration_key_after = match apply_and_take_patch(
        env,
        services,
        target.get_label(),
        configuration_key,
        &rule_transition,
        event_handler,
    )? {
        None => return Ok(None),
        Some(Err(message)) => return Ok(Some(Err(emit_error_message(message)))),
        Some(Ok(k)) => k,
    };

    // Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:332 RuleTransitionApplier.processTransitionedKey
    let parent_configuration = configuration_key;
    if configuration_key_after == *parent_configuration {
        // This key owns the configuration and the computation completes normally.
        return Ok(Some(Ok((
            configuration_key_after,
            IdempotencyState::Identity,
        ))));
    }
    event_handler.post(Arc::new(
        blaze_analysis::config::configuration_transition_event::ConfigurationTransitionEvent::create(
            parent_configuration.get_options_checksum(),
            configuration_key_after.get_options_checksum(),
        ),
    ));

    // Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:349 RuleTransitionApplier.IdempotencyChecker
    let configuration_key2 = match apply_and_take_patch(
        env,
        services,
        target.get_label(),
        &configuration_key_after,
        &rule_transition,
        event_handler,
    )? {
        None => return Ok(None),
        Some(Err(message)) => return Ok(Some(Err(emit_error_message(message)))),
        Some(Ok(k)) => k,
    };
    let idempotency_state = if configuration_key_after == configuration_key2 {
        IdempotencyState::Idempotent
    } else {
        IdempotencyState::NonIdempotent
    };
    Ok(Some(Ok((configuration_key_after, idempotency_state))))
}

/// Applies the rule transition and takes the single patch result (Java
/// `acceptTransitionedConfigurations` with its `checkState`s).
fn apply_and_take_patch(
    env: &mut Environment<'_>,
    services: &AnalysisServices,
    label: &Label,
    from: &BuildConfigurationKey,
    transition: &ConfigurationTransition,
    event_handler: &StoredEventHandler,
) -> Produced<Result<BuildConfigurationKey, String>> {
    match apply_transition(env, services, label, from, transition, event_handler)? {
        None => Ok(None),
        Some(Err(e)) => Ok(Some(Err(e.get_message()))),
        Some(Ok(transition_result)) => {
            blaze_util::check_state!(
                transition_result.len() == 1,
                "Expected exactly one result: {:?}",
                transition_result.keys().collect::<Vec<_>>()
            );
            let key = transition_result
                .get(PATCH_TRANSITION_KEY)
                .unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Transition result missing patch transition entry: {:?}",
                        transition_result.keys().collect::<Vec<_>>()
                    )
                })
                .clone();
            Ok(Some(Ok(key)))
        }
    }
}

/// The toolchain context's options are the parent rule's options with manual trimming
/// auto-applied. This means toolchains don't inherit feature flags.
// Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:190 RuleTransitionApplier.computeToolchainConfigurationKey
pub fn compute_toolchain_configuration_key(
    build_options: &BuildOptions,
    services: &AnalysisServices,
    event_handler: &StoredEventHandler,
) -> BuildConfigurationKey {
    let toolchain_options =
        patch_toolchain_tagged_trimming(build_options, event_handler, &services.starlark_semantics);
    BuildConfigurationKey::create(Arc::new(toolchain_options))
}

/// Java `targetAndConfiguration.getConfiguration()` of a resolved node.
pub fn configuration_of(tac: &TargetAndConfiguration) -> Option<&Arc<BuildConfigurationValue>> {
    tac.get_configuration()
}
