//! Port of `actions/ExecutableSymlinkActionTest.java` (`SymlinkAction.toExecutable`).

#![allow(dead_code)]

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot,
    PlatformInfoRef, RootType,
};
use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------------------------
// Test utilities (ActionsTestUtil / BuildViewTestCase / ActionTester equivalents)
// ---------------------------------------------------------------------------------------------

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn owner_with_platform(platform: PlatformInfoRef) -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(platform),
        Vec::new(),
        IndexMap::new(),
    )
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    owner_with_platform(empty_platform_info())
}

/// `getBinArtifactWithNoOwner(rootRelativePath)`.
fn get_bin_artifact_with_no_owner(root_relative_path: &str) -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
    Artifact::create_derived_artifact(root, exec_path, ActionLookupKeyRef::new(NullArtifactOwner))
}

/// `Actions.canBeShared`.
fn can_be_shared(ctx: &ActionKeyContext, a: &dyn Action, b: &dyn Action) -> bool {
    if !(a.get_mnemonic() == b.get_mnemonic()
        && a.get_key(ctx, None).unwrap() == b.get_key(ctx, None).unwrap())
    {
        return false;
    }
    // Java: com/google/devtools/build/lib/actions/Actions.java:145 Actions.artifactsEqualWithoutOwner
    fn equal_without_owner(x: &[Artifact], y: &[Artifact]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.equals_without_owner(q))
    }
    if a.is_shareable() && b.is_shareable() {
        return equal_without_owner(
            &a.get_mandatory_inputs().to_list()[..],
            &b.get_mandatory_inputs().to_list()[..],
        ) && equal_without_owner(&a.get_outputs()[..], &b.get_outputs()[..]);
    }
    if a.is_shareable() || b.is_shareable() {
        return false;
    }
    *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
        && a.get_outputs() == b.get_outputs()
}

/// `ActionTester.runTest(attributeClass, factory, actionKeyContext)` for `attribute_count`
/// attributes; the factory gets the flipped attributes as a bit set.
fn action_tester_run_test(
    attribute_count: usize,
    factory: impl Fn(&[bool]) -> Box<dyn Action>,
    ctx: &ActionKeyContext,
) {
    let flags =
        |seed: usize| -> Vec<bool> { (0..attribute_count).map(|i| seed & (1 << i) != 0).collect() };
    let count = 1usize << attribute_count;
    let mut actions = Vec::new();
    for i in 0..count {
        let action = factory(&flags(i));
        // Check that creating the same action twice results in equal actions.
        assert!(can_be_shared(ctx, &*action, &*factory(&flags(i))));
        actions.push(action);
    }
    // Check that the count is correct.
    assert!(can_be_shared(ctx, &*actions[0], &*factory(&flags(count))));
    for i in 0..actions.len() {
        for j in i + 1..actions.len() {
            assert!(
                !can_be_shared(ctx, &*actions[i], &*actions[j]),
                "{} and {}",
                i,
                j
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ExecutableSymlinkActionTest
// ---------------------------------------------------------------------------------------------

/// `createExecutor`: input root `in` and output root `out`, both derived roots of the exec root
/// `/`.
fn roots() -> (ArtifactRoot, ArtifactRoot) {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/"), &fs);
    let input_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["in"]);
    let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
    (input_root, output_root)
}

/// `ActionsTestUtil.createArtifact(root, path)` for a derived root.
fn create_artifact(root: &ArtifactRoot, name: &str) -> Artifact {
    Artifact::create_derived_artifact(
        root.clone(),
        root.get_exec_path().get_relative_str(name),
        ActionLookupKeyRef::new(NullArtifactOwner),
    )
}

#[test]
#[ignore = "needs action execution (ActionExecutionContext, DummyExecutor; wave W6)"]
fn test_simple() {}

#[test]
#[ignore = "needs action execution (ActionExecutionContext, DummyExecutor; wave W6)"]
fn test_fail_if_input_is_not_a_file() {}

#[test]
#[ignore = "needs action execution (ActionExecutionContext, DummyExecutor; wave W6)"]
fn test_fail_if_input_is_not_executable() {}

#[test]
#[ignore = "serialization codecs are not ported"]
fn test_codec() {}

/// The analysis-time part of `testSimple` (the action as constructed before `execute`).
#[test]
fn to_executable_analysis_metadata() {
    let (input_root, output_root) = roots();
    let input = create_artifact(&input_root, "some-file");
    let output = create_artifact(&output_root, "some-output");
    let action = SymlinkAction::to_executable(
        null_action_owner(),
        input.clone(),
        output.clone(),
        Some("progress".to_string()),
    );
    assert_eq!(action.get_mnemonic(), "ExecutableSymlink");
    assert_eq!(*action.get_inputs().to_list(), vec![input.clone()]);
    assert_eq!(action.get_outputs(), vec![output]);
    assert_eq!(action.get_progress_message(None).unwrap(), "progress");
    assert_eq!(
        action.get_input_path(),
        PathFragment::create("in/some-file")
    );
    // Same key as a plain symlink (only the GUID is fingerprinted), as the oracle's
    // ExecutableSymlink and Symlink actions show.
    let ctx = ActionKeyContext::new();
    let plain = SymlinkAction::to_artifact(
        null_action_owner(),
        input,
        create_artifact(&output_root, "other"),
        Some("progress".to_string()),
    );
    assert_eq!(
        action.get_key(&ctx, None).unwrap(),
        plain.get_key(&ctx, None).unwrap()
    );
    assert_eq!(
        action.get_key(&ctx, None).unwrap(),
        "c90914cc1e7ea06f84518cf4f179e63dbd20dad9358a550644494828e4a4ed28"
    );
}
