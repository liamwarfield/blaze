//! Port of `lib/analysis/platform/PlatformUtils.java`: utilities for accessing platform
//! properties.
//!
//! PORT: `blaze_exec::exec::spawn_log_context` holds a private copy of
//! `getPlatformProto(spawn, remoteOptions)` for the execution log (it cannot depend on this
//! crate); the two must stay in step.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use blaze_exec::actions::{ExecException, Spawn, UserExecException};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn as spawn_code};
use blaze_util::java_hash_map::JavaHashMap;
use blaze_util::string_encoding::{internal_to_unicode, unicode_to_internal};
use indexmap::IndexMap;

use crate::reapi::{Platform, platform};
use crate::text_format::{self, ParseException};

/// `String.compareTo`: lexicographic order of UTF-16 code units. For proto strings (which hold
/// exactly the chars of the Java string) this is Java's order.
fn java_string_compare(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Sorts the properties by name (a stable sort, `Ordering.sortedCopy`).
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:43 PlatformUtils.sortPlatformProperties
fn sort_platform_properties(builder: &mut Platform) {
    let mut properties: Vec<platform::Property> = std::mem::take(&mut builder.properties);
    properties.sort_by(|a, b| java_string_compare(&a.name, &b.name));
    builder.properties = properties;
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:52 PlatformUtils.buildPlatformProto
pub fn build_platform_proto(execution_properties: &IndexMap<String, String>) -> Option<Platform> {
    if execution_properties.is_empty() {
        return None;
    }
    let mut builder = Platform::default();
    for (key, value) in execution_properties {
        let property = platform::Property {
            name: internal_to_unicode(key).into_owned(),
            value: internal_to_unicode(value).into_owned(),
        };
        builder.properties.push(property);
    }

    sort_platform_properties(&mut builder);
    Some(builder)
}

/// `getPlatformProto(spawn, remoteOptions)`.
///
/// PORT: `throws UserExecException` is `ExecException::User`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:71 PlatformUtils.getPlatformProto
pub fn get_platform_proto(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
) -> Result<Option<Platform>, ExecException> {
    get_platform_proto_with_additional_properties(spawn, remote_options, &IndexMap::new())
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:76 PlatformUtils.shouldProducePlatformProto
fn should_produce_platform_proto(
    spawn: &dyn Spawn,
    default_exec_properties: &BTreeMap<String, String>,
    additional_properties: &IndexMap<String, String>,
) -> bool {
    let execution_platform = spawn.get_execution_platform();
    if let Some(execution_platform) = execution_platform {
        if !execution_platform.exec_properties.is_empty() {
            return true;
        }
        if !execution_platform.remote_execution_properties.is_empty() {
            return true;
        }
    }
    if !spawn.get_combined_exec_properties().is_empty() {
        return true;
    }
    if !default_exec_properties.is_empty() {
        return true;
    }
    if !additional_properties.is_empty() {
        return true;
    }
    false
}

/// `Platform.Builder platformBuilder = Platform.newBuilder();
/// TextFormat.getParser().merge(text, platformBuilder)`; the `Err` is the `ParseException`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:129 PlatformUtils.getPlatformProto
fn text_format_merge_platform(text: &str) -> Result<Platform, ParseException> {
    let mut platform_builder = Platform::default();
    text_format::merge_platform(text, &mut platform_builder)?;
    Ok(platform_builder)
}

fn entries_of<'a>(
    map: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Vec<(String, String)> {
    map.into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// `getPlatformProto(spawn, remoteOptions, additionalProperties)`.
///
/// `properties` holds the entries of Java's `properties` map in that map's iteration order: a
/// `HashMap` (see `blaze_util::java_hash_map::JavaHashMap`), the spawn's `ImmutableMap`, the `SortedMap` of default exec
/// properties, or the caller's additional properties.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:102 PlatformUtils.getPlatformProto
pub fn get_platform_proto_with_additional_properties(
    spawn: &dyn Spawn,
    remote_options: Option<&RemoteOptions>,
    additional_properties: &IndexMap<String, String>,
) -> Result<Option<Platform>, ExecException> {
    let default_exec_properties: BTreeMap<String, String> = match remote_options {
        Some(remote_options) => remote_options.get_remote_default_exec_properties(),
        None => BTreeMap::new(),
    };

    if !should_produce_platform_proto(spawn, &default_exec_properties, additional_properties) {
        // Execution platform is null or functionally empty
        return Ok(None);
    }

    let mut properties: Vec<(String, String)> = Vec::new();
    let combined_exec_properties = spawn.get_combined_exec_properties();
    if !combined_exec_properties.is_empty() {
        // Apply default exec properties if the execution platform does not already set
        // exec_properties
        if spawn
            .get_execution_platform()
            .is_none_or(|p| p.exec_properties.is_empty())
        {
            let mut map = JavaHashMap::new();
            map.put_map_entries(&entries_of(&default_exec_properties));
            map.put_map_entries(&entries_of(&combined_exec_properties));
            properties = map.into_entries();
        } else {
            properties = entries_of(&combined_exec_properties);
        }
    } else if let Some(execution_platform) = spawn.get_execution_platform() {
        let remote_execution_properties = &execution_platform.remote_execution_properties;
        if !remote_execution_properties.is_empty() {
            // Try and get the platform info from the execution properties.
            let platform_builder = match text_format_merge_platform(&internal_to_unicode(
                &execution_platform.remote_execution_properties,
            )) {
                Ok(platform_builder) => platform_builder,
                Err(e) => {
                    let message = format!(
                        "Failed to parse remote_execution_properties from platform {}",
                        execution_platform.label
                    );
                    return Err(UserExecException::with_cause(
                        &e,
                        create_failure_detail(
                            message,
                            spawn_code::Code::InvalidRemoteExecutionProperties,
                        ),
                    )
                    .into());
                }
            };

            let mut map = JavaHashMap::new();
            for property in &platform_builder.properties {
                map.put(
                    unicode_to_internal(&property.name).into_owned(),
                    unicode_to_internal(&property.value).into_owned(),
                );
            }
            properties = map.into_entries();
        } else {
            let mut map = JavaHashMap::new();
            map.put_map_entries(&entries_of(&execution_platform.exec_properties));
            properties = map.into_entries();
        }
    }

    if properties.is_empty() {
        properties = entries_of(&default_exec_properties);
    }

    if !additional_properties.is_empty() {
        if properties.is_empty() {
            properties = entries_of(additional_properties);
        } else {
            // Merge the two maps.
            let mut map = JavaHashMap::from_entries(&properties);
            map.put_map_entries(&entries_of(additional_properties));
            properties = map.into_entries();
        }
    }

    let mut platform_builder = Platform::default();
    for (key, value) in &properties {
        platform_builder.properties.push(platform::Property {
            name: internal_to_unicode(key).into_owned(),
            value: internal_to_unicode(value).into_owned(),
        });
    }
    sort_platform_properties(&mut platform_builder);
    Ok(Some(platform_builder))
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:179 PlatformUtils.createFailureDetail
fn create_failure_detail(message: String, detailed_code: spawn_code::Code) -> FailureDetail {
    FailureDetail {
        message,
        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
            code: detailed_code as i32,
            ..Default::default()
        })),
    }
}
