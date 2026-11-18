//! Port of `src/test/java/com/google/devtools/build/lib/analysis/actions/PathMappersTest.java`.

use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_vfs::PathFragment;

#[test]
#[ignore = "needs BuildViewTestCase (java_library analysis and spawn creation with StrippingPathMapper)"]
fn java_library_with_javacopts() {}

#[test]
#[ignore = "needs BuildViewTestCase (Starlark rule analysis) and StrippingPathMapper"]
fn starlark_rule_opted_in_via_execution_requirements() {}

#[test]
#[ignore = "needs BuildViewTestCase (Starlark rule analysis) and StrippingPathMapper"]
fn starlark_rule_opted_in_via_modify_execution_info() {}

#[test]
#[ignore = "needs BuildViewTestCase (Starlark rule analysis) and StrippingPathMapper"]
fn starlark_rule_string_executable_path() {}

#[test]
fn for_action_key() {
    let path_mapper = blaze_actions::actions::path_mapper::for_action_key(OutputPathsMode::STRIP);
    assert!(!path_mapper.is_noop());
    assert_eq!(
        path_mapper.map(&PathFragment::create("pkg/file")),
        PathFragment::create("pkg/file")
    );
    assert_eq!(
        path_mapper.map(&PathFragment::create(
            "bazel-out/k8-fastbuild-ST-12345/bin/pkg/file"
        )),
        PathFragment::create("bazel-out/pm-k8-fastbuild-ST-12345/bin/pkg/file")
    );
}

#[test]
#[ignore = "needs BuildViewTestCase (Starlark rule analysis, execution) and StrippingPathMapper"]
fn starlark_rule_inputs_outputs_collision() {}
