//! StarlarkSemantics (port in spirit of src/test/java/com/google/devtools/build/lib/packages/
//! semantics/ConsistencyTest.java: defaults match the Java option defaults; flags round-trip).

use std::collections::BTreeMap;

use blaze_starlark::StarlarkSemantics;
use blaze_starlark::semantics::Utf8EnforcementMode;

#[test]
fn check_defaults_match() {
    let s = StarlarkSemantics::default();
    assert_eq!(s, StarlarkSemantics::from_flags(&BTreeMap::new()).unwrap());
    assert!(s.experimental_enable_starlark_set);
    assert!(s.incompatible_always_check_depset_elements);
    assert!(s.incompatible_unambiguous_label_stringification);
    assert!(s.incompatible_disallow_empty_glob);
    assert!(s.experimental_starlark_type_syntax);
    assert!(!s.experimental_starlark_type_checking);
    assert_eq!(s.nested_set_depth_limit, 3500);
    assert_eq!(s.max_computation_steps, 0);
    assert_eq!(s.experimental_builtins_bzl_path, "%bundled%");
    assert_eq!(
        s.incompatible_enforce_starlark_utf8,
        Utf8EnforcementMode::Warning
    );
    assert_eq!(s.experimental_builtins_injection_override, None);
    assert!(!s.allow_recursion);
    assert_eq!(StarlarkSemantics::FLAG_NAMES.len(), 65);
}

#[test]
fn options_to_semantics() {
    let mut flags = BTreeMap::new();
    flags.insert(
        "experimental_enable_starlark_set".to_owned(),
        "false".to_owned(),
    );
    flags.insert("nested_set_depth_limit".to_owned(), "100".to_owned());
    flags.insert(
        "incompatible_disable_transitions_on".to_owned(),
        "b,a,b".to_owned(),
    );
    flags.insert(
        "experimental_cc_skylark_api_enabled_packages".to_owned(),
        "z,y".to_owned(),
    );
    flags.insert(
        "incompatible_enforce_starlark_utf8".to_owned(),
        "error".to_owned(),
    );
    flags.insert(
        "experimental_builtins_injection_override".to_owned(),
        "+x,-y".to_owned(),
    );
    let s = StarlarkSemantics::from_flags(&flags).unwrap();
    assert!(!s.experimental_enable_starlark_set);
    assert_eq!(s.nested_set_depth_limit, 100);
    // Set converter: distinct and sorted; list converter keeps order.
    assert_eq!(s.incompatible_disable_transitions_on, ["a", "b"]);
    assert_eq!(s.experimental_cc_skylark_api_enabled_packages, ["z", "y"]);
    assert_eq!(
        s.incompatible_enforce_starlark_utf8,
        Utf8EnforcementMode::Error
    );
    assert_eq!(
        s.experimental_builtins_injection_override,
        Some(vec!["+x".to_owned(), "-y".to_owned()])
    );
}

#[test]
fn bad_flags() {
    let mut flags = BTreeMap::new();
    flags.insert("no_such_flag".to_owned(), "1".to_owned());
    assert_eq!(
        StarlarkSemantics::from_flags(&flags).unwrap_err().0,
        "Unrecognized option: --no_such_flag"
    );
    let mut flags = BTreeMap::new();
    flags.insert("check_bzl_visibility".to_owned(), "maybe".to_owned());
    assert!(StarlarkSemantics::from_flags(&flags).is_err());
    let mut flags = BTreeMap::new();
    flags.insert("check_bzl_visibility".to_owned(), "no".to_owned());
    assert!(
        !StarlarkSemantics::from_flags(&flags)
            .unwrap()
            .check_bzl_visibility
    );
}
