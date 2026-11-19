//! Port of `RemoteOutputCheckerTest.java` (lib/remote).

use std::any::Any;
use std::fmt;

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::{
    ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactRoot, RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_exec::actions::ActionInputRef;
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
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
        1
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

struct Fixture {
    remote_output_checker: RemoteOutputChecker,
    exec_root: ArtifactRoot,
}

impl Fixture {
    fn new() -> Fixture {
        // PORT: Java's InMemoryFileSystem; no file is touched, so the Unix file system serves.
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        Fixture {
            remote_output_checker: RemoteOutputChecker::new(
                "build",
                RemoteOutputsMode::MINIMAL,
                Vec::new(),
            ),
            exec_root: ArtifactRoot::as_derived_root(
                &fs.get_path("/execroot"),
                RootType::OUTPUT,
                &["out"],
            ),
        }
    }

    /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, rootRelativePath)`.
    // Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:298 ActionsTestUtil.createTreeArtifactWithGeneratingAction
    fn create_tree_artifact_with_generating_action(&self, root_relative_path: &str) -> Artifact {
        let tree_artifact = Artifact::create_special_artifact(
            self.exec_root.clone(),
            self.exec_root
                .get_exec_path()
                .get_relative(&PathFragment::create(root_relative_path)),
            null_artifact_owner(),
            SpecialArtifactType::TREE,
        );
        tree_artifact.set_generating_action_key(ActionLookupData::create(null_artifact_owner(), 0));
        tree_artifact
    }

    /// `ActionsTestUtil.createArtifact(root, path)`.
    // Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:250 ActionsTestUtil.createArtifact
    fn create_artifact(&self, path: &str) -> Artifact {
        Artifact::create_derived_artifact(
            self.exec_root.clone(),
            self.exec_root
                .get_exec_path()
                .get_relative(&PathFragment::create(path)),
            null_artifact_owner(),
        )
    }
}

#[test]
fn test_should_download_output() {
    let f = Fixture::new();
    f.remote_output_checker
        .add_output_to_download(&*ActionInputRef::new(
            f.create_tree_artifact_with_generating_action("foo/bar"),
        ));
    f.remote_output_checker
        .add_output_to_download(&*ActionInputRef::new(f.create_artifact("foo/bar-baz")));
    assert!(
        !f.remote_output_checker
            .should_download_output_path(&PathFragment::create("out/foo/bar-quz"), None)
    );
    assert!(
        f.remote_output_checker
            .should_download_output_path(&PathFragment::create("out/foo/bar"), None)
    );
    assert!(f.remote_output_checker.should_download_output_path(
        &PathFragment::create("out/foo/bar/data.txt"),
        Some(&PathFragment::create("out/foo/bar"))
    ));
    assert!(
        f.remote_output_checker
            .should_download_output_path(&PathFragment::create("out/foo/bar-baz"), None)
    );
}
