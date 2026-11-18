//! Reconstructs the oracle's recorded actions (brotli, re2) from the aquery jsonproto and
//! requires the action keys computed by blaze_actions to equal the oracle's `actionKey`.
//!
//! Prints a per-mnemonic table (matched / mismatched / not attempted). Mnemonics this crate can
//! reconstruct must match 100%; the test fails on any mismatch.

mod oracle_common;

use std::sync::Arc;

use blaze_actions::SpecialArtifactType;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::command_line;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
use blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration;
use blaze_actions::analysis::actions::starlark_action::starlark_action_builder;
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::runfiles::{EmptyFilesSupplier, Runfiles, RunfilesBuilder};
use blaze_actions::analysis::runfiles_support::RunfilesTreeImpl;
use blaze_actions::analysis::source_manifest_action::{ManifestType, SourceManifestAction};
use blaze_actions::rules::cpp::solib_symlink_action::get_dynamic_library_symlink_internal;
use blaze_actions::{ActionKeyContext, Artifact};
use blaze_collect::Order;
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use oracle_common::{Recording, Tally, kv_list, str_list};
use serde_json::Value;

/// The configuration's default shell environment: `--incompatible_strict_action_env`'s fixed
/// PATH, nothing inherited.
struct OracleConfiguration {
    action_env: Arc<ActionEnvironment>,
}

impl OracleConfiguration {
    fn new() -> Self {
        let mut fixed = IndexMap::new();
        fixed.insert(
            "PATH".to_string(),
            "/bin:/usr/bin:/usr/local/bin".to_string(),
        );
        OracleConfiguration {
            action_env: Arc::new(ActionEnvironment::create(fixed, IndexSet::new())),
        }
    }
}

impl SpawnActionConfiguration for OracleConfiguration {
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

/// `ctx.actions.run(executable = <string>, arguments = [<strings>], use_default_shell_env = ...)`
/// and `ctx.actions.run_shell(command = ..., arguments = [<strings>])`.
fn starlark_string_args_action(rec: &Recording, a: &Value, shell: bool) -> Option<Arc<dyn Action>> {
    let args = str_list(a.get("arguments"));
    let env = kv_list(a.get("environmentVariables"));
    let config = OracleConfiguration::new();
    let mut b = starlark_action_builder();
    let rest: Vec<String> = if shell {
        // ShellCommand(sh, command, pad): `<sh> -c <command> ""`.
        if args.len() < 4 || args[1] != "-c" || !args[3].is_empty() {
            return None;
        }
        b.set_shell_command(&PathFragment::create(&args[0]), args[2].clone(), true);
        args[4..].to_vec()
    } else {
        b.set_executable_as_string(PathFragment::create(&args[0]).get_path_string());
        args[1..].to_vec()
    };
    if !rest.is_empty() {
        b.add_command_line(command_line::of(rest));
    }
    for id in a
        .get("inputDepSetIds")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        b.add_transitive_inputs(&rec.nested_set(id.as_u64().unwrap(), Order::STABLE_ORDER));
    }
    b.add_outputs(rec.outputs(a));
    b.set_mnemonic(a["mnemonic"].as_str().unwrap());
    // The default shell environment comes first in the fixed env; the action's own variables
    // follow (SpawnAction.createActionEnvironment).
    let default_path = config.action_env.get_fixed_env();
    if env.get_index_of("PATH") == Some(0) && env.get("PATH") == default_path.get("PATH") {
        let mut own = env.clone();
        own.shift_remove("PATH");
        b.use_default_shell_environment(own);
    } else {
        b.set_environment(env);
    }
    b.set_execution_info(kv_list(a.get("executionInfo")));
    Some(Arc::new(b.build(rec.action_owner(a), &config)))
}

/// aquery does not record `is_executable` of a TemplateExpansionAction (it is not a
/// FileContentsProvider). rules_python (py_executable.bzl:673 and :769) passes
/// `is_executable = True` for the stage-2 bootstrap and for the bootstrap executable itself,
/// which is the target's own name (no extension).
fn template_is_executable(output: &Artifact) -> bool {
    let name = output.get_filename();
    name.ends_with("_stage2_bootstrap.py") || !name.contains('.')
}

/// The executable path `<exe>` of a runfiles action output (`<exe>.runfiles`,
/// `<exe>.runfiles_manifest`, `<exe>.runfiles/MANIFEST`).
fn runfiles_executable_path(mnemonic: &str, output: &str) -> Option<String> {
    match mnemonic {
        "RunfilesTree" => output.strip_suffix(".runfiles"),
        "SourceSymlinkManifest" => output.strip_suffix(".runfiles_manifest"),
        "SymlinkTree" => output.strip_suffix(".runfiles/MANIFEST"),
        _ => None,
    }
    .map(|s| s.to_string())
}

/// Rebuilds the target's `Runfiles` from its RunfilesTree action, whose inputs are
/// `stableOrder().addTransitive(runfiles.getAllArtifacts()).add(MANIFEST).add(repo_mapping)`
/// (RunfilesSupport.createRunfilesTreeArtifactAction). Only runfiles without symlinks (where
/// getAllArtifacts() is the COMPILE_ORDER artifacts set itself) are reconstructed.
fn recorded_runfiles(rec: &Recording, a: &Value, exe: &str) -> Option<Runfiles> {
    let tree_action = rec.actions().iter().find(|b| {
        b["mnemonic"] == "RunfilesTree"
            && b["targetId"] == a["targetId"]
            && b["configurationId"] == a["configurationId"]
            && rec.primary_output(b).get_exec_path_string() == format!("{exe}.runfiles")
    })?;
    let ids = tree_action["inputDepSetIds"].as_array()?;
    let (transitive, direct) = rec.depset_parts(ids[0].as_u64()?);
    let tail: Vec<String> = direct
        .iter()
        .rev()
        .take(2)
        .map(|id| rec.artifact_path(*id).to_string())
        .collect();
    if tail
        != [
            format!("{exe}.repo_mapping"),
            format!("{exe}.runfiles/MANIFEST"),
        ]
    {
        return None;
    }
    let leaves = &direct[..direct.len() - 2];
    let mut b = RunfilesBuilder::new("_main");
    match (transitive.as_slice(), leaves) {
        ([t], []) => {
            b.add_transitive_artifacts(&rec.nested_set(*t, Order::COMPILE_ORDER));
        }
        ([], [leaf]) => {
            b.add_artifact(rec.artifact_by_id(*leaf));
        }
        _ => return None,
    }
    // Python targets: runfiles from `py_builtins.merge_runfiles_with_generated_inits_empty_files_supplier`
    // carry BazelPyBuiltins.GET_INIT_PY_FILES (their manifests list empty `__init__.py` files).
    let manifest = rec.actions().iter().find(|m| {
        m["mnemonic"] == "SourceSymlinkManifest"
            && rec.primary_output(m).get_exec_path_string() == format!("{exe}.runfiles_manifest")
    })?;
    let contents = manifest["fileContents"].as_str().unwrap_or("");
    // Without symlinks or root symlinks, every manifest entry with a target is an artifact at its
    // runfiles path under the workspace prefix (external ones as `<repo>/...`). Anything else means
    // the runfiles had symlinks, which the recording cannot reconstruct.
    let built = b.build();
    let covered: std::collections::HashSet<String> = built
        .get_artifacts()
        .to_list()
        .iter()
        .map(|a| {
            PathFragment::create("_main")
                .get_relative(&a.get_runfiles_path())
                .get_path_string()
                .to_string()
        })
        .collect();
    for line in contents.lines() {
        if let Some((path, target)) = line.split_once(' ')
            && !target.is_empty()
            && path != "_repo_mapping"
            && !covered.contains(path)
        {
            return None;
        }
    }
    if contents.lines().any(|l| l.ends_with("__init__.py ")) {
        b.set_empty_files_supplier(Arc::new(GetInitPyFiles));
    }
    Some(b.build())
}

/// `PythonUtils.GetInitPyFiles` with `BazelPyBuiltins.GUID` (rules/python, not ported here);
/// only its fingerprint matters for the keys.
struct GetInitPyFiles;

impl EmptyFilesSupplier for GetInitPyFiles {
    fn get_extra_paths(&self, _manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        unreachable!()
    }
    fn fingerprint(&self, fp: &mut Fingerprint) {
        // UUID 0211a192-1b1e-40e6-80e9-7352360b12b1: addUUID = addLong(least).addLong(most).
        fp.add_uuid(
            0x80e9_7352_360b_12b1_u64 as i64,
            0x0211_a192_1b1e_40e6_u64 as i64,
        );
    }
    fn get_class_name(&self) -> String {
        "com.google.devtools.build.lib.rules.python.PythonUtils$GetInitPyFiles".to_string()
    }
}

fn runfiles_action(rec: &Recording, a: &Value) -> Option<Arc<dyn Action>> {
    let mnemonic = a["mnemonic"].as_str().unwrap();
    let output = rec.primary_output(a);
    let exe = runfiles_executable_path(mnemonic, output.get_exec_path_string())?;
    let owner = rec.action_owner(a);
    if mnemonic == "RunfilesTree" {
        // RunfilesTreeAction.computeKey adds nothing; the tree only needs its runfiles.
        let tree = RunfilesTreeImpl::new_for_testing(
            output.get_exec_path().clone(),
            RunfilesBuilder::new("_main").build(),
        );
        let runfiles_tree_artifact = Artifact::create_special_artifact(
            output.get_root().clone(),
            output.get_exec_path().clone(),
            rec.owner_key("//:oracle_owner"),
            SpecialArtifactType::RUNFILES,
        );
        let inputs = rec.nested_set(a["inputDepSetIds"][0].as_u64()?, Order::STABLE_ORDER);
        return Some(Arc::new(RunfilesTreeAction::new(
            owner,
            Arc::new(tree),
            inputs,
            vec![runfiles_tree_artifact],
        )));
    }
    let runfiles = recorded_runfiles(rec, a, &exe)?;
    let repo_mapping = rec.artifact(&format!("{exe}.repo_mapping"), false);
    let input_manifest = rec.artifact(&format!("{exe}.runfiles_manifest"), false);
    match mnemonic {
        "SourceSymlinkManifest" => Some(Arc::new(SourceManifestAction::new(
            Arc::new(ManifestType::SOURCE_SYMLINKS),
            owner,
            output,
            runfiles,
            Some(repo_mapping),
            // --experimental_remotable_source_manifests default
            false,
        ))),
        "SymlinkTree" => Some(Arc::new(SymlinkTreeAction::new(
            owner,
            input_manifest,
            Some(runfiles),
            output,
            Some(repo_mapping),
            (*OracleConfiguration::new().action_env).clone(),
            // BuildConfigurationValue.getRunfileSymlinksMode with --enable_runfiles=auto (Linux)
            RunfileSymlinksMode::CREATE,
            "_main".to_string(),
        ))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Starlark Args-based actions

thread_local! {
    /// Artifacts the test Starlark `file(path)` global returns.
    static FILES: std::cell::RefCell<IndexMap<String, Artifact>> =
        std::cell::RefCell::new(IndexMap::new());
}

/// Test globals standing in for `ctx.actions.args()` and for File values of the recording.
#[starlark::starlark_module]
fn oracle_natives(builder: &mut starlark::environment::GlobalsBuilder) {
    fn make_args<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        let semantics = blaze_starlark::thread::ThreadContext::get(eval)?
            .semantics
            .clone();
        Ok(blaze_actions::analysis::starlark::args::new_args(
            eval.heap(),
            semantics,
        ))
    }

    fn file<'v>(
        path: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        let artifact = FILES.with(|f| f.borrow().get(path).cloned()).unwrap();
        Ok(eval
            .heap()
            .alloc(blaze_actions::analysis::starlark::starlark_file::StarlarkFile::new(artifact)))
    }
}

/// Evaluates `code` and builds the named frozen Args values into command lines.
fn eval_args(
    code: &str,
    names: &[&str],
) -> Vec<blaze_actions::analysis::starlark::args::BuiltArgs> {
    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics)
        .with(blaze_starlark::depset::register_depset)
        .with(oracle_natives)
        .build();
    let compiled = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Bzl,
        "oracle/args.bzl",
        code.to_owned(),
    )
    .unwrap();
    let ctx = blaze_starlark::thread::ThreadContext::new(
        blaze_starlark::thread::Phase::BzlLoad,
        semantics,
        Arc::new(blaze_starlark::thread::CollectingEvents::default()),
    );
    let module = blaze_starlark::eval::exec_module(
        compiled,
        &globals,
        &blaze_starlark::loader::LoadMap::new(),
        &ctx,
        &blaze_starlark::eval::EvalOptions::default(),
    )
    .unwrap();
    names
        .iter()
        .map(|n| {
            blaze_actions::analysis::starlark::args::build_args(
                &module.get(n).unwrap(),
                &mut || (*blaze_cmdline::repository_mapping::EMPTY).clone(),
            )
            .unwrap()
        })
        .collect()
}

fn starlark_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// rules_python's precompile.bzl (`_precompile`): `ctx.actions.run(executable = <interpreter>,
/// arguments = [precompiler_startup_args, precompile_request_args], env = ..., mnemonic =
/// "PyCompile")`, with the startup args `add_all(interpreter_args)` (empty) +
/// `add(<precompiler executable>)` and the request args using an always-on multiline param file.
fn py_compile_action(rec: &Recording, a: &Value) -> Option<Arc<dyn Action>> {
    let args = str_list(a.get("arguments"));
    if args.len() != 14 {
        return None;
    }
    let exe = &args[0];
    let req = &args[2..];
    let keys = [
        "--invalidation_mode",
        "--src",
        "--src_name",
        "--pyc",
        "--optimize",
        "--python_version",
    ];
    for (i, k) in keys.iter().enumerate() {
        if req[2 * i] != *k {
            return None;
        }
    }
    FILES.with(|f| {
        let mut f = f.borrow_mut();
        f.clear();
        for p in [&args[1], &req[3], &req[7]] {
            f.insert(p.clone(), rec.artifact(p, false));
        }
    });
    let code = format!(
        "startup = make_args()\n\
         startup.add_all([])\n\
         startup.add(file({precompiler}))\n\
         request = make_args()\n\
         request.use_param_file(\"@%s\", use_always = True)\n\
         request.set_param_file_format(\"multiline\")\n\
         request.add(\"--invalidation_mode\", {mode})\n\
         request.add(\"--src\", file({src}))\n\
         request.add(\"--src_name\", {src_name})\n\
         request.add(\"--pyc\", file({pyc}))\n\
         request.add(\"--optimize\", {optimize})\n\
         request.add(\"--python_version\", {version})\n",
        precompiler = starlark_str(&args[1]),
        mode = starlark_str(&req[1]),
        src = starlark_str(&req[3]),
        src_name = starlark_str(&req[5]),
        pyc = starlark_str(&req[7]),
        optimize = starlark_str(&req[9]),
        version = starlark_str(&req[11]),
    );
    let built = eval_args(&code, &["startup", "request"]);
    let config = OracleConfiguration::new();
    let mut b = starlark_action_builder();
    b.set_executable_as_string(exe.clone());
    for ba in built {
        b.add_command_line_with_param_file(ba.command_line, ba.param_file_info);
    }
    b.add_outputs(rec.outputs(a));
    b.set_mnemonic("PyCompile");
    b.set_environment(kv_list(a.get("environmentVariables")));
    b.set_execution_info(kv_list(a.get("executionInfo")));
    Some(Arc::new(b.build(rec.action_owner(a), &config)))
}

fn single(v: &[Artifact]) -> Option<Artifact> {
    if v.len() == 1 {
        Some(v[0].clone())
    } else {
        None
    }
}

/// Builds the action for a recorded entry, or `None` if this mnemonic is not reconstructed.
fn reconstruct(rec: &Recording, a: &Value) -> Option<Arc<dyn Action>> {
    match a["mnemonic"].as_str().unwrap() {
        "CcStrip" => starlark_string_args_action(rec, a, false),
        "CopyFile" => starlark_string_args_action(rec, a, true),
        "Symlink" | "ExecutableSymlink" => {
            let inputs = rec.flattened_inputs(a);
            let input = single(&inputs)?;
            let owner = rec.action_owner(a);
            let output = rec.primary_output(a);
            Some(if a["mnemonic"] == "Symlink" {
                Arc::new(SymlinkAction::to_artifact(owner, input, output, None))
            } else {
                Arc::new(SymlinkAction::to_executable(owner, input, output, None))
            })
        }
        "SolibSymlink" => {
            let inputs = rec.flattened_inputs(a);
            let library = single(&inputs)?;
            Some(Arc::new(get_dynamic_library_symlink_internal(
                rec.action_owner(a),
                library,
                rec.primary_output(a),
            )))
        }
        "FileWrite" => {
            if a.get("inputDepSetIds").is_some() {
                return None;
            }
            // ctx.actions.write(output, content, is_executable)
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
        "TemplateExpand" => {
            // ctx.actions.expand_template(template = <file>, substitutions = {...})
            let inputs = rec.flattened_inputs(a);
            let template = single(&inputs)?;
            let substitutions = kv_list(a.get("substitutions"))
                .into_iter()
                .map(|(k, v)| Substitution::of(k, v))
                .collect();
            Some(Arc::new(TemplateExpansionAction::for_template_artifact(
                rec.action_owner(a),
                template,
                rec.primary_output(a),
                substitutions,
                template_is_executable(&rec.primary_output(a)),
            )))
        }
        "RunfilesTree" | "SourceSymlinkManifest" | "SymlinkTree" => runfiles_action(rec, a),
        "PyCompile" => py_compile_action(rec, a),
        _ => None,
    }
}

fn check(project: &str) -> Option<Tally> {
    let rec = Recording::load(project)?;
    let mut tally = Tally::default();
    for a in rec.actions() {
        let mnemonic = a["mnemonic"].as_str().unwrap();
        let expected = a["actionKey"].as_str().unwrap();
        match reconstruct(&rec, a) {
            None => tally.record(mnemonic, None, String::new),
            Some(action) => {
                let ctx = ActionKeyContext::new();
                let key = action.get_key(&ctx, None).unwrap();
                // File-writing actions: the contents must also match aquery's `fileContents`.
                if let Some(p) = action.as_file_contents_provider()
                    && let Some(want) = a.get("fileContents").and_then(|c| c.as_str())
                {
                    let got = p.get_file_contents().unwrap();
                    tally.record(&format!("{mnemonic} (contents)"), Some(got == want), || {
                        format!(
                            "{project} {mnemonic} contents differ for {}",
                            rec.primary_output(a).get_exec_path_string()
                        )
                    });
                }
                tally.record(mnemonic, Some(key == expected), || {
                    format!(
                        "{project} {mnemonic} {}: got {key}, oracle {expected}",
                        rec.primary_output(a).get_exec_path_string()
                    )
                });
            }
        }
    }
    Some(tally)
}

#[test]
fn oracle_action_keys_match() {
    let mut failures = Vec::new();
    for project in ["brotli", "re2"] {
        let Some(tally) = check(project) else {
            eprintln!("no oracle recording for {project}; skipped");
            continue;
        };
        eprintln!("{}", tally.report(project));
        failures.extend(tally.mismatches);
    }
    let _: Option<Artifact> = None;
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures[..failures.len().min(30)].join("\n")
    );
}
