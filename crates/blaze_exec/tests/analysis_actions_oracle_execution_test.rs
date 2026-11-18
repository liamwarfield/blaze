//! Executes actions rebuilt from the oracle's aquery recordings
//! (`~/.cache/blaze/oracle-records/oracle-9.3.2/<project>/raw/aquery_deps_jsonproto.stdout`) into
//! a temporary execroot and compares what they write with the oracle: file bytes with aquery's
//! `fileContents`, and sha256 / symlink targets with the oracle's built outputs
//! (`items/outputs.json`, paths normalised to `<EXECROOT>` / `<WORKSPACE>`).
//!
//! Skips (passes vacuously, printing why) when the recordings are absent.

#[path = "../../blaze_actions/tests/oracle_common/mod.rs"]
mod oracle_common;

mod analysis_actions_common;

use std::sync::Arc;

use analysis_actions_common::TestContext;
use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
use blaze_actions::analysis::actions::substitution::Substitution;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::symlink_tree_action::{
    RunfileSymlinksMode, SymlinkTreeAction,
};
use blaze_actions::analysis::actions::template::Template;
use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;
use blaze_actions::analysis::repo_mapping_manifest_action::{
    RepoMappingManifestAction, RepoMappingPackageMetadata,
};
use blaze_actions::analysis::runfiles::RunfilesBuilder;
use blaze_actions::analysis::source_manifest_action::{ManifestType, SourceManifestAction};
use blaze_actions::{ActionEnvironment, Artifact};
use blaze_cmdline::{PackageIdentifier, RepositoryMapping, RepositoryName};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::as_executable;
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use oracle_common::{Recording, kv_list, records_dir};
use serde_json::Value;
use sha2::{Digest, Sha256};

struct Env {
    rec: Recording,
    outputs: serde_json::Map<String, Value>,
    _tmp: tempfile::TempDir,
    oracle_execroot: String,
    ctx: TestContext,
}

/// Loads a recording and points its execroot and workspace at a temporary directory, so that
/// every derived artifact is written there.
fn load(project: &str) -> Option<Env> {
    let mut rec = Recording::load(project)?;
    let outputs: Value = serde_json::from_str(
        &std::fs::read_to_string(records_dir().join(project).join("items/outputs.json")).ok()?,
    )
    .ok()?;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_str().unwrap().to_string();
    let oracle_execroot = rec.execroot.clone();
    rec.execroot = format!("{base}/execroot/_main");
    rec.workspace = format!("{base}/workspace");
    std::fs::create_dir_all(&rec.execroot).unwrap();
    std::fs::create_dir_all(&rec.workspace).unwrap();
    let ctx = TestContext::new(rec.exec_root());
    Some(Env {
        rec,
        outputs: outputs["items"].as_object().unwrap().clone(),
        _tmp: tmp,
        oracle_execroot,
        ctx,
    })
}

impl Env {
    fn execute(&self, action: &dyn Action) {
        for o in action.get_outputs() {
            o.get_path()
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
        }
        as_executable(action)
            .expect("registered")
            .execute(&self.ctx)
            .unwrap_or_else(|e| panic!("{}: {}", action.describe(), e.get_message()));
    }

    /// The `outputs.json` key of a `bazel-out/<cfg>/bin/...` exec path.
    fn output_key(exec_path: &str) -> Option<String> {
        let rest = exec_path.strip_prefix("bazel-out/k8-fastbuild/bin/")?;
        Some(format!("bin/{rest}"))
    }

    fn normalise(&self, s: &str) -> String {
        s.replace(&self.rec.execroot, "<EXECROOT>")
            .replace(&self.rec.workspace, "<WORKSPACE>")
    }

    /// Compares an output file with the oracle's built output (sha256 of the path-normalised
    /// bytes). Returns false when the oracle did not record that output.
    fn check_output_file(&self, artifact: &Artifact) -> bool {
        let Some(key) = Self::output_key(artifact.get_exec_path_string()) else {
            return false;
        };
        let Some(entry) = self.outputs.get(&key) else {
            return false;
        };
        let bytes = std::fs::read(artifact.get_path().get_path_string()).unwrap();
        let norm = self.normalise(&String::from_utf8(bytes).unwrap());
        assert_eq!(
            hex::encode(Sha256::digest(norm.as_bytes())),
            entry["sha256"].as_str().unwrap(),
            "{key}: content differs from the oracle's output:\n{norm}"
        );
        true
    }

    /// Compares a symlink with the oracle's built output (normalised target).
    fn check_output_symlink(&self, path: &Path, key: &str) {
        let entry = self
            .outputs
            .get(key)
            .unwrap_or_else(|| panic!("oracle has no output {key}"));
        assert_eq!(entry["type"], "symlink", "{key}");
        let target = path.read_symbolic_link().unwrap();
        assert_eq!(
            self.normalise(target.get_path_string()),
            entry["target"].as_str().unwrap(),
            "{key}"
        );
    }
}

fn actions<'a>(rec: &'a Recording, mnemonic: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    rec.actions()
        .iter()
        .filter(move |a| a["mnemonic"] == mnemonic)
}

/// `FileWrite` (`ctx.actions.write`) actions: the written bytes equal aquery's `fileContents`,
/// and the oracle's output where it recorded one.
fn check_file_writes(env: &Env) -> (usize, usize) {
    let (mut n, mut with_output) = (0, 0);
    for a in actions(&env.rec, "FileWrite") {
        if a.get("inputDepSetIds").is_some() {
            continue;
        }
        let output = env.rec.primary_output(a);
        let contents = a
            .get("fileContents")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let make_executable = a
            .get("isExecutable")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let action = FileWriteAction::create(
            env.rec.action_owner(a),
            output.clone(),
            contents.clone(),
            make_executable,
            Compression::ALLOW,
        );
        env.execute(&action);
        assert_eq!(
            std::fs::read(output.get_path().get_path_string()).unwrap(),
            contents.as_bytes(),
            "{}",
            output.get_exec_path_string()
        );
        assert_eq!(output.get_path().is_executable().unwrap(), make_executable);
        n += 1;
        if env.check_output_file(&output) {
            with_output += 1;
        }
    }
    (n, with_output)
}

/// `TemplateExpand` actions, with the recorded template content and substitutions: the expanded
/// file equals the oracle's output.
fn check_template_expansions(env: &Env) -> (usize, usize) {
    let (mut n, mut with_output) = (0, 0);
    for a in actions(&env.rec, "TemplateExpand") {
        let output = env.rec.primary_output(a);
        let substitutions = kv_list(a.get("substitutions"))
            .into_iter()
            .map(|(k, v)| Substitution::of(k, v))
            .collect();
        let action = TemplateExpansionAction::new(
            env.rec.action_owner(a),
            output.clone(),
            Template::for_string(a["templateContent"].as_str().unwrap()),
            substitutions,
            false,
        );
        env.execute(&action);
        n += 1;
        if env.check_output_file(&output) {
            with_output += 1;
        }
    }
    (n, with_output)
}

/// brotli's runfiles: `RepoMappingManifest`, `SourceSymlinkManifest` and `SymlinkTree` for
/// `//:brotli`, whose runfiles are the binary alone.
fn check_brotli_runfiles(env: &Env) {
    let rec = &env.rec;
    let exe = rec.artifact("bazel-out/k8-fastbuild/bin/brotli", false);
    let repo_mapping = rec.artifact("bazel-out/k8-fastbuild/bin/brotli.repo_mapping", false);
    let input_manifest = rec.artifact("bazel-out/k8-fastbuild/bin/brotli.runfiles_manifest", false);
    let output_manifest =
        rec.artifact("bazel-out/k8-fastbuild/bin/brotli.runfiles/MANIFEST", false);
    std::fs::create_dir_all(
        exe.get_path()
            .get_parent_directory()
            .unwrap()
            .get_path_string(),
    )
    .unwrap();
    std::fs::write(exe.get_path().get_path_string(), b"binary").unwrap();

    let find = |mnemonic: &str| actions(rec, mnemonic).next().unwrap().clone();
    let mut b = RunfilesBuilder::new("_main");
    b.add_artifact(exe.clone());
    let runfiles = b.build();

    // RepoMappingManifestAction: the main repository's mapping (module name `org_brotli`).
    let a = find("RepoMappingManifest");
    let mapping = RepositoryMapping::create(
        [
            ("org_brotli", RepositoryName::create("").unwrap()),
            (
                "bazel_tools",
                RepositoryName::create("bazel_tools").unwrap(),
            ),
        ],
        RepositoryName::create("").unwrap(),
    );
    let action = RepoMappingManifestAction::new(
        rec.action_owner(&a),
        repo_mapping.clone(),
        NestedSetBuilder::create(
            Order::STABLE_ORDER,
            [RepoMappingPackageMetadata::new(
                PackageIdentifier::create_in_main_repo(PathFragment::empty_fragment()),
                mapping,
            )],
        ),
        runfiles.get_artifacts().clone(),
        runfiles.get_symlinks(),
        runfiles.get_root_symlinks().clone(),
        "_main".to_string(),
        false,
    );
    env.execute(&action);
    assert_eq!(
        std::fs::read_to_string(repo_mapping.get_path().get_path_string()).unwrap(),
        a["fileContents"].as_str().unwrap()
    );
    assert!(env.check_output_file(&repo_mapping));

    // SourceManifestAction (SOURCE_SYMLINKS).
    let a = find("SourceSymlinkManifest");
    let action = SourceManifestAction::new(
        Arc::new(ManifestType::SOURCE_SYMLINKS),
        rec.action_owner(&a),
        input_manifest.clone(),
        runfiles.clone(),
        Some(repo_mapping.clone()),
        false,
    );
    env.execute(&action);
    let written = std::fs::read_to_string(input_manifest.get_path().get_path_string()).unwrap();
    assert_eq!(
        written.replace(&env.rec.execroot, &env.oracle_execroot),
        a["fileContents"].as_str().unwrap()
    );
    assert!(env.check_output_file(&input_manifest));

    // SymlinkTreeAction (--enable_runfiles=auto on Linux: CREATE).
    let a = find("SymlinkTree");
    let mut fixed = IndexMap::new();
    fixed.insert(
        "PATH".to_string(),
        "/bin:/usr/bin:/usr/local/bin".to_string(),
    );
    let action = SymlinkTreeAction::new(
        rec.action_owner(&a),
        input_manifest.clone(),
        Some(runfiles),
        output_manifest.clone(),
        Some(repo_mapping),
        ActionEnvironment::create(fixed, IndexSet::new()),
        RunfileSymlinksMode::CREATE,
        "_main".to_string(),
    );
    env.execute(&action);
    let tree = output_manifest.get_path().get_parent_directory().unwrap();
    env.check_output_symlink(&output_manifest.get_path(), "bin/brotli.runfiles/MANIFEST");
    env.check_output_symlink(
        &tree.get_relative_str("_main/brotli"),
        "bin/brotli.runfiles/_main/brotli",
    );
    env.check_output_symlink(
        &tree.get_relative_str("_repo_mapping"),
        "bin/brotli.runfiles/_repo_mapping",
    );
    // The oracle's tree holds exactly these entries.
    let mut entries: Vec<String> = Vec::new();
    for e in walkdir(tree.get_path_string()) {
        entries.push(format!("bin/brotli.runfiles/{e}"));
    }
    entries.sort();
    let mut expected: Vec<String> = env
        .outputs
        .keys()
        .filter(|k| k.starts_with("bin/brotli.runfiles/"))
        .cloned()
        .collect();
    expected.sort();
    // Directories are not recorded unless empty.
    entries.retain(|e| e != "bin/brotli.runfiles/_main");
    assert_eq!(entries, expected);
}

fn walkdir(root: &str) -> Vec<String> {
    fn go(dir: &std::path::Path, rel: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_str().unwrap().to_string();
            let r = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            out.push(r.clone());
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                go(&e.path(), &r, out);
            }
        }
    }
    let mut out = Vec::new();
    go(std::path::Path::new(root), "", &mut out);
    out
}

/// brotli's `Symlink` actions (`_virtual_includes`): the links point at the source artifacts in
/// the workspace, as the oracle's do.
fn check_brotli_symlinks(env: &Env) -> usize {
    let mut n = 0;
    for a in actions(&env.rec, "Symlink") {
        let output = env.rec.primary_output(a);
        let Some(key) = Env::output_key(output.get_exec_path_string()) else {
            continue;
        };
        if !env.outputs.contains_key(&key) {
            continue;
        }
        let inputs = env.rec.flattened_inputs(a);
        assert_eq!(inputs.len(), 1);
        let input = inputs[0].clone();
        std::fs::create_dir_all(
            input
                .get_path()
                .get_parent_directory()
                .unwrap()
                .get_path_string(),
        )
        .unwrap();
        std::fs::write(input.get_path().get_path_string(), b"header").unwrap();
        env.ctx.metadata.put_for_testing(&input);
        let action =
            SymlinkAction::to_artifact(env.rec.action_owner(a), input, output.clone(), None);
        env.execute(&action);
        env.check_output_symlink(&output.get_path(), &key);
        n += 1;
    }
    n
}

#[test]
fn brotli_execution_matches_oracle() {
    let Some(env) = load("brotli") else {
        eprintln!("SKIP: no brotli oracle recording");
        return;
    };
    let (writes, _) = check_file_writes(&env);
    assert!(writes >= 1);
    check_brotli_runfiles(&env);
    let symlinks = check_brotli_symlinks(&env);
    assert_eq!(symlinks, 5);
    eprintln!("brotli: {writes} FileWrite, 3 runfiles actions, {symlinks} Symlink match");
}

#[test]
fn re2_execution_matches_oracle() {
    let Some(env) = load("re2") else {
        eprintln!("SKIP: no re2 oracle recording");
        return;
    };
    let (writes, writes_with_output) = check_file_writes(&env);
    let (templates, templates_with_output) = check_template_expansions(&env);
    assert_eq!(writes_with_output, 1);
    assert_eq!(templates_with_output, 3);
    eprintln!(
        "re2: {writes} FileWrite ({writes_with_output} vs oracle output), {templates} TemplateExpand ({templates_with_output} vs oracle output) match"
    );
}
