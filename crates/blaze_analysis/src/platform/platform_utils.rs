//! Port of `analysis/platform/PlatformUtils.java`: utilities for accessing platform properties.
//!
//! PORT: `getPlatformProto(Spawn, RemoteOptions, ...)` is ported in
//! `blaze_exec::exec::spawn_log_context` next to its only caller.

use indexmap::IndexMap;

/// Java `Platform` proto (`build.bazel.remote.execution.v2.Platform`): its sorted properties.
pub type PlatformProto = Vec<(String, String)>;

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:43 PlatformUtils.sortPlatformProperties
fn sort_platform_properties(properties: &mut PlatformProto) {
    // Java: Ordering.from(Comparator.comparing(Platform.Property::getName)), a stable sort.
    properties.sort_by(|a, b| a.0.cmp(&b.0));
}

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:52 PlatformUtils.buildPlatformProto
pub fn build_platform_proto(
    execution_properties: &IndexMap<String, String>,
) -> Option<PlatformProto> {
    if execution_properties.is_empty() {
        return None;
    }
    let mut builder: PlatformProto = Vec::new();
    for (key, value) in execution_properties {
        // PORT: StringEncoding identity (internalToUnicode).
        builder.push((key.clone(), value.clone()));
    }
    sort_platform_properties(&mut builder);
    Some(builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_platform_proto_sorts_properties() {
        let mut props = IndexMap::new();
        props.insert("zz".to_owned(), "1".to_owned());
        props.insert("aa".to_owned(), "2".to_owned());
        assert_eq!(
            build_platform_proto(&props).unwrap(),
            vec![
                ("aa".to_owned(), "2".to_owned()),
                ("zz".to_owned(), "1".to_owned())
            ]
        );
    }

    #[test]
    fn build_platform_proto_empty() {
        assert_eq!(build_platform_proto(&IndexMap::new()), None);
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java testParsePlatformLegacyOptions
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn test_parse_platform_legacy_options() {}

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java testParsePlatformSortsProperties
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn test_parse_platform_sorts_properties() {}

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java testParsePlatformHandlesNull
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn test_parse_platform_handles_null() {}

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java testParsePlatformSortsProperties_execProperties
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn test_parse_platform_sorts_properties_exec_properties() {}

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java getPlatformProto_mergeTargetExecPropertiesWithPlatform
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn get_platform_proto_merge_target_exec_properties_with_platform() {}

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java getPlatformProto_targetExecPropertiesConflictWithPlatform_override
    #[test]
    #[ignore = "getPlatformProto is ported in blaze_exec (needs Spawn)"]
    fn get_platform_proto_target_exec_properties_conflict_with_platform_override() {}
}
