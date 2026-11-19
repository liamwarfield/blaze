//! Port of `src/test/java/com/google/devtools/build/lib/analysis/SourceManifestActionTest.java`.
//!
//! The Java test runs under `BuildViewTestCase` with the workspace at `/workspace`; these tests
//! build the same artifacts directly. Only `testUnresolvedSymlink` touches the file system (it
//! reads two symlinks), under `CARGO_TARGET_TMPDIR`.

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::artifact::Artifact;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::runfiles::{EmptyFilesSupplier, Runfiles, RunfilesBuilder};
use blaze_actions::analysis::source_manifest_action::{
    ManifestType, ManifestWriter, SourceManifestAction,
};
use blaze_actions::{
    Action, ActionKeyContext, ActionLookupKeyRef, ActionOwner, ArtifactOwner, ArtifactRoot,
    FileContentsProvider, RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_events::Location;
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};
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

/// `ActionsTestUtil.NULL_LABEL`.
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

/// The `@Before createFiles()` state.
struct Fixture {
    fs: FileSystemRef,
    root_directory: Path,
    fake_manifest: IndexMap<PathFragment, Artifact>,
    build_file: Artifact,
    manifest_output_file: Artifact,
}

fn fixture() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root_directory = Path::create(PathFragment::create("/workspace"), &fs);
    let trivial_root =
        ArtifactRoot::as_source_root(Root::from_path(root_directory.get_relative_str("trivial")));
    // ActionsTestUtil.createArtifact(trivialRoot, path): root-relative "BUILD".
    let build_file_path = root_directory.get_relative_str("trivial/BUILD");
    let build_file = Artifact::new_source_artifact(
        trivial_root.clone(),
        PathFragment::create("BUILD"),
        ArtifactOwner::NullOwner,
    );
    let python_source_path = root_directory.get_relative_str("trivial/trivial.py");
    let python_source_file = Artifact::new_source_artifact(
        trivial_root,
        PathFragment::create("trivial.py"),
        ArtifactOwner::NullOwner,
    );
    let mut fake_manifest = IndexMap::new();
    fake_manifest.insert(
        build_file_path.relative_to(&root_directory),
        build_file.clone(),
    );
    fake_manifest.insert(
        python_source_path.relative_to(&root_directory),
        python_source_file,
    );
    let output_dir =
        ArtifactRoot::as_derived_root(&root_directory, RootType::OUTPUT, &["blaze-output"]);
    let manifest_output_file = Artifact::create_derived_artifact(
        output_dir,
        PathFragment::create("blaze-output/trivial.runfiles_manifest"),
        ActionLookupKeyRef::new(NullArtifactOwner),
    );
    Fixture {
        fs,
        root_directory,
        fake_manifest,
        build_file,
        manifest_output_file,
    }
}

impl Fixture {
    /// `getBinArtifactWithNoOwner(rootRelativePath)`.
    fn get_bin_artifact_with_no_owner(&self, root_relative_path: &str) -> Artifact {
        let exec_root = Path::create(PathFragment::create("/execroot"), &self.fs);
        let bin = ArtifactRoot::as_derived_root(
            &exec_root,
            RootType::OUTPUT,
            &["bazel-out", "k8-fastbuild", "bin"],
        );
        let exec_path = bin.get_exec_path().get_relative_str(root_relative_path);
        Artifact::create_derived_artifact(
            bin,
            exec_path,
            ActionLookupKeyRef::new(NullArtifactOwner),
        )
    }

    fn create_action(&self, type_: ManifestType, add_init_py: bool) -> SourceManifestAction {
        let mut builder = RunfilesBuilder::new("TESTING");
        builder.add_symlinks_map(self.fake_manifest.clone());
        if add_init_py {
            builder.set_empty_files_supplier(Arc::new(PyInitStandInSupplier));
        }
        SourceManifestAction::new_for_testing(
            Arc::new(type_),
            null_action_owner(),
            self.manifest_output_file.clone(),
            builder.build(),
        )
    }

    fn create_symlink_action(&self) -> SourceManifestAction {
        self.create_action(ManifestType::SOURCE_SYMLINKS, true)
    }

    fn create_source_only_action(&self) -> SourceManifestAction {
        self.create_action(ManifestType::SOURCES_ONLY, true)
    }
}

/// Stand-in for `analysisMock.pySupport().getEmptyRunfilesSupplier()` (Bazel's
/// `PythonUtils.GET_INIT_PY_FILES`, not ported in this crate) in the tests whose assertions do
/// not depend on the generated `__init__.py` entries (mnemonic and progress message). It adds no
/// paths.
struct PyInitStandInSupplier;

impl EmptyFilesSupplier for PyInitStandInSupplier {
    fn get_extra_paths(&self, _manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        Vec::new()
    }

    fn fingerprint(&self, _fingerprint: &mut Fingerprint) {}

    fn get_class_name(&self) -> String {
        "PyInitStandInSupplier".to_string()
    }
}

/// Manifest writer that validates an expected call sequence.
#[derive(Debug)]
struct MockManifestWriter {
    expected_sequence: Mutex<Vec<(PathFragment, Artifact)>>,
}

impl MockManifestWriter {
    fn new(fake_manifest: &IndexMap<PathFragment, Artifact>) -> MockManifestWriter {
        MockManifestWriter {
            expected_sequence: Mutex::new(
                fake_manifest
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        }
    }

    fn unconsumed_inputs(&self) -> usize {
        self.expected_sequence.lock().unwrap().len()
    }
}

impl ManifestWriter for MockManifestWriter {
    fn write_entry(
        &self,
        _manifest_writer: &mut dyn Write,
        root_relative_path: &PathFragment,
        symlink_target: Option<&PathFragment>,
    ) -> io::Result<()> {
        let mut expected_sequence = self.expected_sequence.lock().unwrap();
        assert!(
            !expected_sequence.is_empty(),
            "Expected manifest input to be exhausted"
        );
        let (expected_key, expected_artifact) = expected_sequence.remove(0);
        assert_eq!(
            *root_relative_path,
            PathFragment::create("TESTING").get_relative(&expected_key)
        );
        assert_eq!(
            symlink_target,
            Some(expected_artifact.get_path().as_fragment())
        );
        Ok(())
    }

    fn get_mnemonic(&self) -> String {
        String::new()
    }

    fn get_raw_progress_message(&self) -> String {
        String::new()
    }

    fn is_remotable(&self) -> bool {
        false
    }

    fn emits_absolute_paths(&self) -> bool {
        false
    }
}

/// Tests that SourceManifestAction calls its manifest writer with the expected call sequence.
#[test]
fn test_manifest_writer_integration() {
    let f = fixture();
    let mock_writer = Arc::new(MockManifestWriter::new(&f.fake_manifest));
    let manifest_contents = SourceManifestAction::new_for_testing(
        mock_writer.clone(),
        null_action_owner(),
        f.manifest_output_file.clone(),
        RunfilesBuilder::new("TESTING")
            .add_symlinks_map(f.fake_manifest.clone())
            .build(),
    )
    .get_file_contents()
    .unwrap();
    assert_eq!(mock_writer.unconsumed_inputs(), 0);
    assert!(manifest_contents.is_empty());
}

#[test]
#[ignore = "needs PythonUtils.GET_INIT_PY_FILES (rules/python; analysisMock.pySupport())"]
fn test_simple_file_writing() {}

/// Tests that the source-only formatting strategy includes relative paths only (i.e. not
/// symlinks).
#[test]
#[ignore = "needs PythonUtils.GET_INIT_PY_FILES (rules/python; analysisMock.pySupport())"]
fn test_source_only_formatting() {}

/// Test that a directory which has only a .so file in the manifest triggers the inclusion of a
/// __init__.py file for that directory.
#[test]
#[ignore = "needs PythonUtils.GET_INIT_PY_FILES (rules/python; analysisMock.pySupport())"]
fn test_swig_libraries_trigger_init_dot_py_inclusion() {}

#[test]
#[ignore = "needs PythonUtils.GET_INIT_PY_FILES (rules/python; analysisMock.pySupport())"]
fn test_no_python_or_swig_libraries_do_not_trigger_init_dot_py_inclusion() {}

#[test]
fn test_get_mnemonic() {
    let f = fixture();
    assert_eq!(
        f.create_symlink_action().get_mnemonic(),
        "SourceSymlinkManifest"
    );
    assert_eq!(
        f.create_action(ManifestType::SOURCE_SYMLINKS, false)
            .get_mnemonic(),
        "SourceSymlinkManifest"
    );
    assert_eq!(
        f.create_source_only_action().get_mnemonic(),
        "PackagingSourcesManifest"
    );
}

#[test]
fn test_symlink_progress_message() {
    let f = fixture();
    let progress = f
        .create_symlink_action()
        .get_progress_message(None)
        .unwrap();
    assert!(
        progress.contains("//null/action:owner"),
        "null action not found in {}",
        progress
    );
}

#[test]
fn test_symlink_progress_message_no_py_init_files() {
    let f = fixture();
    let progress = f
        .create_action(ManifestType::SOURCE_SYMLINKS, false)
        .get_progress_message(None)
        .unwrap();
    assert!(
        progress.contains("//null/action:owner"),
        "null action not found in {}",
        progress
    );
}

#[test]
fn test_source_only_progress_message() {
    let f = fixture();
    let action = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCES_ONLY),
        null_action_owner(),
        f.get_bin_artifact_with_no_owner("trivial.runfiles_manifest"),
        Runfiles::empty(),
    );
    let progress = action.get_progress_message(None).unwrap();
    assert!(
        progress.contains("//null/action:owner"),
        "null action not found in {}",
        progress
    );
}

fn compute_key(action: &SourceManifestAction) -> String {
    let mut fp = Fingerprint::new();
    action
        .compute_key(
            &ActionKeyContext::new(),
            /* input_metadata_provider= */ None,
            &mut fp,
        )
        .unwrap();
    fp.hex_digest_and_reset()
}

#[test]
fn test_root_symlinks_affect_key() {
    let f = fixture();
    let manifest1 = f.get_bin_artifact_with_no_owner("manifest1");
    let manifest2 = f.get_bin_artifact_with_no_owner("manifest2");

    let action1 = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest1,
        RunfilesBuilder::new("TESTING")
            .add_root_symlinks_map([(PathFragment::create("a"), f.build_file.clone())])
            .build(),
    );

    let action2 = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest2,
        RunfilesBuilder::new("TESTING")
            .add_root_symlinks_map([(PathFragment::create("b"), f.build_file.clone())])
            .build(),
    );

    assert_ne!(compute_key(&action2), compute_key(&action1));
}

/// The test's anonymous suppliers: `<basename><suffix>` next to each manifest path, with a
/// distinct fingerprint.
struct SuffixSupplier {
    suffix: &'static str,
    fingerprint_int: i32,
}

impl EmptyFilesSupplier for SuffixSupplier {
    fn get_extra_paths(&self, manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        // ImmutableSet: duplicates removed, first occurrence kept.
        let set: IndexSet<PathFragment> = manifest_paths
            .iter()
            .map(|p| {
                p.replace_name(&format!("{}{}", p.get_base_name(), self.suffix))
                    .unwrap()
            })
            .collect();
        set.into_iter().collect()
    }

    fn fingerprint(&self, fingerprint: &mut Fingerprint) {
        fingerprint.add_int(self.fingerprint_int);
    }

    fn get_class_name(&self) -> String {
        "com.google.devtools.build.lib.analysis.SourceManifestActionTest$1".to_string()
    }
}

// Regression test for b/116254698.
#[test]
fn test_empty_files_affect_key() {
    let f = fixture();
    let manifest1 = f.get_bin_artifact_with_no_owner("manifest1");
    let manifest2 = f.get_bin_artifact_with_no_owner("manifest2");

    let action1 = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest1,
        RunfilesBuilder::new("TESTING")
            .add_symlink(PathFragment::create("a"), f.build_file.clone())
            .set_empty_files_supplier(Arc::new(SuffixSupplier {
                suffix: "~",
                fingerprint_int: 1,
            }))
            .build(),
    );

    let action2 = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest2,
        RunfilesBuilder::new("TESTING")
            .add_symlink(PathFragment::create("a"), f.build_file.clone())
            .set_empty_files_supplier(Arc::new(SuffixSupplier {
                suffix: "~~",
                fingerprint_int: 2,
            }))
            .build(),
    );

    assert_ne!(compute_key(&action2), compute_key(&action1));
}

#[test]
fn test_unresolved_symlink() {
    let f = fixture();
    // The output directory holds real symlinks; the Java test creates them with
    // `createSymbolicLink` in its scratch file system.
    let real_exec_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "source_manifest_action_test_{}",
        std::process::id()
    ));
    let real_output_dir = real_exec_root.join("blaze-output");
    let _ = std::fs::remove_dir_all(&real_exec_root);
    std::fs::create_dir_all(&real_output_dir).unwrap();
    std::os::unix::fs::symlink(
        "../some/relative/path",
        real_output_dir.join("relative_symlink"),
    )
    .unwrap();
    std::os::unix::fs::symlink("/absolute/path", real_output_dir.join("absolute_symlink")).unwrap();

    let exec_root = Path::create(
        PathFragment::create(real_exec_root.to_str().unwrap()),
        &f.fs,
    );
    let output_dir = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["blaze-output"]);
    let relative_symlink = Artifact::create_special_artifact(
        output_dir.clone(),
        output_dir.get_exec_path().get_child("relative_symlink"),
        ActionLookupKeyRef::new(NullArtifactOwner),
        SpecialArtifactType::UNRESOLVED_SYMLINK,
    );
    let absolute_symlink = Artifact::create_special_artifact(
        output_dir.clone(),
        output_dir.get_exec_path().get_child("absolute_symlink"),
        ActionLookupKeyRef::new(NullArtifactOwner),
        SpecialArtifactType::UNRESOLVED_SYMLINK,
    );

    let manifest = f.get_bin_artifact_with_no_owner("manifest1");

    let action = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest,
        RunfilesBuilder::new("TESTING")
            .add_artifact(absolute_symlink.clone())
            .add_artifact(f.build_file.clone())
            .add_artifact(relative_symlink.clone())
            .build(),
    );

    let inputs = action.get_inputs();
    let mut input_list: Vec<Artifact> = inputs.to_list().iter().cloned().collect();
    input_list.sort_by(Artifact::exec_path_comparator);
    let mut expected_inputs = vec![absolute_symlink, relative_symlink];
    expected_inputs.sort_by(Artifact::exec_path_comparator);
    assert_eq!(input_list, expected_inputs);

    // Verify that the return value of getInputs is cached.
    assert!(inputs == action.get_inputs());
    assert_eq!(*inputs.to_list(), *action.get_inputs().to_list());

    assert_eq!(
        action.get_file_contents().unwrap(),
        "TESTING/BUILD /workspace/trivial/BUILD\n\
         TESTING/absolute_symlink /absolute/path\n\
         TESTING/relative_symlink ../some/relative/path\n"
    );
    let _ = std::fs::remove_dir_all(&real_exec_root);
    let _ = &f.root_directory;
}

#[test]
fn test_escaping() {
    let f = fixture();
    let manifest = f.get_bin_artifact_with_no_owner("manifest1");

    let trivial_root = ArtifactRoot::as_source_root(Root::from_path(
        f.root_directory.get_relative_str("trivial"),
    ));
    let file_with_space_and_backslash = Artifact::new_source_artifact(
        trivial_root.clone(),
        PathFragment::create("file with sp\\ace"),
        ArtifactOwner::NullOwner,
    );
    let file_with_newline_and_backslash = Artifact::new_source_artifact(
        trivial_root,
        PathFragment::create("file\nwith\\newline"),
        ArtifactOwner::NullOwner,
    );
    let build_file = f.build_file.clone();

    let action = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        manifest,
        RunfilesBuilder::new("TESTING")
            .add_symlink(PathFragment::create("no/sp\\ace"), build_file.clone())
            .add_symlink(
                PathFragment::create("also/no/sp\\ace"),
                file_with_space_and_backslash.clone(),
            )
            .add_symlink(
                PathFragment::create("still/no/sp\\ace"),
                file_with_newline_and_backslash.clone(),
            )
            .add_symlink(PathFragment::create("with sp\\ace"), build_file.clone())
            .add_symlink(
                PathFragment::create("also/with sp\\ace"),
                file_with_space_and_backslash.clone(),
            )
            .add_symlink(
                PathFragment::create("more/with sp\\ace"),
                file_with_newline_and_backslash.clone(),
            )
            .add_symlink(PathFragment::create("with\nnew\\line"), build_file.clone())
            .add_symlink(
                PathFragment::create("also/with\nnewline"),
                file_with_space_and_backslash.clone(),
            )
            .add_symlink(
                PathFragment::create("more/with\nnewline"),
                file_with_newline_and_backslash.clone(),
            )
            .add_symlink(
                PathFragment::create("with\nnew\\line and space"),
                build_file.clone(),
            )
            .add_symlink(
                PathFragment::create("also/with\nnewline and space"),
                file_with_space_and_backslash.clone(),
            )
            .add_symlink(
                PathFragment::create("more/with\nnewline and space"),
                file_with_newline_and_backslash.clone(),
            )
            .build(),
    );
    // PORT: the Windows branch of the Java test is skipped (Linux only).
    assert_eq!(
        action.get_file_contents().unwrap(),
        r"TESTING/also/no/sp\ace /workspace/trivial/file with sp\ace
 TESTING/also/with\nnewline /workspace/trivial/file with sp\bace
 TESTING/also/with\nnewline\sand\sspace /workspace/trivial/file with sp\bace
 TESTING/also/with\ssp\bace /workspace/trivial/file with sp\bace
 TESTING/more/with\nnewline /workspace/trivial/file\nwith\bnewline
 TESTING/more/with\nnewline\sand\sspace /workspace/trivial/file\nwith\bnewline
 TESTING/more/with\ssp\bace /workspace/trivial/file\nwith\bnewline
TESTING/no/sp\ace /workspace/trivial/BUILD
 TESTING/still/no/sp\bace /workspace/trivial/file\nwith\bnewline
 TESTING/with\nnew\bline /workspace/trivial/BUILD
 TESTING/with\nnew\bline\sand\sspace /workspace/trivial/BUILD
 TESTING/with\ssp\bace /workspace/trivial/BUILD
"
    );
}

/// Not in the Java test: the manifest is written through an ISO-8859-1 `Writer`, so paths come
/// out as the bytes their internal strings stand for, and `getFileContents` reads them back with
/// ISO-8859-1. `é` (c3 a9), `🐷` (f0 9f 90 b7) and the byte ff; only space, newline and
/// backslash are escaped.
#[test]
fn test_non_ascii_paths() {
    use blaze_util::string_encoding::latin1_decode;
    let f = fixture();
    let trivial_root = ArtifactRoot::as_source_root(Root::from_path(
        f.root_directory.get_relative_str("trivial"),
    ));
    let target = Artifact::new_source_artifact(
        trivial_root,
        PathFragment::create(&latin1_decode(b"\xf0\x9f\x90\xb7\xff")),
        ArtifactOwner::NullOwner,
    );
    let action = SourceManifestAction::new_for_testing(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        null_action_owner(),
        f.get_bin_artifact_with_no_owner("manifest1"),
        RunfilesBuilder::new("TESTING")
            .add_symlink(
                PathFragment::create(&latin1_decode(b"dir_\xc3\xa9/a b")),
                target,
            )
            .build(),
    );
    let want: &[u8] = b" TESTING/dir_\xc3\xa9/a\\sb /workspace/trivial/\xf0\x9f\x90\xb7\xff\n";
    let mut out = Vec::new();
    action.write_to(&mut out).unwrap();
    assert_eq!(out, want);
    assert_eq!(action.get_file_contents().unwrap(), latin1_decode(want));
}
