//! Port of `src/test/java/com/google/devtools/build/lib/analysis/config/FragmentRegistryTest.java`.
//!
//! The Java test declares test-only `FragmentOptions` classes (`OptionsA`, `OptionsB`,
//! `MoreOptions`, `EvenMoreOptions`) and fragments requiring them. Fragment classes can be declared
//! here (`FragmentClassInfo` has public fields), but options classes cannot, so only the test
//! that never reads the required options runs.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::{
    Fragment, FragmentClassInfo, FragmentRegistry, InvalidConfigurationException,
};
use blaze_build_config::fragment_options_class::FragmentOptionsClass;

/// `@RequiresOptions(options = OptionsA.class)`: `OptionsA` is a test-only class.
fn options_a() -> Vec<FragmentOptionsClass> {
    panic!("needs test-only FragmentOptions class FragmentRegistryTest.OptionsA")
}

/// `@RequiresOptions(options = OptionsB.class)`: `OptionsB` is a test-only class.
fn options_b() -> Vec<FragmentOptionsClass> {
    panic!("needs test-only FragmentOptions class FragmentRegistryTest.OptionsB")
}

fn never_constructed(
    _options: &BuildOptions,
) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    panic!("FragmentRegistryTest fragments are never constructed")
}

static FRAGMENT_A: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.config.FragmentRegistryTest$FragmentA",
    starlark_name: None,
    required_options: options_a,
    requires_starlark_options: false,
    constructor: never_constructed,
};

static FRAGMENT_B: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.config.FragmentRegistryTest$FragmentB",
    starlark_name: None,
    required_options: options_b,
    requires_starlark_options: false,
    constructor: never_constructed,
};

/// The message of the panic `f` raises.
fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(f).expect_err("expected IllegalArgumentException");
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    }
}

#[test]
#[ignore = "needs test-only FragmentOptions class (FragmentRegistryTest.OptionsA, OptionsB, MoreOptions, EvenMoreOptions)"]
fn creates_registry() {}

#[test]
#[ignore = "needs test-only FragmentOptions class (FragmentRegistryTest.OptionsA, OptionsB, MoreOptions, EvenMoreOptions)"]
fn canonicalizes_order() {}

#[test]
fn all_fragments_must_contain_universal_fragments() {
    let message = panic_message(|| {
        FragmentRegistry::create(
            /* all_fragments= */ vec![&FRAGMENT_A],
            /* universal_fragments= */ vec![&FRAGMENT_B],
            /* additional_options= */ vec![],
        );
    });

    assert!(
        message.starts_with("IllegalArgumentException"),
        "{}",
        message
    );
    assert!(
        message.contains("Missing universally required fragments"),
        "{}",
        message
    );
    assert!(message.contains("FragmentB"), "{}", message);
}
