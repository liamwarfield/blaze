//! W6 build replay: executes the oracle's recorded action graph (aquery jsonproto) through
//! Blaze's execution phase (BuildDriver → TargetCompletion → ARTIFACT / ACTION_EXECUTION →
//! `ExecutableAction`s → spawn strategies) into a fresh output base, then prints Bazel's
//! summary lines. `scripts/w6b/replay-build` drives it and compares the output tree with the
//! oracle's `outputs` surface.
//!
//! Not a port of a Java class (harness). It stands in for analysis: actions are rebuilt from the
//! recording (blaze_actions classes where they can be rebuilt exactly, otherwise a SpawnAction
//! with the recorded argv/env/inputs/outputs), top-level targets and their default output group
//! come from the recorded BEP `targetCompleted` events.
//!
//! Usage: blaze-build-replay --records <dir> --project <p> --workspace <ws> --output-base <ob>
//!   --external-from <oracle output base> --install-base <ib> [--jobs N] [--keep-going]

#[path = "replay/recording.rs"]
mod recording;

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::command_line;
use blaze_actions::actions::param_file_info::ParamFileInfo;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
use blaze_actions::analysis::actions::spawn_action::{
    SpawnActionBuilder, SpawnActionConfiguration,
};
use blaze_actions::analysis::actions::starlark_action::starlark_action_builder;
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::runfiles::{Runfiles, RunfilesBuilder};
use blaze_actions::analysis::runfiles_support::RunfilesTreeImpl;
use blaze_actions::analysis::source_manifest_action::{ManifestType, SourceManifestAction};
use blaze_actions::rules::cpp::solib_symlink_action::get_dynamic_library_symlink_internal;
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact, SpecialArtifactType};
use blaze_buildtool::buildtool::execution_tool::{
    ExecutionEventCollector, ExecutionToolConfig, evaluate_build_driver_keys, prepare_for_execution,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::io::OutErr;
use blaze_events::ui::{UiEventHandler, UiEventHandlerOptions};
use blaze_events::{ExtendedEventHandler, Reporter};
use blaze_sky_exec::analysis_seam::{
    ActionLookupValue, AnalysisSeam, ConfiguredTargetView, TestParams,
};
use blaze_sky_exec::keys::{BuildDriverKey, TestType};
use blaze_skyframe::{ComputeError, Environment, EvaluatorBuilder};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};
use recording::{Recording, kv_list, str_list};
use serde_json::Value;

/// The configuration's default shell environment (`--incompatible_strict_action_env`).
struct ReplayConfiguration {
    action_env: Arc<ActionEnvironment>,
}

impl ReplayConfiguration {
    fn new() -> Self {
        let mut fixed = IndexMap::new();
        fixed.insert(
            "PATH".to_string(),
            "/bin:/usr/bin:/usr/local/bin".to_string(),
        );
        ReplayConfiguration {
            action_env: Arc::new(ActionEnvironment::create(fixed, IndexSet::new())),
        }
    }
}

impl SpawnActionConfiguration for ReplayConfiguration {
    fn get_action_environment(&self) -> Arc<ActionEnvironment> {
        Arc::clone(&self.action_env)
    }
    fn modified_execution_info(
        &self,
        execution_info: &IndexMap<String, String>,
        _mnemonic: &str,
    ) -> IndexMap<String, String> {
        execution_info.clone()
    }
    fn get_output_paths_mode(&self) -> OutputPathsMode {
        OutputPathsMode::OFF
    }
}

fn add_inputs(rec: &Recording, a: &Value, b: &mut SpawnActionBuilder) {
    for id in a
        .get("inputDepSetIds")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        b.add_transitive_inputs(&rec.nested_set(id.as_u64().unwrap(), Order::STABLE_ORDER));
    }
}

/// A SpawnAction running the recorded command (CppCompile, CppLink, CppArchive, ...). When the
/// oracle's spawn passed the arguments in a param file (`<exe> @<file>`), the arguments go
/// through an always-used param file.
fn generic_spawn_action(rec: &Recording, a: &Value, spawn_args: Option<&Vec<String>>) -> ActionRef {
    let args = str_list(a.get("arguments"));
    let config = ReplayConfiguration::new();
    let mut b = starlark_action_builder();
    if args.is_empty() {
        // Recorded actions without a command line (TranslateBuildInfo): not rebuilt; executing
        // one fails.
        b.set_executable_as_string("/bin/false");
        b.add_outputs(rec.outputs(a));
        b.set_mnemonic(a["mnemonic"].as_str().unwrap());
        return Arc::new(b.build(rec.action_owner(a), &config));
    }
    b.set_executable_as_string(args[0].clone());
    let uses_param_file = spawn_args
        .is_some_and(|s| s.len() == 2 && s[1].starts_with('@') && s[1].ends_with(".params"));
    if uses_param_file {
        let info: ParamFileInfo = ParamFileInfo::builder(ParameterFileType::GCC_QUOTED)
            .set_flag_format_string("@%s")
            .set_use_always(true)
            .build();
        b.add_command_line_with_param_file(command_line::of(args[1..].to_vec()), Some(info));
    } else if args.len() > 1 {
        b.add_command_line(command_line::of(args[1..].to_vec()));
    }
    add_inputs(rec, a, &mut b);
    b.add_outputs(rec.outputs(a));
    b.set_mnemonic(a["mnemonic"].as_str().unwrap());
    b.set_environment(kv_list(a.get("environmentVariables")));
    b.set_execution_info(kv_list(a.get("executionInfo")));
    Arc::new(b.build(rec.action_owner(a), &config))
}

fn single(v: &[Artifact]) -> Option<Artifact> {
    if v.len() == 1 {
        Some(v[0].clone())
    } else {
        None
    }
}

fn template_is_executable(output: &Artifact) -> bool {
    let name = output.get_filename();
    name.ends_with("_stage2_bootstrap.py") || !name.contains('.')
}

fn runfiles_executable_path(mnemonic: &str, output: &str) -> Option<String> {
    match mnemonic {
        "RunfilesTree" => output.strip_suffix(".runfiles"),
        "SourceSymlinkManifest" => output.strip_suffix(".runfiles_manifest"),
        "SymlinkTree" => output.strip_suffix(".runfiles/MANIFEST"),
        _ => None,
    }
    .map(|s| s.to_string())
}

/// The target's runfiles rebuilt from its RunfilesTree action inputs (see
/// crates/blaze_actions/tests/oracle_action_keys.rs `recorded_runfiles`).
fn recorded_runfiles(rec: &Recording, a: &Value, exe: &str) -> Option<Runfiles> {
    let tree_action = rec.actions().iter().find(|b| {
        b["mnemonic"] == "RunfilesTree"
            && b["targetId"] == a["targetId"]
            && b["configurationId"] == a["configurationId"]
            && rec.primary_output(b).get_exec_path_string() == format!("{exe}.runfiles")
    })?;
    let ids = tree_action["inputDepSetIds"].as_array()?;
    let (transitive, direct) = rec.depset_parts(ids[0].as_u64()?);
    let leaves = &direct[..direct.len().saturating_sub(2)];
    let mut b = RunfilesBuilder::new("_main");
    for t in &transitive {
        b.add_transitive_artifacts(&rec.nested_set(*t, Order::COMPILE_ORDER));
    }
    for leaf in leaves {
        b.add_artifact(rec.artifact_by_id(*leaf));
    }
    Some(b.build())
}

fn runfiles_action(rec: &Recording, a: &Value) -> Option<ActionRef> {
    let mnemonic = a["mnemonic"].as_str().unwrap();
    let output = rec.primary_output(a);
    let exe = runfiles_executable_path(mnemonic, output.get_exec_path_string())?;
    let owner = rec.action_owner(a);
    let runfiles = recorded_runfiles(rec, a, &exe)?;
    let repo_mapping = rec.artifact(&format!("{exe}.repo_mapping"), false);
    match mnemonic {
        "RunfilesTree" => {
            let tree = RunfilesTreeImpl::new_for_testing(output.get_exec_path().clone(), runfiles);
            let inputs = rec.nested_set(a["inputDepSetIds"][0].as_u64()?, Order::STABLE_ORDER);
            Some(Arc::new(RunfilesTreeAction::new(
                owner,
                Arc::new(tree),
                inputs,
                vec![output],
            )))
        }
        "SourceSymlinkManifest" => Some(Arc::new(SourceManifestAction::new(
            Arc::new(ManifestType::SOURCE_SYMLINKS),
            owner,
            output,
            runfiles,
            Some(repo_mapping),
            false,
        ))),
        "SymlinkTree" => Some(Arc::new(SymlinkTreeAction::new(
            owner,
            rec.artifact(&format!("{exe}.runfiles_manifest"), false),
            Some(runfiles),
            output,
            Some(repo_mapping),
            (*ReplayConfiguration::new().action_env).clone(),
            RunfileSymlinksMode::CREATE,
            "_main".to_string(),
        ))),
        _ => None,
    }
}

/// Rebuilds the recorded action `a`.
fn reconstruct(rec: &Recording, a: &Value, spawn_args: Option<&Vec<String>>) -> ActionRef {
    let mnemonic = a["mnemonic"].as_str().unwrap();
    let rebuilt: Option<ActionRef> = match mnemonic {
        "Symlink" | "ExecutableSymlink" => {
            let input = single(&rec.flattened_inputs(a));
            input.map(|input| -> ActionRef {
                let owner = rec.action_owner(a);
                let output = rec.primary_output(a);
                if mnemonic == "Symlink" {
                    Arc::new(SymlinkAction::to_artifact(owner, input, output, None))
                } else {
                    Arc::new(SymlinkAction::to_executable(owner, input, output, None))
                }
            })
        }
        "SolibSymlink" => single(&rec.flattened_inputs(a)).map(|library| -> ActionRef {
            Arc::new(get_dynamic_library_symlink_internal(
                rec.action_owner(a),
                library,
                rec.primary_output(a),
            ))
        }),
        // PORT (harness): RepoMappingManifestAction needs the repository mapping analysis
        // computes; the recorded manifest contents are written by a FileWriteAction instead.
        "FileWrite" | "RepoMappingManifest" if a.get("inputDepSetIds").is_none() => {
            Some(Arc::new(FileWriteAction::create(
                rec.action_owner(a),
                rec.primary_output(a),
                a.get("fileContents")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
                a.get("isExecutable")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
                Compression::ALLOW,
            )))
        }
        "TemplateExpand" => single(&rec.flattened_inputs(a)).map(|template| -> ActionRef {
            let substitutions = kv_list(a.get("substitutions"))
                .into_iter()
                .map(|(k, v)| Substitution::of(k, v))
                .collect();
            Arc::new(TemplateExpansionAction::for_template_artifact(
                rec.action_owner(a),
                template,
                rec.primary_output(a),
                substitutions,
                template_is_executable(&rec.primary_output(a)),
            ))
        }),
        "RunfilesTree" | "SourceSymlinkManifest" | "SymlinkTree" => runfiles_action(rec, a),
        _ => None,
    };
    rebuilt.unwrap_or_else(|| generic_spawn_action(rec, a, spawn_args))
}

/// One analyzed target of the replay: its actions and (for top-level targets) its outputs.
#[derive(Debug)]
struct ReplayTarget {
    key: ActionLookupKeyRef,
    label: Label,
    checksum: String,
    rule: String,
    actions: Vec<ActionRef>,
    output_groups: IndexMap<String, NestedSet<Artifact>>,
    executable: Option<Artifact>,
}

impl ActionLookupValue for ReplayTarget {
    fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }
}

impl ConfiguredTargetView for ReplayTarget {
    fn get_label(&self) -> Label {
        self.label.clone()
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some(self.checksum.clone())
    }
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        self.key.clone()
    }
    fn get_rule_class_or_kind(&self) -> String {
        self.rule.clone()
    }
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        self.output_groups.clone()
    }
    fn get_executable(&self) -> Option<Artifact> {
        self.executable.clone()
    }
    fn get_test_params(&self) -> Option<TestParams> {
        None
    }
}

struct ReplaySeam {
    targets: HashMap<ActionLookupKeyRef, Arc<ReplayTarget>>,
}

impl AnalysisSeam for ReplaySeam {
    fn get_action_lookup_value(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
        let t = self
            .targets
            .get(key)
            .unwrap_or_else(|| panic!("no replay target for {}", key.0));
        Ok(Some(t.clone() as Arc<dyn ActionLookupValue>))
    }
    fn get_configured_target(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError> {
        let t = self
            .targets
            .get(key)
            .unwrap_or_else(|| panic!("no replay target for {}", key.0));
        Ok(Some(t.clone() as Arc<dyn ConfiguredTargetView>))
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .map(|i| args[i + 1].clone())
}

fn read_json_stream(text: &str) -> Vec<Value> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map(|v| v.unwrap())
        .collect()
}

/// Plants the execroot with `SymlinkForest` for the main-repository packages of the recording,
/// then links `<output_base>/external/<repo>` to the oracle's fetched repository (read-only) and
/// plants `<execroot>/external/<repo>` → `<output_base>/external/<repo>`
/// (`SymlinkForest.plantSingleSymlinkForExternalRepo`).
// PORT (harness): repository fetching is the repo-rules lead's; the fetched trees are borrowed.
fn plant(
    packages: &[blaze_cmdline::PackageIdentifier],
    workspace: &Path,
    output_base: &Path,
    exec_root: &Path,
    external_from: &Path,
) {
    exec_root.create_directory_and_parents().unwrap();
    let mut package_roots = IndexMap::new();
    for p in packages {
        package_roots.insert(p.clone(), blaze_vfs::Root::from_path(workspace.clone()));
    }
    blaze_buildtool::buildtool::symlink_forest::SymlinkForest::new(
        package_roots,
        exec_root.clone(),
        "bazel",
    )
    .plant_symlink_forest()
    .unwrap();
    let ext = output_base.get_child("external");
    ext.create_directory_and_parents().unwrap();
    let exec_ext = exec_root.get_child("external");
    exec_ext.create_directory_and_parents().unwrap();
    for repo in external_from
        .get_child("external")
        .get_directory_entries()
        .unwrap()
    {
        if !repo.is_directory() {
            continue;
        }
        let name = repo.get_base_name().to_string();
        ext.get_child(&name).create_symbolic_link(&repo).unwrap();
        exec_ext
            .get_child(&name)
            .create_symbolic_link(&ext.get_child(&name))
            .unwrap();
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "help" {
        eprintln!(
            "usage: blaze-build-replay --records <records dir> --project <p> --workspace <ws> \
             --output-base <ob> --external-from <oracle output base> --install-base <ib> \
             [--jobs N] [--keep-going]\n(driven by scripts/w6b/replay-build)"
        );
        return ExitCode::from(2);
    }
    let records = std::path::PathBuf::from(arg(&args, "--records").unwrap());
    let project = arg(&args, "--project").unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let workspace = fs.get_path(&arg(&args, "--workspace").unwrap());
    let output_base = fs.get_path(&arg(&args, "--output-base").unwrap());
    let exec_root = output_base.get_relative_str("execroot/_main");
    let external_from = fs.get_path(&arg(&args, "--external-from").unwrap());
    let install_base = fs.get_path(&arg(&args, "--install-base").unwrap());
    let jobs: usize = arg(&args, "--jobs")
        .map(|j| j.parse().unwrap())
        .unwrap_or(16);
    let keep_going = args.iter().any(|a| a == "--keep-going");

    // ---- Analysis stand-in: the recorded action graph. ----
    let mut rec = Recording::load_from(&records.join(&project), &project).expect("recording");
    rec.output_base = output_base.get_path_string().to_string();
    rec.execroot = exec_root.get_path_string().to_string();
    rec.workspace = workspace.get_path_string().to_string();

    let execlog_text =
        std::fs::read_to_string(records.join(&project).join("raw/execlog.json")).unwrap();
    let mut spawn_args: HashMap<String, Vec<String>> = HashMap::new();
    for s in read_json_stream(&execlog_text) {
        if let Some(first) = s["listedOutputs"].as_array().and_then(|o| o.first()) {
            spawn_args.insert(
                first.as_str().unwrap().to_string(),
                str_list(s.get("commandArgs")),
            );
        }
    }

    // Owners: each derived output is owned by the key of its generating target.
    let actions: Vec<Value> = rec.actions().clone();
    let mut group_of: Vec<(ActionLookupKeyRef, i32)> = Vec::new();
    let mut group_sizes: IndexMap<ActionLookupKeyRef, i32> = IndexMap::new();
    let mut owners = HashMap::new();
    for a in &actions {
        let label = rec.target_label(a["targetId"].as_u64().unwrap());
        let checksum = rec.configuration(a["configurationId"].as_u64().unwrap())["checksum"]
            .as_str()
            .unwrap()
            .to_string();
        let key = rec.target_key(&label, &checksum);
        let index = {
            let n = group_sizes.entry(key.clone()).or_insert(0);
            *n += 1;
            *n - 1
        };
        group_of.push((key.clone(), index));
        let special = (a["mnemonic"] == "RunfilesTree").then_some(SpecialArtifactType::RUNFILES);
        for id in a["outputIds"].as_array().unwrap() {
            let path = rec.artifact_path(id.as_u64().unwrap()).to_string();
            owners.insert(path, (key.clone(), special));
        }
    }
    rec.owners = owners;

    let mut target_actions: IndexMap<ActionLookupKeyRef, Vec<ActionRef>> = IndexMap::new();
    for (a, (key, index)) in actions.iter().zip(&group_of) {
        let primary = rec.primary_output(a);
        let action = reconstruct(&rec, a, spawn_args.get(primary.get_exec_path_string()));
        for output in action.get_outputs() {
            output.set_generating_action_key(ActionLookupData::create(key.clone(), *index));
        }
        target_actions.entry(key.clone()).or_default().push(action);
    }

    // Top-level targets and their default output group from the recorded BEP.
    let bep: Value = serde_json::from_str(
        &std::fs::read_to_string(records.join(&project).join("norm/bep.json")).unwrap(),
    )
    .unwrap();
    let mut targets: HashMap<ActionLookupKeyRef, Arc<ReplayTarget>> = HashMap::new();
    let mut driver_keys = Vec::new();
    let checksum_of_default =
        rec.configuration(actions[0]["configurationId"].as_u64().unwrap())["checksum"]
            .as_str()
            .unwrap()
            .to_string();
    for (id, value) in bep.as_object().unwrap() {
        let id: Value = serde_json::from_str(id).unwrap();
        let Some(tc) = id.get("targetCompleted") else {
            continue;
        };
        let label = tc["label"].as_str().unwrap().to_string();
        let checksum = tc["configuration"]["id"]
            .as_str()
            .unwrap_or(&checksum_of_default)
            .to_string();
        let key = rec.target_key(&label, &checksum);
        let mut default = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        let mut executable = None;
        for group in value["completed"]["outputGroup"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if group["name"] != "default" {
                continue;
            }
            for f in group["files"].as_array().into_iter().flatten() {
                let uri = f["uri"].as_str().unwrap();
                let exec_path = uri
                    .strip_prefix("file://<EXECROOT>/")
                    .or_else(|| uri.strip_prefix("file://<WORKSPACE>/"))
                    .unwrap_or_else(|| panic!("unexpected uri {uri}"));
                let a = rec.artifact(exec_path, false);
                default.add(a);
            }
        }
        let mut output_groups = IndexMap::new();
        let default = default.build();
        // Executables: the runfiles tree is built through _hidden_top_level_INTERNAL_.
        let mut hidden = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
        for a in default.to_list().iter() {
            let tree = format!("{}.runfiles", a.get_exec_path_string());
            if rec.owners.contains_key(&tree) {
                hidden.add(rec.artifact(&tree, false));
                executable = Some(a.clone());
            }
        }
        output_groups.insert("default".to_string(), default);
        output_groups.insert("_hidden_top_level_INTERNAL_".to_string(), hidden.build());
        targets.insert(
            key.clone(),
            Arc::new(ReplayTarget {
                key: key.clone(),
                label: Label::parse_canonical(&label).unwrap(),
                checksum,
                rule: String::new(),
                actions: target_actions.get(&key).cloned().unwrap_or_default(),
                output_groups,
                executable,
            }),
        );
        driver_keys.push(BuildDriverKey {
            action_lookup_key: key,
            output_groups: vec![
                "default".to_string(),
                "_hidden_top_level_INTERNAL_".to_string(),
            ],
            explicitly_requested: true,
            skip_incompatible_explicit_targets: false,
            extra_action_top_level_only: false,
            test_type: TestType::NOT_TEST,
        });
    }
    for (key, actions) in target_actions {
        targets.entry(key.clone()).or_insert_with(|| {
            Arc::new(ReplayTarget {
                label: key.get_label().unwrap(),
                checksum: key.get_configuration_checksum().unwrap(),
                key,
                rule: String::new(),
                actions,
                output_groups: IndexMap::new(),
                executable: None,
            })
        });
    }
    driver_keys.sort_by_key(|k| k.action_lookup_key.get_label().unwrap().to_string());

    // The workspace status action (`WorkspaceStatusValue.BUILD_INFO_KEY`), whose outputs every
    // build requests (SkyframeBuildView.analyzeAndExecuteTargets: workspaceStatusArtifacts).
    let build_info_key =
        ActionLookupKeyRef::new(blaze_actions::analysis::workspace_status_action::BUILD_INFO_KEY);
    let bazel_out_root = blaze_actions::ArtifactRoot::as_derived_root(
        &exec_root,
        blaze_actions::RootType::OUTPUT,
        &["bazel-out"],
    );
    let status_artifact = |name: &str| {
        Artifact::create_derived_artifact(
            bazel_out_root.clone(),
            PathFragment::create(&format!("bazel-out/{name}")),
            build_info_key.clone(),
        )
    };
    let stable_status = status_artifact("stable-status.txt");
    let volatile_status = status_artifact("volatile-status.txt");
    let workspace_status: ActionRef = Arc::new(
        blaze_actions::bazel::bazel_workspace_status_module::BazelWorkspaceStatusAction::new(
            stable_status.clone(),
            volatile_status.clone(),
            std::env::var("USER").unwrap_or_default(),
            "localhost",
        ),
    );
    for output in workspace_status.get_outputs() {
        output.set_generating_action_key(ActionLookupData::create(build_info_key.clone(), 0));
    }
    targets.insert(
        build_info_key.clone(),
        Arc::new(ReplayTarget {
            key: build_info_key.clone(),
            label: Label::parse_canonical("//:build_info").unwrap(),
            checksum: String::new(),
            rule: String::new(),
            actions: vec![workspace_status],
            output_groups: IndexMap::new(),
            executable: None,
        }),
    );
    let workspace_status_artifacts = vec![stable_status, volatile_status];

    let mut packages: Vec<blaze_cmdline::PackageIdentifier> = targets
        .values()
        .filter(|t| t.label.get_repository().is_main())
        .map(|t| t.label.get_package_identifier().clone())
        .collect();
    packages.sort_by_key(|p| p.to_string());
    packages.dedup();
    plant(
        &packages,
        &workspace,
        &output_base,
        &exec_root,
        &external_from,
    );

    // ---- Execution. ----
    blaze_exec::analysis::actions::register();
    let collector = Arc::new(ExecutionEventCollector::new());
    let reporter = Reporter::new(Some(collector.clone()));
    // --color=no: the ANSI-stripping out/err (BlazeCommandDispatcher).
    let out_err = blaze_events::ui::ansi_strip_err(&blaze_events::ui::ansi_strip_out(
        OutErr::system_out_err(),
    ));
    let ui = Arc::new(UiEventHandler::new(
        out_err,
        &UiEventHandlerOptions {
            show_progress: false,
            ..UiEventHandlerOptions::default()
        },
        false,
        Arc::new(blaze_events::JavaClock),
        Some(workspace.get_path_string().to_string()),
    ));
    reporter.add_handler(ui.clone());
    let reporter: Arc<dyn ExtendedEventHandler> = reporter;
    let mut client_env = IndexMap::new();
    for k in ["HOME", "PATH"] {
        if let Ok(v) = std::env::var(k) {
            client_env.insert(k.to_string(), v);
        }
    }
    let config = ExecutionToolConfig {
        file_system: fs.clone(),
        output_base: output_base.clone(),
        exec_root: exec_root.clone(),
        install_base,
        client_env,
        keep_going,
        verbose_failures: false,
        jobs,
    };
    let ws_for_status = workspace.clone();
    let prepared = match prepare_for_execution(&config, reporter.clone(), None, &|contexts, env| {
        for (type_id, ctx) in blaze_exec::analysis::actions::default_action_contexts("_main") {
            contexts.register_raw(type_id, ctx);
        }
        use blaze_exec::analysis::actions::bazel_workspace_status_module as ws;
        let status: Arc<dyn ws::WorkspaceStatusActionContext> =
            Arc::new(ws::BazelWorkspaceStatusActionContext::new(
                ws::WorkspaceStatusOptions {
                    embed_label: String::new(),
                    workspace_status_command: PathFragment::empty_fragment(),
                },
                env.client_env.clone(),
                ws_for_status.clone(),
            ));
        let (type_id, ctx) = ws::workspace_status_context_entry(status);
        contexts.register_raw(type_id, ctx);
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: {e}");
            return ExitCode::from(37);
        }
    };
    let mut prepared = prepared;
    let seam_targets = targets.clone();
    let seam: Arc<dyn AnalysisSeam> = Arc::new(ReplaySeam { targets });
    let evaluator = blaze_sky_exec::register_execution_functions(
        EvaluatorBuilder::new(),
        prepared.executor.clone(),
        seam,
        None,
    )
    .build();

    let top_level: Vec<Arc<dyn ConfiguredTargetView>> = driver_keys
        .iter()
        .map(|k| seam_targets[&k.action_lookup_key].clone() as Arc<dyn ConfiguredTargetView>)
        .collect();
    blaze_buildtool::buildtool::build_result_printer::report_targets(&*reporter, &top_level);
    let result = evaluate_build_driver_keys(
        &evaluator,
        &driver_keys,
        &workspace_status_artifacts,
        &config,
        reporter.clone(),
    )
    .expect("interrupted");
    prepared.sandbox_module.after_command();

    // Convenience symlinks (PORT (harness): OutputDirectoryLinksUtils replaces this).
    for (name, target) in [
        (
            "bazel-bin",
            exec_root.get_relative_str("bazel-out/k8-fastbuild/bin"),
        ),
        ("bazel-out", exec_root.get_child("bazel-out")),
        (
            "bazel-testlogs",
            exec_root.get_relative_str("bazel-out/k8-fastbuild/testlogs"),
        ),
    ] {
        let link = workspace.get_child(name);
        let _ = link.delete();
        link.create_symbolic_link(&target).unwrap();
    }

    let success = !result.evaluation.has_error();
    for (key, error) in result.evaluation.error_map() {
        eprintln!("(replay) failed: {key:?}: {error:?}");
    }
    let elapsed = (result.stop - result.start).as_secs_f64();
    blaze_buildtool::buildtool::build_tool::report_build_complete(
        &*reporter, &collector, &result, elapsed, 0,
    );
    ui.after_command();
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
