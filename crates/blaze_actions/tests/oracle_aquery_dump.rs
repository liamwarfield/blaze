//! Replays the oracle's recorded aquery graphs through `ActionGraphDump` and requires the
//! jsonproto bytes to equal the oracle's.
//!
//! Each recorded action becomes a `RecordedAction` that reports the recorded key, command line,
//! environment, file contents and template data; artifacts and dep sets are rebuilt from the
//! recording (same nested-set structure). The dump then has to reproduce Java's id assignment,
//! emission order and JSON formatting exactly.

mod oracle_common;

use std::any::Any;
use std::sync::Arc;

use blaze_actions::actions::action::{
    AbstractAction, Action, CommandAction, CommandLineError, ComputeKeyError, FileContentsProvider,
    InputMetadataProvider,
};
use blaze_actions::actions::action_owner::BuildConfigurationEvent;
use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{ActionGraphDump, DumpedTarget};
use blaze_actions::skyframe::actiongraph::v2::aquery_output_handler::{
    AqueryOutputHandler, MonolithicOutputHandler,
};
use blaze_actions::{ActionKeyContext, ActionOwner, PlatformInfoRef};
use blaze_cmdline::Label;
use blaze_collect::command_line_item::{CommandLineExpansionException, InterruptedException};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_protos::build_event_stream;
use blaze_util::Fingerprint;
use indexmap::IndexMap;
use oracle_common::{Recording, kv_list, str_list};
use serde_json::Value;

#[derive(Debug)]
struct RecordedAction {
    base: AbstractAction,
    mnemonic: String,
    key: String,
    discovers_inputs: bool,
    arguments: Option<Vec<String>>,
    env: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    file_contents: Option<String>,
    is_executable: bool,
    template: Option<(String, Vec<(String, String)>)>,
    unresolved_symlink_target: Option<String>,
}

impl Action for RecordedAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }
    fn compute_key(
        &self,
        _: &ActionKeyContext,
        _: Option<&dyn InputMetadataProvider>,
        _: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        unreachable!()
    }
    fn get_key(
        &self,
        _: &ActionKeyContext,
        _: Option<&dyn InputMetadataProvider>,
    ) -> Result<String, InterruptedException> {
        Ok(self.key.clone())
    }
    fn discovers_inputs(&self) -> bool {
        self.discovers_inputs
    }
    fn inputs_discovered(&self) -> bool {
        true
    }
    fn get_original_inputs(&self) -> blaze_collect::NestedSet<blaze_actions::Artifact> {
        self.get_inputs()
    }
    fn get_effective_environment(
        &self,
        _client_env: &IndexMap<String, String>,
        _path_mapper: &dyn blaze_actions::actions::path_mapper::PathMapper,
    ) -> Result<IndexMap<String, String>, CommandLineExpansionException> {
        Ok(self.env.clone())
    }
    fn get_execution_info(&self) -> IndexMap<String, String> {
        self.execution_info.clone()
    }
    fn as_command_action(&self) -> Option<&dyn CommandAction> {
        if self.arguments.is_some() {
            Some(self)
        } else {
            None
        }
    }
    fn as_file_contents_provider(&self) -> Option<&dyn FileContentsProvider> {
        if self.file_contents.is_some() {
            Some(self)
        } else {
            None
        }
    }
    fn aquery_template_expansion(&self) -> Option<Result<(String, Vec<(String, String)>), String>> {
        self.template.clone().map(Ok)
    }
    fn aquery_unresolved_symlink_target(&self) -> Option<String> {
        self.unresolved_symlink_target.clone()
    }
}

impl CommandAction for RecordedAction {
    fn get_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        Ok(self.arguments.clone().unwrap())
    }
}

impl FileContentsProvider for RecordedAction {
    fn get_file_contents(&self) -> Result<String, std::io::Error> {
        Ok(self.file_contents.clone().unwrap_or_default())
    }
    fn make_executable(&self) -> bool {
        self.is_executable
    }
}

/// A platform that only has a label (aquery prints `executionPlatform` from the label).
#[derive(Debug)]
struct LabelOnlyPlatform(Label);

impl blaze_actions::PlatformInfo for LabelOnlyPlatform {
    fn label(&self) -> &Label {
        &self.0
    }
    fn add_to(&self, _fp: &mut Fingerprint) {
        unreachable!()
    }
    fn exec_properties(&self) -> IndexMap<String, String> {
        IndexMap::new()
    }
    fn remote_execution_properties(&self) -> String {
        unreachable!()
    }
    fn constraint_value(&self, _setting: &Label) -> Option<Label> {
        unreachable!()
    }
}

fn recorded_action(rec: &Recording, a: &Value) -> RecordedAction {
    let config = rec.configuration(a["configurationId"].as_u64().unwrap());
    let event = BuildConfigurationEvent {
        event_id: config["checksum"].as_str().unwrap().to_string(),
        configuration: build_event_stream::Configuration {
            mnemonic: config["mnemonic"].as_str().unwrap().to_string(),
            platform_name: config["platformName"].as_str().unwrap_or("").to_string(),
            cpu: String::new(),
            make_variable: Vec::new(),
            is_tool: config
                .get("isTool")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
        },
    };
    let platform: Option<PlatformInfoRef> = a
        .get("executionPlatform")
        .and_then(|p| p.as_str())
        .map(|p| {
            Arc::new(LabelOnlyPlatform(Label::parse_canonical(p).unwrap())) as PlatformInfoRef
        });
    let target_id = a["targetId"].as_u64().unwrap();
    assert!(
        a.get("aspectDescriptorIds").is_none(),
        "aspects not replayed"
    );
    let owner = ActionOwner::create_dummy(
        Some(Label::parse_canonical(&rec.target_label(target_id)).unwrap()),
        Location::from_file("BUILD"),
        "rule",
        config["mnemonic"].as_str().unwrap(),
        config["checksum"].as_str().unwrap(),
        Some(event),
        config
            .get("isTool")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        platform,
        Vec::new(),
        IndexMap::new(),
    );
    let inputs = match a.get("inputDepSetIds").and_then(|v| v.as_array()) {
        None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Some(ids) => {
            assert_eq!(ids.len(), 1);
            rec.nested_set(ids[0].as_u64().unwrap(), Order::STABLE_ORDER)
        }
    };
    // The primary output is the first output.
    let mut outputs = rec.outputs(a);
    let primary = rec.primary_output(a);
    assert_eq!(outputs[0], primary);
    outputs.dedup();
    let template = a.get("templateContent").map(|t| {
        (
            t.as_str().unwrap().to_string(),
            kv_list(a.get("substitutions")).into_iter().collect(),
        )
    });
    let has_args = a.get("arguments").is_some();
    RecordedAction {
        base: AbstractAction::new(owner, inputs, outputs),
        mnemonic: a["mnemonic"].as_str().unwrap().to_string(),
        key: a["actionKey"].as_str().unwrap().to_string(),
        discovers_inputs: a
            .get("discoversInputs")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        arguments: if has_args {
            Some(str_list(a.get("arguments")))
        } else {
            None
        },
        env: kv_list(a.get("environmentVariables")),
        execution_info: kv_list(a.get("executionInfo")),
        file_contents: a
            .get("fileContents")
            .map(|c| c.as_str().unwrap().to_string()),
        is_executable: a
            .get("isExecutable")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        template,
        unresolved_symlink_target: a
            .get("unresolvedSymlinkTarget")
            .map(|c| c.as_str().unwrap().to_string()),
    }
}

/// Dumps every recorded action through `ActionGraphDump` and the jsonproto handler.
fn dump_jsonproto(rec: &Recording) -> Vec<u8> {
    let mut handler = MonolithicOutputHandler::new(Vec::<u8>::new());
    {
        let mut dump = ActionGraphDump::new(true, true, false, None, true, true, &mut handler);
        for a in rec.actions() {
            let target_id = a["targetId"].as_u64().unwrap();
            let target = DumpedTarget {
                label: rec.target_label(target_id),
                rule_class_string: rec.rule_class(target_id),
            };
            let action: Arc<dyn Action> = Arc::new(recorded_action(rec, a));
            dump.dump_configured_target(&target, &[action]).unwrap();
        }
    }
    handler.close().unwrap();
    handler.into_inner()
}

fn assert_same_bytes(what: &str, actual: &[u8], expected: &[u8]) {
    if actual != expected {
        let a: Vec<&[u8]> = actual.split(|b| *b == b'\n').collect();
        let e: Vec<&[u8]> = expected.split(|b| *b == b'\n').collect();
        let first = a.iter().zip(e.iter()).position(|(x, y)| x != y);
        panic!(
            "{what}: aquery jsonproto differs ({} vs {} lines); first difference at line {:?}:\n  actual:   {:?}\n  expected: {:?}",
            a.len(),
            e.len(),
            first.map(|i| i + 1),
            first.map(|i| a[i].escape_ascii().to_string()),
            first.map(|i| e[i].escape_ascii().to_string()),
        );
    }
}

fn replay(project: &str) {
    let Some(rec) = Recording::load(project) else {
        eprintln!("no oracle recording for {project}; skipped");
        return;
    };
    let expected = std::fs::read(
        oracle_common::records_dir()
            .join(project)
            .join("raw/aquery_deps_jsonproto.stdout"),
    )
    .unwrap();
    assert_same_bytes(project, &dump_jsonproto(&rec), &expected);
}

#[test]
fn brotli_aquery_jsonproto_bytes_match_oracle() {
    replay("brotli");
}

#[test]
fn re2_aquery_jsonproto_bytes_match_oracle() {
    replay("re2");
}

// ---------------------------------------------------------------------------------------------
// Non-ASCII strings (review 2026-11-18-actions round 2, N1).
//
// `data/non_ascii_probe/aquery.jsonproto` is the oracle's output for the probe workspace (see
// `non_ascii_action_keys.rs`). Bazel prints its internal strings through a Latin-1 stream, so the
// file holds each string's bytes: UTF-8 text reads as UTF-8, and `"\377"` is the byte ff.

const NON_ASCII_AQUERY: &[u8] = include_bytes!("data/non_ascii_probe/aquery.jsonproto");

fn non_ascii_recording(json: Value) -> Recording {
    Recording::from_json(
        "non_ascii_probe",
        json,
        "/probe/output_base",
        "/probe/output_base/execroot/_main",
        "/probe/workspace",
    )
}

/// Blaze holds Bazel's strings (the file read as Latin-1: internal strings), and the dump is the
/// oracle's file, byte for byte, the `ff` of the raw-byte probes included.
#[test]
fn non_ascii_aquery_jsonproto_bytes_match_oracle() {
    use blaze_util::string_encoding::latin1_decode;
    let json: Value = serde_json::from_str(&latin1_decode(NON_ASCII_AQUERY)).unwrap();
    let rec = non_ascii_recording(json);
    assert_same_bytes("non-ASCII probe", &dump_jsonproto(&rec), NON_ASCII_AQUERY);
}
