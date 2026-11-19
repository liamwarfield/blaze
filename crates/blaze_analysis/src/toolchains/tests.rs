//! Tests of the toolchain context types.

use blaze_cmdline::Label;
use blaze_packages::rule_class::ToolchainTypeRequirement;
use indexmap::IndexMap;

use crate::configured_target_key::ConfiguredTargetKey;
use crate::toolchains::{
    ExecGroupCollectionBuilder, InvalidExecGroupException, NoMatchingPlatformData,
};

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn key(s: &str) -> ConfiguredTargetKey {
    ConfiguredTargetKey::builder().set_label(label(s)).build()
}

#[test]
fn no_matching_platform_data_format_error() {
    let data = NoMatchingPlatformData {
        toolchain_types: vec![],
        available_execution_platform_keys: vec![key("//p:a"), key("//p:b")],
        target_platform_key: key("//p:t"),
    };
    assert_eq!(
        data.format_error(),
        "Unable to find an execution platform for target platform //p:t from available execution platforms [//p:a, //p:b]"
    );
    let data = NoMatchingPlatformData {
        toolchain_types: vec![ToolchainTypeRequirement::create(label("//t:type"))],
        ..data
    };
    assert_eq!(
        data.format_error(),
        "Unable to find an execution platform for toolchains [//t:type] and target platform //p:t from available execution platforms [//p:a, //p:b]"
    );
}

#[test]
fn exec_group_collection_without_toolchains() {
    let mut raw = IndexMap::new();
    raw.insert("key".to_owned(), "v1".to_owned());
    raw.insert("other.key2".to_owned(), "v2".to_owned());
    let collection = ExecGroupCollectionBuilder::default()
        .build(None, &raw, "//foo:bar")
        .unwrap();
    let mut expected = IndexMap::new();
    expected.insert("key".to_owned(), "v1".to_owned());
    assert_eq!(
        collection.get_exec_properties("default-exec-group"),
        expected
    );
    // Unknown exec groups are only validated when toolchains were resolved.
    assert!(collection.get_exec_properties("other").is_empty());
}

#[test]
fn invalid_exec_group_exception_message() {
    let e = InvalidExecGroupException::new(
        "exec_properties",
        "//foo:bar",
        &["compil"],
        &["compile", "default-exec-group"],
    );
    assert_eq!(
        e.get_message(),
        "Tried to set exec_properties for non-existent exec groups on //foo:bar: compil (did you mean 'compile'?)"
    );
}

// ResolvedToolchainContextTest now lives in crates/blaze_sky_top/src/tests/resolved_toolchain_context_test.rs.
