//! Ports of `analysis/actions/FileWriteActionTest.java`, `FileWriteActionTestCase.java` and
//! `AbstractFileWriteActionTest.java`, plus action keys checked against the oracle.

#![allow(dead_code)]

mod oracle_common;

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::abstract_file_write_action::AbstractFileWriteAction;
use blaze_actions::analysis::actions::file_write_action::{
    Compression, FileWriteAction, OnDemandString,
};
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

/// `java.util.Random`.
struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5DEECE66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.seed >> (48 - bits)) as i32
    }

    /// `nextInt(bound)` for a power-of-two bound.
    fn next_int_pow2(&mut self, bound: i32) -> i32 {
        assert!(bound > 0 && bound & (bound - 1) == 0);
        ((bound as i64 * self.next(31) as i64) >> 31) as i32
    }
}

// ---------------------------------------------------------------------------------------------
// FileWriteActionTestCase
// ---------------------------------------------------------------------------------------------

// Java: FileWriteActionTest.createAction
fn create_action(
    action_owner: ActionOwner,
    output_artifact: Artifact,
    data: &str,
    make_executable: bool,
) -> FileWriteAction {
    FileWriteAction::create(
        action_owner,
        output_artifact,
        data,
        make_executable,
        Compression::DISALLOW,
    )
}

// Java: FileWriteActionTestCase.createAction (@Before)
fn setup() -> (FileWriteAction, Artifact) {
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
    assert!(!AbstractFileWriteAction::make_executable(&action));
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
    assert!(AbstractFileWriteAction::make_executable(&action));
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

// ---------------------------------------------------------------------------------------------
// FileWriteActionTest
// ---------------------------------------------------------------------------------------------

#[test]
fn test_file_write_action_with_short_string() {
    let output_artifact = get_bin_artifact_with_no_owner("destination.txt");
    let contents = "Hello world";
    let action = FileWriteAction::create(
        null_action_owner(),
        output_artifact,
        contents,
        /* make_executable= */ false,
        Compression::DISALLOW,
    );
    assert_eq!(action.get_file_contents(), contents);
}

struct BackedOnDemandString {
    backing: String,
    forced: AtomicUsize,
}

impl OnDemandString for BackedOnDemandString {
    fn to_string(&self) -> String {
        self.forced.fetch_add(1, Ordering::SeqCst);
        self.backing.clone()
    }
}

#[test]
fn test_file_write_action_with_lazy_string() {
    let output_artifact = get_bin_artifact_with_no_owner("destination.txt");
    let backing_string = "Hello world".to_string();
    let contents = Arc::new(BackedOnDemandString {
        backing: backing_string.clone(),
        forced: AtomicUsize::new(0),
    });
    let action = FileWriteAction::create(
        null_action_owner(),
        output_artifact,
        blaze_actions::analysis::actions::file_write_action::CharSequence::OnDemand(contents),
        /* make_executable= */ false,
        Compression::DISALLOW,
    );
    assert_eq!(action.get_file_contents(), backing_string);
}

/// Returns a string filled with (deterministic) random characters to get a string that won't
/// compress to a tiny size.
fn generate_long_random_string() -> String {
    let mut sb = String::new();
    let mut random = JavaRandom::new(0);
    for _ in 0..16 * 1024 {
        let c = random.next_int_pow2(128) as u8 as char;
        sb.push(c);
    }
    sb
}

#[test]
fn test_file_write_action_with_long_string_and_compression() {
    let output_artifact = get_bin_artifact_with_no_owner("destination.txt");
    let contents = generate_long_random_string();
    let action = FileWriteAction::create(
        null_action_owner(),
        output_artifact,
        contents.clone(),
        /* make_executable= */ false,
        Compression::ALLOW,
    );
    assert!(action.is_compressed());
    assert_eq!(action.get_file_contents(), contents);
    assert_eq!(written_bytes(&action), contents.as_bytes());
}

#[test]
fn test_file_write_action_with_compression_does_not_force_lazy_string() {
    let output_artifact = get_bin_artifact_with_no_owner("destination.txt");
    let backing_contents = generate_long_random_string();
    let contents = Arc::new(BackedOnDemandString {
        backing: backing_contents.clone(),
        forced: AtomicUsize::new(0),
    });
    let action = FileWriteAction::create(
        null_action_owner(),
        output_artifact,
        blaze_actions::analysis::actions::file_write_action::CharSequence::OnDemand(
            contents.clone(),
        ),
        /* make_executable= */ false,
        Compression::ALLOW,
    );

    // The string should only be forced once we actually read it, not when the action is
    // constructed.
    assert_eq!(contents.forced.load(Ordering::SeqCst), 0);
    assert_eq!(action.get_file_contents(), backing_contents);
    assert_eq!(contents.forced.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------------------------
// AbstractFileWriteActionTest
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; the test is kept there \
            (tests/analysis_actions_execute_test.rs), ignored for: needs an \
            AbstractFileWriteAction subclass overriding afterWrite"]
fn execute_action_successful_write_calls_after_write() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; the test is kept there \
            (tests/analysis_actions_execute_test.rs), ignored for: needs an \
            AbstractFileWriteAction subclass overriding afterWrite"]
fn execute_action_failed_write_does_not_call_after_write() {}

// ---------------------------------------------------------------------------------------------
// Oracle checks (not in the Java tests).
//
// Expected keys come from `bazel-9.3.2 aquery --output=jsonproto --include_file_write_contents
// //:g` on a workspace whose rule does, for target `g`:
//   ctx.actions.write(<g_long.txt>, "".join(["line %d of a long file write action\n" % i
//       for i in range(40)]))
//   ctx.actions.write(<g_long_exec.sh>, "#!/bin/sh\n" + "".join(["echo %d\n" % i
//       for i in range(100)]), is_executable = True)
//   ctx.actions.write(<g_short.txt>, "héllo wörld ☃\n")
// in the k8-fastbuild configuration on this host (execution platform @@platforms//host:host).
//
// The same writes are targets `ascii_write_long`, `ascii_write_long_exec` and `write_plain` of
// the probe workspace `tests/data/non_ascii_probe/workspace`; `scripts/actions-probe/run`
// re-records the oracle's keys (`tests/data/non_ascii_probe/aquery.jsonproto`), and
// `non_ascii_action_keys.rs` checks them from the recording. The empty-contents key is that of
// the recording's empty `FileWrite` actions.
// ---------------------------------------------------------------------------------------------

fn oracle_key(contents: String, make_executable: bool) -> (String, bool) {
    let action = FileWriteAction::create_from_context(
        owner_with_platform(oracle_common::host_platform()),
        get_bin_artifact_with_no_owner("out.txt"),
        contents,
        make_executable,
    );
    (
        action.get_key(&ActionKeyContext::new(), None).unwrap(),
        action.is_compressed(),
    )
}

#[test]
fn oracle_compressed_file_write_key() {
    let contents: String = (0..40)
        .map(|i| format!("line {} of a long file write action\n", i))
        .collect();
    assert_eq!(
        oracle_key(contents, false),
        (
            "2893daebd9cefc46f901d68128d43c5b1fee6cc9c6044f61a3b1e9c962ed82be".to_string(),
            true
        )
    );
}

#[test]
fn oracle_compressed_executable_file_write_key() {
    let contents: String = std::iter::once("#!/bin/sh\n".to_string())
        .chain((0..100).map(|i| format!("echo {}\n", i)))
        .collect();
    assert_eq!(
        oracle_key(contents, true),
        (
            "3093f652632829ed6969219eb29a82c9f00bc23871e05535b3693d8cc946b1d6".to_string(),
            true
        )
    );
}

#[test]
fn oracle_non_ascii_file_write_key() {
    assert_eq!(
        oracle_key("h\u{e9}llo w\u{f6}rld \u{2603}\n".to_string(), false),
        (
            "4ec2facf79da645c562d11098894960e3d241760c39e539c4a68f032c22d0c41".to_string(),
            false
        )
    );
}

#[test]
fn oracle_empty_file_write_key() {
    // The `.dwp` FileWrite actions of brotli and re2 (empty contents).
    assert_eq!(
        oracle_key(String::new(), false),
        (
            "4422f442ec223c47c6b503a3d25860acc406026bf372721c6c7147e09aff2452".to_string(),
            false
        )
    );
}

#[test]
fn progress_message_and_mnemonic() {
    let action = FileWriteAction::create_from_context_with_mnemonic(
        null_action_owner(),
        get_bin_artifact_with_no_owner("x.sh"),
        "echo",
        true,
        "MyWrite",
    );
    assert_eq!(action.get_mnemonic(), "MyWrite");
    assert_eq!(
        action.get_progress_message(None).unwrap(),
        "Writing script x.sh"
    );
    assert!(action.as_file_contents_provider().is_some());
}
