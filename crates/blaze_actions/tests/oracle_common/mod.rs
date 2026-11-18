//! Shared loader for the oracle aquery recordings
//! (`~/.cache/blaze/oracle-records/oracle-9.3.2/<project>/raw/aquery_deps_jsonproto.stdout`).
//!
//! Rebuilds artifacts, nested sets, owners and the host execution platform from the recorded
//! jsonproto so that tests can reconstruct actions and compare keys and dump bytes with the
//! oracle's.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::{ConstraintValueData, PlatformInfoData};
use blaze_actions::{
    ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot, PlatformInfoRef,
    RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;
use serde_json::Value;

pub fn records_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME");
    PathBuf::from(home).join(".cache/blaze/oracle-records/oracle-9.3.2")
}

/// A test `ActionLookupKey` standing in for `ConfiguredTargetKey`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TestTargetKey {
    pub label: Label,
    pub checksum: String,
}

impl fmt::Display for TestTargetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ConfiguredTargetKey{{label={}, config={}}}",
            self.label, self.checksum
        )
    }
}

impl ActionLookupKey for TestTargetKey {
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
            .downcast_ref::<TestTargetKey>()
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

/// The host platform `@@platforms//host:host` as the oracle's analysis built it
/// (`platform(name = "host", constraint_values = HOST_CONSTRAINTS)` with
/// HOST_CONSTRAINTS = ['@platforms//cpu:x86_64', '@platforms//os:linux'] and the attribute
/// defaults of `PlatformRule`).
pub fn host_platform() -> PlatformInfoRef {
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
        // Java: PlatformRule.DEFAULT_MISSING_TOOLCHAIN_ERROR
        missing_toolchain_error_message: Some(
            "For more information on platforms or toolchains see \
             https://bazel.build/concepts/platforms-intro."
                .to_string(),
        ),
    })
}

/// One recorded project.
pub struct Recording {
    pub project: String,
    pub json: Value,
    pub output_base: String,
    pub execroot: String,
    pub workspace: String,
    pub fs: FileSystemRef,
    path_fragments: HashMap<u64, (String, Option<u64>)>,
    artifact_paths: HashMap<u64, String>,
    artifact_is_tree: HashMap<u64, bool>,
    depsets: HashMap<u64, (Vec<u64>, Vec<u64>)>,
    targets: HashMap<u64, (String, Option<u64>)>,
    rule_classes: HashMap<u64, String>,
    configurations: HashMap<u64, Value>,
    artifact_cache: parking_lot::Mutex<HashMap<String, Artifact>>,
    nested_set_cache: parking_lot::Mutex<HashMap<(u64, i32), NestedSet<Artifact>>>,
}

fn u(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

impl Recording {
    /// Loads a project's recording; `None` if the recording is not on this machine.
    pub fn load(project: &str) -> Option<Recording> {
        let dir = records_dir().join(project);
        let raw = std::fs::read_to_string(dir.join("raw/aquery_deps_jsonproto.stdout")).ok()?;
        let json: Value = serde_json::from_str(&raw).unwrap();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).ok()?)
                .unwrap();
        let paths = &manifest["context"]["paths"];
        Some(Recording::from_json(
            project,
            json,
            paths["output_base"].as_str().unwrap(),
            paths["execroot"].as_str().unwrap(),
            paths["workspace"].as_str().unwrap(),
        ))
    }

    /// A recording from an already parsed aquery jsonproto.
    pub fn from_json(
        project: &str,
        json: Value,
        output_base: &str,
        execroot: &str,
        workspace: &str,
    ) -> Recording {
        let mut r = Recording {
            project: project.to_string(),
            output_base: output_base.to_string(),
            execroot: execroot.to_string(),
            workspace: workspace.to_string(),
            fs: UnixFileSystem::create(DigestHashFunction::Sha256),
            path_fragments: HashMap::new(),
            artifact_paths: HashMap::new(),
            artifact_is_tree: HashMap::new(),
            depsets: HashMap::new(),
            targets: HashMap::new(),
            rule_classes: HashMap::new(),
            configurations: HashMap::new(),
            artifact_cache: parking_lot::Mutex::new(HashMap::new()),
            nested_set_cache: parking_lot::Mutex::new(HashMap::new()),
            json,
        };
        for p in r.json["pathFragments"].as_array().unwrap() {
            r.path_fragments.insert(
                u(&p["id"]),
                (
                    p["label"].as_str().unwrap().to_string(),
                    p.get("parentId").map(u),
                ),
            );
        }
        let arts: Vec<(u64, u64, bool)> = r.json["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    u(&a["id"]),
                    u(&a["pathFragmentId"]),
                    a.get("isTreeArtifact")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false),
                )
            })
            .collect();
        for (id, pf, tree) in arts {
            let path = r.path_of_fragment(pf);
            r.artifact_paths.insert(id, path);
            r.artifact_is_tree.insert(id, tree);
        }
        for d in r.json["depSetOfFiles"].as_array().unwrap() {
            let ids = |k: &str| -> Vec<u64> {
                d.get(k)
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().map(u).collect())
                    .unwrap_or_default()
            };
            r.depsets.insert(
                u(&d["id"]),
                (ids("transitiveDepSetIds"), ids("directArtifactIds")),
            );
        }
        for t in r.json["targets"].as_array().unwrap() {
            r.targets.insert(
                u(&t["id"]),
                (
                    t["label"].as_str().unwrap().to_string(),
                    t.get("ruleClassId").map(u),
                ),
            );
        }
        for c in r.json["ruleClasses"].as_array().unwrap() {
            r.rule_classes
                .insert(u(&c["id"]), c["name"].as_str().unwrap().to_string());
        }
        for c in r.json["configuration"].as_array().unwrap() {
            r.configurations.insert(u(&c["id"]), c.clone());
        }
        r
    }

    pub fn path_of_fragment(&self, mut id: u64) -> String {
        let mut parts = Vec::new();
        loop {
            let (label, parent) = &self.path_fragments[&id];
            parts.push(label.clone());
            match parent {
                Some(p) => id = *p,
                None => break,
            }
        }
        parts.reverse();
        parts.join("/")
    }

    pub fn actions(&self) -> &Vec<Value> {
        self.json["actions"].as_array().unwrap()
    }

    pub fn artifact_path(&self, id: u64) -> &str {
        &self.artifact_paths[&id]
    }

    pub fn target_label(&self, target_id: u64) -> String {
        self.targets[&target_id].0.clone()
    }

    pub fn rule_class(&self, target_id: u64) -> Option<String> {
        self.targets[&target_id]
            .1
            .map(|rc| self.rule_classes[&rc].clone())
    }

    pub fn configuration(&self, id: u64) -> &Value {
        &self.configurations[&id]
    }

    pub fn exec_root(&self) -> Path {
        Path::create(PathFragment::create(&self.execroot), &self.fs)
    }

    /// The artifact root an exec path belongs to.
    pub fn root_for(&self, exec_path: &str) -> ArtifactRoot {
        let segs: Vec<&str> = exec_path.split('/').collect();
        if segs[0] == "bazel-out" {
            if segs.len() >= 4 && matches!(segs[2], "bin" | "genfiles" | "testlogs") {
                return ArtifactRoot::as_derived_root(
                    &self.exec_root(),
                    RootType::OUTPUT,
                    &["bazel-out", segs[1], segs[2]],
                );
            }
            return ArtifactRoot::as_derived_root(
                &self.exec_root(),
                RootType::OUTPUT,
                &["bazel-out"],
            );
        }
        if segs[0] == "external" {
            let p = format!("{}/external/{}", self.output_base, segs[1]);
            return ArtifactRoot::as_external_source_root(Root::from_path(Path::create(
                PathFragment::create(&p),
                &self.fs,
            )));
        }
        ArtifactRoot::as_source_root(Root::from_path(Path::create(
            PathFragment::create(&self.workspace),
            &self.fs,
        )))
    }

    /// An owner key for derived artifacts of the given target label.
    pub fn owner_key(&self, label: &str) -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(TestTargetKey {
            label: Label::parse_canonical(label).unwrap(),
            checksum: "test".to_string(),
        })
    }

    /// The artifact for an exec path (derived artifacts get an owner from the exec path's
    /// package-ish location; key computations never read derived owners).
    pub fn artifact(&self, exec_path: &str, is_tree: bool) -> Artifact {
        if let Some(a) = self.artifact_cache.lock().get(exec_path) {
            return a.clone();
        }
        let root = self.root_for(exec_path);
        let a = if root.is_source_root() {
            Artifact::new_source_artifact(
                root,
                PathFragment::create(exec_path),
                ArtifactOwner::NullOwner,
            )
        } else if is_tree {
            Artifact::create_special_artifact(
                root,
                PathFragment::create(exec_path),
                self.owner_key("//:oracle_owner"),
                blaze_actions::SpecialArtifactType::TREE,
            )
        } else {
            Artifact::create_derived_artifact(
                root,
                PathFragment::create(exec_path),
                self.owner_key("//:oracle_owner"),
            )
        };
        self.artifact_cache
            .lock()
            .insert(exec_path.to_string(), a.clone());
        a
    }

    pub fn artifact_by_id(&self, id: u64) -> Artifact {
        self.artifact(&self.artifact_paths[&id], self.artifact_is_tree[&id])
    }

    /// Rebuilds a recorded dep set as a nested set of the given order: transitive members first,
    /// then direct ones (the layout `NestedSet`'s constructor produces for non-naive orders).
    pub fn nested_set(&self, depset_id: u64, order: Order) -> NestedSet<Artifact> {
        // One nested set per recorded id: aquery keys dep sets by node identity.
        let cache_key = (depset_id, order.ordinal());
        if let Some(s) = self.nested_set_cache.lock().get(&cache_key) {
            return s.clone();
        }
        let s = self.build_nested_set(depset_id, order);
        self.nested_set_cache.lock().insert(cache_key, s.clone());
        s
    }

    fn build_nested_set(&self, depset_id: u64, order: Order) -> NestedSet<Artifact> {
        let (transitive, direct) = &self.depsets[&depset_id];
        let mut b = NestedSetBuilder::new_builder(order);
        for t in transitive {
            let child = self.nested_set(*t, order);
            b.add_transitive(&child);
        }
        for d in direct {
            b.add(self.artifact_by_id(*d));
        }
        b.build()
    }

    /// The recorded (transitive ids, direct artifact ids) of a dep set.
    pub fn depset_parts(&self, depset_id: u64) -> (Vec<u64>, Vec<u64>) {
        self.depsets[&depset_id].clone()
    }

    pub fn flattened_inputs(&self, action: &Value) -> Vec<Artifact> {
        let mut out = Vec::new();
        if let Some(ids) = action.get("inputDepSetIds").and_then(|v| v.as_array()) {
            for id in ids {
                out.extend(
                    self.nested_set(u(id), Order::STABLE_ORDER)
                        .to_list()
                        .iter()
                        .cloned(),
                );
            }
        }
        out
    }

    pub fn outputs(&self, action: &Value) -> Vec<Artifact> {
        action["outputIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| self.artifact_by_id(u(id)))
            .collect()
    }

    pub fn primary_output(&self, action: &Value) -> Artifact {
        self.artifact_by_id(u(&action["primaryOutputId"]))
    }

    /// An owner for an action of the given recorded target and configuration.
    pub fn action_owner(&self, action: &Value) -> ActionOwner {
        self.action_owner_with_exec_properties(action, IndexMap::new())
    }

    /// [`Recording::action_owner`] for a target with an `exec_properties` attribute.
    pub fn action_owner_with_exec_properties(
        &self,
        action: &Value,
        exec_properties: IndexMap<String, String>,
    ) -> ActionOwner {
        let target_id = u(&action["targetId"]);
        let label = self.target_label(target_id);
        let config = self.configuration(u(&action["configurationId"]));
        ActionOwner::create_dummy(
            Some(Label::parse_canonical(&label).unwrap()),
            Location::from_file("BUILD"),
            self.rule_class(target_id).unwrap_or_default() + " rule",
            config["mnemonic"].as_str().unwrap(),
            config["checksum"].as_str().unwrap(),
            None,
            config
                .get("isTool")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
            Some(host_platform()),
            Vec::new(),
            exec_properties,
        )
    }
}

pub fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

pub fn kv_list(v: Option<&Value>) -> IndexMap<String, String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|kv| {
                    (
                        kv["key"].as_str().unwrap().to_string(),
                        kv.get("value")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Per-mnemonic match tally.
#[derive(Default)]
pub struct Tally {
    pub by_mnemonic: std::collections::BTreeMap<String, (usize, usize, usize)>,
    pub mismatches: Vec<String>,
}

impl Tally {
    /// Records one action: `Some(true)` matched, `Some(false)` mismatched, `None` not attempted.
    pub fn record(
        &mut self,
        mnemonic: &str,
        result: Option<bool>,
        detail: impl FnOnce() -> String,
    ) {
        let e = self.by_mnemonic.entry(mnemonic.to_string()).or_default();
        match result {
            Some(true) => e.0 += 1,
            Some(false) => {
                e.1 += 1;
                self.mismatches.push(detail());
            }
            None => e.2 += 1,
        }
    }

    pub fn report(&self, project: &str) -> String {
        let mut s = format!("{project}: mnemonic matched/mismatched/not-attempted\n");
        for (m, (a, b, c)) in &self.by_mnemonic {
            s.push_str(&format!("  {m:<24} {a:>5} {b:>5} {c:>5}\n"));
        }
        s
    }
}
