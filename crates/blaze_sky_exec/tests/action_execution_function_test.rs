//! End-to-end tests of ACTION_EXECUTION / TARGET_COMPLETION / BUILD_DRIVER over a small action
//! graph in a temporary execroot. Expected texts come from the Java sources
//! (SkyframeActionExecutor.printError, ActionExecutionFunction.createSourceErrorCodeAndMessage,
//! ArtifactFunction.constructErrorMessage, OutputPermissions.READONLY), not from running Blaze.
//!
//! PORT: `ActionExecutionFunctionTest` in Java exercises input discovery and rewinding against a
//! full SkyframeExecutor; the tests below cover what this crate ports.

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::{
    ActionLookupData, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot,
    RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_events::{EventKind, StoredEventHandler};
use blaze_exec::actions::ActionExecutionException;
use blaze_sky_exec::analysis_seam::{
    ActionLookupValue, AnalysisSeam, ConfiguredTargetView, TestParams,
};
use blaze_sky_exec::events::{TargetCompleteEvent, downcast_event};
use blaze_sky_exec::keys::{ActionExecutionKey, BuildDriverKey, TestType};
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_skyframe::{ComputeError, Environment, EvaluationContext, EvaluatorBuilder, SkyKey};
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key(String);

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
    }
}

impl ActionLookupKey for Key {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical(&self.0).unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some("abc".to_string())
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().downcast_ref::<Key>() == Some(self)
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

#[derive(Debug)]
struct Target {
    key: ActionLookupKeyRef,
    actions: Vec<ActionRef>,
    default: NestedSet<Artifact>,
}

impl ActionLookupValue for Target {
    fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }
}

impl ConfiguredTargetView for Target {
    // PORT (harness/test): the target has no BUILD file; a fixed location.
    fn get_location(&self) -> blaze_events::location::Location {
        blaze_events::location::Location::from_file_line_column("/BUILD", 1, 1)
    }

    fn get_label(&self) -> Label {
        self.key.get_label().unwrap()
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some("abc".to_string())
    }
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        self.key.clone()
    }
    fn get_rule_class_or_kind(&self) -> String {
        "genrule".to_string()
    }
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        let mut m = IndexMap::new();
        m.insert("default".to_string(), self.default.clone());
        m
    }
    fn get_executable(&self) -> Option<Artifact> {
        None
    }
    fn get_test_params(&self) -> Option<TestParams> {
        None
    }
}

struct Seam(IndexMap<ActionLookupKeyRef, Arc<Target>>);

impl AnalysisSeam for Seam {
    fn get_action_lookup_value(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        Ok(Some(self.0[key].clone() as Arc<dyn ActionLookupValue>))
    }
    fn get_configured_target(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        Ok(Some(self.0[key].clone() as Arc<dyn ConfiguredTargetView>))
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    exec_root: Path,
    events: Arc<StoredEventHandler>,
}

fn owner(label: &str) -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical(label).unwrap()),
        Location::from_file_line_column("/ws/BUILD", 1, 1),
        "genrule rule",
        "k8-fastbuild",
        "abc",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    )
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = fs
            .get_path(dir.path().to_str().unwrap())
            .get_child("execroot");
        exec_root.create_directory_and_parents().unwrap();
        Fixture {
            _dir: dir,
            exec_root,
            events: Arc::new(StoredEventHandler::new()),
        }
    }

    fn bin(&self, key: &ActionLookupKeyRef, name: &str) -> Artifact {
        let root = ArtifactRoot::as_derived_root(
            &self.exec_root,
            RootType::OUTPUT,
            &["bazel-out", "k8-fastbuild", "bin"],
        );
        let exec_path = root.get_exec_path().get_relative_str(name);
        Artifact::create_derived_artifact(root, exec_path, key.clone())
    }

    fn source(&self, name: &str) -> Artifact {
        Artifact::new_source_artifact(
            ArtifactRoot::as_source_root(Root::from_path(self.exec_root.clone())),
            PathFragment::create(name),
            ArtifactOwner::NullOwner,
        )
    }

    fn evaluate(
        &self,
        seam: Seam,
        roots: Vec<SkyKey>,
        keep_going: bool,
    ) -> blaze_skyframe::EvaluationResult {
        blaze_exec::analysis::actions::register();
        let mut contexts = ActionContexts::default();
        for (type_id, ctx) in blaze_exec::analysis::actions::default_action_contexts("_main") {
            contexts.register_raw(type_id, ctx);
        }
        let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
            exec_root: self.exec_root.clone(),
            action_out_dir: self
                .exec_root
                .get_parent_directory()
                .unwrap()
                .get_child("action_outs"),
            external_directory: self
                .exec_root
                .get_parent_directory()
                .unwrap()
                .get_child("external"),
            reporter: self.events.clone(),
            contexts,
            client_env: IndexMap::new(),
            keep_going,
            verbose_failures: false,
            output_permissions: OutputPermissions::READONLY,
        }));
        // ARTIFACT requests the FILE value of a source artifact (Java's TimestampBuilderTestCase
        // registers FILE_STATE and FILE with it).
        let root = self.exec_root.get_parent_directory().unwrap();
        let config =
            blaze_sky_loading::skyframe_executor::LoadingFunctionsConfig::for_bazel_default(
                blaze_sky_loading::directories::LoadingDirectories {
                    workspace: self.exec_root.clone(),
                    output_base: root.get_child("output_base"),
                    install_base: root.get_child("install_base"),
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
        let evaluator =
            blaze_sky_exec::register_execution_functions(builder, executor, Arc::new(seam), None)
                .build();
        let ctx = EvaluationContext {
            keep_going,
            event_handler: self.events.clone(),
            ..EvaluationContext::default()
        };
        evaluator.evaluate(&roots, &ctx).unwrap()
    }
}

fn target(
    key: &ActionLookupKeyRef,
    actions: Vec<ActionRef>,
    default: Vec<Artifact>,
) -> Arc<Target> {
    for (i, a) in actions.iter().enumerate() {
        for o in a.get_outputs() {
            o.set_generating_action_key(ActionLookupData::create(key.clone(), i as i32));
        }
    }
    let mut b = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    for d in default {
        b.add(d);
    }
    Arc::new(Target {
        key: key.clone(),
        actions,
        default: b.build(),
    })
}

fn driver_key(key: &ActionLookupKeyRef) -> SkyKey {
    SkyKey::new(BuildDriverKey {
        action_lookup_key: key.clone(),
        output_groups: vec!["default".to_string()],
        explicitly_requested: true,
        skip_incompatible_explicit_targets: false,
        extra_action_top_level_only: false,
        test_type: TestType::NOT_TEST,
        keep_going: false,
    })
}

#[test]
fn file_write_then_symlink_builds_and_sets_output_permissions() {
    let f = Fixture::new();
    let key = ActionLookupKeyRef::new(Key("//pkg:t".to_string()));
    let out = f.bin(&key, "pkg/out.txt");
    let link = f.bin(&key, "pkg/link.txt");
    let write: ActionRef = Arc::new(FileWriteAction::create(
        owner("//pkg:t"),
        out.clone(),
        "hello\n".to_string(),
        false,
        Compression::DISALLOW,
    ));
    let symlink: ActionRef = Arc::new(SymlinkAction::to_artifact(
        owner("//pkg:t"),
        out.clone(),
        link.clone(),
        None,
    ));
    let t = target(&key, vec![write, symlink], vec![link.clone()]);
    let mut m = IndexMap::new();
    m.insert(key.clone(), t);
    let result = f.evaluate(Seam(m), vec![driver_key(&key)], false);
    assert!(!result.has_error(), "{:?}", result.error_map());
    let out_path = out.get_path();
    assert_eq!(
        std::fs::read_to_string(out_path.get_path_string()).unwrap(),
        "hello\n"
    );
    // OutputPermissions.READONLY: 0555.
    assert_eq!(out_path.stat().unwrap().get_permissions() & 0o777, 0o555);
    assert!(link.get_path().is_symbolic_link());
    let completed: Vec<TargetCompleteEvent> = f
        .events
        .get_posts()
        .iter()
        .filter_map(|p| downcast_event::<TargetCompleteEvent>(p).cloned())
        .collect();
    assert_eq!(completed.len(), 1);
    assert!(!completed[0].failed());
}

#[test]
fn missing_mandatory_source_input_fails_with_java_messages() {
    let f = Fixture::new();
    let key = ActionLookupKeyRef::new(Key("//pkg:t".to_string()));
    let input = f.source("pkg/missing.txt");
    let link = f.bin(&key, "pkg/link.txt");
    let symlink: ActionRef = Arc::new(SymlinkAction::to_artifact(
        owner("//pkg:t"),
        input,
        link.clone(),
        None,
    ));
    let t = target(&key, vec![symlink], vec![link.clone()]);
    let mut m = IndexMap::new();
    m.insert(key.clone(), t);
    let action_key = ActionExecutionKey(link.get_generating_action_key().clone());
    let result = f.evaluate(Seam(m), vec![SkyKey::new(action_key.clone())], true);
    let error = result.get_error(&SkyKey::new(action_key)).expect("error");
    let e = error
        .get_exception()
        .unwrap()
        .downcast_ref::<ActionExecutionException>()
        .unwrap()
        .clone();
    // ActionExecutionFunction.createSourceErrorCodeAndMessage
    assert_eq!(e.get_message(), "1 input file(s) do not exist");
    let errors: Vec<String> = f
        .events
        .get_events()
        .iter()
        .filter(|e| e.get_kind() == EventKind::Error)
        .map(|e| e.get_message().to_string())
        .collect();
    // SkyframeActionExecutor.printError for the missing input, then for the thrown exception.
    // The source artifact has no owner: ArtifactFunction.constructErrorMessage uses its exec path.
    // AbstractAction.defaultProgressMessage: "<mnemonic> <primary output>".
    let describe = "Symlink pkg/link.txt";
    assert!(
        errors.contains(&format!(
            "{describe} failed: missing input file 'pkg/missing.txt'"
        )),
        "{errors:?}"
    );
    assert!(
        errors.contains(&format!("{describe} failed: 1 input file(s) do not exist")),
        "{errors:?}"
    );
}

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
