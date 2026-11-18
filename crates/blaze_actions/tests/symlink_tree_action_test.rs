//! Port of `src/test/java/com/google/devtools/build/lib/analysis/actions/SymlinkTreeActionTest.java`.
//!
//! `ActionTester.combinations` / `runTest` are ported inline: every attribute subset yields an
//! action, regenerating a subset yields an action with the same key, and all actions of the
//! tester have pairwise different keys.

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::artifact::Artifact;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_actions::analysis::runfiles::RunfilesBuilder;
use blaze_actions::{
    Action, ActionEnvironment, ActionKeyContext, ActionLookupKeyRef, ActionOwner, ArtifactRoot,
    RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_events::Location;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};

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
        Some(null_label())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().downcast_ref::<NullArtifactOwner>().is_some()
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

fn null_label() -> Label {
    Label::parse_canonical_unchecked("//null/action:owner")
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        Some(null_label()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    )
}

fn bin_root() -> ArtifactRoot {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = Path::create(PathFragment::create("/execroot"), &fs);
    ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    )
}

/// `getBinArtifactWithNoOwner(rootRelativePath)`.
fn get_bin_artifact_with_no_owner(root_relative_path: &str) -> Artifact {
    let bin = bin_root();
    let exec_path = bin.get_exec_path().get_relative_str(root_relative_path);
    Artifact::create_derived_artifact(bin, exec_path, ActionLookupKeyRef::new(NullArtifactOwner))
}

// private enum FilesetActionAttributes { FIXED_ENVIRONMENT, VARIABLE_ENVIRONMENT }
const FILESET_FIXED_ENVIRONMENT: u32 = 0;
const FILESET_VARIABLE_ENVIRONMENT: u32 = 1;
const FILESET_ATTRIBUTE_COUNT: u32 = 2;

// private enum RunfilesActionAttributes { RUNFILES, FIXED_ENVIRONMENT, VARIABLE_ENVIRONMENT }
const RUNFILES_RUNFILES: u32 = 0;
const RUNFILES_FIXED_ENVIRONMENT: u32 = 1;
const RUNFILES_VARIABLE_ENVIRONMENT: u32 = 2;
const RUNFILES_ATTRIBUTE_COUNT: u32 = 3;

/// `makeEnumSetInitializedTo(attributeClass, i)`: attribute `bit` is in the set.
fn contains(set: u32, bit: u32) -> bool {
    set & (1 << bit) != 0
}

fn create_action_environment(fixed: bool, variable: bool) -> ActionEnvironment {
    let mut fixed_env = IndexMap::new();
    if fixed {
        fixed_env.insert("a".to_string(), "b".to_string());
    }
    let mut inherited_env = IndexSet::new();
    if variable {
        inherited_env.insert("c".to_string());
    }
    ActionEnvironment::create(fixed_env, inherited_env)
}

/// `ActionTester`: collects keys of generated actions.
struct ActionTester {
    action_key_context: ActionKeyContext,
    keys: Vec<String>,
}

impl ActionTester {
    fn key(&self, action: &SymlinkTreeAction) -> String {
        action.get_key(&self.action_key_context, None).unwrap()
    }

    fn combinations(&mut self, attributes_count: u32, factory: &dyn Fn(u32) -> SymlinkTreeAction) {
        let count = 1u32 << attributes_count;
        let mut first_key: Option<String> = None;
        for i in 0..count {
            let key = self.key(&factory(i));
            // Check that creating the same action twice results in equal actions.
            assert_eq!(key, self.key(&factory(i)));
            if i == 0 {
                first_key = Some(key.clone());
            }
            self.keys.push(key);
        }
        // Check that the count is correct: `count` sets no attribute bit, like 0.
        assert_eq!(first_key.unwrap(), self.key(&factory(count)));
    }

    fn run_test(&self) {
        assert!(!self.keys.is_empty());
        for i in 0..self.keys.len() {
            for j in (i + 1)..self.keys.len() {
                assert_ne!(self.keys[i], self.keys[j], "{} and {}", i, j);
            }
        }
    }
}

#[test]
fn test_compute_key() {
    let bin = bin_root();
    let fileset_input_manifest = Artifact::create_special_artifact(
        bin.clone(),
        bin.get_exec_path().get_relative_str("dir/manifest.in"),
        ActionLookupKeyRef::new(NullArtifactOwner),
        SpecialArtifactType::FILESET,
    );
    let runfiles_input_manifest = get_bin_artifact_with_no_owner("dir/manifest.in");
    let output_manifest = get_bin_artifact_with_no_owner("dir/MANIFEST");
    let runfile = get_bin_artifact_with_no_owner("dir/runfile");
    let runfile2 = get_bin_artifact_with_no_owner("dir/runfile2");

    let mut tester = ActionTester {
        action_key_context: ActionKeyContext::new(),
        keys: Vec::new(),
    };

    for runfile_symlinks_mode in RunfileSymlinksMode::VALUES {
        tester.combinations(RUNFILES_ATTRIBUTE_COUNT, &|attributes_to_flip| {
            SymlinkTreeAction::new(
                null_action_owner(),
                runfiles_input_manifest.clone(),
                /* runfiles= */
                Some(if contains(attributes_to_flip, RUNFILES_RUNFILES) {
                    RunfilesBuilder::new("TESTING")
                        .add_artifact(runfile.clone())
                        .build()
                } else {
                    RunfilesBuilder::new("TESTING")
                        .add_artifact(runfile2.clone())
                        .build()
                }),
                output_manifest.clone(),
                /* repo_mapping_manifest= */ None,
                create_action_environment(
                    contains(attributes_to_flip, RUNFILES_FIXED_ENVIRONMENT),
                    contains(attributes_to_flip, RUNFILES_VARIABLE_ENVIRONMENT),
                ),
                runfile_symlinks_mode,
                "workspace".to_string(),
            )
        });

        tester.combinations(FILESET_ATTRIBUTE_COUNT, &|attributes_to_flip| {
            SymlinkTreeAction::new(
                null_action_owner(),
                fileset_input_manifest.clone(),
                /* runfiles= */ None,
                output_manifest.clone(),
                /* repo_mapping_manifest= */ None,
                create_action_environment(
                    contains(attributes_to_flip, FILESET_FIXED_ENVIRONMENT),
                    contains(attributes_to_flip, FILESET_VARIABLE_ENVIRONMENT),
                ),
                runfile_symlinks_mode,
                "workspace".to_string(),
            )
        });
    }

    tester.run_test();
}

#[test]
#[should_panic(expected = "NullPointerException")]
fn test_null_runfiles_throws() {
    let input_manifest = get_bin_artifact_with_no_owner("dir/manifest.in");
    let output_manifest = get_bin_artifact_with_no_owner("dir/MANIFEST");
    SymlinkTreeAction::new(
        null_action_owner(),
        input_manifest,
        /* runfiles= */ None,
        output_manifest,
        /* repo_mapping_manifest= */ None,
        create_action_environment(false, false),
        RunfileSymlinksMode::SKIP,
        "workspace".to_string(),
    );
}
