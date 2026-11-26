//! Test harness for scripts/w7/replay-tests: builds `TestRunnerAction`s from the oracle's
//! recorded aquery (as prepared by the driver script) and executes them through
//! `TestRunnerAction.execute` with `StandaloneTestStrategy` registered as the
//! `TestActionContext`, Blaze's spawn strategies (linux-sandbox) and an expanded exec log.
//!
//! Not a port of a Java class. Usage: `blaze-test-replay --spec <spec.json>
//! --execution-log-json <out.json> --results <results.json>`; the spec is written by
//! scripts/w7/replay-tests (see its help).

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use blaze_actions::actions::action::Action;
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::command_line;
use blaze_actions::actions::platform_info::{ConstraintValueData, PlatformInfoData};
use blaze_actions::{
    ActionEnvironment, ActionKeyContext, ActionLookupKeyRef, ActionOwner,
    Artifact as BlazeArtifact, ArtifactOwner, ArtifactRoot, PlatformInfoRef, RootType,
    SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_events::{Clock, ExtendedEventHandler, JavaClock, StoredEventHandler};
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::actions::resource_manager::ResourceManager;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInput, ActionInputRef, Artifact, FileArtifactValue,
    FilesetOutputTree, InputMetadataProvider, RunfilesArtifactValue, RunfilesTree, Spawn,
    TreeArtifactValue, as_executable,
};
use blaze_exec::analysis::test::test_action_context::TestActionContext;
use blaze_exec::analysis::test::test_attempt::TestAttempt;
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_exec::analysis::test::test_runner_action::{
    TestRunnerAction, TestRunnerActionConfiguration,
};
use blaze_exec::analysis::test::test_strategy::TestStrategy;
use blaze_exec::analysis::test::test_target_execution_settings::TestTargetExecutionSettings;
use blaze_exec::analysis::test::test_target_properties::TestTargetProperties;
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::executor_setup::build_spawn_strategy_registry;
use blaze_exec::exec::expanded_spawn_log_context::{Encoding, ExpandedSpawnLogContext};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::exec::standalone_test_strategy::StandaloneTestStrategy;
use blaze_exec::exec::{BinTools, ExecCommandEnvironment, ProgressStatus};
use blaze_exec::sandbox::linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
use blaze_exec::sandbox::sandbox_module::SandboxModule;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::{TestTimeout, TestTimeoutConverter};
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptionsCancelConcurrentTests;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::{Options, TriState};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Root, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};

/// A test `ActionLookupKey` standing in for `ConfiguredTargetKey`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ReplayTargetKey {
    label: Label,
    checksum: String,
}

impl fmt::Display for ReplayTargetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ConfiguredTargetKey{{label={}, config={}}}",
            self.label, self.checksum
        )
    }
}

impl ActionLookupKey for ReplayTargetKey {
    fn get_label(&self) -> Option<Label> {
        Some(self.label.clone())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some(self.checksum.clone())
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<ReplayTargetKey>()
            .is_some_and(|o| o == self)
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The host platform `@@platforms//host:host` as the oracle's analysis built it.
fn host_platform() -> PlatformInfoRef {
    let l = |s: &str| Label::parse_canonical(s).unwrap();
    Arc::new(PlatformInfoData {
        label: l("@@platforms//host:host"),
        constraints_parent: None,
        constraints: vec![
            ConstraintValueData {
                setting: l("@@platforms//cpu:cpu"),
                value: l("@@platforms//cpu:x86_64"),
            },
            ConstraintValueData {
                setting: l("@@platforms//os:os"),
                value: l("@@platforms//os:linux"),
            },
        ],
        remote_execution_properties: String::new(),
        exec_properties: IndexMap::new(),
        flags: Vec::new(),
        required_settings: Vec::new(),
        allowed_toolchain_types: Vec::new(),
        check_toolchain_types: false,
        missing_toolchain_error_message: Some(
            "For more information on platforms or toolchains see \
             https://bazel.build/concepts/platforms-intro."
                .to_string(),
        ),
    })
}

/// A runfiles tree rebuilt from the recorded runfiles inputs (Java: the action's
/// `RunfilesTree`): runfiles-relative path to an input at the recorded (display) path, or to
/// nothing for an empty file.
#[derive(Debug)]
struct ReplayRunfilesTree {
    exec_path: PathFragment,
    mapping: Vec<(PathFragment, Option<ActionInputRef>)>,
}

impl RunfilesTree for ReplayRunfilesTree {
    fn get_exec_path(&self) -> PathFragment {
        self.exec_path.clone()
    }
    fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
        self.mapping.clone()
    }
    fn get_workspace_name(&self) -> String {
        "_main".to_string()
    }
    fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
        blaze_util::unported!("ReplayRunfilesTree (compact log)")
    }
    fn get_symlinks_for_logging(&self) -> NestedSet<blaze_exec::actions::SymlinkEntry> {
        blaze_util::unported!("ReplayRunfilesTree (compact log)")
    }
    fn get_root_symlinks_for_logging(&self) -> NestedSet<blaze_exec::actions::SymlinkEntry> {
        blaze_util::unported!("ReplayRunfilesTree (compact log)")
    }
    fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
        blaze_util::unported!("ReplayRunfilesTree (compact log)")
    }
    fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef> {
        blaze_util::unported!("ReplayRunfilesTree (compact log)")
    }
}

/// A plain derived file in a runfiles mapping, at its recorded runfiles path.
#[derive(Debug)]
struct RunfilesEntryArtifact {
    exec_path: PathFragment,
}

impl ActionInput for RunfilesEntryArtifact {
    fn get_exec_path(&self) -> &PathFragment {
        &self.exec_path
    }
    fn is_directory(&self) -> bool {
        false
    }
    fn is_symlink(&self) -> bool {
        false
    }
    fn as_artifact(&self) -> Option<&dyn Artifact> {
        Some(self)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Artifact for RunfilesEntryArtifact {
    fn is_source_artifact(&self) -> bool {
        false
    }
    fn is_tree_artifact(&self) -> bool {
        false
    }
    fn is_runfiles_tree(&self) -> bool {
        false
    }
    fn is_fileset(&self) -> bool {
        false
    }
    fn get_root_relative_path(&self) -> PathFragment {
        self.exec_path.sub_fragment_from(2)
    }
    fn get_root_exec_path(&self) -> PathFragment {
        self.exec_path.sub_fragment(0, 2)
    }
}

/// Metadata of the scratch execroot's files, computed on first use (Java: the action's
/// `ActionInputMap`).
struct ReplayMetadataProvider {
    exec_root: Path,
    cache: Mutex<HashMap<String, Option<FileArtifactValue>>>,
    runfiles: Mutex<HashMap<String, Arc<RunfilesArtifactValue>>>,
}

impl InputMetadataProvider for ReplayMetadataProvider {
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        let key = input.get_exec_path_string();
        let mut cache = self.cache.lock().unwrap();
        if let Some(v) = cache.get(&key) {
            return Ok(v.clone());
        }
        let path = self.exec_root.get_relative(input.get_exec_path());
        let v = if path.exists() {
            Some(FileArtifactValue::create_for_testing(&path)?)
        } else {
            None
        };
        cache.insert(key, v.clone());
        Ok(v)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles
            .lock()
            .unwrap()
            .get(&input.get_exec_path_string())
            .cloned()
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// The harness's `ActionExecutionContext` (Java: built by `SkyframeActionExecutor` per action).
struct ReplayActionContext<'a> {
    exec_root: Path,
    metadata: &'a ReplayMetadataProvider,
    out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    events: &'a StoredEventHandler,
    clock: JavaClock,
    registry: Arc<SpawnStrategyRegistry>,
    log_context: Arc<dyn SpawnLogContext>,
    test_action_context: ActionContextRef,
}

impl ActionContextRegistry for ReplayActionContext<'_> {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        Some(self.registry.clone())
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        Some(self.log_context.clone())
    }
    fn get_context_any(&self, type_id: TypeId) -> Option<ActionContextRef> {
        if type_id == TypeId::of::<Arc<dyn TestActionContext>>() {
            return Some(self.test_action_context.clone());
        }
        None
    }
}

impl ActionExecutionContext for ReplayActionContext<'_> {
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        self.metadata
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        self.events
    }
    fn get_clock(&self) -> &dyn Clock {
        &self.clock
    }
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {}
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {}
}

fn arg(args: &[String], name: &str) -> String {
    let i = args
        .iter()
        .position(|a| a == name)
        .unwrap_or_else(|| panic!("missing {name}"));
    args[i + 1].clone()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_string()
}

/// The artifact roots of the scratch execroot.
struct Roots {
    exec_root: Path,
    output_base: Path,
    workspace: Path,
}

impl Roots {
    fn root_for(&self, exec_path: &str) -> ArtifactRoot {
        let segs: Vec<&str> = exec_path.split('/').collect();
        if segs[0] == "bazel-out" {
            return ArtifactRoot::as_derived_root(
                &self.exec_root,
                RootType::OUTPUT,
                &["bazel-out", segs[1], segs[2]],
            );
        }
        if segs[0] == "external" {
            return ArtifactRoot::as_external_source_root(Root::from_path(
                self.output_base
                    .get_relative_str(&format!("external/{}", segs[1])),
            ));
        }
        ArtifactRoot::as_source_root(Root::from_path(self.workspace.clone()))
    }

    fn artifact(&self, exec_path: &str, owner: &ActionLookupKeyRef, kind: &str) -> BlazeArtifact {
        let root = self.root_for(exec_path);
        let exec_path = PathFragment::create(exec_path);
        if root.is_source_root() {
            return BlazeArtifact::new_source_artifact(root, exec_path, ArtifactOwner::NullOwner);
        }
        match kind {
            "tree" => BlazeArtifact::create_special_artifact(
                root,
                exec_path,
                owner.clone(),
                SpecialArtifactType::TREE,
            ),
            "runfiles" => BlazeArtifact::create_special_artifact(
                root,
                exec_path,
                owner.clone(),
                SpecialArtifactType::RUNFILES,
            ),
            _ => BlazeArtifact::create_derived_artifact(root, exec_path, owner.clone()),
        }
    }
}

/// Builds the `TestRunnerAction` of one recorded test (Java: `TestActionBuilder`).
fn build_action(t: &Value, roots: &Roots, fs: &FileSystemRef) -> Arc<TestRunnerAction> {
    let label = Label::parse_canonical(t["label"].as_str().unwrap()).unwrap();
    let checksum = s(&t["config_checksum"]);
    let owner_key = ActionLookupKeyRef::new(ReplayTargetKey {
        label: label.clone(),
        checksum: checksum.clone(),
    });
    let owner = ActionOwner::create_dummy(
        Some(label.clone()),
        Location::from_file("BUILD"),
        format!("{} rule", s(&t["rule_class"])),
        s(&t["config_mnemonic"]),
        checksum.clone(),
        None,
        false,
        Some(host_platform()),
        Vec::new(),
        IndexMap::new(),
    );
    let mut inputs = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    let mut runfiles_tree = None;
    let mut test_setup = None;
    let mut xml_generator = None;
    for i in t["inputs"].as_array().unwrap() {
        let p = i.as_str().unwrap();
        let kind = if p.ends_with(".runfiles") {
            "runfiles"
        } else {
            "file"
        };
        let a = roots.artifact(p, &owner_key, kind);
        if kind == "runfiles" {
            runfiles_tree = Some(a.clone());
        } else if p.ends_with("/test-setup.sh") {
            test_setup = Some(a.clone());
        } else if p.ends_with("/generate-xml.sh") {
            xml_generator = Some(a.clone());
        }
        inputs.add(a);
    }
    let runfiles_tree = runfiles_tree.expect("runfiles tree input");
    let outputs: Vec<BlazeArtifact> = t["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let p = o[0].as_str().unwrap();
            roots.artifact(
                p,
                &owner_key,
                if o[1].as_bool().unwrap() {
                    "tree"
                } else {
                    "file"
                },
            )
        })
        .collect();
    let find = |name: &str| {
        outputs
            .iter()
            .find(|a| a.get_exec_path().get_base_name() == name)
            .cloned()
            .unwrap_or_else(|| panic!("no output {name}"))
    };
    let test_log = find("test.log");
    let test_xml = find("test.xml");
    let cache_status = find("test.cache_status");
    let undeclared_outputs_dir = find("test.outputs");

    let size = TestSize::get_test_size(t["size"].as_str().unwrap()).unwrap();
    let timeout = TestTimeout::get_test_timeout(t["timeout"].as_str().unwrap()).unwrap();
    let tags: Vec<String> = t["tags"].as_array().unwrap().iter().map(s).collect();
    // TargetUtils.getExecutionInfo(rule): the tags that are execution requirements.
    if !tags.is_empty() {
        blaze_util::unported!("TargetUtils.getExecutionInfo (tags) in the replay harness");
    }
    let rule_execution_info = IndexMap::new();
    let execution_info = TestTargetProperties::compute_execution_info(
        &rule_execution_info,
        false,
        tags.iter().any(|t| t == "local"),
        tags.iter().any(|t| t == "exclusive"),
        false,
        None,
    );
    let rule_class = s(&t["rule_class"]);
    let properties = TestTargetProperties::new(
        size,
        timeout,
        tags.clone(),
        t["flaky"].as_bool().unwrap(),
        tags.iter().any(|t| t == "external"),
        rule_class.split('_').next().unwrap().to_string(),
        execution_info,
        Some(IndexMap::new()),
    );

    let runfiles_exec_path = runfiles_tree.get_exec_path().get_path_string().to_string();
    let executable_path = runfiles_exec_path.trim_end_matches(".runfiles").to_string();
    let executable = roots.artifact(&executable_path, &owner_key, "file");
    let input_manifest = roots.artifact(
        &format!("{executable_path}.runfiles_manifest"),
        &owner_key,
        "file",
    );
    let settings = TestTargetExecutionSettings::new(
        command_line::empty(),
        None,
        0,
        1,
        None,
        None,
        executable,
        true,
        Some(roots.exec_root.get_relative_str(&runfiles_exec_path)),
        Some(input_manifest),
        None,
        false,
    );

    let mut fixed_env = IndexMap::new();
    for kv in t["fixed_env"].as_array().unwrap() {
        fixed_env.insert(s(&kv[0]), s(&kv[1]));
    }
    let configuration = Arc::new(TestRunnerActionConfiguration {
        checksum,
        action_environment: Arc::new(ActionEnvironment::create(fixed_env, IndexSet::new())),
        test_action_environment: Arc::new(ActionEnvironment::Empty),
        code_coverage_enabled: false,
        runfiles_enabled: true,
        // --test_timeout default "-1".
        test_timeout: TestTimeoutConverter.convert_map("-1").unwrap().0,
        cache_test_results: TriState::AUTO,
        zip_undeclared_test_outputs: false,
        test_runner_fail_fast: false,
        check_sharding_support: true,
        allow_local_tests: true,
    });
    let _ = fs;
    TestRunnerAction::new(
        owner,
        inputs.build(),
        runfiles_tree,
        test_setup.expect("test-setup.sh input"),
        xml_generator.expect("generate-xml.sh input"),
        None,
        test_log,
        ActionInputRef::new(test_xml),
        cache_status,
        None,
        None,
        undeclared_outputs_dir,
        properties,
        IndexMap::new(),
        Arc::new(ActionEnvironment::Empty),
        settings,
        0,
        0,
        configuration,
        "_main".to_string(),
        Some(PathFragment::create("/bin/bash")),
        TestConfigurationTestOptionsCancelConcurrentTests::NEVER,
        false,
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        None,
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "help" {
        eprintln!(
            "usage: blaze-test-replay --spec <spec.json> --execution-log-json <out.json> --results <results.json>\n(driven by scripts/w7/replay-tests)"
        );
        return ExitCode::from(2);
    }
    blaze_exec::analysis::test::register();
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(arg(&args, "--spec")).unwrap()).unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let output_base = fs.get_path(spec["output_base"].as_str().unwrap());
    let exec_root = fs.get_path(spec["exec_root"].as_str().unwrap());
    let install_base = fs.get_path(spec["install_base"].as_str().unwrap());
    let workspace = fs.get_path(spec["workspace"].as_str().unwrap());
    // Bazel's client env in the oracle recording: HOME and PATH only.
    let mut client_env = IndexMap::new();
    for k in ["HOME", "PATH"] {
        if let Ok(v) = std::env::var(k) {
            client_env.insert(k.to_string(), v);
        }
    }
    let execution_options = Arc::new(Options::<ExecutionOptions>::get_defaults());
    let bin_tools = Arc::new(BinTools::for_production(install_base.clone()).unwrap());
    let env = ExecCommandEnvironment {
        file_system: fs.clone(),
        exec_root: exec_root.clone(),
        output_base: output_base.clone(),
        client_env: client_env.clone(),
        bin_tools,
        sandbox_options: Arc::new(Options::<SandboxOptions>::get_defaults()),
        execution_options: execution_options.clone(),
        local_execution_options: Arc::new(Options::<LocalExecutionOptions>::get_defaults()),
        remote_options: Arc::new(Options::get_defaults()),
        dynamic_execution_options: Arc::new(Options::get_defaults()),
        worker_options: Arc::new(Options::get_defaults()),
        resource_manager: ResourceManager::new(),
        reporter: Arc::new(StoredEventHandler::new()),
        package_path_entries: vec![],
        repo_contents_cache_path: None,
        test_arguments: Vec::new(),
        jobs: None,
    };
    blaze_exec::actions::resource_manager::configure_resource_manager(
        &env.resource_manager,
        &env.execution_options,
    );
    if !LinuxSandboxedSpawnRunner::is_supported(&env) {
        eprintln!("linux-sandbox is not supported here");
        return ExitCode::from(1);
    }
    let mut sandbox_module = SandboxModule::new();
    let registry = match build_spawn_strategy_registry(&env, &mut sandbox_module) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            eprintln!(
                "cannot build the spawn strategy registry: {}",
                e.get_message()
            );
            return ExitCode::from(1);
        }
    };
    let log_path = fs.get_path(&arg(&args, "--execution-log-json"));
    let log_context: Arc<dyn SpawnLogContext> = Arc::new(
        ExpandedSpawnLogContext::new(
            log_path.clone(),
            fs.get_path(&format!("{}.tmp", log_path.get_path_string())),
            Encoding::Json,
            /* sorted= */ true,
            exec_root.as_fragment().clone(),
            None,
            DigestHashFunction::Sha256,
        )
        .unwrap(),
    );
    // Java: the TestActionContext registered by the standalone module for --test_strategy=standalone
    // (the default), with tmpDirRoot = TestStrategy.getTmpRoot(workspace, execRoot, options).
    let tmp_dir_root = TestStrategy::get_tmp_root(&workspace, &exec_root, &execution_options);
    let test_strategy: Arc<dyn TestActionContext> = Arc::new(StandaloneTestStrategy::new(
        execution_options.clone(),
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
        tmp_dir_root,
    ));
    let test_action_context: ActionContextRef = Arc::new(test_strategy);

    let roots = Roots {
        exec_root: exec_root.clone(),
        output_base: output_base.clone(),
        workspace: workspace.clone(),
    };
    let metadata = ReplayMetadataProvider {
        exec_root: exec_root.clone(),
        cache: Mutex::new(HashMap::new()),
        runfiles: Mutex::new(HashMap::new()),
    };
    let key_context = ActionKeyContext::new();
    let mut results = Vec::new();
    for (n, t) in spec["tests"].as_array().unwrap().iter().enumerate() {
        let action = build_action(t, &roots, &fs);
        let key = action.get_key(&key_context, None).unwrap();
        let mut rec = serde_json::Map::new();
        rec.insert("label".into(), t["label"].clone());
        rec.insert("action_key".into(), key.clone().into());
        rec.insert(
            "action_key_matches".into(),
            (key == s(&t["action_key"])).into(),
        );
        rec.insert(
            "arguments".into(),
            json!(TestStrategy::expanded_args_from_action(&action).unwrap()),
        );

        // The runfiles tree's mapping, from the recorded runfiles inputs.
        let runfiles_exec_path = action.get_runfiles_tree().get_exec_path().clone();
        let mut mapping = Vec::new();
        for r in t["runfiles"].as_array().unwrap() {
            let rel = PathFragment::create(r["rel"].as_str().unwrap());
            let input = match r["kind"].as_str().unwrap() {
                "empty_marker" => None,
                _ => Some(ActionInputRef::new(RunfilesEntryArtifact {
                    exec_path: PathFragment::create(r["path"].as_str().unwrap()),
                })),
            };
            mapping.push((rel, input));
        }
        metadata.runfiles.lock().unwrap().insert(
            runfiles_exec_path.get_path_string().to_string(),
            // The recorded test has no metadata for the members of the tree.
            Arc::new(RunfilesArtifactValue::new(
                Arc::new(ReplayRunfilesTree {
                    exec_path: runfiles_exec_path.clone(),
                    mapping,
                }),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
        );

        // Java: SkyframeActionExecutor.prepareScheduleExecuteAndCompleteAction: action.prepare,
        // then createOutputDirectories.
        let executable = as_executable(&*action).expect("TestRunnerAction is registered");
        executable.prepare(&exec_root).unwrap();
        for o in action.get_outputs() {
            exec_root
                .get_relative(o.get_exec_path())
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
        }
        let outerr_dir = output_base.get_relative_str(&format!("replay-outerr/{n}"));
        outerr_dir.create_directory_and_parents().unwrap();
        let events = StoredEventHandler::new();
        let ctx = ReplayActionContext {
            exec_root: exec_root.clone(),
            metadata: &metadata,
            out_err: FileOutErr::new(
                outerr_dir.get_child("stdout"),
                outerr_dir.get_child("stderr"),
            ),
            client_env: client_env.clone(),
            events: &events,
            clock: JavaClock,
            registry: registry.clone(),
            log_context: log_context.clone(),
            test_action_context: test_action_context.clone(),
        };
        match executable.execute(&ctx) {
            Ok(r) => {
                rec.insert("spawn_results".into(), r.spawn_results().len().into());
                rec.insert(
                    "spawn_exit_codes".into(),
                    json!(
                        r.spawn_results()
                            .iter()
                            .map(|x| x.exit_code())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            Err(e) => {
                rec.insert("error".into(), e.get_message().into());
            }
        }
        let mut attempts = Vec::new();
        for p in events.get_posts() {
            let any: &dyn std::any::Any = &*p;
            if let Some(a) = any.downcast_ref::<TestAttempt>() {
                attempts.push(json!({
                    "attempt": a.get_attempt(),
                    "status": a.get_status().as_str_name(),
                    "exit_code": a.get_execution_info().exit_code,
                    "strategy": a.get_execution_info().strategy,
                    "files": a.get_files().iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
                }));
            } else if let Some(r) = any.downcast_ref::<TestResult>() {
                let d = r.get_data();
                rec.insert(
                    "result".into(),
                    json!({
                        "status": d.status().as_str_name(),
                        "test_passed": d.test_passed(),
                        "cachable": d.cachable(),
                        "exit_code": d.exit_code(),
                        "passed_log": d.passed_log.clone(),
                        "failed_logs": d.failed_logs.clone(),
                        "test_name": r.get_test_name(),
                        "label": r.get_label(),
                        "cache_status_matches": action
                            .read_cache_status()
                            .map(|c| &c == d)
                            .unwrap_or(false),
                    }),
                );
            }
        }
        rec.insert("attempts".into(), Value::Array(attempts));
        let mut ev = Vec::new();
        for e in events.get_events() {
            ev.push(format!("{:?}: {}", e.get_kind(), e.get_message()));
        }
        rec.insert("events".into(), json!(ev));
        rec.insert("stderr".into(), ctx.out_err.err_as_latin1().into());
        eprintln!(
            "{}: {}",
            t["label"].as_str().unwrap(),
            rec.get("result")
                .map(|r| r["status"].to_string())
                .or_else(|| rec.get("error").map(|e| e.to_string()))
                .unwrap_or_default()
        );
        results.push(Value::Object(rec));
    }
    log_context.close().unwrap();
    sandbox_module.after_command();
    std::fs::write(
        arg(&args, "--results"),
        serde_json::to_string_pretty(&Value::Array(results)).unwrap(),
    )
    .unwrap();
    ExitCode::SUCCESS
}
