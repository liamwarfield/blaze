//! Port of `analysis/actions/BinaryFileWriteActionTest.java` (a `FileWriteActionTestCase`).

#![allow(dead_code)]

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::abstract_file_write_action::AbstractFileWriteAction;
use blaze_actions::analysis::actions::binary_file_write_action::{BinaryFileWriteAction, wrap};
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
// FileWriteActionTestCase with BinaryFileWriteActionTest.createAction
// ---------------------------------------------------------------------------------------------

// Java: BinaryFileWriteActionTest.createAction
fn create_action(
    action_owner: ActionOwner,
    output_artifact: Artifact,
    data: &str,
    make_executable: bool,
) -> BinaryFileWriteAction {
    BinaryFileWriteAction::new(
        action_owner,
        output_artifact,
        wrap(data.as_bytes().to_vec()),
        make_executable,
    )
}

// Java: FileWriteActionTestCase.createAction (@Before)
fn setup() -> (BinaryFileWriteAction, Artifact) {
    let output_artifact = get_bin_artifact_with_no_owner("destination.txt");
    let action = create_action(
        null_action_owner(),
        output_artifact.clone(),
        "Hello World",
        false,
    );
    (action, output_artifact)
}

/// What `execute` writes: the bytes of the deterministic writer.
fn written_bytes(action: &dyn AbstractFileWriteAction) -> Vec<u8> {
    action
        .new_deterministic_writer()
        .unwrap()
        .get_bytes()
        .unwrap()
}

#[test]
fn test_no_inputs() {
    // Java: FileWriteActionTestCase.checkNoInputsByDefault
    let (action, _) = setup();
    assert!(action.get_inputs().to_list().is_empty());
    assert!(action.get_primary_input().is_none());
}

#[test]
fn test_destination_artifact_is_output() {
    // Java: FileWriteActionTestCase.checkDestinationArtifactIsOutput
    let (action, output_artifact) = setup();
    assert_eq!(action.get_outputs(), vec![output_artifact.clone()]);
    assert_eq!(action.get_primary_output(), output_artifact);
}

// PORT: `execute` is wave W6. These check what `execute` hands to `FileWriteActionContext`: the
// deterministic writer's bytes and the executable bit.
#[test]
fn test_can_write_non_executable_file() {
    // Java: FileWriteActionTestCase.checkCanWriteNonExecutableFile
    let (action, _) = setup();
    assert_eq!(
        String::from_utf8(written_bytes(&action)).unwrap(),
        "Hello World"
    );
    assert!(!action.make_executable());
}

#[test]
fn test_can_write_executable_file() {
    // Java: FileWriteActionTestCase.checkCanWriteExecutableFile
    let output_artifact = get_bin_artifact_with_no_owner("hello");
    let action = create_action(
        null_action_owner(),
        output_artifact,
        "echo 'Hello World'",
        true,
    );
    assert_eq!(
        String::from_utf8(written_bytes(&action)).unwrap(),
        "echo 'Hello World'"
    );
    assert!(action.make_executable());
}

#[test]
fn test_computes_consistent_keys() {
    // Java: FileWriteActionTestCase.checkComputesConsistentKeys (DATA, MAKE_EXECUTABLE)
    let (_, output_artifact) = setup();
    let ctx = ActionKeyContext::new();
    action_tester_run_test(
        2,
        |flip| {
            Box::new(create_action(
                null_action_owner(),
                output_artifact.clone(),
                if flip[0] { "0" } else { "1" },
                flip[1],
            ))
        },
        &ctx,
    );
}

#[test]
fn mnemonic_and_progress_message() {
    let (action, _) = setup();
    assert_eq!(action.get_mnemonic(), "FileWrite");
    assert_eq!(
        action.get_progress_message(None).unwrap(),
        "Writing file destination.txt"
    );
}
