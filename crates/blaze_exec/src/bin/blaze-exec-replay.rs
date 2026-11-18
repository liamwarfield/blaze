//! Test harness for scripts/w6/replay-spawns: runs recorded spawns through Blaze's
//! LinuxSandboxedSpawnRunner and logs them with ExpandedSpawnLogContext.
//!
//! Not a port of a Java class. Usage: `blaze-exec-replay --spec <spec.json>
//! --execution-log-json <out.json> --results <results.json>`; the spec is written by
//! scripts/w6/replay-spawns (see its help).

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::StoredEventHandler;
use blaze_events::{Clock, ExtendedEventHandler, JavaClock};
use blaze_exec::actions::ActionExecutionMetadata;
use blaze_exec::actions::resource_manager::ResourceManager;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, BasicActionInput, BasicVirtualActionInput,
    ExecException, FakeOwner, FileArtifactValue, FilesetOutputTree, InputMetadataProvider,
    PathMapper, ResourceSet, RunfilesArtifactValue, RunfilesTree, SimpleSpawn, Spawn,
    TreeArtifactValue,
};
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::executor_setup::build_spawn_strategy_registry;
use blaze_exec::exec::expanded_spawn_log_context::{Encoding, ExpandedSpawnLogContext};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_exec::exec::{BinTools, ExecCommandEnvironment, ProgressStatus};
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_exec::sandbox::linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
use blaze_exec::sandbox::sandbox_module::SandboxModule;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::IndexMap;
use serde_json::Value;

/// Metadata of the scratch execroot's files, computed on first use (Java: the action's
/// `ActionInputMap`, filled from Skyframe file values).
struct ReplayMetadataProvider {
    exec_root: Path,
    cache: Mutex<HashMap<String, Option<FileArtifactValue>>>,
    runfiles: Mutex<HashMap<String, Arc<RunfilesArtifactValue>>>,
}

/// A runfiles tree rebuilt from the recorded input paths under `<x>.runfiles/` (Java: the
/// action's `RunfilesTree`). Its mapping maps each runfiles-relative path to an input at the
/// recorded (display) path, or to nothing for an empty file (`VirtualActionInput.EMPTY_MARKER`).
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

/// The runfiles-tree artifact standing for a [`ReplayRunfilesTree`] (`runfiles_tree: true`), or
/// a plain derived file artifact in a runfiles mapping (`runfiles_tree: false`).
#[derive(Debug)]
struct RunfilesTreeArtifact {
    exec_path: PathFragment,
    runfiles_tree: bool,
}

impl ActionInput for RunfilesTreeArtifact {
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

impl Artifact for RunfilesTreeArtifact {
    fn is_source_artifact(&self) -> bool {
        false
    }
    fn is_tree_artifact(&self) -> bool {
        false
    }
    fn is_runfiles_tree(&self) -> bool {
        self.runfiles_tree
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

impl InputMetadataProvider for ReplayMetadataProvider {
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        if let Some(v) = input.as_virtual_action_input() {
            if v.is_empty_action_input() {
                return Ok(None);
            }
            let bytes = v.get_bytes()?;
            use sha2::Digest as _;
            return Ok(Some(FileArtifactValue::create_for_virtual_action_input(
                sha2::Sha256::digest(&bytes).to_vec(),
                bytes.len() as i64,
            )));
        }
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
}

impl ActionContextRegistry for ReplayActionContext<'_> {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        Some(self.registry.clone())
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        Some(self.log_context.clone())
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "help" {
        eprintln!(
            "usage: blaze-exec-replay --spec <spec.json> --execution-log-json <out.json> --results <results.json>\n(driven by scripts/w6/replay-spawns)"
        );
        return ExitCode::from(2);
    }
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(arg(&args, "--spec")).unwrap()).unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let output_base = fs.get_path(spec["output_base"].as_str().unwrap());
    let exec_root = fs.get_path(spec["exec_root"].as_str().unwrap());
    let install_base = fs.get_path(spec["install_base"].as_str().unwrap());
    // Bazel's client env in the oracle recording: HOME and PATH only.
    let mut client_env = IndexMap::new();
    for k in ["HOME", "PATH"] {
        if let Ok(v) = std::env::var(k) {
            client_env.insert(k.to_string(), v);
        }
    }
    let bin_tools = Arc::new(BinTools::for_production(install_base.clone()).unwrap());
    let env = ExecCommandEnvironment {
        file_system: fs.clone(),
        exec_root: exec_root.clone(),
        output_base: output_base.clone(),
        client_env: client_env.clone(),
        bin_tools,
        sandbox_options: Arc::new(Options::<SandboxOptions>::get_defaults()),
        execution_options: Arc::new(Options::<ExecutionOptions>::get_defaults()),
        local_execution_options: Arc::new(Options::<LocalExecutionOptions>::get_defaults()),
        remote_options: Arc::new(Options::get_defaults()),
        dynamic_execution_options: Arc::new(Options::get_defaults()),
        worker_options: Arc::new(Options::get_defaults()),
        resource_manager: ResourceManager::new(),
        reporter: Arc::new(StoredEventHandler::new()),
        package_path_entries: vec![],
        repo_contents_cache_path: None,
        test_arguments: Vec::new(),
    };
    // Java: ExecutionTool.configureResourceManager (--local_resources defaults).
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
    let log_context = ExpandedSpawnLogContext::new(
        log_path.clone(),
        fs.get_path(&format!("{}.tmp", log_path.get_path_string())),
        Encoding::Json,
        /* sorted= */ true,
        exec_root.as_fragment().clone(),
        None,
        DigestHashFunction::Sha256,
    )
    .unwrap();
    let log_context: Arc<dyn SpawnLogContext> = Arc::new(log_context);
    let events = StoredEventHandler::new();
    let stats = SpawnStats::new();
    let metadata = ReplayMetadataProvider {
        exec_root: exec_root.clone(),
        cache: Mutex::new(HashMap::new()),
        runfiles: Mutex::new(HashMap::new()),
    };
    let mut results = Vec::new();
    for (n, s) in spec["spawns"].as_array().unwrap().iter().enumerate() {
        let index = s["index"].as_i64().unwrap();
        let mut inputs = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        let mut tools = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        let mut runfiles_trees: IndexMap<String, Vec<(PathFragment, Option<ActionInputRef>)>> =
            IndexMap::new();
        for i in s["inputs"].as_array().unwrap() {
            let path = i["path"].as_str().unwrap();
            let kind = i["kind"].as_str().unwrap();
            if let Some(pos) = path.find(".runfiles/") {
                let root = &path[..pos + ".runfiles".len()];
                let rel = PathFragment::create(&path[pos + ".runfiles/".len()..]);
                let artifact = match kind {
                    "empty_marker" => None,
                    "file" => Some(ActionInputRef::new(RunfilesTreeArtifact {
                        exec_path: PathFragment::create(path),
                        runfiles_tree: false,
                    })),
                    other => panic!("unsupported runfiles input kind {other} for {path}"),
                };
                runfiles_trees
                    .entry(root.to_string())
                    .or_default()
                    .push((rel, artifact));
                continue;
            }
            let input = match kind {
                "virtual" => ActionInputRef::new(BasicVirtualActionInput::new(
                    PathFragment::create(path),
                    std::fs::read(i["content_file"].as_str().unwrap()).unwrap(),
                )),
                "empty_marker" => panic!("empty file outside a runfiles tree: {path}"),
                _ => ActionInputRef::new(BasicActionInput::from_path(path)),
            };
            if i["is_tool"].as_bool().unwrap_or(false) {
                tools.add(input.clone());
            }
            inputs.add(input);
        }
        for (root, mapping) in runfiles_trees {
            let exec_path = PathFragment::create(&root);
            metadata.runfiles.lock().unwrap().insert(
                root.clone(),
                // The recorded spawn has no metadata for the members of the tree.
                Arc::new(RunfilesArtifactValue::new(
                    Arc::new(ReplayRunfilesTree {
                        exec_path: exec_path.clone(),
                        mapping,
                    }),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
            );
            inputs.add(ActionInputRef::new(RunfilesTreeArtifact {
                exec_path,
                runfiles_tree: true,
            }));
        }
        let outputs: Vec<ActionInputRef> = s["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| ActionInputRef::new(BasicActionInput::from_path(o.as_str().unwrap())))
            .collect();
        // Bazel deletes an action's outputs before executing it.
        for o in &outputs {
            let p = exec_root.get_relative(o.get_exec_path());
            let _ = p.delete();
        }
        let label = s["target_label"]
            .as_str()
            .map(blaze_cmdline::Label::parse_canonical_unchecked);
        let mut owner = FakeOwner::new(s["mnemonic"].as_str().unwrap(), None, label);
        // The oracle's aquery reports executionPlatform @@platforms//host:host (no exec
        // properties) for every action of both projects.
        owner.platform = Some(blaze_exec::actions::PlatformInfo {
            label: blaze_cmdline::Label::parse_canonical_unchecked("@@platforms//host:host"),
            exec_properties: IndexMap::new(),
            remote_execution_properties: String::new(),
        });
        if let Some(first) = outputs.first() {
            owner.primary_output = first.clone();
        }
        let mut env_map = IndexMap::new();
        for kv in s["env"].as_array().unwrap() {
            env_map.insert(
                kv[0].as_str().unwrap().to_string(),
                kv[1].as_str().unwrap().to_string(),
            );
        }
        let mut exec_info = IndexMap::new();
        if let Some(obj) = s["execution_info"].as_object() {
            for (k, v) in obj {
                exec_info.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
        let spawn = SimpleSpawn::new(
            Arc::new(owner),
            s["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_string())
                .collect(),
            env_map,
            exec_info,
            inputs.build(),
            tools.build(),
            outputs,
            None,
            Arc::new(ResourceSet::create_with_ram_cpu(1.0, 1.0)),
            PathMapper::NOOP,
        );
        let outerr_dir = output_base.get_relative_str(&format!("replay-outerr/{n}"));
        outerr_dir.create_directory_and_parents().unwrap();
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
        };
        let mut rec = serde_json::Map::new();
        rec.insert("index".into(), index.into());
        let strategies: Vec<String> = registry
            .get_strategies(&spawn, None)
            .iter()
            .map(|s| s.to_string())
            .collect();
        rec.insert("strategies".into(), strategies.into());
        let outcome = SpawnStrategyResolver::new().exec(&spawn, &ctx);
        let result = match &outcome {
            Ok(results) => Some(results[0].clone()),
            Err(ExecException::Spawn(e)) => {
                rec.insert(
                    "error".into(),
                    e.to_action_execution_exception_message(&spawn.get_resource_owner().describe())
                        .into(),
                );
                Some(e.get_spawn_result().clone())
            }
            Err(e) => {
                rec.insert("error".into(), e.to_string().into());
                None
            }
        };
        match result {
            Some(result) => {
                stats.increment_action_count();
                stats.count_action_result(std::slice::from_ref(&result));
                rec.insert("exit_code".into(), result.exit_code().into());
                rec.insert("status".into(), result.status().name().into());
                rec.insert("runner".into(), result.get_runner_name().into());
            }
            None => {
                rec.insert("exit_code".into(), Value::Null);
                rec.insert("status".into(), "EXEC_EXCEPTION".into());
            }
        }
        rec.insert("stderr".into(), ctx.out_err.err_as_latin1().into());
        results.push(Value::Object(rec));
    }
    log_context.close().unwrap();
    sandbox_module.after_command();
    println!(
        "{}",
        SpawnStats::convert_summary_to_string(&stats.get_summary())
    );
    std::fs::write(
        arg(&args, "--results"),
        serde_json::to_string_pretty(&Value::Array(results)).unwrap(),
    )
    .unwrap();
    ExitCode::SUCCESS
}
