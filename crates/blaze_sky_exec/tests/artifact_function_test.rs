//! Port of `ArtifactFunctionTest` (src/test/java/com/google/devtools/build/lib/skyframe), on a
//! real temporary directory instead of Java's in-memory file system.

use std::sync::Arc;

use blaze_actions::{ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot};
use blaze_events::StoredEventHandler;
use blaze_exec::actions::FileArtifactValue;
use blaze_sky_exec::analysis_seam::{ActionLookupValue, AnalysisSeam, ConfiguredTargetView};
use blaze_sky_exec::artifact_function::{ArtifactValue, SourceArtifactException};
use blaze_sky_exec::keys::ArtifactKey;
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_skyframe::{ComputeError, Environment, EvaluationContext, EvaluatorBuilder, SkyKey};
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, Root, UnixFileSystem};
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

struct Fixture {
    _dir: tempfile::TempDir,
    root: Path,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = fs.get_path(dir.path().to_str().unwrap());
        Fixture { _dir: dir, root }
    }

    // Java: ArtifactFunctionTestCase.createSourceArtifact
    fn create_source_artifact(&self, path: &str) -> Artifact {
        Artifact::new_source_artifact(
            ArtifactRoot::as_source_root(Root::from_path(self.root.clone())),
            PathFragment::create(path),
            ArtifactOwner::NullOwner,
        )
    }

    fn evaluate_artifact_value(
        &self,
        artifact: &Artifact,
    ) -> Result<ArtifactValue, SourceArtifactException> {
        let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
            exec_root: self.root.clone(),
            action_out_dir: self.root.get_child("action_outs"),
            external_directory: self.root.get_child("external"),
            reporter: Arc::new(StoredEventHandler::new()),
            contexts: ActionContexts::default(),
            client_env: IndexMap::new(),
            keep_going: false,
            verbose_failures: false,
            output_permissions: OutputPermissions::READONLY,
        }));
        // Java's ArtifactFunctionTestCase registers FILE_STATE and FILE with ARTIFACT, which
        // requests the FILE value of a source artifact. The package root is the test's root.
        let config = blaze_sky_loading::skyframe_executor::LoadingFunctionsConfig::for_bazel_default(
            blaze_sky_loading::directories::LoadingDirectories {
                workspace: self.root.clone(),
                output_base: self.root.get_child("output_base"),
                install_base: self.root.get_child("install_base"),
            },
            Arc::new(NoRepos),
        );
        let mut builder = blaze_sky_loading::skyframe_executor::register_loading_functions(
            EvaluatorBuilder::new(),
            &config,
        );
        builder = blaze_sky_loading::skyframe_executor::inject_loading_precomputed(
            builder,
            &config,
            blaze_starlark::StarlarkSemantics::from_flags(&Default::default()).unwrap(),
            IndexMap::new(),
        );
        builder = blaze_sky_loading::precomputed::BUILD_ID.set(builder, String::new());
        let evaluator = blaze_sky_exec::register_execution_functions(
            builder,
            executor,
            Arc::new(NoAnalysis),
            None,
        )
        .build();
        let key = ArtifactKey(artifact.clone());
        let result = evaluator
            .evaluate(&[SkyKey::new(key.clone())], &EvaluationContext::default())
            .unwrap();
        if let Some(v) = result.get(&key) {
            return Ok((*v).clone());
        }
        let error = result.get_error(&SkyKey::new(key)).unwrap();
        let e = error.get_exception().unwrap();
        Err(e.downcast_ref::<SourceArtifactException>().unwrap().clone())
    }
}

fn assert_file_artifact_value_matches() {
    let f = Fixture::new();
    let output = f.create_source_artifact("output");
    let path = output.get_path();
    std::fs::write(path.get_path_string(), b"contents").unwrap();
    let value = f.evaluate_artifact_value(&output).unwrap();
    let ArtifactValue::File(value) = value else {
        panic!("not a file value: {value:?}")
    };
    // Java: assertValueMatches(path.stat(), path.getDigest(), value)
    assert_eq!(value.get_size(), path.stat().unwrap().get_size());
    assert_eq!(
        value.get_digest(),
        Some(path.get_digest().unwrap().as_slice())
    );
    assert!(matches!(value, FileArtifactValue::RegularFile { .. }));
}

#[test]
fn test_basic_artifact() {
    assert_file_artifact_value_matches();
}

#[test]
#[ignore = "needs a file system with extended-attribute fast digests (xattr provider)"]
fn test_basic_artifact_with_xattr() {
    assert_file_artifact_value_matches();
}

#[test]
fn test_missing_non_mandatory_artifact() {
    let f = Fixture::new();
    let input = f.create_source_artifact("input1");
    let value = f.evaluate_artifact_value(&input).unwrap();
    assert!(matches!(value, ArtifactValue::Missing(_)));
}

#[test]
#[ignore = "needs a custom in-memory file system returning a digest for an unreadable file"]
fn test_unreadable_input_with_fs_with_available_digest() {}

#[test]
#[ignore = "needs a custom in-memory file system whose statIfFound throws"]
fn test_io_exception_end_to_end() {}

#[test]
#[ignore = "needs a tree-artifact-producing action in ArtifactFunctionTestCase's action graph"]
fn test_action_tree_artifact_output() {}

#[test]
#[ignore = "needs action templates (ACTION_TEMPLATE_EXPANSION), not ported"]
fn test_spawn_action_template() {}

#[test]
#[ignore = "needs action templates (ACTION_TEMPLATE_EXPANSION), not ported"]
fn test_consecutive_spawn_action_templates() {}

#[test]
#[ignore = "needs action templates (ACTION_TEMPLATE_EXPANSION), not ported"]
fn test_action_template_generates_multiple_output_trees_from_different_actions() {}

/// No external repository (the loading functions the FILE dependency of ARTIFACT needs).
struct NoRepos;

impl blaze_sky_loading::repos::ExternalRepos for NoRepos {
    fn compute_repository_mapping(
        &self,
        _repository_name: &blaze_cmdline::RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<blaze_sky_loading::repos::RepositoryMappingValue>, ComputeError> {
        Ok(Some(
            blaze_sky_loading::repos::RepositoryMappingValue::value_for_empty_root_module(),
        ))
    }

    fn compute_repository_directory(
        &self,
        repository_name: &blaze_cmdline::RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<blaze_sky_loading::repos::RepositoryDirectoryValue>, ComputeError> {
        panic!("no external repository in this test: {repository_name}")
    }
}
