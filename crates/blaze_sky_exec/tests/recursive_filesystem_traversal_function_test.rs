//! Port of `RecursiveFilesystemTraversalFunctionTest` (src/test/java/com/google/devtools/build/
//! lib/skyframe), for the one `TraversalRequest` Bazel creates
//! (`ArtifactFunction.DirectoryArtifactTraversalRequest`), on a real temporary directory.
//!
//! PORT: Java's test request (`BasicFileTraversalRequest`) can be generated, non-strict or skip
//! empty directory nodes; `DirectoryArtifactTraversalRequest` is a source request with
//! `strictOutputFiles` and `emitEmptyDirectoryNodes` true, so the tests of other kinds are
//! ignored. Blaze's evaluator is not incremental: where a Java test changes the file system and
//! re-evaluates, the port evaluates in a new evaluator.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::{ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot};
use blaze_cmdline::RepositoryName;
use blaze_events::StoredEventHandler;
use blaze_sky_exec::analysis_seam::{ActionLookupValue, AnalysisSeam, ConfiguredTargetView};
use blaze_sky_exec::artifact_function::{ArtifactValue, SourceArtifactException};
use blaze_sky_exec::keys::ArtifactKey;
use blaze_sky_exec::recursive_filesystem_traversal_function::{
    DirectTraversalRoot, DirectoryArtifactTraversalRequest, FileType, HasDigest,
    RecursiveFilesystemTraversalException, RecursiveFilesystemTraversalExceptionType,
    RecursiveFilesystemTraversalValue, ResolvedFile, TrackSourceDirectoriesFlag, with_digest,
};
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_sky_loading::actions::{FileContentsProxy, FileStateValue};
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::repos::{ExternalRepos, RepositoryDirectoryValue, RepositoryMappingValue};
use blaze_sky_loading::skyframe_executor::{
    LoadingFunctionsConfig, inject_loading_precomputed, register_loading_functions,
};
use blaze_skyframe::{
    ComputeError, Environment, EvaluationContext, Evaluator, EvaluatorBuilder, SkyKey,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, Path, PathFragment, Root, RootedPath, UnixFileSystem,
};
use indexmap::IndexMap;

struct NoAnalysis;

impl AnalysisSeam for NoAnalysis {
    fn get_action_lookup_value(
        &self,
        _key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        unreachable!()
    }
    fn get_configured_target(
        &self,
        _key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        unreachable!()
    }
}

/// REPOSITORY_MAPPING answers the empty root module; no external repository exists.
#[derive(Default)]
struct FakeRepos {
    _dirs: HashMap<String, Path>,
}

impl ExternalRepos for FakeRepos {
    fn compute_repository_mapping(
        &self,
        _repository_name: &RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMappingValue>, ComputeError> {
        Ok(Some(RepositoryMappingValue::value_for_empty_root_module()))
    }

    fn compute_repository_directory(
        &self,
        repository_name: &RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryDirectoryValue>, ComputeError> {
        Ok(Some(RepositoryDirectoryValue::Failure {
            error_msg: format!("Repository '{}' is not defined", repository_name),
        }))
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: Path,
    workspace: Path,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = fs.get_path(dir.path().to_str().unwrap());
        let workspace = root.get_child("workspace");
        for d in ["workspace", "output_base", "install_base"] {
            root.get_child(d).create_directory_and_parents().unwrap();
        }
        Fixture {
            _dir: dir,
            root,
            workspace,
        }
    }

    fn evaluator(&self) -> Evaluator {
        let config = LoadingFunctionsConfig::for_bazel_default(
            LoadingDirectories {
                workspace: self.workspace.clone(),
                output_base: self.root.get_child("output_base"),
                install_base: self.root.get_child("install_base"),
            },
            Arc::new(FakeRepos::default()),
        );
        let builder = register_loading_functions(EvaluatorBuilder::new(), &config);
        let builder = inject_loading_precomputed(
            builder,
            &config,
            StarlarkSemantics::from_flags(&Default::default()).unwrap(),
            IndexMap::new(),
        );
        let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
            exec_root: self.root.get_relative_str("output_base/execroot/_main"),
            action_out_dir: self.root.get_relative_str("output_base/action_outs"),
            external_directory: self.root.get_relative_str("output_base/external"),
            reporter: Arc::new(StoredEventHandler::new()),
            contexts: ActionContexts::default(),
            client_env: IndexMap::new(),
            keep_going: false,
            verbose_failures: false,
            output_permissions: OutputPermissions::READONLY,
        }));
        blaze_sky_exec::register_execution_functions(builder, executor, Arc::new(NoAnalysis), None)
            .build()
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.sourceArtifact
    fn source_artifact(&self, path: &str) -> Artifact {
        Artifact::new_source_artifact(
            ArtifactRoot::as_source_root(Root::from_path(self.workspace.clone())),
            PathFragment::create(path),
            ArtifactOwner::NullOwner,
        )
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.rootedPath(Artifact)
    fn rooted_path(&self, artifact: &Artifact) -> RootedPath {
        RootedPath::to_rooted_path(
            artifact.get_root().get_root().clone(),
            &artifact.get_root_relative_path(),
        )
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.childOf
    fn child_of(&self, artifact: &Artifact, relative: &str) -> RootedPath {
        RootedPath::to_rooted_path(
            artifact.get_root().get_root().clone(),
            &artifact.get_root_relative_path().get_relative_str(relative),
        )
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.createFile
    fn create_file(&self, path: &RootedPath, contents: &str) -> RootedPath {
        let p = path.as_path();
        p.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        std::fs::write(p.get_path_string(), contents).unwrap();
        path.clone()
    }

    fn symlink(&self, path: &RootedPath, target: &str) {
        let p = path.as_path();
        p.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        std::os::unix::fs::symlink(target, p.get_path_string()).unwrap();
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.fileLikeRoot / pkgRoot: a request as
    // ArtifactFunction creates it (skipTestingForSubpackage = true).
    fn request(&self, artifact: &Artifact) -> DirectoryArtifactTraversalRequest {
        DirectoryArtifactTraversalRequest::create(
            DirectTraversalRoot::for_rooted_path(&self.rooted_path(artifact)),
            /* skip_testing_for_subpackage= */ true,
            artifact.clone(),
        )
    }

    fn eval(
        &self,
        request: &DirectoryArtifactTraversalRequest,
    ) -> Result<Arc<RecursiveFilesystemTraversalValue>, RecursiveFilesystemTraversalException> {
        let evaluator = self.evaluator();
        let result = evaluator
            .evaluate(
                &[SkyKey::new(request.clone())],
                &EvaluationContext::default(),
            )
            .unwrap();
        if let Some(v) = result.get(request) {
            return Ok(v);
        }
        let error = result.get_error(&SkyKey::new(request.clone())).unwrap();
        let e = error.get_exception().unwrap();
        Err(e
            .downcast_ref::<RecursiveFilesystemTraversalException>()
            .unwrap_or_else(|| panic!("unexpected exception {e}"))
            .clone())
    }

    // Java: RecursiveFilesystemTraversalFunctionTest.traverseAndAssertFiles: compares types and
    // paths (and the targets of symlinks), not metadata (Java's EMPTY_METADATA).
    fn traverse_and_assert_files(
        &self,
        request: &DirectoryArtifactTraversalRequest,
        expected: &[ResolvedFile],
    ) -> Arc<RecursiveFilesystemTraversalValue> {
        let value = self.eval(request).unwrap();
        let actual: Vec<(FileType, RootedPath)> = value
            .get_transitive_files()
            .to_list()
            .iter()
            .map(|f| (f.get_type(), f.get_path().clone()))
            .collect();
        let mut expected: Vec<(FileType, RootedPath)> = expected
            .iter()
            .map(|f| (f.get_type(), f.get_path().clone()))
            .collect();
        let mut actual_sorted = actual.clone();
        actual_sorted.sort_by_key(|(t, p)| (format!("{t}"), p.to_string()));
        expected.sort_by_key(|(t, p)| (format!("{t}"), p.to_string()));
        assert_eq!(actual_sorted, expected);
        value
    }
}

fn regular_file(path: RootedPath) -> ResolvedFile {
    ResolvedFile::RegularFile {
        path,
        metadata: HasDigest::empty(),
    }
}

fn directory(path: RootedPath) -> ResolvedFile {
    ResolvedFile::Directory { path }
}

fn symlink_to_file(target: RootedPath, link: RootedPath, value: &str) -> ResolvedFile {
    ResolvedFile::SymlinkToFile {
        target_path: target,
        link_name: link,
        unresolved_link_target: PathFragment::create(value),
        metadata: HasDigest::empty(),
    }
}

fn symlink_to_directory(target: RootedPath, link: RootedPath, value: &str) -> ResolvedFile {
    ResolvedFile::SymlinkToDirectory {
        target_path: target,
        link_name: link,
        unresolved_link_target: PathFragment::create(value),
        metadata: HasDigest::empty(),
    }
}

fn dangling_symlink(link: RootedPath, value: &str) -> ResolvedFile {
    ResolvedFile::DanglingSymlink {
        link_name: link,
        unresolved_link_target: PathFragment::create(value),
        metadata: HasDigest::empty(),
    }
}

fn root_digest(v: &RecursiveFilesystemTraversalValue) -> Option<Vec<u8>> {
    v.get_resolved_root()
        .and_then(|r| r.get_metadata().get_digest().map(|d| d.to_vec()))
}

#[test]
fn track_source_directories_default_is_true() {
    // Java: TrackSourceDirectoriesFlag.TRACK_SOURCE_DIRECTORIES_DEFAULT (Bazel).
    assert!(TrackSourceDirectoriesFlag::track_source_directories());
}

#[test]
#[ignore = "BasicFileTraversalRequest for a file root; Bazel only traverses directories"]
fn test_traversal_of_source_file() {}

#[test]
#[ignore = "generated roots (BasicFileTraversalRequest with isRootGenerated) are not ported"]
fn test_traversal_of_generated_file() {}

#[test]
#[ignore = "generated roots (BasicFileTraversalRequest with isRootGenerated) are not ported"]
fn test_traversal_of_generated_file_with_strict_output() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunctionTest.java:509 RecursiveFilesystemTraversalFunctionTest.testTraversalOfSymlinkToFile
// PORT: Java's fileLikeRoot(artifact) is a BasicTraversalRequest (skipTestingForSubpackage
// false); the request here is the DirectoryArtifactTraversalRequest ArtifactFunction creates for
// the source artifact, whose root is the same (subpackage testing does not apply to a file root).
// The progress receiver's invalidation/evaluation assertions have no counterpart (one evaluator
// per evaluation, no incremental state).
#[test]
fn test_traversal_of_symlink_to_file() {
    let f = Fixture::new();
    let link_name_artifact = f.source_artifact("foo/baz/qux.sym");
    let link_target_artifact = f.source_artifact("foo/bar/baz.txt");
    let link_value = "../bar/baz.txt";
    let traversal_root = f.request(&link_name_artifact);
    let target = f.create_file(&f.rooted_path(&link_target_artifact), "");
    f.symlink(&f.rooted_path(&link_name_artifact), link_value);

    // Assert that the SkyValue is built and looks right.
    let symlink_name_path = f.rooted_path(&link_name_artifact);
    let symlink_target_path = f.rooted_path(&link_target_artifact);
    let expected = symlink_to_file(symlink_target_path, symlink_name_path, link_value);
    let v1 = f.traverse_and_assert_files(&traversal_root, std::slice::from_ref(&expected));

    // Edit the target of the symlink and verify that the value is rebuilt.
    // appendToFile(linkTargetArtifact, "bar")
    std::fs::write(target.as_path().get_path_string(), "bar").unwrap();
    let v2 = f.traverse_and_assert_files(&traversal_root, &[expected]);
    // assertTraversalRootHashesAreNotEqual(v1, v2)
    assert_ne!(root_digest(&v1), root_digest(&v2));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunctionTest.java:538 RecursiveFilesystemTraversalFunctionTest.testTraversalOfTransitiveSymlinkToFile
// PORT: fileLikeRoot as in test_traversal_of_symlink_to_file.
#[test]
fn test_traversal_of_transitive_symlink_to_file() {
    let f = Fixture::new();
    let direct_link_artifact = f.source_artifact("direct/file.sym");
    let transitive_link_artifact = f.source_artifact("transitive/sym.sym");
    let file_a = f.create_file(&f.rooted_path(&f.source_artifact("a/file.a")), "");
    let direct_link = f.rooted_path(&direct_link_artifact);
    let transitive_link = f.rooted_path(&transitive_link_artifact);
    let direct_link_path = "../a/file.a";
    let transitive_link_path = "../direct/file.sym";

    f.symlink(&direct_link, direct_link_path);
    f.symlink(&transitive_link, transitive_link_path);

    f.traverse_and_assert_files(
        &f.request(&direct_link_artifact),
        &[symlink_to_file(
            file_a.clone(),
            direct_link,
            direct_link_path,
        )],
    );

    f.traverse_and_assert_files(
        &f.request(&transitive_link_artifact),
        &[symlink_to_file(
            file_a,
            transitive_link,
            transitive_link_path,
        )],
    );
}

#[test]
fn test_traversal_of_source_directory() {
    let f = Fixture::new();
    let dir = f.source_artifact("dir");
    let file1 = f.create_file(&f.child_of(&dir, "bar.txt"), "");
    let file2 = f.create_file(&f.child_of(&dir, "baz/qux.txt"), "");
    let request = f.request(&dir);
    let v1 = f.traverse_and_assert_files(
        &request,
        &[regular_file(file1.clone()), regular_file(file2.clone())],
    );

    // Add a new file to the directory and see that the value is rebuilt.
    let file3 = f.create_file(&f.child_of(&dir, "foo.txt"), "");
    let v2 = f.traverse_and_assert_files(
        &request,
        &[
            regular_file(file1.clone()),
            regular_file(file2.clone()),
            regular_file(file3.clone()),
        ],
    );
    assert_ne!(
        v2.get_transitive_files().to_list(),
        v1.get_transitive_files().to_list()
    );
    // Directories always have the same hash code.
    assert_eq!(root_digest(&v1), root_digest(&v2));

    // Edit a file in the directory and see that the value is rebuilt.
    std::fs::write(file1.as_path().get_path_string(), "bar").unwrap();
    let v3 = f.traverse_and_assert_files(
        &request,
        &[
            regular_file(file1.clone()),
            regular_file(file2.clone()),
            regular_file(file3.clone()),
        ],
    );
    assert_ne!(
        v3.get_transitive_files().to_list(),
        v2.get_transitive_files().to_list()
    );

    // Add a new file *outside* of the directory: the value is the same.
    f.create_file(
        &f.rooted_path(&f.source_artifact("somewhere/else/a.file")),
        "new file",
    );
    let v4 = f.traverse_and_assert_files(
        &request,
        &[
            regular_file(file1.clone()),
            regular_file(file2.clone()),
            regular_file(file3.clone()),
        ],
    );
    assert_eq!(
        v4.get_transitive_files().to_list(),
        v3.get_transitive_files().to_list()
    );

    // Add a new empty subdirectory: an empty directory node is part of the files.
    // PORT: Java's BasicFileTraversalRequest in this test does not emit empty directory nodes;
    // DirectoryArtifactTraversalRequest does.
    f.child_of(&dir, "empty_dir")
        .as_path()
        .create_directory()
        .unwrap();
    f.traverse_and_assert_files(
        &request,
        &[
            regular_file(file1),
            regular_file(file2),
            regular_file(file3),
            directory(f.child_of(&dir, "empty_dir")),
        ],
    );
}

#[test]
#[ignore = "tree artifact roots (generated) are not ported"]
fn test_traversal_of_source_tree_artifact() {}

#[test]
#[ignore = "generated roots (BasicFileTraversalRequest with isRootGenerated) are not ported"]
fn test_traversal_of_generated_directory() {}

#[test]
fn test_traversal_of_source_directory_with_empty_directory_nodes() {
    let f = Fixture::new();
    let dir = f.source_artifact("dir");
    dir.get_path().create_directory_and_parents().unwrap();
    let request = f.request(&dir);
    let root_node = directory(f.rooted_path(&dir));
    let v1 = f.traverse_and_assert_files(&request, &[root_node]);

    let empty_dir = f.child_of(&dir, "empty_dir");
    empty_dir.as_path().create_directory().unwrap();
    // The value only contains nodes for empty directories - the root dir is no longer empty at
    // this point and thus not represented as a node.
    let v2 = f.traverse_and_assert_files(&request, &[directory(empty_dir)]);
    assert_ne!(
        v2.get_transitive_files().to_list(),
        v1.get_transitive_files().to_list()
    );
    assert_eq!(root_digest(&v1), root_digest(&v2));
}

#[test]
fn test_traversal_of_transitive_symlink_to_directory() {
    let f = Fixture::new();
    let direct_link_artifact = f.source_artifact("direct/dir.sym");
    let transitive_link_artifact = f.source_artifact("transitive/sym.sym");
    let file_a = f.create_file(&f.rooted_path(&f.source_artifact("a/file.a")), "");
    let direct_link = f.rooted_path(&direct_link_artifact);
    let transitive_link = f.rooted_path(&transitive_link_artifact);
    f.symlink(&direct_link, "../a");
    f.symlink(&transitive_link, "../direct/dir.sym");
    let parent_of_a = file_a.get_parent_directory().unwrap();

    // Expect the file as if was a child of the direct symlink, not of the actual directory.
    f.traverse_and_assert_files(
        &f.request(&direct_link_artifact),
        &[
            symlink_to_directory(parent_of_a.clone(), direct_link, "../a"),
            regular_file(f.child_of(&direct_link_artifact, "file.a")),
        ],
    );

    // Expect the file as if was a child of the transitive symlink, not of the actual directory.
    f.traverse_and_assert_files(
        &f.request(&transitive_link_artifact),
        &[
            symlink_to_directory(parent_of_a, transitive_link, "../direct/dir.sym"),
            regular_file(f.child_of(&transitive_link_artifact, "file.a")),
        ],
    );
}

#[test]
fn test_traverse_package() {
    let f = Fixture::new();
    let build_file = f.source_artifact("pkg/BUILD");
    let build_file_path = f.create_file(&f.rooted_path(&build_file), "");
    let file1 = f.create_file(&f.rooted_path(&f.source_artifact("pkg/subdir/file.a")), "");
    f.traverse_and_assert_files(
        &f.request(&f.source_artifact("pkg")),
        &[regular_file(build_file_path), regular_file(file1)],
    );
}

#[test]
fn test_traversal_of_symlink_to_directory() {
    let f = Fixture::new();
    let link_name_artifact = f.source_artifact("link/foo.sym");
    let link_target_artifact = f.source_artifact("dir");
    let link_name = f.rooted_path(&link_name_artifact);
    let file1 = f.create_file(&f.child_of(&link_target_artifact, "file.1"), "");
    f.create_file(&f.child_of(&link_target_artifact, "sub/file.2"), "");
    f.symlink(&link_name, "../dir");

    // We expect to see all the files from the symlink'd directory, under the symlink's path, not
    // under the symlink target's path.
    let request = f.request(&link_name_artifact);
    let expected1 = symlink_to_directory(
        f.rooted_path(&link_target_artifact),
        link_name.clone(),
        "../dir",
    );
    let expected2 = regular_file(f.child_of(&link_name_artifact, "file.1"));
    let expected3 = regular_file(f.child_of(&link_name_artifact, "sub/file.2"));
    let v1 = f.traverse_and_assert_files(
        &request,
        &[expected1.clone(), expected2.clone(), expected3.clone()],
    );

    // Add a new file to the directory and see that the value is rebuilt.
    f.create_file(&f.child_of(&link_target_artifact, "file.3"), "");
    let expected4 = regular_file(f.child_of(&link_name_artifact, "file.3"));
    let v2 = f.traverse_and_assert_files(
        &request,
        &[
            expected1.clone(),
            expected2.clone(),
            expected3.clone(),
            expected4.clone(),
        ],
    );
    assert_ne!(root_digest(&v1), root_digest(&v2));

    // Edit a file in the directory and see that the value is rebuilt.
    std::fs::write(file1.as_path().get_path_string(), "bar").unwrap();
    let v3 = f.traverse_and_assert_files(
        &request,
        &[
            expected1.clone(),
            expected2.clone(),
            expected3.clone(),
            expected4.clone(),
        ],
    );
    assert_ne!(root_digest(&v2), root_digest(&v3));

    // Add a new file *outside* of the directory and see that the value is *not* rebuilt.
    f.create_file(
        &f.rooted_path(&f.source_artifact("somewhere/else/a.file")),
        "new file",
    );
    let v4 = f.traverse_and_assert_files(&request, &[expected1, expected2, expected3, expected4]);
    assert_eq!(root_digest(&v3), root_digest(&v4));
}

#[test]
fn test_traversal_of_dangling_symlink() {
    let f = Fixture::new();
    let link_artifact = f.source_artifact("a/dangling.sym");
    let link = f.rooted_path(&link_artifact);
    f.symlink(&link, "non_existent");
    f.traverse_and_assert_files(
        &f.request(&link_artifact),
        &[dangling_symlink(link, "non_existent")],
    );
}

#[test]
fn test_traversal_of_dangling_symlink_in_a_directory() {
    let f = Fixture::new();
    let dir_artifact = f.source_artifact("a");
    let file = f.create_file(&f.child_of(&dir_artifact, "file.txt"), "");
    let link = f.rooted_path(&f.source_artifact("a/dangling.sym"));
    f.symlink(&link, "non_existent");
    f.traverse_and_assert_files(
        &f.request(&dir_artifact),
        &[regular_file(file), dangling_symlink(link, "non_existent")],
    );
}

#[test]
fn test_report_error_when_traversing_subpackages() {
    let f = Fixture::new();
    let pkg_dir_artifact = f.source_artifact("pkg1/foo");
    let subpkg_dir_artifact = f.source_artifact("pkg1/foo/subdir/subpkg");
    f.create_file(&f.child_of(&pkg_dir_artifact, "BUILD"), "");
    f.create_file(&f.child_of(&subpkg_dir_artifact, "BUILD"), "");

    let e = f.eval(&f.request(&pkg_dir_artifact)).unwrap_err();
    assert_eq!(
        e.get_type(),
        RecursiveFilesystemTraversalExceptionType::CannotCrossPackageBoundary
    );
    assert!(
        e.get_message()
            .contains("crosses package boundary into package rooted at"),
        "{}",
        e
    );
}

#[test]
fn test_file_digest_change_causes_rebuild() {
    // PORT: Java checks that the FileStateValue invalidation re-evaluates the traversal; without
    // incrementality, the port checks that the file's metadata follows its contents.
    let f = Fixture::new();
    let dir = f.source_artifact("foo");
    let path = f.create_file(&f.child_of(&dir, "bar.txt"), "hello");
    let v1 = f.eval(&f.request(&dir)).unwrap();
    std::fs::write(path.as_path().get_path_string(), "world").unwrap();
    let v2 = f.eval(&f.request(&dir)).unwrap();
    assert_ne!(
        v1.get_transitive_files().to_list()[0].get_metadata(),
        v2.get_transitive_files().to_list()[0].get_metadata()
    );
}

#[test]
fn test_file_mtime_change_does_not_cause_rebuild_if_digest_is_unchanged() {
    let f = Fixture::new();
    let dir = f.source_artifact("foo");
    let path = f.create_file(&f.child_of(&dir, "bar.txt"), "hello");
    let v1 = f.eval(&f.request(&dir)).unwrap();
    std::fs::write(path.as_path().get_path_string(), "hello").unwrap();
    let v2 = f.eval(&f.request(&dir)).unwrap();
    assert_eq!(
        v1.get_transitive_files().to_list()[0].get_metadata(),
        v2.get_transitive_files().to_list()[0].get_metadata()
    );
}

#[test]
#[ignore = "generated roots (BasicFileTraversalRequest with isRootGenerated) are not ported"]
fn test_generated_directory_conflicts_with_package() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunctionTest.java:917 RecursiveFilesystemTraversalFunctionTest.unboundedSymlinkExpansionError
#[test]
fn unbounded_symlink_expansion_error() {
    let f = Fixture::new();
    let baz_link = f.source_artifact("foo/baz.sym");
    // scratch.dir("foo")
    let parent_dir = f.workspace.get_child("foo");
    parent_dir.create_directory_and_parents().unwrap();
    // bazLink.getPath().createSymbolicLink(parentDir)
    f.symlink(&f.rooted_path(&baz_link), parent_dir.get_path_string());
    // pkgRoot(parentOf(rootedPath(bazLink)))
    let e = f.eval(&f.request(&f.source_artifact("foo"))).unwrap_err();
    assert_eq!(
        e.get_type(),
        RecursiveFilesystemTraversalExceptionType::SymlinkCycleOrInfiniteExpansion,
        "{e}"
    );
    assert!(e.to_string().contains("Infinite symlink expansion"), "{e}");
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunctionTest.java:932 RecursiveFilesystemTraversalFunctionTest.symlinkChainError
#[test]
fn symlink_chain_error() {
    let f = Fixture::new();
    // scratch.dir("a")
    f.workspace
        .get_child("a")
        .create_directory_and_parents()
        .unwrap();
    let foo_link = f.source_artifact("a/foo.sym");
    let bar_link = f.source_artifact("a/bar.sym");
    let baz_link = f.source_artifact("a/baz.sym");
    f.symlink(
        &f.rooted_path(&foo_link),
        bar_link.get_path().get_path_string(),
    );
    f.symlink(
        &f.rooted_path(&bar_link),
        baz_link.get_path().get_path_string(),
    );
    f.symlink(
        &f.rooted_path(&baz_link),
        foo_link.get_path().get_path_string(),
    );

    // pkgRoot(parentOf(rootedPath(bazLink)))
    let e = f.eval(&f.request(&f.source_artifact("a"))).unwrap_err();
    assert_eq!(
        e.get_type(),
        RecursiveFilesystemTraversalExceptionType::SymlinkCycleOrInfiniteExpansion,
        "{e}"
    );
    assert!(e.to_string().contains("Symlink cycle"), "{e}");
}

#[test]
#[ignore = "generated roots (FileArtifactValue metadata of output files) are not ported"]
fn test_file_artifact_value_retains_data() {}

#[test]
#[ignore = "withDigest of a FileArtifactValue is only reached for generated roots (not ported)"]
fn test_with_digest_file_artifact_value() {}

#[test]
fn test_with_digest_file_state_value() {
    // RegularFileStateValue with actual digest will be transformed with the same digest
    let expected_bytes = vec![1u8, 2, 3];
    let with_digest_value = FileStateValue::RegularFileWithDigest {
        size: 10,
        digest: expected_bytes.clone(),
    };
    let result = with_digest(&with_digest_value, None).unwrap();
    assert!(matches!(result, HasDigest::FileArtifactValue(_)));
    assert_eq!(result.get_digest(), Some(expected_bytes.as_slice()));

    // FileStateValue will be transformed with fingerprinted digest
    let f = Fixture::new();
    let rooted_path = RootedPath::to_rooted_path(
        Root::from_path(f.root.get_child("bar")),
        &PathFragment::create("foo"),
    );
    let fsv = FileStateValue::create(&rooted_path).unwrap();
    let fsv_result = with_digest(&fsv, None).unwrap();
    assert!(matches!(fsv_result, HasDigest::ByteStringDigest(_)));
    assert!(fsv_result.get_digest().is_some());
}

#[test]
fn test_regular_file_state_value_without_digest() {
    let f = Fixture::new();
    let artifact = f.source_artifact("foo/fooy.txt");
    let rooted_path = f.create_file(&f.rooted_path(&artifact), "fooy-content");
    let status = rooted_path.as_path().stat().unwrap();
    let without_digest = FileStateValue::RegularFileWithContentsProxy {
        size: status.get_size(),
        contents_proxy: FileContentsProxy::create(&status),
    };
    let result = with_digest(&without_digest, Some(&rooted_path.as_path())).unwrap();
    // withDigest will construct a FileArtifactValue using the Path
    assert!(matches!(result, HasDigest::FileArtifactValue(_)));
    assert!(result.get_digest().is_some());
}

#[test]
#[ignore = "withDigest of a HasDigest.ByteStringDigest is only reached for generated roots (not ported)"]
fn test_with_digest_byte_string_digest() {}

#[test]
#[ignore = "needs a file system that reports a file and then fails to read it"]
fn test_gracefully_handles_inconsistent_filesystem() {}

/// ArtifactFunction.createSourceValue for a source directory: a hashed directory value over
/// the root-relative names and digests of the files under it.
#[test]
fn source_directory_artifact_value_is_hash_of_contents() {
    let f = Fixture::new();
    let dir = f.source_artifact("data");
    f.create_file(&f.child_of(&dir, "a.txt"), "a\n");
    f.create_file(&f.child_of(&dir, "sub/b.txt"), "b\n");
    let evaluator = f.evaluator();
    let key = ArtifactKey(dir.clone());
    let result = evaluator
        .evaluate(&[SkyKey::new(key.clone())], &EvaluationContext::default())
        .unwrap();
    let value = result.get(&key).unwrap_or_else(|| {
        let e = result.get_error(&SkyKey::new(key.clone())).unwrap();
        panic!(
            "{:?}",
            e.get_exception()
                .and_then(|e| e.downcast_ref::<SourceArtifactException>().cloned())
        )
    });
    let ArtifactValue::File(value) = &*value else {
        panic!("not a file value");
    };
    // Java: ArtifactFunction.createSourceValue (ArtifactFunction.java:364-369):
    // `new Fingerprint()` (SHA-256), then per file in traversal order (sorted dirents)
    // `fp.addString(file.getNameInSymlinkTree().getPathString())` (Fingerprint.java:235 addString:
    // CodedOutputStream.writeStringNoTag, a varint length and the UTF-8 bytes) and
    // `fp.addBytes(file.getMetadata().getDigest())` (Fingerprint.java:159, raw bytes, no
    // framing). The expected digest is computed here from that byte layout, not with Blaze's
    // Fingerprint. (No Java test asserts this value.)
    use sha2::Digest;
    let mut message: Vec<u8> = Vec::new();
    for (name, contents) in [("data/a.txt", "a\n"), ("data/sub/b.txt", "b\n")] {
        assert!(name.len() < 0x80, "one-byte varint");
        message.push(name.len() as u8);
        message.extend_from_slice(name.as_bytes());
        message.extend_from_slice(&sha2::Sha256::digest(contents.as_bytes()));
    }
    let expected_digest = sha2::Sha256::digest(&message).to_vec();
    assert_eq!(
        *value,
        blaze_exec::actions::FileArtifactValue::create_for_directory_with_hash(expected_digest)
    );
}
