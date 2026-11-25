//! Change pruning of the configuration keys (package I2): `Key::values_equal` against the Java
//! value classes' `equals`, observed through the evaluator.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_build_config::platform::{OpaquePlatformInfo, PlatformValue, PlatformValueKey};
use blaze_cmdline::Label;
use blaze_skyframe::testing::parent_rebuilt;

fn key() -> PlatformValueKey {
    PlatformValueKey {
        platform_label: Label::parse_canonical_unchecked("//:p"),
        flag_alias_mappings: vec![],
    }
}

/// A value whose `PlatformInfo` stands in as a string (any `PartialEq` type works: the value
/// keeps its type's equality).
fn platform(info: Option<&str>) -> PlatformValue {
    PlatformValue {
        label: Label::parse_canonical_unchecked("//:p"),
        constraints: BTreeMap::new(),
        has_parsed_flags: false,
        parsed_flags: None,
        platform_info: info.map(|i| OpaquePlatformInfo::new(Arc::new(i.to_string()))),
    }
}

/// Java: `PlatformValue` is a record of the `PlatformInfo` (value `equals`) and the parsed
/// flags: a platform recomputed into an equal, newly made `PlatformInfo` prunes.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformValue.java:35 PlatformValue
#[test]
fn recomputed_equal_platform_prunes_parent() {
    assert!(!parent_rebuilt(
        key(),
        platform(Some("info")),
        platform(Some("info"))
    ));
    assert!(parent_rebuilt(
        key(),
        platform(Some("info")),
        platform(Some("other"))
    ));
}

/// A value recorded from the oracle has no `PlatformInfo`: never equal (less pruning than
/// Java, never more).
#[test]
fn platform_without_platform_info_never_prunes() {
    assert!(parent_rebuilt(key(), platform(None), platform(None)));
}

/// `OpaquePlatformInfo`'s `==` stays "the same object"; `java_equals` is the type's equality.
#[test]
fn opaque_platform_info_equality() {
    let a = OpaquePlatformInfo::new(Arc::new(1u32));
    let b = OpaquePlatformInfo::new(Arc::new(1u32));
    assert!(a != b);
    assert!(a == a.clone());
    assert!(a.java_equals(&b));
    assert!(!a.java_equals(&OpaquePlatformInfo::new(Arc::new(2u32))));
    assert!(!a.java_equals(&OpaquePlatformInfo::new(Arc::new(1u64))));
}
