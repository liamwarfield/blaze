#![cfg(feature = "wip")]
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
#[ignore = "needs a protobuf text-format parser (TextFormat.getParser().merge) for PlatformInfo.remoteExecutionProperties; PlatformUtils.getPlatformProto(remote_execution_properties) is unported!"]
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
