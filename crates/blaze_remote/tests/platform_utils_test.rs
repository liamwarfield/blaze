//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/platform/PlatformUtilsTest.java`: tests
//! for `PlatformUtils`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::{
    ActionInputRef, FakeOwner, PathMapper, PlatformInfo, ResourceSet, SimpleSpawn,
};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_remote::platform_utils;
use blaze_remote::reapi::{Platform, platform};
use indexmap::IndexMap;

fn platform_options_string() -> String {
    [
        "properties: {",
        " name: \"b\"",
        " value: \"2\"",
        "}",
        "properties: {",
        " name: \"a\"",
        " value: \"1\"",
        "}",
    ]
    .join("\n")
}

// Java mutates the defaults instance in place.
#[allow(clippy::field_reassign_with_default)]
fn remote_options() -> RemoteOptions {
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_default_exec_properties = vec![
        ("b".to_string(), "2".to_string()),
        ("a".to_string(), "1".to_string()),
    ];
    remote_options
}

/// `new SpawnBuilder("dummy")` with `withPlatform(platform)` and
/// `withCombinedExecProperties(execProperties)`, then `build()`.
// Java: src/test/java/com/google/devtools/build/lib/exec/util/SpawnBuilder.java SpawnBuilder.build
fn spawn(platform: Option<PlatformInfo>, exec_properties: &[(&str, &str)]) -> SimpleSpawn {
    let mut owner = FakeOwner::new(
        "Mnemonic",
        Some("progress message"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    owner.owner.exec_properties = exec_properties
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    owner.platform = platform;
    SimpleSpawn::new(
        Arc::new(owner),
        vec!["dummy".to_string()],
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::<ActionInputRef>::stable_order().build(),
        NestedSetBuilder::<ActionInputRef>::stable_order().build(),
        Vec::new(),
        None,
        Arc::new(ResourceSet::zero()),
        PathMapper::NOOP,
    )
}

/// `Platform.Property.newBuilder().setName(name).setValue(value)`.
fn property(name: &str, value: &str) -> platform::Property {
    platform::Property {
        name: name.to_string(),
        value: value.to_string(),
    }
}

#[test]
fn test_parse_platform_legacy_options() {
    let expected = Platform {
        properties: vec![property("a", "1"), property("b", "2")],
    };
    // PlatformInfo.builder().setRemoteExecutionProperties(platformOptionsString()).build(); the
    // builder's label is unset in Java, which this PlatformInfo cannot express.
    let platform = PlatformInfo {
        label: Label::parse_canonical_unchecked("//dummy:platform"),
        exec_properties: IndexMap::new(),
        remote_execution_properties: platform_options_string(),
        os_constraint: None,
    };
    let s = spawn(Some(platform), &[]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, None).unwrap(),
        Some(expected)
    );
}

#[test]
fn test_parse_platform_sorts_properties() {
    let expected = Platform {
        properties: vec![property("a", "1"), property("b", "2")],
    };
    let s = spawn(None, &[]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, Some(&remote_options())).unwrap(),
        Some(expected)
    );
}

#[test]
fn test_parse_platform_handles_null() {
    let s = spawn(None, &[]);
    assert_eq!(platform_utils::get_platform_proto(&s, None).unwrap(), None);
}

#[test]
fn test_parse_platform_sorts_properties_exec_properties() {
    // execProperties are chosen even if there are remoteOptions
    let map = [("aa", "99"), ("zz", "66"), ("dd", "11")];
    let s = spawn(None, &map);

    let expected = Platform {
        properties: vec![
            property("aa", "99"),
            property("dd", "11"),
            property("zz", "66"),
        ],
    };
    // execProperties are sorted by key
    assert_eq!(
        platform_utils::get_platform_proto(&s, None).unwrap(),
        Some(expected)
    );
}

#[test]
fn get_platform_proto_merge_target_exec_properties_with_platform() {
    let spawn = spawn(None, &[("c", "3")]);
    let expected = Platform {
        properties: vec![property("a", "1"), property("b", "2"), property("c", "3")],
    };
    assert_eq!(
        platform_utils::get_platform_proto(&spawn, Some(&remote_options())).unwrap(),
        Some(expected)
    );
}

#[test]
fn get_platform_proto_target_exec_properties_conflict_with_platform_override() {
    let spawn = spawn(None, &[("b", "3")]);
    let expected = Platform {
        properties: vec![property("a", "1"), property("b", "3")],
    };
    assert_eq!(
        platform_utils::get_platform_proto(&spawn, Some(&remote_options())).unwrap(),
        Some(expected)
    );
}

// The tests below are not in PlatformUtilsTest.java. Their expected values come from the oracle:
// `scripts/remote/platform-probe oracle td-o1 <case>` (Bazel 9.3.2 against Bazel's remote
// worker; the Platform of the Action and Command it sent, and its ERROR line). Each names its
// probe case.

/// The execution platform `//:p` with these `remote_execution_properties` and
/// `exec_properties`.
fn platform_p(remote_execution_properties: &str, exec_properties: &[(&str, &str)]) -> PlatformInfo {
    PlatformInfo {
        label: Label::parse_canonical_unchecked("//:p"),
        exec_properties: exec_properties
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        remote_execution_properties: remote_execution_properties.to_string(),
        os_constraint: None,
    }
}

// Java: --remote_default_exec_properties=k=v
#[allow(clippy::field_reassign_with_default)]
fn remote_options_k_v() -> RemoteOptions {
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_default_exec_properties = vec![("k".to_string(), "v".to_string())];
    remote_options
}

/// Probe case `unsorted`: the parsed properties are sorted by name.
#[test]
fn oracle_remote_execution_properties_are_sorted() {
    let text = "properties { name: \"zz\" value: \"1\" }\n\
                properties { name: \"aa\" value: \"2\" }\n\
                properties { name: \"mm\" value: \"3\" }\n";
    let s = spawn(Some(platform_p(text, &[])), &[]);
    let expected = Platform {
        properties: vec![
            property("aa", "2"),
            property("mm", "3"),
            property("zz", "1"),
        ],
    };
    assert_eq!(
        platform_utils::get_platform_proto(&s, None).unwrap(),
        Some(expected)
    );
}

/// Probe case `duplicate-name`: a repeated singular field keeps the last value, and a later
/// property with the same name replaces the earlier one (`properties.put`).
#[test]
fn oracle_remote_execution_properties_duplicate_names() {
    let text = "properties { name: \"a\" name: \"b\" value: \"1\" }\n\
                properties { name: \"b\" value: \"2\" }\n";
    let s = spawn(Some(platform_p(text, &[])), &[]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, None).unwrap(),
        Some(Platform {
            properties: vec![property("b", "2")],
        })
    );
}

/// Probe case `broken`: the ParseException becomes a UserExecException with
/// INVALID_REMOTE_EXECUTION_PROPERTIES. Oracle: `ERROR: <ws>/BUILD:8:6: Probe t.out failed:
/// Failed to parse remote_execution_properties from platform //:p: 1:19: Expected ":".`
#[test]
fn oracle_remote_execution_properties_parse_error() {
    let s = spawn(Some(platform_p("properties { name \"a\" }\n", &[])), &[]);
    let e = platform_utils::get_platform_proto(&s, None).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Failed to parse remote_execution_properties from platform //:p: 1:19: Expected \":\"."
    );
    let detail = e.get_failure_detail(&e.get_message()).unwrap();
    assert_eq!(
        detail.category,
        Some(
            blaze_protos::failure_details::failure_detail::Category::Spawn(
                blaze_protos::failure_details::Spawn {
                    code:
                        blaze_protos::failure_details::spawn::Code::InvalidRemoteExecutionProperties
                            as i32,
                    ..Default::default()
                }
            )
        )
    );
}

/// Probe case `target-exec-properties`: a target's exec_properties win over the platform's text
/// (the text is not read at all).
#[test]
fn oracle_target_exec_properties_hide_remote_execution_properties() {
    let text = "properties { name: \"from-text\" value: \"1\" }\n";
    let s = spawn(Some(platform_p(text, &[])), &[("from-target", "2")]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, None).unwrap(),
        Some(Platform {
            properties: vec![property("from-target", "2")],
        })
    );
}

/// Probe case `defaults-with-text`: --remote_default_exec_properties is not applied when the
/// platform's text gives properties.
#[test]
fn oracle_defaults_not_applied_with_remote_execution_properties() {
    let text = "properties { name: \"from-text\" value: \"1\" }\n";
    let s = spawn(Some(platform_p(text, &[])), &[]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, Some(&remote_options_k_v())).unwrap(),
        Some(Platform {
            properties: vec![property("from-text", "1")],
        })
    );
}

/// Probe case `defaults-with-target`: the defaults are merged under a target's exec_properties
/// when the platform has no exec_properties.
#[test]
fn oracle_defaults_merged_with_target_exec_properties() {
    let s = spawn(Some(platform_p("", &[])), &[("from-target", "2")]);
    assert_eq!(
        platform_utils::get_platform_proto(&s, Some(&remote_options_k_v())).unwrap(),
        Some(Platform {
            properties: vec![property("from-target", "2"), property("k", "v")],
        })
    );
}

/// Probe case `empty-text` (and `parent-inherit`, whose child has `remote_execution_properties`
/// "" from the attribute's default): no platform at all.
#[test]
fn oracle_empty_remote_execution_properties_give_no_platform() {
    let s = spawn(Some(platform_p("", &[])), &[]);
    assert_eq!(platform_utils::get_platform_proto(&s, None).unwrap(), None);
}
