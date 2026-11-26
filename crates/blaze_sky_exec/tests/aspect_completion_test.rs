//! `BuildDriverFunction` for a `TopLevelAspectsKey` and `CompletionFunction` with the
//! `AspectCompletor` (`ASPECT_COMPLETION`), over a fake analysis seam.
//!
//! PORT: the JUnit tests that cover these classes at the pin are integration tests of the
//! analysis side (`AspectTest.aspectApplyingToFiles` and the `AspectCompleteEvent` recorder of
//! `AspectTest`, `RewindingTestsHelper`), which need the analysis test harness of blaze_sky_top;
//! the expectations below are the Java code's (AspectCompletor.getRootCauseError,
//! CompletionFunction.compute's `locationPrefix + " " + message`, AspectCompleteEvent).

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::{ActionLookupKey, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{EventKind, Location, StoredEventHandler};
use blaze_sky_exec::analysis_seam::{
    ActionLookupValue, AnalysisSeam, ConfiguredAspectView, ConfiguredTargetView, TestParams,
};
use blaze_sky_exec::build_driver_function::TopLevelSkyValue;
use blaze_sky_exec::events::{AspectCompleteEvent, downcast_event};
use blaze_sky_exec::keys::BuildDriverKey;
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_skyframe::{ComputeError, Environment, EvaluationContext, EvaluatorBuilder, SkyKey};
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

/// An action lookup key of a given function name.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    function: &'static str,
    name: String,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.function, self.name)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ActionLookupKey for Key {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical(self.name.split(' ').next().unwrap()).unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        self.function
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().downcast_ref::<Key>() == Some(self)
    }
    fn key_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn key(function: &'static str, name: &str) -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(Key {
        function,
        name: name.to_owned(),
    })
}

struct Aspect {
    key: ActionLookupKeyRef,
    class_name: String,
    groups: IndexMap<String, NestedSet<Artifact>>,
}

impl ConfiguredAspectView for Aspect {
    fn get_label(&self) -> Label {
        Label::parse_canonical("//pkg:t").unwrap()
    }
    fn get_aspect_class_name(&self) -> String {
        self.class_name.clone()
    }
    fn get_base_configured_target_key(&self) -> ActionLookupKeyRef {
        key("CONFIGURED_TARGET", "//pkg:t")
    }
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        self.groups.clone()
    }
    fn get_aspect_key(&self) -> ActionLookupKeyRef {
        self.key.clone()
    }
    fn get_writes_output_to_master_log(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct BaseTarget;

impl ConfiguredTargetView for BaseTarget {
    fn get_label(&self) -> Label {
        Label::parse_canonical("//pkg:t").unwrap()
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        key("CONFIGURED_TARGET", "//pkg:t")
    }
    fn get_rule_class_or_kind(&self) -> String {
        "my_rule".to_owned()
    }
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        IndexMap::new()
    }
    fn get_executable(&self) -> Option<Artifact> {
        None
    }
    fn get_test_params(&self) -> Option<TestParams> {
        None
    }
    fn get_location(&self) -> Location {
        Location::from_file_line_column("/ws/pkg/BUILD", 3, 8)
    }
}

struct Seam {
    aspects: Vec<Arc<Aspect>>,
}

impl AnalysisSeam for Seam {
    fn get_action_lookup_value(
        &self,
        _key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        unreachable!()
    }
    fn get_configured_target(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        assert_eq!(key.function_name(), "CONFIGURED_TARGET");
        Ok(Some(Arc::new(BaseTarget)))
    }
    fn get_top_level_aspects(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Vec<Arc<dyn ConfiguredAspectView>>>, ComputeError> {
        assert_eq!(key.function_name(), "TOP_LEVEL_ASPECTS");
        Ok(Some(
            self.aspects
                .iter()
                .map(|a| a.clone() as Arc<dyn ConfiguredAspectView>)
                .collect(),
        ))
    }
    fn get_configured_aspect(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredAspectView>>, ComputeError> {
        Ok(self
            .aspects
            .iter()
            .find(|a| &a.key == key)
            .map(|a| a.clone() as Arc<dyn ConfiguredAspectView>))
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

    fn source_artifact(&self, path: &str, owner: &str) -> Artifact {
        Artifact::new_source_artifact(
            ArtifactRoot::as_source_root(Root::from_path(self.root.clone())),
            PathFragment::create(path),
            ArtifactOwner::Label(Label::parse_canonical(owner).unwrap()),
        )
    }

    fn build(
        &self,
        seam: Seam,
        driver_key: &BuildDriverKey,
    ) -> (blaze_skyframe::EvaluationResult, Arc<StoredEventHandler>) {
        let events = Arc::new(StoredEventHandler::new());
        let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
            exec_root: self.root.clone(),
            action_out_dir: self.root.get_child("action_outs"),
            external_directory: self.root.get_child("external"),
            reporter: events.clone(),
            contexts: ActionContexts::default(),
            client_env: IndexMap::new(),
            keep_going: false,
            verbose_failures: false,
            output_permissions: OutputPermissions::READONLY,
        }));
        let evaluator = blaze_sky_exec::register_execution_functions(
            with_loading_functions(EvaluatorBuilder::new(), &self.root),
            executor,
            Arc::new(seam),
            None,
        )
        .build();
        let ctx = EvaluationContext {
            event_handler: events.clone(),
            ..EvaluationContext::default()
        };
        let result = evaluator
            .evaluate(&[SkyKey::new(driver_key.clone())], &ctx)
            .unwrap();
        (result, events)
    }
}

fn driver_key() -> BuildDriverKey {
    BuildDriverKey::of_top_level_aspect(
        key("TOP_LEVEL_ASPECTS", "//pkg:t //pkg:a.bzl%a"),
        vec!["default".to_owned()],
        true,
        false,
        false,
    )
}

fn aspect_events(events: &StoredEventHandler) -> Vec<AspectCompleteEvent> {
    events
        .get_posts()
        .iter()
        .filter_map(|p| downcast_event::<AspectCompleteEvent>(p).cloned())
        .collect()
}

#[test]
fn of_top_level_aspect_is_an_aspect_driver_and_not_a_test() {
    let k = driver_key();
    assert!(k.is_top_level_aspect_driver());
    assert!(!k.test_type.is_test());
    assert_eq!(
        format!("{k:?}"),
        "BuildDriverKey of ActionLookupKey: TOP_LEVEL_ASPECTS://pkg:t //pkg:a.bzl%a"
    );
}

#[test]
#[should_panic(expected = "is not a TopLevelAspectsKey")]
fn of_top_level_aspect_rejects_a_configured_target_key() {
    BuildDriverKey::of_top_level_aspect(
        key("CONFIGURED_TARGET", "//pkg:t"),
        vec![],
        true,
        false,
        false,
    );
}

#[test]
fn build_driver_requests_one_aspect_completion_per_top_level_aspect() {
    let f = Fixture::new();
    let file = f.source_artifact("pkg/in.txt", "//pkg:in.txt");
    std::fs::create_dir_all(format!("{}/pkg", f.root.get_path_string())).unwrap();
    std::fs::write(file.get_path().get_path_string(), "in").unwrap();
    let mut groups = IndexMap::new();
    groups.insert(
        "default".to_owned(),
        NestedSetBuilder::create(Order::STABLE_ORDER, [file.clone()]),
    );
    let seam = Seam {
        aspects: vec![
            Arc::new(Aspect {
                key: key("ASPECT", "//pkg:t a1"),
                class_name: "//pkg:a.bzl%a".to_owned(),
                groups,
            }),
            Arc::new(Aspect {
                key: key("ASPECT", "//pkg:t a2"),
                class_name: "//pkg:a.bzl%b".to_owned(),
                groups: IndexMap::new(),
            }),
        ],
    };
    let driver = driver_key();
    let (result, events) = f.build(seam, &driver);
    assert!(!result.has_error(), "{result:?}");
    let value = result.get(&driver).unwrap();
    let TopLevelSkyValue::TopLevelAspects(aspects) = &value.top_level_sky_value else {
        panic!("not a top-level aspects value");
    };
    assert_eq!(aspects.len(), 2);
    assert!(!value.skipped);

    let completed = aspect_events(&events);
    let names: Vec<&str> = completed.iter().map(|e| e.aspect_name.as_str()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"//pkg:a.bzl%a") && names.contains(&"//pkg:a.bzl%b"));
    let first = completed
        .iter()
        .find(|e| e.aspect_name == "//pkg:a.bzl%a")
        .unwrap();
    assert!(!first.failed());
    assert!(first.print_to_master_log);
    assert_eq!(first.artifacts_by_output_group.len(), 1);
    assert_eq!(first.artifacts_by_output_group[0].0, "default");
    assert_eq!(first.important_input_map.len(), 1);
    assert_eq!(first.important_input_map[0].0, file);
}

#[test]
fn aspect_completion_reports_a_missing_source_file() {
    let f = Fixture::new();
    let missing = f.source_artifact("pkg/missing.txt", "//pkg:missing.txt");
    let mut groups = IndexMap::new();
    groups.insert(
        "default".to_owned(),
        NestedSetBuilder::create(Order::STABLE_ORDER, [missing]),
    );
    let seam = Seam {
        aspects: vec![Arc::new(Aspect {
            key: key("ASPECT", "//pkg:t a1"),
            class_name: "//pkg:a.bzl%a".to_owned(),
            groups,
        })],
    };
    let (result, events) = f.build(seam, &driver_key());
    assert!(result.has_error());
    let errors: Vec<(Option<Location>, String)> = events
        .get_events()
        .iter()
        .filter(|e| e.get_kind() == EventKind::Error)
        .map(|e| (e.get_location().cloned(), e.get_message().to_string()))
        .collect();
    // AspectCompletor.getRootCauseError: at the base target's location.
    assert!(
        errors.contains(&(
            Some(Location::from_file_line_column("/ws/pkg/BUILD", 3, 8)),
            "//pkg:t, aspect //pkg:a.bzl%a: missing input file '//pkg:missing.txt'".to_owned()
        )),
        "{errors:?}"
    );
    // CompletionFunction.compute: `locationPrefix + " " + message` without a location.
    assert!(
        errors.contains(&(
            None,
            "//pkg:t, aspect //pkg:a.bzl%a 1 input file(s) do not exist".to_owned()
        )),
        "{errors:?}"
    );
    let completed = aspect_events(&events);
    assert_eq!(completed.len(), 1);
    assert!(completed[0].failed());
    assert_eq!(
        completed[0].root_causes,
        vec![Label::parse_canonical("//pkg:missing.txt").unwrap()]
    );
}

/// No external repository (the loading functions the FILE dependency of ARTIFACT needs).
struct NoRepos;

impl blaze_sky_loading::repos::ExternalRepos for NoRepos {
    fn compute_repository_mapping(
        &self,
        _repository_name: &blaze_cmdline::RepositoryName,
        _env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<
        Option<blaze_sky_loading::repos::RepositoryMappingValue>,
        blaze_skyframe::ComputeError,
    > {
        Ok(Some(
            blaze_sky_loading::repos::RepositoryMappingValue::value_for_empty_root_module(),
        ))
    }

    fn compute_repository_directory(
        &self,
        repository_name: &blaze_cmdline::RepositoryName,
        _env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<
        Option<blaze_sky_loading::repos::RepositoryDirectoryValue>,
        blaze_skyframe::ComputeError,
    > {
        panic!("no external repository here: {repository_name}")
    }
}

/// The loading functions (FILE for source artifacts) and the per-command BUILD_ID that the
/// execution functions request (Java's test cases register FILE_STATE and FILE).
fn with_loading_functions(
    builder: blaze_skyframe::EvaluatorBuilder,
    workspace: &blaze_vfs::Path,
) -> blaze_skyframe::EvaluatorBuilder {
    let config = blaze_sky_loading::skyframe_executor::LoadingFunctionsConfig::for_bazel_default(
        blaze_sky_loading::directories::LoadingDirectories {
            workspace: workspace.clone(),
            output_base: workspace.get_child("output_base_for_loading"),
            install_base: workspace.get_child("install_base_for_loading"),
        },
        std::sync::Arc::new(NoRepos),
    );
    let builder =
        blaze_sky_loading::skyframe_executor::register_loading_functions(builder, &config);
    let builder = blaze_sky_loading::skyframe_executor::inject_loading_precomputed(
        builder,
        &config,
        blaze_starlark::StarlarkSemantics::from_flags(&Default::default()).unwrap(),
        indexmap::IndexMap::new(),
    );
    blaze_sky_loading::precomputed::BUILD_ID.set(builder, String::new())
}
