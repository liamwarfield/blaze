//! The streamer against the oracle's recorded `--build_event_json_file` (Bazel 9.3.2) for the
//! corpus projects (`~/.cache/blaze/oracle-records/oracle-9.3.2/<project>/raw/bep.json`).
//!
//! - `replay_*`: decodes the oracle's events into protos and posts them, in the recorded order
//!   and without the progress events, through `BuildEventStreamer` + `JsonFormatFileTransport`.
//!   The streamer must regenerate the progress chain (ids, children), the announced children
//!   and `last_message` exactly, and serialize every event to the same JSON line; only the
//!   progress events' stdout/stderr (the console output, which a replay has not got) differ.
//! - `reconstruct_brotli`: builds brotli's `build //...` stream from first principles with the
//!   ported event classes (targets, configurations, output artifacts with their metadata as
//!   nested sets, workspace status, exec root, symlinks, completion), fed only with the facts
//!   the oracle recorded; writes the file for `scripts/bep-compare`, which scores it with the
//!   recorder's normalisation (`scripts/bep-compare --project brotli --candidate <file>`).
//!
//! Skipped (passing) when the oracle recordings are absent.

use std::fmt;
use std::sync::Arc;

use blaze_actions::{
    ActionLookupKey, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot,
    BuildConfigurationEvent, RootType,
};
use blaze_bep::bep_module::{BuildEventPost, events_to_json_bytes};
use blaze_bep::buildeventstream::build_event_context::BuildEventContext;
use blaze_bep::buildeventstream::{
    BuildEvent, BuildEventClass, BuildToolLogs, Interrupted, LocalFile, LocalFileType, LogFileEntry,
};
use blaze_bep::events::analysis_events::{
    BuildInfoEvent, ConfiguredTargetInfo, PackagesTestSize, TargetConfiguredEvent,
};
use blaze_bep::events::buildtool_events::{
    BuildResultData, BuildStartingData, BuildStartingEvent, ConvenienceSymlinksIdentifiedEvent,
    build_complete_event, testing_complete_event,
};
use blaze_bep::events::completion_context::{CompletionContext, ImportantInputMap};
use blaze_bep::events::pkgcache_events::{TargetParsingCompleteEvent, ThinTarget};
use blaze_bep::events::target_complete_event::{
    ArtifactsInOutputGroup, CompletedTargetData, TargetCompleteEvent,
};
use blaze_bep::metrics::metrics_collector::BuildMetricsEvent;
use blaze_bep::runtime::command_line_event::OriginalUnstructuredCommandLineEvent;
use blaze_bep::runtime::runtime_events::{BuildMetadataEvent, ExecRootEvent};
use blaze_bep::runtime::test_summary_event::TestSummaryEvent;
use blaze_cmdline::label::Label;
use blaze_collect::nestedset::{NestedSet, Order};
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_options::Options;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::Id;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_protos::json::JsonFormat;
use blaze_protos::tools::protos::SpawnExec;
use blaze_util::{DetailedExitCode, ExitCode};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

fn record_dir(project: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let d = std::path::PathBuf::from(home)
        .join(".cache/blaze/oracle-records/oracle-9.3.2")
        .join(project);
    d.join("raw/bep.json").is_file().then_some(d)
}

fn load_oracle(project: &str) -> Option<(Vec<bes::BuildEvent>, Vec<String>, serde_json::Value)> {
    let d = record_dir(project)?;
    let text = std::fs::read_to_string(d.join("raw/bep.json")).unwrap();
    let parser = JsonFormat::parser().using_type_registry(
        blaze_protos::json::TypeRegistry::new_builder()
            .add::<SpawnExec>()
            .build(),
    );
    let mut events = Vec::new();
    let mut lines = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut e = bes::BuildEvent::default();
        parser.merge(line, &mut e).unwrap();
        events.push(e);
        lines.push(line.to_string());
    }
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("manifest.json")).unwrap()).unwrap();
    Some((events, lines, manifest))
}

fn out_dir() -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bep-oracle");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn is_progress(id: &BuildEventId) -> bool {
    matches!(id.id, Some(Id::Progress(_)))
}

/// A recorded event, posted again: its id, its children, its payload.
#[derive(Debug)]
struct RecordedEvent {
    proto: bes::BuildEvent,
    posted_after: Option<Vec<BuildEventId>>,
}

impl RecordedEvent {
    fn new(proto: bes::BuildEvent) -> Self {
        RecordedEvent {
            proto,
            posted_after: None,
        }
    }
}

impl BuildEvent for RecordedEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.proto.id.clone().unwrap()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        // The recorded children, `started`'s initial progress id included.
        self.proto.children.clone()
    }
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut p = self.proto.clone();
        p.last_message = false;
        p.children = self.get_children_events();
        Ok(p)
    }
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        self.posted_after.clone()
    }
    fn event_class(&self) -> BuildEventClass {
        match (&self.proto.id.as_ref().unwrap().id, &self.proto.payload) {
            (Some(Id::Started(_)), Some(Payload::Started(s))) => BuildEventClass::BuildStarting {
                command_name: s.command.clone(),
            },
            (Some(Id::BuildFinished(_)), _) => BuildEventClass::BuildCompleting {
                build_complete: None,
            },
            (Some(Id::WorkspaceStatus(_)), _) => BuildEventClass::BuildInfo,
            (Some(Id::Configuration(_)), _) => BuildEventClass::BuildConfiguration,
            (Some(Id::Pattern(p)), _) => BuildEventClass::TargetParsingComplete {
                original_target_pattern_count: p.pattern.len(),
                failed_target_patterns_empty: true,
            },
            (Some(Id::TestSummary(_)), Some(Payload::TestSummary(s))) => {
                BuildEventClass::TestSummary {
                    total_runs: s.total_run_count,
                }
            }
            _ => BuildEventClass::Other,
        }
    }
}

fn defaults() -> (BuildEventProtocolOptions, BuildEventStreamOptions) {
    (
        Options::<BuildEventProtocolOptions>::get_defaults(),
        Options::<BuildEventStreamOptions>::get_defaults(),
    )
}

/// Each event as JSON, the progress events without their stdout/stderr.
fn skeleton(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .map(|l| {
            let mut v: serde_json::Value = serde_json::from_str(l).unwrap();
            if let Some(p) = v.get_mut("progress") {
                *p = serde_json::json!({});
            }
            v
        })
        .collect()
}

fn replay(project: &str) {
    let Some((events, lines, _)) = load_oracle(project) else {
        eprintln!("oracle recording for {project} absent; skipped");
        return;
    };
    // BuildTool.stopRequest posts BuildToolLogs right after BuildCompleteEvent; under `test`
    // that event is discarded, so the logs wait (postedAfter buildFinished) for
    // TestingCompleteEvent and are released before the final progress event. An event the
    // oracle wrote directly after its prerequisite (no progress event between) was posted
    // before it.
    let mut posted: Vec<Arc<dyn BuildEvent>> = Vec::new();
    let mut last_non_progress: Option<BuildEventId> = None;
    let mut previous_was_progress = false;
    for e in events {
        let id = e.id.clone().unwrap();
        if is_progress(&id) {
            previous_was_progress = true;
            continue;
        }
        let mut event = RecordedEvent::new(e);
        if matches!(
            id.id,
            Some(Id::BuildToolLogs(_)) | Some(Id::BuildMetrics(_))
        ) {
            let finished = blaze_bep::buildeventstream::build_event_id_util::build_finished();
            event.posted_after = Some(vec![finished.clone()]);
            if !previous_was_progress && last_non_progress.as_ref() == Some(&finished) {
                let at = posted.len() - 1;
                posted.insert(at, Arc::new(event));
                previous_was_progress = false;
                continue;
            }
        }
        posted.push(Arc::new(event));
        last_non_progress = Some(id);
        previous_was_progress = false;
    }
    let (bep, bes_options) = defaults();
    let bytes = events_to_json_bytes(posted, &bep, &bes_options);
    let path = out_dir().join(format!("{project}.replay.json"));
    std::fs::write(&path, &bytes).unwrap();
    let ours: Vec<String> = String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let (a, b) = (skeleton(&lines), skeleton(&ours));
    let same = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    // Concurrent completions in Bazel can write two named sets in the opposite order of the
    // progress events that announce them; compare those links without the set's name.
    let unordered = |v: &[serde_json::Value]| {
        let mut keys: Vec<String> = v
            .iter()
            .map(|x| {
                let mut x = x.clone();
                if x.get("progress").is_some()
                    && let Some(children) = x.get_mut("children").and_then(|c| c.as_array_mut())
                {
                    for c in children {
                        if let Some(n) = c.get_mut("namedSet") {
                            *n = serde_json::json!({});
                        }
                    }
                }
                x.to_string()
            })
            .collect();
        keys.sort();
        keys
    };
    for (x, y) in a.iter().zip(b.iter()).filter(|(x, y)| x != y) {
        eprintln!(
            "oracle: {:.300}\nblaze:  {:.300}",
            x.to_string(),
            y.to_string()
        );
    }
    eprintln!(
        "{project} replay: {same}/{} lines identical in order (progress text ignored), {} written to {}",
        a.len(),
        b.len(),
        path.display()
    );
    assert_eq!(a.len(), b.len());
    assert_eq!(unordered(&a), unordered(&b));
}

#[test]
fn replay_brotli() {
    replay("brotli");
}

#[test]
fn replay_re2() {
    replay("re2");
}

// ---------------------------------------------------------------------------------------------
// brotli from first principles
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct TargetKey(Label);

impl fmt::Display for TargetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ActionLookupKey for TargetKey {
    fn get_label(&self) -> Option<Label> {
        Some(self.0.clone())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<TargetKey>()
            .is_some_and(|o| o.0 == self.0)
    }
    fn key_hash(&self) -> u64 {
        0
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn file_system() -> FileSystemRef {
    Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256))
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn proto_of(events: &[bes::BuildEvent], pred: impl Fn(&Id) -> bool) -> Vec<&bes::BuildEvent> {
    events
        .iter()
        .filter(|e| pred(e.id.as_ref().unwrap().id.as_ref().unwrap()))
        .collect()
}

fn label_of(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// Builds the project's stream from first principles: the event classes the runtime will post,
/// at the points Bazel posts them, fed with the facts the oracle recorded.
fn reconstruct(project: &str) {
    let Some((events, _, manifest)) = load_oracle(project) else {
        eprintln!("oracle recording for {project} absent; skipped");
        return;
    };
    let paths = &manifest["context"]["paths"];
    let fs = file_system();
    let workspace = Path::create_from_str(paths["workspace"].as_str().unwrap(), &fs);
    let exec_root = Path::create_from_str(paths["execroot"].as_str().unwrap(), &fs);
    let output_base = Path::create_from_str(paths["output_base"].as_str().unwrap(), &fs);
    let started_id = blaze_bep::buildeventstream::build_event_id_util::build_started_id();

    let one = |pred: &dyn Fn(&Id) -> bool| -> &bes::BuildEvent { proto_of(&events, pred)[0] };
    let Some(Payload::Started(s)) = &one(&|i| matches!(i, Id::Started(_))).payload else {
        panic!()
    };
    let command = s.command.clone();
    let Some(Id::Pattern(pattern_id)) = &one(&|i| matches!(i, Id::Pattern(_)))
        .id
        .as_ref()
        .unwrap()
        .id
    else {
        panic!()
    };
    let patterns = pattern_id.pattern.clone();

    // Configurations: BuildConfigurationValue.toBuildEvent() of each configuration.
    let mut config_events: IndexMap<String, Arc<dyn BuildEvent>> = IndexMap::new();
    for e in proto_of(&events, |i| matches!(i, Id::Configuration(_))) {
        let (Some(Id::Configuration(c)), Some(Payload::Configuration(p))) =
            (&e.id.as_ref().unwrap().id, &e.payload)
        else {
            panic!()
        };
        config_events.insert(
            c.id.clone(),
            Arc::new(BuildConfigurationEvent {
                event_id: c.id.clone(),
                configuration: p.clone(),
            }),
        );
    }
    // The configuration of each top-level target (from its completion id).
    let mut target_config: IndexMap<String, String> = IndexMap::new();
    for e in proto_of(&events, |i| matches!(i, Id::TargetCompleted(_))) {
        let Some(Id::TargetCompleted(tc)) = &e.id.as_ref().unwrap().id else {
            panic!()
        };
        target_config.insert(tc.label.clone(), tc.configuration.clone().unwrap().id);
    }
    let named: IndexMap<String, bes::NamedSetOfFiles> =
        proto_of(&events, |i| matches!(i, Id::NamedSet(_)))
            .into_iter()
            .map(|e| match (&e.id.as_ref().unwrap().id, &e.payload) {
                (Some(Id::NamedSet(n)), Some(Payload::NamedSetOfFiles(f))) => {
                    (n.id.clone(), f.clone())
                }
                _ => panic!(),
            })
            .collect();
    let configured_kind = |label: &str| -> Option<(String, i32)> {
        proto_of(
            &events,
            |i| matches!(i, Id::TargetConfigured(t) if t.label == label),
        )
        .first()
        .and_then(|e| match &e.payload {
            Some(Payload::Configured(c)) => Some((c.target_kind.clone(), c.test_size)),
            _ => None,
        })
    };
    let source_root = ArtifactRoot::as_source_root(Root::from_path(workspace.clone()));
    let bin_root = ArtifactRoot::as_derived_root(
        &exec_root,
        RootType::OUTPUT,
        &["bazel-out", "k8-fastbuild", "bin"],
    );

    let mut posted: Vec<Arc<dyn BuildEvent>> = Vec::new();
    let mut first_configuration_posted = false;
    let mut shared_sets: IndexMap<Vec<String>, NestedSet<Artifact>> = IndexMap::new();
    let mut artifacts: IndexMap<(Vec<String>, String), Artifact> = IndexMap::new();
    for e in &events {
        let id = e.id.as_ref().unwrap().id.as_ref().unwrap();
        match (id, &e.payload) {
            // BuildTool.startRequest.
            (Id::Started(_), Some(Payload::Started(s))) => {
                #[allow(deprecated)]
                let data = BuildStartingData {
                    uuid: s.uuid.clone(),
                    start_time_millis: s.start_time_millis,
                    build_tool_version: "9.3.2".to_string(),
                    options_description: s.options_description.clone(),
                    command_name: command.clone(),
                    server_pid: s.server_pid,
                    pwd: workspace.get_path_string().to_string(),
                    workspace: Some(workspace.get_path_string().to_string()),
                    host: s.host.clone(),
                    user: s.user.clone(),
                    targets: patterns.clone(),
                };
                posted.push(Arc::new(BuildStartingEvent::create(data)));
            }
            (Id::BuildMetadata(_), _) => {
                posted.push(Arc::new(BuildMetadataEvent::new(IndexMap::new())))
            }
            (Id::UnstructuredCommandLine(_), Some(Payload::UnstructuredCommandLine(u))) => {
                posted.push(Arc::new(OriginalUnstructuredCommandLineEvent::new(
                    u.args.clone(),
                )));
            }
            // PORT: the parsed options (optionsParsed, structured command lines) need the
            // command's option classes and the client's startup options from the runtime;
            // replayed here (CommandLineEventTest covers the conversion).
            (Id::OptionsParsed(_) | Id::StructuredCommandLine(_), _) => {
                posted.push(Arc::new(RecordedEvent {
                    proto: e.clone(),
                    posted_after: Some(vec![started_id.clone()]),
                }))
            }
            // TargetPatternPhaseFunction: the expansion (test suites included, as targets).
            (Id::Pattern(_), Some(Payload::Expanded(x))) => {
                let mut expanded: Vec<ThinTarget> = Vec::new();
                for c in &e.children {
                    if let Some(Id::TargetConfigured(t)) = &c.id {
                        let (kind, _) = configured_kind(&t.label).unwrap();
                        let rule_class = kind.strip_suffix(" rule").map(str::to_string);
                        expanded.push(ThinTarget::new(label_of(&t.label), kind, rule_class));
                    }
                }
                let mut suites = Vec::new();
                for x in &x.test_suite_expansions {
                    expanded.push(ThinTarget::new(
                        label_of(&x.suite_label),
                        "test_suite rule".to_string(),
                        Some("test_suite".to_string()),
                    ));
                    suites.push((
                        label_of(&x.suite_label),
                        x.test_labels.iter().map(|l| label_of(l)).collect(),
                    ));
                }
                posted.push(Arc::new(TargetParsingCompleteEvent::new(
                    expanded.clone(),
                    Vec::new(),
                    Vec::new(),
                    patterns.clone(),
                    expanded,
                    Vec::new(),
                    Vec::new(),
                    suites,
                )));
            }
            (Id::Workspace(_), _) => posted.push(Arc::new(ExecRootEvent::new(exec_root.clone()))),
            // The first configuration is posted on its own; the others come with the events
            // that mention them (getConfigurations).
            (Id::Configuration(c), _) => {
                if !first_configuration_posted {
                    first_configuration_posted = true;
                    posted.push(Arc::new(BuildEventPost(config_events[&c.id].clone())));
                }
            }
            (Id::WorkspaceStatus(_), Some(Payload::WorkspaceStatus(ws))) => {
                posted.push(Arc::new(BuildInfoEvent::new(
                    ws.item
                        .iter()
                        .map(|i| (i.key.clone(), i.value.clone()))
                        .collect(),
                )))
            }
            // AnalysisPhaseRunner: per top-level target.
            (Id::TargetConfigured(t), Some(Payload::Configured(c))) => {
                let checksum = target_config[&t.label].clone();
                let is_test = c.target_kind.ends_with("_test rule");
                let size = match c.test_size {
                    1 => Some(PackagesTestSize::Small),
                    2 => Some(PackagesTestSize::Medium),
                    3 => Some(PackagesTestSize::Large),
                    4 => Some(PackagesTestSize::Enormous),
                    _ => None,
                };
                posted.push(Arc::new(TargetConfiguredEvent::new(
                    ConfiguredTargetInfo {
                        label: label_of(&t.label),
                        target_kind: c.target_kind.clone(),
                        // The rule's `tags` attribute (every rule has one).
                        tags: Some(c.tag.clone()),
                        test_size: is_test.then_some(size),
                    },
                    Some(config_events[&checksum].clone()),
                    Some(checksum),
                )));
            }
            (Id::NamedSet(_) | Id::Progress(_) | Id::BuildToolLogs(_), _) => {}
            // CompletionFunction: the target's important output groups, with their metadata.
            (Id::TargetCompleted(tc), Some(Payload::Completed(c))) => {
                let label = label_of(&tc.label);
                let checksum = tc.configuration.clone().unwrap().id;
                let owner = ActionLookupKeyRef::new(TargetKey(label.clone()));
                let mut inputs = ImportantInputMap::new();
                let mut outputs: IndexMap<String, ArtifactsInOutputGroup> = IndexMap::new();
                for group in &c.output_group {
                    // One NestedSet per set of files: targets that forward another target's
                    // outputs (py_extension's `_re2` and `_re2.so`) share the instance.
                    let set_key: Vec<String> =
                        group.file_sets.iter().map(|f| f.id.clone()).collect();
                    let mut builder = NestedSet::builder(Order::STABLE_ORDER);
                    for fs in &group.file_sets {
                        for f in &named[&fs.id].files {
                            let artifact = artifacts
                                .entry((f.path_prefix.clone(), f.name.clone()))
                                .or_insert_with(|| {
                                    if f.path_prefix.is_empty() {
                                        Artifact::new_source_artifact(
                                            source_root.clone(),
                                            PathFragment::create(&f.name),
                                            ArtifactOwner::NullOwner,
                                        )
                                    } else {
                                        let exec_path = bin_root
                                            .get_exec_path()
                                            .get_relative(&PathFragment::create(&f.name));
                                        Artifact::create_derived_artifact(
                                            bin_root.clone(),
                                            exec_path,
                                            owner.clone(),
                                        )
                                    }
                                })
                                .clone();
                            inputs.put(
                                artifact.clone(),
                                FileArtifactValue::RegularFile {
                                    digest: Some(unhex(&f.digest)),
                                    proxy: None,
                                    size: f.length,
                                },
                            );
                            builder.add(artifact);
                        }
                    }
                    let set = shared_sets
                        .entry(set_key)
                        .or_insert_with(|| builder.build())
                        .clone();
                    outputs.insert(
                        group.name.clone(),
                        ArtifactsInOutputGroup::new(set, true, false),
                    );
                }
                #[allow(deprecated)]
                let data = CompletedTargetData {
                    label: label.clone(),
                    original_label: label,
                    configuration_checksum: Some(checksum.clone()),
                    configuration_event: Some(config_events[&checksum].clone()),
                    rule_tags: c.tag.clone(),
                    // TestParams defaults: one run, no sharding.
                    test_runs: 1,
                    test_shards: 0,
                    test_timeout_seconds: c.test_timeout.as_ref().map(|_| c.test_timeout_seconds),
                };
                let ctx = Arc::new(CompletionContext::new(inputs, false));
                posted.push(Arc::new(if c.test_timeout.is_some() {
                    TargetCompleteEvent::successful_build_scheduling_test(data, ctx, outputs, false)
                } else {
                    TargetCompleteEvent::successful_build(data, ctx, outputs, false)
                }));
            }
            // PORT: TestAttempt needs the TestRunnerAction of an executed test; replayed with its
            // order constraint.
            (Id::TestResult(r), _) => posted.push(Arc::new(RecordedEvent {
                proto: e.clone(),
                posted_after: Some(vec![
                    blaze_bep::buildeventstream::build_event_id_util::target_completed(
                        &label_of(&r.label),
                        &blaze_bep::buildeventstream::build_event_id_util::configuration_id(
                            &r.configuration.as_ref().unwrap().id,
                        ),
                    ),
                ]),
            })),
            // AggregatingTestListener: the test target's summary.
            (Id::TestSummary(t), Some(Payload::TestSummary(ts))) => {
                #[allow(deprecated)]
                let summary = TestSummaryEvent {
                    label: label_of(&t.label),
                    configuration_checksum: t.configuration.as_ref().unwrap().id.clone(),
                    status: match ts.overall_status {
                        1 => BlazeTestStatus::Passed,
                        _ => unimplemented!("test status {}", ts.overall_status),
                    },
                    num_cached: ts.total_num_cached,
                    total_runs: ts.total_run_count,
                    num_attempts: ts.attempt_count,
                    runs: ts.run_count,
                    shards: ts.shard_count,
                    first_start_time_millis: ts.first_start_time_millis,
                    last_stop_time_millis: ts.last_stop_time_millis,
                    total_run_duration_millis: ts.total_run_duration_millis,
                    passed_logs: ts
                        .passed
                        .iter()
                        .map(|f| match &f.file {
                            Some(bes::file::File::Uri(u)) => {
                                Path::create_from_str(u.strip_prefix("file://").unwrap(), &fs)
                            }
                            _ => panic!(),
                        })
                        .collect(),
                    failed_logs: Vec::new(),
                };
                posted.push(Arc::new(summary));
            }
            (
                Id::ConvenienceSymlinksIdentified(_),
                Some(Payload::ConvenienceSymlinksIdentified(cs)),
            ) => {
                posted.push(Arc::new(ConvenienceSymlinksIdentifiedEvent::new(
                    cs.convenience_symlinks.clone(),
                )));
            }
            // BuildTool.stopRequest: BuildCompleteEvent, then the tool logs; for `test`,
            // TestCommand then posts TestingCompleteEvent.
            (Id::BuildFinished(_), Some(Payload::Finished(f))) => {
                #[allow(deprecated)]
                let finish_time_millis = f.finish_time_millis;
                posted.push(Arc::new(build_complete_event(
                    &BuildResultData {
                        detailed_exit_code: DetailedExitCode::success(),
                        stop_time_millis: finish_time_millis,
                        has_unhandled_throwable: false,
                        was_catastrophe: false,
                        success: true,
                        stop_on_first_failure: true,
                    },
                    vec![
                        blaze_bep::buildeventstream::build_event_id_util::build_tool_logs(),
                        blaze_bep::buildeventstream::build_event_id_util::build_metrics(),
                    ],
                )));
                let Some(Payload::BuildToolLogs(logs)) =
                    &one(&|i| matches!(i, Id::BuildToolLogs(_))).payload
                else {
                    panic!()
                };
                let direct: Vec<(String, Vec<u8>)> = logs
                    .log
                    .iter()
                    .filter_map(|f| match &f.file {
                        Some(bes::file::File::Contents(c)) => Some((f.name.clone(), c.clone())),
                        _ => None,
                    })
                    .collect();
                let profile = output_base.get_relative(&PathFragment::create(&format!(
                    "command-{}.profile.gz",
                    s.uuid
                )));
                posted.push(Arc::new(BuildToolLogs::new(
                    direct,
                    Vec::new(),
                    vec![LogFileEntry {
                        name: "command.profile.gz".to_string(),
                        local_file: LocalFile::new(profile, LocalFileType::Log, None),
                    }],
                )));
                if command == "test" {
                    posted.push(Arc::new(testing_complete_event(
                        ExitCode::SUCCESS,
                        finish_time_millis,
                    )));
                }
            }
            // PORT: the metrics' inputs are Skyframe/executor counters the build reports to
            // MetricsCollector (metrics_collector_test checks the collector against the brotli
            // numbers); here the event carries the oracle's BuildMetrics.
            (Id::BuildMetrics(_), Some(Payload::BuildMetrics(m))) => {
                posted.push(Arc::new(BuildMetricsEvent::new(m.clone())))
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    let (bep, bes_options) = defaults();
    let bytes = events_to_json_bytes(posted, &bep, &bes_options);
    let path = out_dir().join(format!("{project}.reconstructed.json"));
    std::fs::write(&path, &bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    eprintln!(
        "{project} reconstruction: {} events written to {} (score: scripts/bep-compare --project {project} --candidate <file>)",
        lines.len(),
        path.display()
    );
    assert_eq!(lines.len(), events.len());
    assert!(lines.last().unwrap().contains("\"lastMessage\":true"));
    assert!(
        lines
            .last()
            .unwrap()
            .starts_with("{\"id\":{\"buildMetrics\"")
    );
}

#[test]
fn reconstruct_brotli() {
    reconstruct("brotli");
}

#[test]
fn reconstruct_re2() {
    reconstruct("re2");
}
