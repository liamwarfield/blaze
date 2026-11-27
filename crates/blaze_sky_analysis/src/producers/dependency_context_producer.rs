//! Ports of `analysis/producers/DependencyContextProducerWithCompatibilityCheck.java`,
//! `analysis/producers/DependencyContextProducer.java` and
//! `analysis/producers/DependencyContext.java`.

use std::sync::Arc;

use blaze_analysis::config::config_conditions::ConfigConditions;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::platform_info::PlatformInfo;
use blaze_analysis::toolchains::{
    ToolchainCollection, UnloadedToolchainContext, UnloadedToolchainContextsInputs,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_skyframe::Environment;

use crate::incompatible_target_checker::produce_incompatible_target;
use crate::platform::platform_producer::{PlatformProducerError, produce_platform};
use crate::producers::config_conditions_producer::produce_config_conditions;
use crate::producers::errors::{
    ConfiguredValueCreationException, DependencyContextError, TargetAndConfiguration,
    TransitiveDependencyState,
};
use crate::producers::transition_applier::Produced;
use crate::toolchains::unloaded_toolchain_contexts_producer::produce_unloaded_toolchain_contexts;

/// The unloaded toolchain contexts and config conditions of a target.
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContext.java:29 DependencyContext
#[derive(Clone, Debug)]
pub struct DependencyContext {
    /// `None` if the target doesn't require toolchain resolution.
    pub unloaded_toolchain_contexts: Option<Arc<ToolchainCollection<UnloadedToolchainContext>>>,
    pub config_conditions: ConfigConditions,
}

impl DependencyContext {
    /// Java `toolchainContexts()`: the unloaded contexts as `ToolchainContext`s.
    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContext.java:37 DependencyContext.toolchainContexts
    pub fn toolchain_contexts(&self) -> Option<&ToolchainCollection<UnloadedToolchainContext>> {
        self.unloaded_toolchain_contexts.as_deref()
    }
}

/// Computes the dependency context, checking for directly incompatible targets before
/// toolchain resolution (Java `DependencyContextProducerWithCompatibilityCheck`).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:81 DependencyContextProducerWithCompatibilityCheck.step
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:67 DependencyContextProducerWithCompatibilityCheck.<init>
pub fn produce_dependency_context_with_compatibility_check(
    env: &mut Environment<'_>,
    target_and_configuration: &TargetAndConfiguration,
    configured_target_key: &ConfiguredTargetKey,
    unloaded_toolchain_contexts_inputs: &UnloadedToolchainContextsInputs,
    transitive_state: &mut TransitiveDependencyState,
) -> Produced<Result<DependencyContext, DependencyContextError>> {
    let target = target_and_configuration.get_target();
    if unloaded_toolchain_contexts_inputs
        .target_toolchain_context_key()
        .is_none()
    {
        // If `defaultToolchainContextKey` is null, there's no platform info, incompatibility
        // check or toolchain resolution. Short-circuits and computes only the ConfigConditions.
        return Ok(
            match produce_config_conditions(
                env,
                target,
                target.get_label(),
                configured_target_key.get_configuration_key(),
                /* target_platform_info= */ None,
                transitive_state,
            )? {
                None => None,
                Some(Err(e)) => Some(Err(DependencyContextError::ConfiguredValueCreation(e))),
                Some(Ok(config_conditions)) => Some(Ok(DependencyContext {
                    unloaded_toolchain_contexts: None,
                    config_conditions,
                })),
            },
        );
    }

    // Non-null `defaultToolchainContextKey` guarantees that `platformConfiguration` is non-null.
    let configuration = target_and_configuration
        .get_configuration()
        .expect("configured target with toolchain resolution");
    let platform_configuration = configuration
        .get_fragment::<PlatformConfiguration>()
        .expect("PlatformConfiguration");
    // Checks for incompatibility before toolchain resolution so that known missing toolchains
    // mark the target incompatible instead of failing the build.
    let target_platform_info: Arc<PlatformInfo> = match produce_platform(
        env,
        platform_configuration.get_target_platform(),
        &configuration
            .get_options()
            .get::<CoreOptions>()
            .expect("CoreOptions")
            .get_command_line_flag_aliases(),
    )? {
        None => return Ok(None),
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:113 DependencyContextProducerWithCompatibilityCheck.acceptPlatformValue
        Some(Ok(info)) => info,
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:118 DependencyContextProducerWithCompatibilityCheck.acceptPlatformInfoError
        Some(Err(PlatformProducerError::InvalidPlatform(e))) => {
            return Ok(Some(Err(DependencyContextError::Toolchain(e))));
        }
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:124 DependencyContextProducerWithCompatibilityCheck.acceptOptionsParsingError
        Some(Err(PlatformProducerError::OptionsParsing(message))) => {
            return Ok(Some(Err(DependencyContextError::ConfiguredValueCreation(
                ConfiguredValueCreationException::for_target(target, message),
            ))));
        }
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:132 DependencyContextProducerWithCompatibilityCheck.computeConfigConditions
    let config_conditions = match produce_config_conditions(
        env,
        target,
        target.get_label(),
        configured_target_key.get_configuration_key(),
        Some(&*target_platform_info),
        transitive_state,
    )? {
        None => return Ok(None),
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:154 DependencyContextProducerWithCompatibilityCheck.acceptConfigConditionsError
        Some(Err(e)) => {
            return Ok(Some(Err(DependencyContextError::ConfiguredValueCreation(
                e,
            ))));
        }
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:149 DependencyContextProducerWithCompatibilityCheck.acceptConfigConditions
        Some(Ok(c)) => c,
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:159 DependencyContextProducerWithCompatibilityCheck.checkCompatibility
    match produce_incompatible_target(
        env,
        target_and_configuration,
        configured_target_key,
        &config_conditions,
        Some(&*target_platform_info),
        transitive_state,
    )? {
        None => return Ok(None),
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:184 DependencyContextProducerWithCompatibilityCheck.acceptValidationException
        Some(Err(e)) => return Ok(Some(Err(DependencyContextError::Validation(e)))),
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:175 DependencyContextProducerWithCompatibilityCheck.acceptIncompatibleTarget
        Some(Ok(Some(incompatible_target))) => {
            return Ok(Some(Err(DependencyContextError::IncompatibleTarget(
                incompatible_target,
            ))));
        }
        Some(Ok(None)) => {}
    }

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:189 DependencyContextProducerWithCompatibilityCheck.computeUnloadedToolchainContexts
    let unloaded_toolchain_contexts =
        match produce_unloaded_toolchain_contexts(env, unloaded_toolchain_contexts_inputs)? {
            None => return Ok(None),
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:207 DependencyContextProducerWithCompatibilityCheck.acceptUnloadedToolchainContextsError
            Some(Err(e)) => return Ok(Some(Err(DependencyContextError::Toolchain(e)))),
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:201 DependencyContextProducerWithCompatibilityCheck.acceptUnloadedToolchainContexts
            Some(Ok(c)) => c,
        };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducerWithCompatibilityCheck.java:212 DependencyContextProducerWithCompatibilityCheck.constructResult
    Ok(Some(Ok(DependencyContext {
        unloaded_toolchain_contexts,
        config_conditions,
    })))
}

/// Computes the dependency context without the compatibility check (Java
/// `DependencyContextProducer`, used for aspects).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:87 DependencyContextProducer.step
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:52 DependencyContextProducer.ResultSink.acceptDependencyContext
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:54 DependencyContextProducer.ResultSink.acceptDependencyContextError
// PORT: the result sink is the return value (`Ok(DependencyContext)` or the error).
// Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:72 DependencyContextProducer.<init>
pub fn produce_dependency_context(
    env: &mut Environment<'_>,
    unloaded_toolchain_contexts_inputs: &UnloadedToolchainContextsInputs,
    target_and_configuration: &TargetAndConfiguration,
    build_configuration_key: Option<&BuildConfigurationKey>,
    transitive_state: &mut TransitiveDependencyState,
) -> Produced<Result<DependencyContext, DependencyContextError>> {
    let unloaded_toolchain_contexts =
        match produce_unloaded_toolchain_contexts(env, unloaded_toolchain_contexts_inputs)? {
            None => return Ok(None),
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:101 DependencyContextProducer.acceptUnloadedToolchainContextsError
            Some(Err(e)) => return Ok(Some(Err(DependencyContextError::Toolchain(e)))),
            // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:95 DependencyContextProducer.acceptUnloadedToolchainContexts
            Some(Ok(c)) => c,
        };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:106 DependencyContextProducer.computeConfigConditions
    let target = target_and_configuration.get_target();
    let config_conditions = match produce_config_conditions(
        env,
        target,
        target.get_label(),
        build_configuration_key,
        unloaded_toolchain_contexts
            .as_deref()
            .map(|c| &**c.get_target_platform()),
        transitive_state,
    )? {
        None => return Ok(None),
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:127 DependencyContextProducer.acceptConfigConditionsError
        Some(Err(e)) => {
            return Ok(Some(Err(DependencyContextError::ConfiguredValueCreation(
                e,
            ))));
        }
        // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:122 DependencyContextProducer.acceptConfigConditions
        Some(Ok(c)) => c,
    };

    // Java: com/google/devtools/build/lib/analysis/producers/DependencyContextProducer.java:132 DependencyContextProducer.constructResult
    Ok(Some(Ok(DependencyContext {
        unloaded_toolchain_contexts,
        config_conditions,
    })))
}
