//! Port of `analysis/actions/SymlinkActionTest.java`, plus action keys of `SymlinkAction`,
//! `SolibSymlinkAction` and `UnresolvedSymlinkAction` checked against the oracle.

#![allow(dead_code)]

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::ArtifactOwner;
use blaze_actions::SpecialArtifactType;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::starlark::unresolved_symlink_action::{
    SymlinkTargetType, UnresolvedSymlinkAction,
};
use blaze_actions::rules::cpp::solib_symlink_action::{
    get_dynamic_library_soname, get_dynamic_library_symlink_internal,
    get_dynamic_library_symlink_path,
};
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot,
    PlatformInfoRef, RootType,
};
use blaze_cmdline::Label;
use blaze_events::location::Location;
use blaze_vfs::Root;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

mod oracle_common;

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
// SymlinkActionTest
// ---------------------------------------------------------------------------------------------

/// `getSourceArtifact(name)`: a source artifact in the main workspace.
fn get_source_artifact(name: &str) -> Artifact {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &fs,
    )));
    Artifact::new_source_artifact(root, PathFragment::create(name), ArtifactOwner::NullOwner)
}

/// The analysis-time assertions of `runSymlinkAction` (before `execute`).
fn check_symlink_action(action: &SymlinkAction, inputs: &[Artifact], output: &Artifact) {
    assert_eq!(*action.get_inputs().to_list(), inputs.to_vec());
    assert_eq!(action.get_outputs(), vec![output.clone()]);
    assert_eq!(
        action.get_progress_message(None).as_deref(),
        Some("Test symlink action")
    );
}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; ported there: \
            tests/analysis_actions_execute_test.rs test_symlink_to_source_file_use_exec_root_for_source_false and ..._true"]
fn test_symlink_to_source_file() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; ported there: \
            tests/analysis_actions_execute_test.rs test_symlink_to_source_directory_use_exec_root_for_source_false and ..._true"]
fn test_symlink_to_source_directory() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; ported there: \
            tests/analysis_actions_execute_test.rs test_symlink_to_output_file"]
fn test_symlink_to_output_file() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; ported there: \
            tests/analysis_actions_execute_test.rs test_symlink_to_output_tree"]
fn test_symlink_to_output_tree() {}

#[test]
#[ignore = "execution is in blaze_exec, which depends on this crate; ported there: \
            tests/analysis_actions_execute_test.rs test_symlink_to_absolute_path"]
fn test_symlink_to_absolute_path() {}

#[test]
#[ignore = "serialization codecs are not ported"]
fn test_codec() {}

// The analysis-time halves of the tests above (`runSymlinkAction` before `execute`).

#[test]
fn run_symlink_action_to_source_file() {
    for use_exec_root_for_source in [false, true] {
        let input_artifact = get_source_artifact("input");
        let output_artifact = get_bin_artifact_with_no_owner("output");
        let action = SymlinkAction::to_artifact_with_exec_root(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
            use_exec_root_for_source,
        );
        check_symlink_action(
            &action,
            std::slice::from_ref(&input_artifact),
            &output_artifact,
        );
        assert_eq!(action.get_input_path(), PathFragment::create("input"));
        assert_eq!(action.get_mnemonic(), "Symlink");
        assert_eq!(
            action.describe_key().unwrap(),
            if use_exec_root_for_source {
                "GUID: 7f4fab4d-d0a7-4f0f-8649-1d0337a21fee\ninputPath: input\n"
            } else {
                "GUID: 7f4fab4d-d0a7-4f0f-8649-1d0337a21fee\ninputPath: null\n"
            }
        );
        assert!(!action.is_volatile());
        assert!(!action.execute_unconditionally());
    }
}

#[test]
fn run_symlink_action_to_output_tree() {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let tree = |name: &str| {
        Artifact::create_special_artifact(
            root.clone(),
            root.get_exec_path().get_relative_str(name),
            ActionLookupKeyRef::new(NullArtifactOwner),
            SpecialArtifactType::TREE,
        )
    };
    let input_artifact = tree("input");
    let output_artifact = tree("output");
    let action = SymlinkAction::to_artifact(
        null_action_owner(),
        input_artifact.clone(),
        output_artifact.clone(),
        Some("Test symlink action".to_string()),
    );
    check_symlink_action(&action, &[input_artifact], &output_artifact);
}

#[test]
fn run_symlink_action_to_absolute_path() {
    let output_artifact = get_bin_artifact_with_no_owner("output");
    let action = SymlinkAction::to_absolute_path(
        null_action_owner(),
        PathFragment::create("/some/path"),
        output_artifact.clone(),
        Some("Test symlink action".to_string()),
    );
    check_symlink_action(&action, &[], &output_artifact);
    assert_eq!(action.get_input_path(), PathFragment::create("/some/path"));
    assert!(action.is_volatile());
    assert!(action.execute_unconditionally());
}

#[test]
fn symlink_key_depends_on_input_path_only() {
    let ctx = ActionKeyContext::new();
    let key = |use_exec_root_for_source: bool, name: &str| {
        SymlinkAction::to_artifact_with_exec_root(
            null_action_owner(),
            get_source_artifact(name),
            get_bin_artifact_with_no_owner("output"),
            Some("Test symlink action".to_string()),
            use_exec_root_for_source,
        )
        .get_key(&ctx, None)
        .unwrap()
    };
    assert_eq!(key(false, "a"), key(false, "b"));
    assert_ne!(key(true, "a"), key(true, "b"));
    assert_ne!(key(false, "a"), key(true, "a"));
}

// ---------------------------------------------------------------------------------------------
// Oracle checks (not in the Java tests).
// ---------------------------------------------------------------------------------------------

/// Symlink and ExecutableSymlink actions of the recordings: `toArtifact`/`toExecutable` without
/// an input path, so the key is the GUID and `EMPTY_PLATFORM_INFO`.
#[test]
fn oracle_symlink_keys() {
    let mut checked = 0;
    for project in ["brotli", "re2"] {
        let Some(rec) = oracle_common::Recording::load(project) else {
            eprintln!("no oracle recording; skipped");
            return;
        };
        for a in rec.actions().iter() {
            let mnemonic = a["mnemonic"].as_str().unwrap();
            if mnemonic != "Symlink" && mnemonic != "ExecutableSymlink" {
                continue;
            }
            let inputs = rec.flattened_inputs(a);
            assert_eq!(inputs.len(), 1);
            let action = if mnemonic == "Symlink" {
                SymlinkAction::to_artifact(
                    rec.action_owner(a),
                    inputs[0].clone(),
                    rec.primary_output(a),
                    Some("Creating symlink %{output}".to_string()),
                )
            } else {
                SymlinkAction::to_executable(
                    rec.action_owner(a),
                    inputs[0].clone(),
                    rec.primary_output(a),
                    Some("Creating symlink %{output}".to_string()),
                )
            };
            assert_eq!(action.get_mnemonic(), mnemonic);
            assert_eq!(
                action.get_key(&ActionKeyContext::new(), None).unwrap(),
                a["actionKey"].as_str().unwrap()
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
}

/// SolibSymlink actions of the recordings: the key (symlink and library exec paths and
/// `EMPTY_PLATFORM_INFO`) and the mangled symlink path.
#[test]
fn oracle_solib_symlink_keys_and_paths() {
    let mut checked = 0;
    for project in ["brotli", "re2"] {
        let Some(rec) = oracle_common::Recording::load(project) else {
            eprintln!("no oracle recording; skipped");
            return;
        };
        for a in rec
            .actions()
            .iter()
            .filter(|a| a["mnemonic"] == "SolibSymlink")
        {
            let inputs = rec.flattened_inputs(a);
            assert_eq!(inputs.len(), 1);
            let library = inputs[0].clone();
            let symlink = rec.primary_output(a);
            let action = get_dynamic_library_symlink_internal(
                rec.action_owner(a),
                library.clone(),
                symlink.clone(),
            );
            assert_eq!(action.get_mnemonic(), "SolibSymlink");
            assert_eq!(action.get_progress_message(None), None);
            assert_eq!(
                action.get_key(&ActionKeyContext::new(), None).unwrap(),
                a["actionKey"].as_str().unwrap()
            );

            // The recorded path is the mangled name for one of the (preserveName,
            // prefixConsumer) combinations (aquery does not report them).
            let label =
                Label::parse_canonical(&rec.target_label(a["targetId"].as_u64().unwrap())).unwrap();
            let config = rec.configuration(a["configurationId"].as_u64().unwrap());
            let mnemonic = config["mnemonic"].as_str().unwrap();
            let solib_dir = format!("_solib_{}", config["platformName"].as_str().unwrap());
            let candidates: Vec<String> =
                [(false, false), (false, true), (true, false), (true, true)]
                    .iter()
                    .map(|&(preserve_name, prefix_consumer)| {
                        get_dynamic_library_symlink_path(
                            &label,
                            &solib_dir,
                            mnemonic,
                            &library,
                            preserve_name,
                            prefix_consumer,
                        )
                        .get_path_string()
                        .to_string()
                    })
                    .collect();
            let recorded = symlink
                .get_root_relative_path()
                .get_path_string()
                .to_string();
            assert!(
                candidates.contains(&recorded),
                "{} not in {:?}",
                recorded,
                candidates
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
}

#[test]
fn solib_soname() {
    // brotli: //:brotlicommon's libbrotlicommon.so -> _solib_k8/liblibbrotlicommon.so (oracle).
    assert_eq!(
        get_dynamic_library_soname(
            &PathFragment::create("libbrotlicommon.so"),
            false,
            "k8-fastbuild"
        ),
        "liblibbrotlicommon.so"
    );
    assert_eq!(
        get_dynamic_library_soname(&PathFragment::create("a/libx.so"), false, "k8-ST-abc123"),
        "libST-abc123_a_Slibx.so"
    );
}

// Expected keys from `bazel-9.3.2 aquery --output=jsonproto //:g` on a workspace whose rule does
//   ctx.actions.symlink(output = <declare_symlink g_link>, target_path = "../some/./target")
//   ctx.actions.symlink(output = <declare_symlink g_link2>, target_path = "/abs/dir",
//       target_type = "directory")
// (k8-fastbuild, execution platform @@platforms//host:host). aquery reported
// unresolvedSymlinkTarget "../some/target" and "/abs/dir".
//
// Both are targets `symlink_unresolved_ascii` and `symlink_unresolved_dir` of the probe workspace
// `tests/data/non_ascii_probe/workspace`; `scripts/actions-probe/run` re-records the oracle's
// keys, and `non_ascii_action_keys.rs` checks them from the recording (re-derived 2026-11-18:
// both keys below are the oracle's).
#[test]
fn oracle_unresolved_symlink_keys() {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot/_main"), &fs);
    let root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );
    let symlink = |name: &str| {
        Artifact::create_special_artifact(
            root.clone(),
            root.get_exec_path().get_relative_str(name),
            ActionLookupKeyRef::new(NullArtifactOwner),
            SpecialArtifactType::UNRESOLVED_SYMLINK,
        )
    };
    let owner = owner_with_platform(oracle_common::host_platform());
    let a = UnresolvedSymlinkAction::create(
        owner.clone(),
        symlink("g_link"),
        "../some/./target",
        SymlinkTargetType::UNSPECIFIED,
        "Creating symlink %{output}",
    );
    assert_eq!(
        a.get_key(&ActionKeyContext::new(), None).unwrap(),
        "5a7cb3bcfe9bef03594e4b31ef259851cb04b5c3d7436e16403176bdebb7e6e0"
    );
    assert_eq!(
        a.aquery_unresolved_symlink_target().unwrap(),
        "../some/target"
    );
    assert_eq!(a.get_mnemonic(), "UnresolvedSymlink");
    assert_eq!(
        a.get_progress_message(None).unwrap(),
        "Creating symlink g_link"
    );
    let b = UnresolvedSymlinkAction::create(
        owner,
        symlink("g_link2"),
        "/abs/dir",
        SymlinkTargetType::DIRECTORY,
        "Creating symlink %{output}",
    );
    assert_eq!(
        b.get_key(&ActionKeyContext::new(), None).unwrap(),
        "d1966e8debf9a65adec1bddb2af76f60bd47fc7934d3d5eb775eccf40b8bcb1a"
    );
    assert_eq!(b.get_target(), "/abs/dir");
}
