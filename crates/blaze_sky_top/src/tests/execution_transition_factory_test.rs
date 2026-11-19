//! The `BuildViewTestCase` parts of
//! `src/test/java/com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactoryTest.java`
//! (the other tests of the class are in
//! `crates/blaze_build_config/tests/execution_transition_factory_test.rs`).

use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_skyframe::SkyKey;

use crate::testing::BuildViewTestCase;
use crate::testing::build_view_test_case::parse_label;

/// The panic message of `f` (Java `assertThrows`), or `None` if it returned.
fn panic_message(f: impl FnOnce()) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => None,
        Err(payload) => Some(
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default(),
        ),
    }
}

/// The `@TestParameters` cases of `starlarkExecFlagBadReferences` that load `//test:defs.bzl`
/// (the others are `starlark_exec_flag_bad_references` in blaze_build_config). Java runs each
/// case in a fresh test instance.
///
/// PORT: the harness reports Java's `InvalidConfigurationException` from `useConfiguration` as a
/// panic carrying the exception's message (module documentation of `build_view_test_case.rs`).
// Java: ExecutionTransitionFactoryTest.starlarkExecFlagBadReferences
#[test]
#[ignore = "BLAZE GAP: unported StarlarkExecTransitionLoader.loadStarlarkExecTransition for a .bzl outside @_builtins (blaze_build_config/src/starlark_transition.rs load_starlark_exec_transition)"]
fn starlark_exec_flag_bad_references_loaded_bzl() {
    let cases = [
        (
            "//test:defs.bzl%symbol_doesnt_exist",
            "symbol_doesnt_exist not found in //test:defs.bzl",
        ),
        (
            "//test:file_doesnt_exist.bzl%symbol",
            "'//test:file_doesnt_exist.bzl': no such file",
        ),
        (
            "//test:defs.bzl%not_a_transition",
            "not_a_transition is not a Starlark transition.",
        ),
    ];
    for (cmd_line_ref, expected_error) in cases {
        let mut t = BuildViewTestCase::new();
        t.scratch.file("test/defs.bzl", &["not_a_transition = 4"]);
        t.scratch.file("test/BUILD", &[]);

        let flag = format!("--experimental_exec_config={cmd_line_ref}");
        let e = panic_message(|| t.use_configuration(&[flag.as_str()]))
            .unwrap_or_else(|| panic!("{cmd_line_ref}: expected InvalidConfigurationException"));
        assert!(
            e.contains(expected_error),
            "{cmd_line_ref}: '{e}' does not contain '{expected_error}'"
        );
    }
}

/// `getDirectPrerequisite(target, label)`: the configured target of the dependency `label` of
/// `target`.
///
/// PORT: Java asks the build view for the target's prerequisites; here they are the
/// `ConfiguredTargetKey` edges of the target's node in the graph.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java BuildViewTestCase.getDirectPrerequisite
fn get_direct_prerequisite(
    t: &mut BuildViewTestCase,
    target: &ConfiguredTarget,
    label: &str,
) -> Option<ConfiguredTarget> {
    let label = parse_label(label);
    let key = SkyKey::new(
        ConfiguredTargetKey::builder()
            .set_label(target.get_label())
            .set_configuration_key(target.get_configuration_key())
            .build(),
    );
    let graph = t.skyframe_executor().get_walkable_graph();
    for dep in graph.get_direct_deps(&key) {
        let Some(dep_key) = dep.downcast::<ConfiguredTargetKey>() else {
            continue;
        };
        if dep_key.get_label() != &label {
            continue;
        }
        let value = crate::skyframe_executor::get_configured_target_value(&*graph, &dep)?;
        return Some(value.get_configured_target().clone());
    }
    None
}

// Java: ExecutionTransitionFactoryTest.platformInOutputPathWorksInExecMode
#[test]
fn platform_in_output_path_works_in_exec_mode() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(name = "mock_platform")
"#],
    );
    t.scratch.file(
        "test/lib.bzl",
        &[r#"my_rule = rule(
    implementation = lambda ctx: [],
    attrs = {
        "exec_deps": attr.label_list(cfg = "exec"),
    },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(":lib.bzl", "my_rule")
my_rule(
    name = "parent",
    exec_deps = [":child"]
)
my_rule(name = "child")
"#],
    );

    t.use_configuration(&[
        "--experimental_platform_in_output_dir",
        "--extra_execution_platforms=//platforms:mock_platform",
        "--experimental_override_name_platform_in_output_dir=//platforms:mock_platform=mock_platform_path_string",
    ]);
    let parent = t.get_configured_target("//test:parent").unwrap();
    let child =
        get_direct_prerequisite(&mut t, &parent, "//test:child").expect("getDirectPrerequisite");
    let exec_config = t.get_configuration(&child).expect("configuration");

    assert!(exec_config.is_exec_configuration());
    assert_eq!(
        exec_config.get_output_directory_name(),
        "mock_platform_path_string-opt-exec"
    );
}
