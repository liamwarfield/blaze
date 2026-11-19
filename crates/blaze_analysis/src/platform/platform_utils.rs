//! Port of `analysis/platform/PlatformUtils.java`: utilities for accessing platform properties.
//!
//! PORT: `getPlatformProto(Spawn, RemoteOptions, ...)` is ported in
//! `blaze_exec::exec::spawn_log_context` next to its only caller.

use blaze_util::string_encoding::internal_to_unicode;
use indexmap::IndexMap;

/// Java `Platform` proto (`build.bazel.remote.execution.v2.Platform`): its sorted properties.
pub type PlatformProto = Vec<(String, String)>;

// Java: com/google/devtools/build/lib/analysis/platform/PlatformUtils.java:43 PlatformUtils.sortPlatformProperties
fn sort_platform_properties(properties: &mut PlatformProto) {
    // Java: Ordering.from(Comparator.comparing(Platform.Property::getName)), a stable sort by
    // `String.compareTo` of the (Unicode) names: UTF-16 code unit order.
    properties.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
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
        builder.push((
            internal_to_unicode(key).into_owned(),
            internal_to_unicode(value).into_owned(),
        ));
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

    /// The proto holds Unicode (`internalToUnicode`): the internal string of "é🐷" becomes "é🐷",
    /// a raw byte FF becomes U+FFFD; names sort by UTF-16 units (U+FFFD after U+1F437's high
    /// surrogate D83D).
    #[test]
    fn build_platform_proto_converts_to_unicode() {
        let i = |s: &[u8]| blaze_util::string_encoding::latin1_decode(s).into_owned();
        let mut props = IndexMap::new();
        props.insert(i(b"k\xff"), i("\u{e9}\u{1F437}".as_bytes()));
        props.insert(i("k\u{1F437}".as_bytes()), i(b"\xff"));
        assert_eq!(
            build_platform_proto(&props).unwrap(),
            vec![
                ("k\u{1F437}".to_owned(), "\u{FFFD}".to_owned()),
                ("k\u{FFFD}".to_owned(), "\u{e9}\u{1F437}".to_owned()),
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
