//! `FailActionTest` (src/test/java/com/google/devtools/build/lib/actions/FailActionTest.java).

#![allow(clippy::result_large_err)]

mod analysis_actions_common;

use analysis_actions_common::{TestDirs, null_action_owner};
use blaze_actions::Action;
use blaze_actions::actions::fail_action::FailAction;
use blaze_exec::actions::{ExecutableAction, as_executable};
use blaze_protos::failure_details::fail_action::Code;

struct Fixture {
    _dirs: TestDirs,
    error_message: String,
    an_output: blaze_actions::Artifact,
    outputs: Vec<blaze_actions::Artifact>,
    fail_action: FailAction,
}

// Java: FailActionTest.setUp
// PORT: setUp also registers the action in a MapBasedActionGraph and checks that it is the
// generating action of its output; that graph is blaze_sky_top's, above this crate.
fn set_up() -> Fixture {
    blaze_exec::analysis::actions::register();
    let dirs = TestDirs::new();
    let error_message = "An error just happened.".to_string();
    let an_output = dirs.bin_artifact("foo");
    let outputs = vec![an_output.clone()];
    let fail_action = FailAction::new(
        null_action_owner(),
        outputs.clone(),
        &error_message,
        Code::FailActionUnknown,
    );
    Fixture {
        _dirs: dirs,
        error_message,
        an_output,
        outputs,
        fail_action,
    }
}

#[test]
fn test_executing_it_yields_exception_with_error_message() {
    let f = set_up();
    let ctx = f._dirs.context();
    let executable: &dyn ExecutableAction =
        as_executable(&f.fail_action).expect("FailAction has a registered ExecutableAction");
    let e = executable
        .execute(&ctx)
        .expect_err("FailAction.execute throws ActionExecutionException");
    assert!(
        e.get_message().contains(&f.error_message),
        "{}",
        e.get_message()
    );
}

#[test]
fn test_inputs_are_empty_set() {
    let f = set_up();
    assert!(f.fail_action.get_inputs().to_list().is_empty());
}

#[test]
fn test_retains_its_outputs() {
    let f = set_up();
    let mut actual = f.fail_action.get_outputs();
    let mut expected = f.outputs.clone();
    actual.sort_by(|a, b| a.get_exec_path_string().cmp(b.get_exec_path_string()));
    expected.sort_by(|a, b| a.get_exec_path_string().cmp(b.get_exec_path_string()));
    assert_eq!(actual, expected);
}

#[test]
fn test_primary_output() {
    let f = set_up();
    assert_eq!(f.fail_action.get_primary_output(), f.an_output);
}
