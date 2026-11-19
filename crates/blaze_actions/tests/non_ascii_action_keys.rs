//! Action keys of actions whose strings are not ASCII, against keys recorded from the oracle
//! (review 2026-11-18-actions S1-1).
//!
//! `data/non_ascii_probe/workspace` is a Bazel workspace with one target per placement of a
//! non-ASCII string that reaches an action key: every `Args` parameter, `map_each` results, file
//! paths, environment, execution info, exec properties, shell commands, file contents (plain,
//! compressed, from `Args`), template substitutions and symlink targets. A few ASCII targets
//! record the keys that older tests of this crate hard-code (`file_write_action_test.rs`,
//! `template_expansion_action_test.rs`, `symlink_action_test.rs`,
//! `executable_symlink_action_test.rs`), so that those can be regenerated too.
//! `data/non_ascii_probe/aquery.jsonproto` is the oracle's `aquery --output=jsonproto` of it,
//! byte for byte (`scripts/actions-probe/run` records it).
//!
//! The test rebuilds every recorded action from the same Starlark (`cases.bzl`) and from the
//! recorded strings, and requires the oracle's `actionKey`. Sources and recorded strings are
//! decoded as Latin-1, as Bazel decodes them, so every string is an internal string
//! (`blaze_util::string_encoding`); all 78 actions must match, the four `"\377"` probes included.
//! A check with the strings decoded as UTF-8 instead shows that the keys depend on it.

mod oracle_common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::SpecialArtifactType;
use blaze_actions::actions::action::Action;
use blaze_actions::actions::action_environment::ActionEnvironment;
use blaze_actions::actions::command_line;
use blaze_actions::actions::path_mapper::OutputPathsMode;
use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
use blaze_actions::analysis::actions::parameter_file_write_action::ParameterFileWriteAction;
use blaze_actions::analysis::actions::spawn_action::SpawnActionConfiguration;
use blaze_actions::analysis::actions::starlark_action::starlark_action_builder;
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::starlark::args::BuiltArgs;
use blaze_actions::analysis::starlark::unresolved_symlink_action::{
    SymlinkTargetType, UnresolvedSymlinkAction,
};
use blaze_actions::{ActionKeyContext, Artifact};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::string_encoding::{internal_to_unicode, latin1_decode};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use oracle_common::{Recording, kv_list, str_list};
use serde_json::Value;

const AQUERY: &[u8] = include_bytes!("data/non_ascii_probe/aquery.jsonproto");
const CASES_BZL: &[u8] = include_bytes!("data/non_ascii_probe/workspace/cases.bzl");

/// The `ctx.actions.write(content = <Args>)` probes (`WRITE_ARGS_CASES` of cases.bzl).
const WRITE_ARGS: &[&str] = &["write_args", "write_args_multiline", "write_args_raw_byte"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    /// Strings are internal strings (sources read as Latin-1): Bazel's strings.
    Internal,
    /// Strings are Unicode (sources read as UTF-8): not Bazel's strings; only to show that the
    /// keys depend on the encoding.
    Unicode,
}

/// What a Starlark source decoder gives for a file: Bazel's (`ParserInput.fromLatin1`) or a
/// UTF-8 one.
fn decode_source(side: Side, bytes: &[u8]) -> String {
    match side {
        Side::Unicode => String::from_utf8(bytes.to_vec()).expect("cases.bzl is UTF-8"),
        Side::Internal => latin1_decode(bytes).into_owned(),
    }
}

/// The recorded jsonproto holds Bazel's internal strings as raw bytes, so reading it as Latin-1
/// gives exactly Bazel's strings; [`Side::Unicode`] converts them to Unicode.
fn recorded_json(side: Side) -> Value {
    fn to_unicode(v: &mut Value) {
        match v {
            Value::String(s) => *s = internal_to_unicode(s).into_owned(),
            Value::Array(a) => a.iter_mut().for_each(to_unicode),
            Value::Object(o) => o.values_mut().for_each(to_unicode),
            _ => {}
        }
    }
    let mut json: Value = serde_json::from_str(&latin1_decode(AQUERY)).unwrap();
    if side == Side::Unicode {
        to_unicode(&mut json);
    }
    json
}

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

thread_local! {
    /// Artifacts the test Starlark `file(path)` global returns.
    static FILES: std::cell::RefCell<HashMap<String, Artifact>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Test globals standing in for `ctx.actions.args()`, `ctx.actions.template_dict()` and for the
/// File values of the recording.
#[starlark::starlark_module]
fn probe_natives(builder: &mut starlark::environment::GlobalsBuilder) {
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

    fn make_template_dict<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        Ok(blaze_actions::analysis::starlark::template_dict::new_template_dict(eval.heap()))
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

fn starlark_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// The probe target's name (`//:<name>`).
fn target_name(rec: &Recording, a: &Value) -> String {
    rec.target_label(a["targetId"].as_u64().unwrap())
        .rsplit(':')
        .next()
        .unwrap()
        .to_string()
}

/// Evaluates cases.bzl, decoded as the source decoder of this side does, with one `Args` per
/// `Args` probe built the way defs.bzl builds it, and the template dict.
fn eval_cases(side: Side, rec: &Recording) -> (HashMap<String, BuiltArgs>, Vec<Substitution>) {
    let mut code = decode_source(side, CASES_BZL);
    let mut names = Vec::new();
    // `args` probes: the target also has the FileWrite of its `dir_é/<name>_é.txt` input.
    let dir_marker = decode_source(side, "/dir_é/".as_bytes());
    for a in rec.actions() {
        if a["mnemonic"] != "FileWrite" {
            continue;
        }
        let f = rec.primary_output(a);
        if !f.get_exec_path_string().contains(&dir_marker) {
            continue;
        }
        let name = target_name(rec, a);
        let run = rec
            .actions()
            .iter()
            .find(|b| b["mnemonic"] == "Action" && b["targetId"] == a["targetId"])
            .unwrap();
        let out = rec.primary_output(run);
        FILES.with(|files| {
            let mut files = files.borrow_mut();
            for artifact in [&f, &out] {
                files.insert(
                    artifact.get_exec_path_string().to_string(),
                    artifact.clone(),
                );
            }
        });
        // defs.bzl: `a = ctx.actions.args(); a.add(out); ARGS_CASES[name](a, f)`.
        code.push_str(&format!(
            "args__{name} = make_args()\n\
             args__{name}.add(file({out}))\n\
             ARGS_CASES[{name_str}](args__{name}, file({f}))\n",
            name_str = starlark_str(&name),
            out = starlark_str(out.get_exec_path_string()),
            f = starlark_str(f.get_exec_path_string()),
        ));
        names.push(name);
    }
    assert_eq!(names.len(), 26, "ARGS_CASES probes in the recording");
    // defs.bzl: `a = ctx.actions.args(); WRITE_ARGS_CASES[name](a, None)`.
    for name in WRITE_ARGS {
        code.push_str(&format!(
            "args__{name} = make_args()\n\
             WRITE_ARGS_CASES[{name_str}](args__{name}, None)\n",
            name_str = starlark_str(name),
        ));
        names.push(name.to_string());
    }
    // defs.bzl: `d = ctx.actions.template_dict(); fill_template_dict(d)`.
    code.push_str("template_dict__ = make_template_dict()\nfill_template_dict(template_dict__)\n");

    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics)
        .with(blaze_starlark::depset::register_depset)
        .with(probe_natives)
        .build();
    let compiled =
        blaze_starlark::eval::compile(blaze_starlark::FileKind::Bzl, "probe/cases.bzl", code)
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
    let built = names
        .into_iter()
        .map(|n| {
            let args = blaze_actions::analysis::starlark::args::build_args(
                &module.get(&format!("args__{n}")).unwrap(),
                &mut || (*blaze_cmdline::repository_mapping::EMPTY).clone(),
            )
            .unwrap();
            (n, args)
        })
        .collect();
    let substitutions = blaze_actions::analysis::starlark::template_dict::get_all(
        &module.get("template_dict__").unwrap(),
    )
    .unwrap();
    (built, substitutions)
}

/// `ctx.actions.run(executable = "/bin/true", arguments = ..., env = ..., ...)` and
/// `ctx.actions.run_shell(command = ..., arguments = ...)` of defs.bzl.
fn spawn_action(
    rec: &Recording,
    a: &Value,
    name: &str,
    args: Option<BuiltArgs>,
) -> Arc<dyn Action> {
    let argv = str_list(a.get("arguments"));
    let env = kv_list(a.get("environmentVariables"));
    let config = OracleConfiguration::new();
    let mut b = starlark_action_builder();
    let rest: Vec<String> = if argv[0] == "/bin/bash" {
        // ShellCommand(sh, command, pad): `<sh> -c <command> [""]`, padded when there are
        // arguments.
        assert_eq!(argv[1], "-c");
        let pad = argv.len() > 3;
        b.set_shell_command(&PathFragment::create(&argv[0]), argv[2].clone(), pad);
        if pad {
            assert_eq!(argv[3], "");
            argv[4..].to_vec()
        } else {
            Vec::new()
        }
    } else {
        b.set_executable_as_string(PathFragment::create(&argv[0]).get_path_string());
        argv[1..].to_vec()
    };
    match args {
        Some(built) => {
            b.add_command_line_with_param_file(built.command_line, built.param_file_info);
        }
        None => {
            if !rest.is_empty() {
                b.add_command_line(command_line::of(rest));
            }
        }
    }
    b.add_outputs(rec.outputs(a));
    b.set_mnemonic("Action");
    // The default shell environment comes first in the fixed env; the action's own variables
    // follow (SpawnAction.createActionEnvironment).
    if name == "env_default_shell" {
        let mut own = env.clone();
        assert_eq!(own.shift_remove_index(0).unwrap().0, "PATH");
        b.use_default_shell_environment(own);
    } else {
        b.set_environment(env);
    }
    // aquery's `executionInfo` is the action's execution info merged with the owner's exec
    // properties; the `exec_properties` probe has only the latter.
    let recorded_info = kv_list(a.get("executionInfo"));
    let owner = if name == "exec_properties" {
        rec.action_owner_with_exec_properties(a, recorded_info)
    } else {
        b.set_execution_info(recorded_info);
        rec.action_owner(a)
    };
    Arc::new(b.build(owner, &config))
}

fn reconstruct(
    rec: &Recording,
    a: &Value,
    name: &str,
    built: &mut HashMap<String, BuiltArgs>,
    template_dict: &[Substitution],
) -> Arc<dyn Action> {
    let owner = rec.action_owner(a);
    match a["mnemonic"].as_str().unwrap() {
        "Action" => spawn_action(rec, a, name, built.remove(name)),
        "FileWrite" if WRITE_ARGS.contains(&name) => {
            // ctx.actions.write(output, content = <Args>)
            let args = built.remove(name).unwrap();
            Arc::new(ParameterFileWriteAction::with_inputs(
                owner,
                NestedSetBuilder::wrap(Order::STABLE_ORDER, args.directory_artifacts),
                rec.primary_output(a),
                args.command_line,
                args.parameter_file_type,
                false,
                "FileWrite",
                &IndexMap::new(),
                OutputPathsMode::OFF,
            ))
        }
        // ctx.actions.write(output, content = <string>, is_executable)
        "FileWrite" => Arc::new(FileWriteAction::create(
            owner,
            rec.primary_output(a),
            a.get("fileContents")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string(),
            a.get("isExecutable")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
            Compression::ALLOW,
        )),
        "TemplateExpand" => {
            let inputs = rec.flattened_inputs(a);
            assert_eq!(inputs.len(), 1);
            let substitutions = if name == "template_dict" {
                template_dict.to_vec()
            } else {
                kv_list(a.get("substitutions"))
                    .into_iter()
                    .map(|(k, v)| Substitution::of(k, v))
                    .collect()
            };
            Arc::new(TemplateExpansionAction::for_template_artifact(
                owner,
                inputs[0].clone(),
                rec.primary_output(a),
                substitutions,
                name == "template_executable",
            ))
        }
        // ctx.actions.symlink(output, target_file = <file>, is_executable)
        "Symlink" | "ExecutableSymlink" => {
            let inputs = rec.flattened_inputs(a);
            assert_eq!(inputs.len(), 1);
            let (input, output) = (inputs[0].clone(), rec.primary_output(a));
            if a["mnemonic"] == "Symlink" {
                Arc::new(SymlinkAction::to_artifact(owner, input, output, None))
            } else {
                Arc::new(SymlinkAction::to_executable(owner, input, output, None))
            }
        }
        "UnresolvedSymlink" => {
            let path = rec.primary_output(a).get_exec_path_string().to_string();
            let link = Artifact::create_special_artifact(
                rec.root_for(&path),
                PathFragment::create(&path),
                rec.owner_key("//:oracle_owner"),
                SpecialArtifactType::UNRESOLVED_SYMLINK,
            );
            // The key holds `target_path` as written in defs.bzl; aquery prints it normalized.
            let recorded = a["unresolvedSymlinkTarget"].as_str().unwrap();
            let target = if name == "symlink_unresolved_ascii" {
                assert_eq!(recorded, "../some/target");
                "../some/./target"
            } else {
                recorded
            };
            let action = UnresolvedSymlinkAction::create(
                owner,
                link,
                target,
                if name == "symlink_unresolved_dir" {
                    SymlinkTargetType::DIRECTORY
                } else {
                    SymlinkTargetType::UNSPECIFIED
                },
                "Creating symlink %{output}",
            );
            assert_eq!(action.aquery_unresolved_symlink_target().unwrap(), recorded);
            Arc::new(action)
        }
        other => panic!("probe action with unexpected mnemonic {other}"),
    }
}

/// Rebuilds the recorded actions (optionally some probes only) from strings decoded as `side`
/// says. Returns (matched, mismatches).
fn check(side: Side, only: Option<&[&str]>) -> (usize, Vec<String>) {
    let rec = Recording::from_json(
        "non_ascii_probe",
        recorded_json(side),
        "/probe/output_base",
        "/probe/output_base/execroot/_main",
        "/probe/workspace",
    );
    let (mut built, template_dict) = eval_cases(side, &rec);
    let (mut matched, mut mismatches) = (0, Vec::new());
    let mut names = HashSet::new();
    for a in rec.actions() {
        let name = target_name(&rec, a);
        names.insert(name.clone());
        let mnemonic = a["mnemonic"].as_str().unwrap();
        let expected = a["actionKey"].as_str().unwrap();
        if only.is_some_and(|only| !only.contains(&name.as_str())) {
            continue;
        }
        let action = reconstruct(&rec, a, &name, &mut built, &template_dict);
        let key = action.get_key(&ActionKeyContext::new(), None).unwrap();
        // File-writing actions: the contents must also be what aquery printed.
        if side == Side::Internal
            && let Some(p) = action.as_file_contents_provider()
            && let Some(want) = a.get("fileContents").and_then(|c| c.as_str())
        {
            assert_eq!(p.get_file_contents().unwrap(), want, "{name} contents");
        }
        if key == expected {
            matched += 1;
        } else {
            mismatches.push(format!(
                "{side:?} {name} {mnemonic}: got {key}, oracle {expected}"
            ));
        }
    }
    assert_eq!(names.len(), 52, "probe targets in the recording");
    (matched, mismatches)
}

#[test]
fn non_ascii_action_keys() {
    let (matched, mismatches) = check(Side::Internal, None);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert_eq!(matched, 78);
}

/// The probe is sensitive to the encoding: with Unicode strings where Bazel holds internal
/// strings (what Blaze held before the decoding switch, hashed the way Bazel hashes its strings),
/// the keys differ from the oracle's. Only the three empty, ASCII input files match.
#[test]
fn non_ascii_action_keys_differ_for_unicode_strings() {
    let probes = ["add_plain", "map_each_depset", "file_add", "env_value"];
    let (matched, mismatches) = check(Side::Unicode, Some(&probes));
    assert_eq!(matched, 3, "{}", mismatches.join("\n"));
    assert_eq!(mismatches.len(), 4, "{}", mismatches.join("\n"));
}

/// The compressed FileWrite probe must really take the compressed path, and the bytes written
/// are the original text.
#[test]
fn compressed_file_write_contents_bytes() {
    let side = Side::Internal;
    let rec = Recording::from_json(
        "non_ascii_probe",
        recorded_json(side),
        "/probe/output_base",
        "/probe/output_base/execroot/_main",
        "/probe/workspace",
    );
    for (name, compressed) in [("write_long", true), ("write_plain", false)] {
        let a = rec
            .actions()
            .iter()
            .find(|a| target_name(&rec, a) == name)
            .unwrap();
        let action = FileWriteAction::create(
            rec.action_owner(a),
            rec.primary_output(a),
            a["fileContents"].as_str().unwrap().to_string(),
            false,
            Compression::ALLOW,
        );
        assert_eq!(action.is_compressed(), compressed, "{name}");
        use blaze_actions::analysis::actions::abstract_file_write_action::AbstractFileWriteAction;
        let out = action
            .new_deterministic_writer()
            .unwrap()
            .get_bytes()
            .unwrap();
        let want: String = if name == "write_long" {
            (0..200).map(|i| format!("ligne {i}: é🐷\n")).collect()
        } else {
            "héllo wörld ☃\n".to_string()
        };
        assert_eq!(out, want.as_bytes(), "{name}");
    }
}
