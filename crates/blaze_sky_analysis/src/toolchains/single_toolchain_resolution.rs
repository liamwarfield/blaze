//! Ports of `skyframe/toolchains/SingleToolchainResolutionFunction.java` and
//! `SingleToolchainResolutionValue.java`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::{
    ConstraintCollection, DeclaredToolchainInfo, PlatformInfo, ToolchainTypeInfo,
};
use blaze_analysis::toolchains::toolchain_context_key::toolchain_type_requirement_to_string;
use blaze_analysis::toolchains::{ToolchainExceptionKind, ToolchainTypeRequirement};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_cmdline::Label;
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, Transience,
};
use indexmap::IndexMap;

use crate::toolchains::lookup_error::catch_toolchain_exception;
use crate::toolchains::platform_lookup_util;
use crate::toolchains::registered_toolchains::RegisteredToolchainsKey;

/// `SkyFunctions.SINGLE_TOOLCHAIN_RESOLUTION`.
pub const SINGLE_TOOLCHAIN_RESOLUTION: SkyFunctionName =
    SkyFunctionName("SINGLE_TOOLCHAIN_RESOLUTION");

/// A value which represents the map of potential execution platforms and resolved toolchains
/// for a single toolchain type. This allows for a Skyframe cache per toolchain type.
///
/// `toolchain_type`: the resolved details about the requested toolchain type.
/// `available_toolchain_labels`: the resolved set of toolchain labels for the requested
/// toolchain type, keyed by the execution platforms. Ordering is not preserved, if the caller
/// cares about the order of platforms it must take care of that directly.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionValue.java:46 SingleToolchainResolutionValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleToolchainResolutionValue {
    pub toolchain_type: ToolchainTypeInfo,
    pub available_toolchain_labels: IndexMap<ConfiguredTargetKey, Label>,
}

impl SingleToolchainResolutionValue {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionValue.java:126 SingleToolchainResolutionValue.create
    pub fn create(
        toolchain_type: ToolchainTypeInfo,
        available_toolchain_labels: IndexMap<ConfiguredTargetKey, Label>,
    ) -> Self {
        SingleToolchainResolutionValue {
            toolchain_type,
            available_toolchain_labels,
        }
    }
}

/// SkyKey implementation used for [`single_toolchain_resolution_function`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionValue.java:89 SingleToolchainResolutionValue.SingleToolchainResolutionKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SingleToolchainResolutionKey {
    configuration_key: BuildConfigurationKey,
    toolchain_type: ToolchainTypeRequirement,
    toolchain_type_info: ToolchainTypeInfo,
    target_platform_key: ConfiguredTargetKey,
    available_execution_platform_keys: Vec<ConfiguredTargetKey>,
    debug_target: bool,
}

impl SingleToolchainResolutionKey {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionValue.java:71 SingleToolchainResolutionValue.key
    pub fn key(
        configuration_key: BuildConfigurationKey,
        toolchain_type: ToolchainTypeRequirement,
        toolchain_type_info: ToolchainTypeInfo,
        target_platform_key: ConfiguredTargetKey,
        available_execution_platform_keys: Vec<ConfiguredTargetKey>,
        debug_target: bool,
    ) -> Self {
        SingleToolchainResolutionKey {
            configuration_key,
            toolchain_type,
            toolchain_type_info,
            target_platform_key,
            available_execution_platform_keys,
            debug_target,
        }
    }

    pub fn configuration_key(&self) -> &BuildConfigurationKey {
        &self.configuration_key
    }

    pub fn toolchain_type(&self) -> &ToolchainTypeRequirement {
        &self.toolchain_type
    }

    pub fn toolchain_type_info(&self) -> &ToolchainTypeInfo {
        &self.toolchain_type_info
    }

    pub fn target_platform_key(&self) -> &ConfiguredTargetKey {
        &self.target_platform_key
    }

    pub fn available_execution_platform_keys(&self) -> &[ConfiguredTargetKey] {
        &self.available_execution_platform_keys
    }

    pub fn debug_target(&self) -> bool {
        self.debug_target
    }
}

impl fmt::Debug for SingleToolchainResolutionKey {
    // Java: AutoValue toString of SingleToolchainResolutionKey.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SingleToolchainResolutionKey{{configurationKey={:?}, toolchainType={}, toolchainTypeInfo={}, targetPlatformKey={:?}, availableExecutionPlatformKeys=[{}], debugTarget={}}}",
            self.configuration_key,
            toolchain_type_requirement_to_string(&self.toolchain_type),
            self.toolchain_type_info,
            self.target_platform_key,
            self.available_execution_platform_keys
                .iter()
                .map(|k| format!("{k:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            self.debug_target
        )
    }
}

impl Key for SingleToolchainResolutionKey {
    type Value = SingleToolchainResolutionValue;
    const FUNCTION: SkyFunctionName = SINGLE_TOOLCHAIN_RESOLUTION;
}

/// SkyFunction which performs toolchain resolution for a single toolchain type.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunction.java:53 SingleToolchainResolutionFunction.compute
pub fn single_toolchain_resolution_function(
    key: &SingleToolchainResolutionKey,
    env: &mut Environment<'_>,
) -> Result<Option<SingleToolchainResolutionValue>, ComputeError> {
    // This call could be combined with the call below, but this SkyFunction is evaluated so
    // rarely it's not worth optimizing.
    let configuration = env.get_value(key.configuration_key().clone())?;
    if env.values_missing() {
        return Ok(None);
    }
    let configuration = configuration.unwrap();

    // Check if debug output should be generated.
    let debug = key.debug_target()
        || configuration
            .get_fragment::<PlatformConfiguration>()
            .expect("NullPointerException: PlatformConfiguration")
            .debug_toolchain_resolution_for_label(&key.toolchain_type().toolchain_type);

    // Get all toolchains.
    let toolchains = match env.get_value_or_throw_with(
        RegisteredToolchainsKey::key(key.configuration_key().clone(), debug),
        catch_toolchain_exception(&[ToolchainExceptionKind::InvalidToolchainLabel]),
    )? {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(None),
        // Java: catch (InvalidToolchainLabelException e).
        Err(e) => return Err(SkyFunctionException::new(e, Transience::Persistent).into()),
    };

    // Find the right one.
    let Some(platforms) = get_platforms(
        env,
        key.target_platform_key(),
        key.available_execution_platform_keys(),
    )?
    else {
        return Ok(None);
    };

    if debug {
        blaze_util::unported!("SingleToolchainResolutionDebugPrinter.create");
    }
    let target_platform = platforms[key.target_platform_key()].clone();

    // Describe rejected toolchains if any are present.
    // PORT: only the debug printer reads `toolchains.rejected_toolchains`.

    let toolchain_resolution = resolve_constraints(
        key.toolchain_type(),
        key.toolchain_type_info(),
        key.available_execution_platform_keys(),
        &platforms,
        &target_platform,
        &toolchains.registered_toolchains,
    );

    Ok(Some(toolchain_resolution))
}

/// Loads [`PlatformInfo`] from Skyframe for checking against toolchain constraints. Returns
/// `None` if a Skyframe value isn't available yet.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunction.java:125 SingleToolchainResolutionFunction.getPlatforms
fn get_platforms(
    env: &mut Environment<'_>,
    target_platform_key: &ConfiguredTargetKey,
    available_execution_platform_keys: &[ConfiguredTargetKey],
) -> Result<Option<HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>>, ComputeError> {
    let mut keys = Vec::with_capacity(available_execution_platform_keys.len() + 1);
    keys.push(target_platform_key.clone());
    keys.extend(available_execution_platform_keys.iter().cloned());
    match platform_lookup_util::get_platform_info(&keys, env) {
        Ok(v) => Ok(v),
        // Java: catch (InvalidPlatformException e) -> ToolchainResolutionFunctionException.
        Err(e) => Err(e.into_compute_error()),
    }
}

/// Given the available execution platforms and toolchains, find the set of platform, toolchain
/// pairs that are compatible a) with each other, and b) with the toolchain type and target
/// platform.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunction.java:149 SingleToolchainResolutionFunction.resolveConstraints
fn resolve_constraints(
    toolchain_type: &ToolchainTypeRequirement,
    toolchain_type_info: &ToolchainTypeInfo,
    available_execution_platform_keys: &[ConfiguredTargetKey],
    platforms: &HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>,
    target_platform: &PlatformInfo,
    toolchains: &[DeclaredToolchainInfo],
) -> SingleToolchainResolutionValue {
    // Platforms may exist multiple times in availableExecutionPlatformKeys. The Set lets this
    // code check whether a platform has already been seen during processing.
    let mut platform_keys_seen: HashSet<ConfiguredTargetKey> = HashSet::new();
    let mut builder: IndexMap<ConfiguredTargetKey, Label> = IndexMap::new();

    // Pre-filter for the correct toolchain type. This simplifies the loop and makes debugging
    // toolchain resolution much, much easier.
    let filtered_toolchains: Vec<&DeclaredToolchainInfo> = toolchains
        .iter()
        .filter(|toolchain| {
            toolchain.toolchain_type().type_label() == &toolchain_type.toolchain_type
        })
        .collect();

    for toolchain in filtered_toolchains {
        // Make sure the target setting matches but watch out for resolution errors.

        // Make sure the target platform matches.
        if !toolchain.has_target_to_exec_constraints()
            && !check_constraints(toolchain.target_constraints(), target_platform)
        {
            continue;
        }

        // Find the matching execution platforms.
        let mut done = true;

        for execution_platform_key in available_execution_platform_keys {
            // Only check the toolchains if this is a new platform.
            if platform_keys_seen.contains(execution_platform_key) {
                continue;
            }

            let execution_platform = &platforms[execution_platform_key];

            // Check if the platform allows this toolchain type.
            if execution_platform.check_toolchain_types()
                && !execution_platform
                    .allowed_toolchain_types()
                    .contains(&toolchain_type.toolchain_type)
            {
                // Keep looking for a valid toolchain for this exec platform
                done = false;
                continue;
            }

            // Check if the execution constraints match.
            let exec_constraints = if toolchain.has_target_to_exec_constraints() {
                target_platform.constraints()
            } else {
                toolchain.exec_constraints()
            };
            if !check_constraints(exec_constraints, execution_platform) {
                // Keep looking for a valid toolchain for this exec platform
                done = false;
                continue;
            }

            if builder.contains_key(execution_platform_key) {
                // Java: ImmutableMap.Builder.buildOrThrow on a duplicate key.
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {execution_platform_key}"
                );
            }
            builder.insert(
                execution_platform_key.clone(),
                toolchain.resolved_toolchain_label().clone(),
            );
            platform_keys_seen.insert(execution_platform_key.clone());
        }

        if done {
            break;
        }
    }

    SingleToolchainResolutionValue::create(toolchain_type_info.clone(), builder)
}

/// Returns `true` iff all constraints set by the toolchain and in the [`PlatformInfo`] match.
// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunction.java:250 SingleToolchainResolutionFunction.checkConstraints
fn check_constraints(
    toolchain_constraints: &ConstraintCollection,
    platform: &PlatformInfo,
) -> bool {
    // Check every constraint_setting in either the toolchain or the platform.
    let mismatch_settings = toolchain_constraints.diff(platform.constraints());

    // If a constraint_setting has a default_constraint_value, and the platform
    // sets a non-default constraint value for the same constraint_setting, then
    // even if the toolchain doesn't set a constraint value for this
    // constraint_setting, it should still be considered a mismatch.
    //
    // The only exception is if the toolchain sets a value for the constraint_setting, and that
    // value is the default.
    let mismatch_settings_with_default: Vec<_> = mismatch_settings
        .into_iter()
        .filter(|s| toolchain_constraints.has_without_default(s))
        .collect();

    mismatch_settings_with_default.is_empty()
}
