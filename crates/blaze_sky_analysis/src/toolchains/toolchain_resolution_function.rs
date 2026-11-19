//! Port of `skyframe/toolchains/ToolchainResolutionFunction.java`: performs toolchain
//! resolution for multiple toolchain types, including selecting the execution platform.

use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::{EMPTY_PLATFORM_INFO, PlatformInfo, ToolchainTypeInfo};
use blaze_analysis::toolchains::{
    NoMatchingPlatformData, ToolchainContextKey, ToolchainException, ToolchainExceptionKind,
    ToolchainTypeRequirement, UnloadedToolchainContext, UnloadedToolchainContextImpl,
    UnloadedToolchainContextImplBuilder,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_cmdline::Label;
use blaze_skyframe::{ComputeError, Environment, SkyKey};
use indexmap::{IndexMap, IndexSet};

use crate::toolchains::lookup_error::{ToolchainLookupError, catch_toolchain_exception};
use crate::toolchains::platform_keys::{PlatformKeys, ResolvedToolchainsTable};
use crate::toolchains::platform_lookup_util;
use crate::toolchains::single_toolchain_resolution::{
    SingleToolchainResolutionKey, SingleToolchainResolutionValue,
};
use crate::toolchains::toolchain_type_lookup_util;

/// A requested toolchain type with its resolved [`ToolchainTypeInfo`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:135 ToolchainResolutionFunction.ToolchainType
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolchainType {
    pub toolchain_type_requirement: ToolchainTypeRequirement,
    pub toolchain_type_info: ToolchainTypeInfo,
}

impl ToolchainType {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:143 ToolchainResolutionFunction.ToolchainType.mandatory
    pub fn mandatory(&self) -> bool {
        self.toolchain_type_requirement.mandatory
    }
}

/// SkyFunction which performs toolchain resolution for multiple toolchain types, including
/// selecting the execution platform.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:62 ToolchainResolutionFunction.compute
pub fn toolchain_resolution_function(
    key: &ToolchainContextKey,
    env: &mut Environment<'_>,
) -> Result<Option<UnloadedToolchainContextImpl>, ComputeError> {
    match compute(key, env) {
        Ok(v) => Ok(v),
        // Java: catch (ToolchainException e) -> ToolchainResolutionFunctionException (persistent).
        Err(e) => Err(e.into_compute_error()),
    }
}

fn compute(
    key: &ToolchainContextKey,
    env: &mut Environment<'_>,
) -> Result<Option<UnloadedToolchainContext>, ToolchainLookupError> {
    let mut builder = UnloadedToolchainContextImpl::builder(key.clone());

    // Determine the configuration being used.
    let Some(configuration) = env.get_value(key.configuration_key().clone())? else {
        return Ok(None);
    };
    let platform_configuration = configuration
        .get_fragment::<PlatformConfiguration>()
        .expect("NullPointerException: PlatformConfiguration");

    // Check if debug output should be generated.
    let toolchain_type_labels: Vec<Label> = {
        let mut labels: Vec<Label> = Vec::new();
        for t in key.toolchain_types() {
            if !labels.contains(&t.toolchain_type) {
                labels.push(t.toolchain_type.clone());
            }
        }
        labels
    };
    let debug = key.debug_target()
        || platform_configuration.debug_toolchain_resolution(&toolchain_type_labels);
    // PORT: ToolchainResolutionDebugPrinter is not ported. Without debugging every call on it
    // is a no-op, so with `--toolchain_resolution_debug` matching this target or one of its
    // toolchain types the function stops here, and the debugPrinter calls below are left out.
    if debug {
        blaze_util::unported!("ToolchainResolutionDebugPrinter.create");
    }

    // Create keys for all platforms that will be used, and validate them early.
    // Do this early, to catch platform errors early.
    let platform_keys = PlatformKeys::load(
        env,
        configuration.get_key(),
        platform_configuration,
        key.exec_constraint_labels(),
    )?;
    if env.values_missing() {
        return Ok(None);
    }
    let Some(platform_keys) = platform_keys else {
        return Ok(None);
    };

    // Load the configured target for the toolchain types to ensure that they are valid and
    // resolve aliases.
    let Some(resolved_toolchain_type_infos) =
        load_toolchain_type_infos(env, &configuration, key.toolchain_types())?
    else {
        return Ok(None);
    };
    builder.set_requested_label_to_toolchain_type(resolved_toolchain_type_infos.clone());
    let resolved_toolchain_types =
        load_toolchain_types(&resolved_toolchain_type_infos, key.toolchain_types());

    // Determine the actual toolchain implementations to use.
    let forced_execution_platform: Option<ConfiguredTargetKey> = key
        .force_execution_platform()
        .and_then(|l| platform_keys.find(l));
    if determine_toolchain_implementations(
        env,
        key.configuration_key(),
        &resolved_toolchain_types,
        forced_execution_platform,
        &mut builder,
        &platform_keys,
        key.debug_target(),
    )?
    .is_none()
    {
        return Ok(None);
    }

    let unloaded_toolchain_context = builder.build();
    // (debugPrinter.reportSelectedToolchains is a no-op without debugging.)
    Ok(Some(unloaded_toolchain_context))
}

/// Returns a map from the requested toolchain type label (after any alias chains) to the
/// [`ToolchainTypeInfo`] provider.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:152 ToolchainResolutionFunction.loadToolchainTypeInfos
fn load_toolchain_type_infos(
    environment: &mut Environment<'_>,
    configuration: &BuildConfigurationValue,
    toolchain_type_requirements: &[ToolchainTypeRequirement],
) -> Result<Option<IndexMap<Label, ToolchainTypeInfo>>, ToolchainLookupError> {
    let resolved_toolchain_types = toolchain_type_lookup_util::resolve_toolchain_types(
        environment,
        toolchain_type_requirements,
        configuration,
    )?;
    if environment.values_missing() {
        return Ok(None);
    }
    Ok(resolved_toolchain_types)
}

/// Returns the requested toolchain types with their post-alias labels.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:170 ToolchainResolutionFunction.loadToolchainTypes
fn load_toolchain_types(
    resolved_toolchain_type_infos: &IndexMap<Label, ToolchainTypeInfo>,
    toolchain_types: &[ToolchainTypeRequirement],
) -> Vec<ToolchainType> {
    let mut resolved: Vec<ToolchainType> = Vec::new();

    for toolchain_type_requirement in toolchain_types {
        // Find the actual Label.
        let toolchain_type_label = &toolchain_type_requirement.toolchain_type;
        let Some(toolchain_type_info) = resolved_toolchain_type_infos.get(toolchain_type_label)
        else {
            continue;
        };

        // If the labels don't match, re-build the TTR.
        let mut toolchain_type_requirement = toolchain_type_requirement.clone();
        let toolchain_type_label = toolchain_type_info.type_label();
        if toolchain_type_label != &toolchain_type_requirement.toolchain_type {
            toolchain_type_requirement.toolchain_type = toolchain_type_label.clone();
        }

        let t = ToolchainType {
            toolchain_type_requirement,
            toolchain_type_info: toolchain_type_info.clone(),
        };
        // Java: ImmutableSet.Builder drops duplicates.
        if !resolved.contains(&t) {
            resolved.push(t);
        }
    }
    resolved
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:195 ToolchainResolutionFunction.determineToolchainImplementations
fn determine_toolchain_implementations(
    environment: &mut Environment<'_>,
    configuration_key: &BuildConfigurationKey,
    toolchain_types: &[ToolchainType],
    forced_execution_platform: Option<ConfiguredTargetKey>,
    builder: &mut UnloadedToolchainContextImplBuilder,
    platform_keys: &PlatformKeys,
    debug_target: bool,
) -> Result<Option<()>, ToolchainLookupError> {
    // Find the toolchains for the requested toolchain types.
    let mut registered_toolchain_keys: Vec<SingleToolchainResolutionKey> = Vec::new();
    for toolchain_type in toolchain_types {
        registered_toolchain_keys.push(SingleToolchainResolutionKey::key(
            configuration_key.clone(),
            toolchain_type.toolchain_type_requirement.clone(),
            toolchain_type.toolchain_type_info.clone(),
            platform_keys.target_platform_key().clone(),
            platform_keys.execution_platform_keys().to_vec(),
            debug_target,
        ));
    }

    let sky_keys: Vec<SkyKey> = registered_toolchain_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let results = environment.get_values_and_exceptions(&sky_keys)?;
    let mut values_missing = false;

    // Determine the potential set of toolchains.
    let mut resolved_toolchains: ResolvedToolchainsTable = IndexMap::new();
    let mut missing_mandatory_toolchains: IndexSet<ToolchainTypeInfo> = IndexSet::new();
    for key in &registered_toolchain_keys {
        let single_toolchain_resolution_value = match results.get_or_throw_with(
            key,
            catch_toolchain_exception(&[
                ToolchainExceptionKind::InvalidToolchainLabel,
                ToolchainExceptionKind::InvalidConfigurationDuringToolchainResolution,
            ]),
        ) {
            Ok(Some(v)) => v,
            Ok(None) => {
                values_missing = true;
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        if !single_toolchain_resolution_value
            .available_toolchain_labels
            .is_empty()
        {
            let required_toolchain_type = &single_toolchain_resolution_value.toolchain_type;
            find_platforms_and_labels(
                &mut resolved_toolchains,
                required_toolchain_type,
                &single_toolchain_resolution_value,
            );
        } else if key.toolchain_type().mandatory {
            // Save the missing type and continue looping to check for more.
            missing_mandatory_toolchains.insert(key.toolchain_type_info().clone());
        }
        // TODO(katre): track missing optional toolchains?
    }

    // Verify that all mandatory toolchain types have a toolchain.
    if !missing_mandatory_toolchains.is_empty() {
        return Err(unresolved_toolchains_exception(
            platform_keys
                .target_platform_info()
                .expect("NullPointerException: target platform info"),
            &missing_mandatory_toolchains,
        )
        .into());
    }

    if values_missing {
        return Ok(None);
    }

    // Find and return the first execution platform which has all mandatory toolchains.
    let selected_execution_platform_key = find_execution_platform_for_toolchains(
        toolchain_types,
        forced_execution_platform,
        platform_keys,
        &resolved_toolchains,
    );

    let toolchain_type_requirements: Vec<ToolchainTypeRequirement> = {
        let mut v: Vec<ToolchainTypeRequirement> = Vec::new();
        for t in toolchain_types {
            if !v.contains(&t.toolchain_type_requirement) {
                v.push(t.toolchain_type_requirement.clone());
            }
        }
        v
    };
    let Some(selected_execution_platform_key) = selected_execution_platform_key else {
        builder.set_toolchain_types(toolchain_type_requirements.clone());
        builder.set_execution_platform(Arc::new(EMPTY_PLATFORM_INFO.clone()));
        builder.set_target_platform(Arc::new(EMPTY_PLATFORM_INFO.clone()));
        builder.set_error_data(Some(NoMatchingPlatformData {
            toolchain_types: toolchain_type_requirements,
            available_execution_platform_keys: platform_keys.execution_platform_keys().to_vec(),
            target_platform_key: platform_keys.target_platform_key().clone(),
        }));
        return Ok(Some(()));
    };

    let Some(platforms) = platform_lookup_util::get_platform_info(
        &[
            selected_execution_platform_key.clone(),
            platform_keys.target_platform_key().clone(),
        ],
        environment,
    )?
    else {
        return Ok(None);
    };

    builder.set_toolchain_types(toolchain_type_requirements);
    builder.set_execution_platform(platforms[&selected_execution_platform_key].clone());
    builder.set_target_platform(platforms[platform_keys.target_platform_key()].clone());

    let toolchains: Vec<(ToolchainTypeInfo, Label)> = resolved_toolchains
        .get(&selected_execution_platform_key)
        .map(|row| row.iter().map(|(t, l)| (t.clone(), l.clone())).collect())
        .unwrap_or_default();
    builder.set_toolchain_type_to_resolved(toolchains);
    Ok(Some(()))
}

/// Adds all of toolchain labels from `single_toolchain_resolution_value` to
/// `resolved_toolchains`.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:306 ToolchainResolutionFunction.findPlatformsAndLabels
fn find_platforms_and_labels(
    resolved_toolchains: &mut ResolvedToolchainsTable,
    required_toolchain_type: &ToolchainTypeInfo,
    single_toolchain_resolution_value: &SingleToolchainResolutionValue,
) {
    // Java builds a fresh table and `putAll`s it, in the same order.
    for (platform, label) in &single_toolchain_resolution_value.available_toolchain_labels {
        resolved_toolchains
            .entry(platform.clone())
            .or_default()
            .insert(required_toolchain_type.clone(), label.clone());
    }
}

/// Finds the first platform from `available_execution_platform_keys` that is present in
/// `resolved_toolchains` and has all required toolchain types.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:323 ToolchainResolutionFunction.findExecutionPlatformForToolchains
fn find_execution_platform_for_toolchains(
    toolchain_types: &[ToolchainType],
    forced_execution_platform: Option<ConfiguredTargetKey>,
    platform_keys: &PlatformKeys,
    resolved_toolchains: &ResolvedToolchainsTable,
) -> Option<ConfiguredTargetKey> {
    if let Some(forced) = &forced_execution_platform {
        // Is the forced platform suitable?
        if platform_keys.is_platform_suitable(
            forced,
            toolchain_types,
            resolved_toolchains,
            // For the forced execution platform, ignore allowed toolchain types.
            false,
        ) {
            return forced_execution_platform;
        }
    }

    let candidate_platforms = platform_keys
        .execution_platform_keys()
        .iter()
        .filter(|epk| {
            platform_keys.is_platform_suitable(epk, toolchain_types, resolved_toolchains, true)
        });

    let mut toolchain_type_infos: Vec<&ToolchainTypeInfo> = Vec::new();
    for t in toolchain_types {
        if !toolchain_type_infos.contains(&&t.toolchain_type_info) {
            toolchain_type_infos.push(&t.toolchain_type_info);
        }
    }

    // Sort by the number of toolchains (the sort is stable).
    // Java: Stream.max keeps the first of equal elements.
    let mut best: Option<(&ConfiguredTargetKey, i64)> = None;
    for epk in candidate_platforms {
        let count = count_toolchains_on_platform(epk, &toolchain_type_infos, resolved_toolchains);
        match best {
            Some((_, best_count)) if best_count >= count => {}
            _ => best = Some((epk, count)),
        }
    }
    best.map(|(k, _)| k.clone())
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:360 ToolchainResolutionFunction.countToolchainsOnPlatform
fn count_toolchains_on_platform(
    execution_platform_key: &ConfiguredTargetKey,
    toolchain_type_infos: &[&ToolchainTypeInfo],
    resolved_toolchains: &ResolvedToolchainsTable,
) -> i64 {
    if toolchain_type_infos.is_empty() {
        return 0;
    }

    // Determine the number of optional toolchains.
    let Some(platform_toolchains) = resolved_toolchains.get(execution_platform_key) else {
        return 0;
    };
    toolchain_type_infos
        .iter()
        .filter(|t| platform_toolchains.contains_key(**t))
        .count() as i64
}

/// Exception used when a toolchain type is required but no matching toolchain is found.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:383 ToolchainResolutionFunction.UnresolvedToolchainsException.<init>
pub fn unresolved_toolchains_exception(
    target_platform_info: &PlatformInfo,
    missing_toolchain_types: &IndexSet<ToolchainTypeInfo>,
) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::UnresolvedToolchains,
        unresolved_toolchains_message(target_platform_info, missing_toolchain_types),
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:393 ToolchainResolutionFunction.UnresolvedToolchainsException.getMessage
fn unresolved_toolchains_message(
    target_platform_info: &PlatformInfo,
    missing_toolchain_types: &IndexSet<ToolchainTypeInfo>,
) -> String {
    let label_strings: Vec<String> = missing_toolchain_types
        .iter()
        .map(|t| PlatformConfiguration::toolchain_resolution_debug_filter(t.type_label()))
        .collect();
    let missing_toolchain_rows: Vec<String> = missing_toolchain_types
        .iter()
        .map(|t| {
            format!(
                "  {}{}",
                t.type_label(),
                match t.none_found_error() {
                    Some(e) => format!(": {e}"),
                    None => String::new(),
                }
            )
        })
        .collect();
    let platform_specific_message = target_platform_info
        .get_missing_toolchain_error_message()
        .unwrap_or("");
    format!(
        "No matching toolchains found for types:\n{}\nTo debug, rerun with --toolchain_resolution_debug='{}'\n{}",
        missing_toolchain_rows.join("\n"),
        label_strings.join("|"),
        platform_specific_message
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    #[test]
    fn unresolved_toolchains_message_text() {
        let platform = PlatformInfo::builder()
            .set_label(label("//platforms:target"))
            .build()
            .unwrap();
        let mut missing = IndexSet::new();
        missing.insert(ToolchainTypeInfo::create(label("//toolchain:type1")));
        missing.insert(ToolchainTypeInfo::create_with(
            label("//toolchain:type2"),
            Some("custom error".to_owned()),
        ));
        assert_eq!(
            unresolved_toolchains_message(&platform, &missing),
            "No matching toolchains found for types:\n  //toolchain:type1\n  //toolchain:type2: custom error\nTo debug, rerun with --toolchain_resolution_debug='//toolchain:type1|//toolchain:type2'\n"
        );
    }

    #[test]
    fn load_toolchain_types_rewrites_aliased_labels() {
        let info = ToolchainTypeInfo::create(label("//toolchain:actual"));
        let mut infos = IndexMap::new();
        infos.insert(label("//toolchain:alias"), info.clone());
        infos.insert(label("//toolchain:actual"), info.clone());
        let requirements = vec![ToolchainTypeRequirement::create(label("//toolchain:alias"))];
        let resolved = load_toolchain_types(&infos, &requirements);
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].toolchain_type_requirement.toolchain_type,
            label("//toolchain:actual")
        );
    }
}
