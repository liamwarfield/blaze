//! Test harness for scripts/remote/replay: runs recorded spawns through blaze_remote
//! (`RemoteSpawnCache` in front of Blaze's local sandbox strategy, or the `remote` strategy)
//! against a disk cache, a remote cache or a remote executor.
//!
//! Not a port of a Java class. The spawn reconstruction follows
//! `crates/blaze_exec/src/bin/blaze-exec-replay.rs`, except that the members of runfiles trees
//! are plain inputs (the Merkle tree of the flat file set is the one of the tree). See
//! `blaze-remote-replay help`; the spec is written by scripts/remote/replay.

use std::collections::HashMap;
use std::io;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::StoredEventHandler;
use blaze_events::{Clock, Event, EventHandler, ExtendedEventHandler, JavaClock, Postable};
use blaze_exec::actions::ActionExecutionMetadata;
use blaze_exec::actions::resource_manager::ResourceManager;
use blaze_exec::actions::spawn_strategy::SpawnStrategy;
use blaze_exec::actions::{
    ActionInput, ActionInputRef, Artifact, BasicVirtualActionInput, ExecException, FakeOwner,
    FileArtifactValue, FilesetOutputTree, InputMetadataProvider, PathMapper, ResourceSet,
    RunfilesArtifactValue, SimpleSpawn, Spawn, TreeArtifactValue,
};
use blaze_exec::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, SpawnCache,
};
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
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_remote::hooks::{RemoteHooks, RemoteOutputInjector};
use blaze_remote::remote_module::{RemoteModule, RemoteModuleEnvironment};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::IndexMap;
use serde_json::Value;

const USAGE: &str = "\
usage: blaze-remote-replay --spec <spec.json> --results <results.json>
                           --execution-log-json <out.json> --mode cache|remote
                           [--build-request-id ID] [--command-id ID]
                           [--no-inline-dotd] [--toplevel-outputs <list.json>]
                           [-- <Bazel remote options>]
       blaze-remote-replay help

Runs every spawn of the spec (written by scripts/remote/replay from an oracle exec log) through
blaze_remote, set up by RemoteModule::before_command from the options after `--` (RemoteOptions
and AuthAndTLSOptions, e.g. --disk_cache=DIR --remote_cache=grpc://localhost:PORT
--remote_executor=... --remote_grpc_log=FILE --remote_instance_name=...):

  --mode cache    Blaze's local strategy (linux-sandbox) with RemoteSpawnCache as the SpawnCache:
                  lookup, on a miss execute locally and upload (what Bazel does with --disk_cache
                  or --remote_cache)
  --mode remote   the `remote` strategy (RemoteSpawnRunner), local fallback through the local
                  strategies (what Bazel does with --remote_executor)

CppCompile spawns get the `internal-inline-outputs` execution requirement for their .d output,
as Bazel's CppCompileAction sets it (--no-inline-dotd disables this); metadata injected for
outputs that are not written to disk is recorded and reported.

--toplevel-outputs names a JSON list of exec paths that are outputs of top-level targets
(Java: RemoteOutputChecker.afterAnalysis collects them from the analysis result): under the
default --remote_download_outputs=toplevel these are downloaded, the others stay remote.

Writes per spawn to --results: index, action digest, runner, cache hit, exit code, status,
error, sha256 of every output on disk, the injected remote outputs, stderr. Prints the events
RemoteModule and the spawns reported (WARNING:/ERROR:/INFO: lines) and the
`N processes: ...` summary. Exit code 0 unless the harness itself failed.
";

/// A derived file artifact at a recorded exec path.
#[derive(Debug)]
struct ReplayArtifact {
    exec_path: PathFragment,
}

impl ActionInput for ReplayArtifact {
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

impl Artifact for ReplayArtifact {
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
        if self.exec_path.segment_count() > 2 {
            self.exec_path.sub_fragment_from(2)
        } else {
            self.exec_path.clone()
        }
    }
    fn get_root_exec_path(&self) -> PathFragment {
        if self.exec_path.segment_count() > 2 {
            self.exec_path.sub_fragment(0, 2)
        } else {
            PathFragment::empty_fragment()
        }
    }
}

/// Metadata of the scratch execroot's files, computed on first use (Java: the action's
/// `ActionInputMap`, filled from Skyframe file values).
struct ReplayMetadataProvider {
    exec_root: Path,
    cache: Mutex<HashMap<String, Option<FileArtifactValue>>>,
    /// The remote metadata injected for outputs that were not downloaded (Java: the
    /// `RemoteActionFileSystem` the output metadata store stats).
    injector: Arc<RecordingInjector>,
}

impl InputMetadataProvider for ReplayMetadataProvider {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        if let Some(v) = input.as_virtual_action_input() {
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
        if !path.exists() {
            // Not on disk: a remote file if its metadata was injected. Not cached, as the file
            // may be produced later.
            let absolute = path.get_path_string().to_string();
            let injected = self.injector.injected.lock().unwrap();
            return Ok(injected
                .iter()
                .rev()
                .find(|(p, _, _, _)| *p == absolute)
                .map(|(_, digest, size, in_memory)| {
                    FileArtifactValue::create_for_remote_file_with_materialization_data(
                        hex::decode(digest).unwrap(),
                        *size,
                        /* location_index= */ 1,
                        None,
                        *in_memory,
                    )
                }));
        }
        let v = Some(FileArtifactValue::create_for_testing(&path)?);
        cache.insert(key, v.clone());
        Ok(v)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// Prints events as Bazel's UI prefixes them and keeps them.
struct PrintingHandler {
    lines: Mutex<Vec<String>>,
}

impl EventHandler for PrintingHandler {
    fn handle(&self, event: &Event) {
        let line = format!("{}: {}", event.get_kind(), event.get_message());
        println!("{line}");
        self.lines.lock().unwrap().push(line);
    }
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for PrintingHandler {
    fn post(&self, _obj: Arc<dyn Postable>) {}
}

/// Records `RemoteActionFileSystem.injectRemoteFile` calls.
#[derive(Default)]
struct RecordingInjector {
    injected: Mutex<Vec<(String, String, i64, bool)>>,
}

impl RemoteOutputInjector for RecordingInjector {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        _expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> io::Result<()> {
        self.injected.lock().unwrap().push((
            path.get_path_string().to_string(),
            hex::encode(digest),
            size,
            in_memory_output,
        ));
        Ok(())
    }
}

/// The harness's `ActionExecutionContext` (Java: built by `SkyframeActionExecutor` per action).
struct ReplayActionContext<'a> {
    exec_root: Path,
    metadata: &'a ReplayMetadataProvider,
    out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    events: &'a dyn ExtendedEventHandler,
    clock: JavaClock,
    registry: Arc<SpawnStrategyRegistry>,
    log_context: Arc<dyn SpawnLogContext>,
    spawn_cache: Option<Arc<dyn SpawnCache>>,
}

impl ActionContextRegistry for ReplayActionContext<'_> {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        Some(self.registry.clone())
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        Some(self.log_context.clone())
    }
    fn get_spawn_cache(&self) -> Option<Arc<dyn SpawnCache>> {
        self.spawn_cache.clone()
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

fn opt_arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .map(|i| args[i + 1].clone())
}

fn sha256_of(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    path.get_digest().ok().map(hex::encode)
}

fn main() -> ExitCode {
    let all_args: Vec<String> = std::env::args().collect();
    if all_args.len() < 2 || all_args[1] == "help" || all_args[1] == "--help" {
        print!("{USAGE}");
        return ExitCode::from(if all_args.len() < 2 { 2 } else { 0 });
    }
    let (args, bazel_options): (Vec<String>, Vec<String>) =
        match all_args.iter().position(|a| a == "--") {
            Some(i) => (all_args[..i].to_vec(), all_args[i + 1..].to_vec()),
            None => (all_args.clone(), Vec::new()),
        };
    let mode = arg(&args, "--mode");
    let inline_dotd = !args.iter().any(|a| a == "--no-inline-dotd");
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(arg(&args, "--spec")).unwrap()).unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let output_base = fs.get_path(spec["output_base"].as_str().unwrap());
    let exec_root = fs.get_path(spec["exec_root"].as_str().unwrap());
    let install_base = fs.get_path(spec["install_base"].as_str().unwrap());

    // The remote options: every option after `--` is tried on both option classes.
    let mut remote_args: Vec<&str> = Vec::new();
    let mut auth_args: Vec<&str> = Vec::new();
    for o in &bazel_options {
        let one = [o.as_str()];
        if Options::<RemoteOptions>::parse(&one).is_ok() {
            remote_args.push(o);
        } else if Options::<AuthAndTLSOptions>::parse(&one).is_ok() {
            auth_args.push(o);
        } else {
            eprintln!("not a RemoteOptions or AuthAndTLSOptions option: {o}");
            return ExitCode::from(2);
        }
    }
    let remote_options = Options::<RemoteOptions>::parse(&remote_args)
        .unwrap()
        .into_options();
    let auth_and_tls_options = Options::<AuthAndTLSOptions>::parse(&auth_args)
        .unwrap()
        .into_options();

    // Bazel's client env in the oracle recording: HOME and PATH only.
    let mut client_env = IndexMap::new();
    for k in ["HOME", "PATH"] {
        if let Ok(v) = std::env::var(k) {
            client_env.insert(k.to_string(), v);
        }
    }
    let bin_tools = Arc::new(BinTools::for_production(install_base.clone()).unwrap());
    let execution_options = Arc::new(Options::<ExecutionOptions>::get_defaults());
    let env = ExecCommandEnvironment {
        file_system: fs.clone(),
        exec_root: exec_root.clone(),
        output_base: output_base.clone(),
        client_env: client_env.clone(),
        bin_tools,
        sandbox_options: Arc::new(Options::<SandboxOptions>::get_defaults()),
        execution_options: execution_options.clone(),
        local_execution_options: Arc::new(Options::<LocalExecutionOptions>::get_defaults()),
        // The local registry is built without remote options (blaze_exec's setup has no hook
        // for the remote module yet); the remote pieces are attached below.
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

    let events = Arc::new(PrintingHandler {
        lines: Mutex::new(Vec::new()),
    });
    let injector = Arc::new(RecordingInjector::default());
    let hooks = RemoteHooks {
        remote_output_injector: Some(injector.clone()),
    };
    let build_request_id = opt_arg(&args, "--build-request-id")
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000001".to_string());
    let command_id = opt_arg(&args, "--command-id")
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000002".to_string());
    let mut module = match RemoteModule::before_command(RemoteModuleEnvironment {
        command_name: "build".to_string(),
        remote_options,
        auth_and_tls_options,
        execution_options: execution_options.clone(),
        rewind_lost_inputs: Some(false),
        experimental_sibling_repository_layout: false,
        reporter: events.clone(),
        file_system: fs.clone(),
        output_base: output_base.clone(),
        exec_root: exec_root.clone(),
        working_directory: exec_root.clone(),
        workspace: None,
        workspace_name: "_main".to_string(),
        action_temps_directory: output_base.get_relative_str("action_temps"),
        build_request_id,
        command_id,
        product_name: "bazel".to_string(),
        // Blaze reports Bazel's version (docs/decisions.md, version impersonation).
        tool_version: "9.3.2".to_string(),
        client_env: client_env.clone(),
        clock: Arc::new(JavaClock),
        scrubber: None,
        hooks,
    }) {
        Ok(m) => m,
        Err(e) => {
            println!("ERROR: {}", e.get_message());
            return ExitCode::from(1);
        }
    };
    if let Some(list) = opt_arg(&args, "--toplevel-outputs") {
        let paths: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(list).unwrap()).unwrap();
        if let Some(checker) = module.get_remote_output_checker() {
            for path in paths {
                checker.add_output_to_download(&ReplayArtifact {
                    exec_path: PathFragment::create(&path),
                });
            }
        }
    }
    let provider = module
        .get_action_context_provider()
        .expect("RemoteModule.setup creates a provider");
    let (spawn_cache, remote_strategy): (Option<Arc<dyn SpawnCache>>, _) = match mode.as_str() {
        "cache" => (Some(provider.get_spawn_cache()), None),
        "remote" => {
            let (strategy, runner) = provider.get_remote_spawn_strategy();
            runner.set_remote_local_fallback_registry(registry.clone());
            (None, Some(strategy))
        }
        other => {
            eprintln!("unknown --mode {other}");
            return ExitCode::from(2);
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
    let stats = SpawnStats::new();
    let metadata = ReplayMetadataProvider {
        exec_root: exec_root.clone(),
        cache: Mutex::new(HashMap::new()),
        injector: injector.clone(),
    };
    let mut results = Vec::new();
    for (n, s) in spec["spawns"].as_array().unwrap().iter().enumerate() {
        let index = s["index"].as_i64().unwrap();
        let mut inputs = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        let mut tools = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        for i in s["inputs"].as_array().unwrap() {
            let path = i["path"].as_str().unwrap();
            let kind = i["kind"].as_str().unwrap();
            let input = match kind {
                "virtual" => ActionInputRef::new(BasicVirtualActionInput::new(
                    PathFragment::create(path),
                    std::fs::read(i["content_file"].as_str().unwrap()).unwrap(),
                )),
                // An empty runfile (`null` in the runfiles mapping): an empty file.
                "empty_marker" => ActionInputRef::new(BasicVirtualActionInput::new(
                    PathFragment::create(path),
                    Vec::new(),
                )),
                "file" => ActionInputRef::new(ReplayArtifact {
                    exec_path: PathFragment::create(path),
                }),
                other => panic!("unsupported input kind {other} for {path}"),
            };
            if i["is_tool"].as_bool().unwrap_or(false) {
                tools.add(input.clone());
            }
            inputs.add(input);
        }
        let outputs: Vec<ActionInputRef> = s["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                ActionInputRef::new(ReplayArtifact {
                    exec_path: PathFragment::create(o.as_str().unwrap()),
                })
            })
            .collect();
        // Bazel deletes an action's outputs before executing it.
        for o in &outputs {
            let p = exec_root.get_relative(o.get_exec_path());
            let _ = p.delete();
            metadata
                .cache
                .lock()
                .unwrap()
                .remove(&o.get_exec_path_string());
        }
        let label = s["target_label"]
            .as_str()
            .map(blaze_cmdline::Label::parse_canonical_unchecked);
        let mnemonic = s["mnemonic"].as_str().unwrap();
        let mut owner = FakeOwner::new(mnemonic, None, label);
        // Java: TestRunnerAction, JavaCompileAction and JavaHeaderCompileAction return true from
        // mayModifySpawnOutputsAfterExecution.
        owner.may_modify_spawn_outputs_after_execution =
            matches!(mnemonic, "TestRunner" | "Javac" | "JavacTurbine");
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
        // The configuration checksum of the action owner (from the oracle; see
        // scripts/remote/replay --owner-metadata-from).
        if let Some(checksum) = s["configuration_checksum"].as_str() {
            owner.owner.configuration_checksum = Some(checksum.to_string());
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
        if inline_dotd
            && mnemonic == "CppCompile"
            && let Some(dotd) = outputs
                .iter()
                .find(|o| o.get_exec_path().get_path_string().ends_with(".d"))
        {
            // Java: CppCompileAction.createSpawn (ExecutionRequirements.REMOTE_EXECUTION_INLINE_OUTPUTS).
            exec_info.insert(
                "internal-inline-outputs".to_string(),
                dotd.get_exec_path_string(),
            );
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
            outputs.clone(),
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
            events: events.as_ref(),
            clock: JavaClock,
            registry: registry.clone(),
            log_context: log_context.clone(),
            spawn_cache: spawn_cache.clone(),
        };
        let mut rec = serde_json::Map::new();
        rec.insert("index".into(), index.into());
        let injected_before = injector.injected.lock().unwrap().len();
        let outcome = match &remote_strategy {
            Some(strategy) => SpawnStrategy::exec(strategy, &spawn, &ctx),
            None => SpawnStrategyResolver::new().exec(&spawn, &ctx),
        };
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
                rec.insert("cache_hit".into(), result.is_cache_hit().into());
                rec.insert(
                    "digest".into(),
                    match result.get_digest() {
                        Some(d) => format!("{}/{}", d.hash, d.size_bytes).into(),
                        None => Value::Null,
                    },
                );
            }
            None => {
                rec.insert("exit_code".into(), Value::Null);
                rec.insert("status".into(), "EXEC_EXCEPTION".into());
            }
        }
        let mut on_disk = serde_json::Map::new();
        for o in &outputs {
            let p = exec_root.get_relative(o.get_exec_path());
            on_disk.insert(
                o.get_exec_path_string(),
                sha256_of(&p).map_or(Value::Null, Value::from),
            );
        }
        rec.insert("outputs".into(), Value::Object(on_disk));
        let exec_root_prefix = format!("{}/", exec_root.get_path_string());
        let injected: Vec<Value> = injector.injected.lock().unwrap()[injected_before..]
            .iter()
            .map(|(path, digest, size, in_memory)| {
                serde_json::json!({
                    "path": path.strip_prefix(&exec_root_prefix).unwrap_or(path),
                    "hash": digest,
                    "size": size,
                    "in_memory": in_memory,
                })
            })
            .collect();
        rec.insert("injected".into(), injected.into());
        rec.insert("stderr".into(), ctx.out_err.err_as_latin1().into());
        results.push(Value::Object(rec));
    }
    log_context.close().unwrap();
    sandbox_module.after_command();
    // Waits for the background uploads, closes the gRPC log.
    if let Err(e) = module.after_command() {
        println!("ERROR: {}", e.get_message());
    }
    println!(
        "INFO: {}",
        SpawnStats::convert_summary_to_string(&stats.get_summary())
    );
    std::fs::write(
        arg(&args, "--results"),
        serde_json::to_string_pretty(&Value::Array(results)).unwrap(),
    )
    .unwrap();
    ExitCode::SUCCESS
}
