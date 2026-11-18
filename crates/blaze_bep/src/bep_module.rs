//! The runtime's entry point: builds the `BuildEventStreamer` and its file transports from the
//! command's options (the BEP part of `BuildEventServiceModule.beforeCommand`), and dispatches
//! the posts of the command's event bus to it.
//!
//! # How the runtime uses it
//!
//! 1. Before the command runs (Java: `BuildEventServiceModule.beforeCommand`), call
//!    [`BuildEventStreamerBuilder::from_options`]. `None` means no `--build_event_*_file` was
//!    given; then nothing else is needed.
//! 2. Register the returned [`BepStreamer`] on the command's event bus: every
//!    `Reporter::post(Arc<dyn Postable>)` must reach [`BepStreamer::on_post`] (it is also a
//!    [`blaze_events::EventBus`]). Posts it does not know are ignored, as Guava's `EventBus`
//!    ignores events a subscriber does not subscribe to.
//! 3. Post the events, in the order and at the points Bazel posts them (see the table below).
//!    Types defined in other crates are recognised after
//!    [`register_build_event_type`]`::<T>()` (`blaze_exec`'s `TestAttempt` is pre-registered).
//! 4. At the end of the build (Java: `BuildTool.stopRequest`) post `build_complete_event(..)`
//!    (children: buildToolLogs, buildMetrics) and right after it the
//!    [`crate::buildeventstream::BuildToolLogs`]; for `test`, `TestCommand` then posts
//!    `testing_complete_event(..)` (the `BuildCompleteEvent` is then discarded and the logs wait
//!    for it). At `CommandPrecomplete`, post the `BuildMetricsEvent` from
//!    [`crate::metrics::metrics_collector::MetricsCollector`]. The streamer closes itself once
//!    the last announced event is posted. Call [`BepStreamer::close`] (or `close_on_abort`) at
//!    the very end in any case: a no-op if already closed; it aborts the missing events and
//!    flushes the file.
//!
//! | event (Rust type) | Java poster | when |
//! |---|---|---|
//! | `BuildStartingEvent` | `BuildTool.startRequest` | before loading |
//! | `UnstructuredCommandLineEvent`, `OriginalCommandLineEvent`, `CanonicalCommandLineEvent`, `ToolCommandLineEvent`, `GotOptionsEvent` (optionsParsed), `BuildMetadataEvent` | `BlazeCommandDispatcher` / `BlazeRuntime.beforeCommand` | after options parsing (they wait for `started`) |
//! | `ExecRootEvent` (workspace) | `BlazeRuntime` (`CommandEnvironment.syncPackageLoading`) | once the exec root is known |
//! | `TargetParsingCompleteEvent` (pattern) | `TargetPatternPhaseFunction` via `BuildTool` | after target pattern expansion |
//! | `BuildConfigurationEvent`s | via `getConfigurations()` of the events below | automatic |
//! | `BuildInfoEvent` (workspaceStatus) | `WorkspaceStatusAction` (`BazelWorkspaceStatusModule`) | when the status action ran |
//! | `TargetConfiguredEvent` | `BuildView` / `AnalysisPhaseRunner` | per top-level target, after it is configured |
//! | `TargetCompleteEvent` | `TargetCompletor` / `CompletionFunction` | per top-level target, when its outputs are built |
//! | `TestAttempt`, `TestSummaryEvent` | `StandaloneTestStrategy` / `AggregatingTestListener` | per test attempt / per test target |
//! | `ActionExecutedEvent` | `SkyframeActionExecutor` | for failed actions (others are discarded) |
//! | `ConvenienceSymlinksIdentifiedEvent` | `BuildTool` (`OutputDirectoryLinksUtils`) | after execution |
//! | `build_complete_event(..)`, then `BuildToolLogs` | `BuildTool.stopRequest` | end of the build |
//! | `testing_complete_event(..)` (`test` only) | `TestCommand` | after the tests |
//! | `BuildMetricsEvent` | `MetricsCollector.onCommandPrecomplete` | before the command completes |

use std::any::Any;
use std::io::Write;
use std::sync::{Arc, OnceLock};

use blaze_events::{EventBus, Postable};
use blaze_exec::analysis::test::test_attempt::TestAttempt;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_protos::build_event_stream::aborted::AbortReason;
use parking_lot::{Mutex, RwLock};

use crate::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, LocalFilesArtifactUploader,
};
use crate::buildeventstream::build_completing_event::{
    BuildCompletingEvent, BuildToolLogs, FetchEvent,
};
use crate::buildeventstream::build_event::BuildEvent;
use crate::buildeventstream::generic_build_event::{
    AbortedEvent, GenericBuildEvent, ProgressEvent,
};
use crate::buildeventstream::transport::BuildEventTransport;
use crate::buildeventstream::transports::file_transport::{
    BinaryFormatFileTransport, JsonFormatFileTransport,
};
use crate::events::actions_events::ActionExecutedEvent;
use crate::events::analysis_events::{BuildInfoEvent, NoBuildEvent, TargetConfiguredEvent};
use crate::events::buildtool_events::{
    BuildInterruptedEvent, BuildStartingEvent, ConvenienceSymlinksIdentifiedEvent, NoAnalyzeEvent,
    NoExecutionEvent,
};
use crate::events::pkgcache_events::TargetParsingCompleteEvent;
use crate::events::target_complete_event::TargetCompleteEvent;
use crate::metrics::metrics_collector::BuildMetricsEvent;
use crate::runtime::build_event_streamer::{
    BuildEventStreamer, Builder, OutErrProvider, ReleaseReplaceableBuildEvent,
};
use crate::runtime::command_line_event::{
    CanonicalCommandLineEvent, OriginalCommandLineEvent, OriginalUnstructuredCommandLineEvent,
    ToolCommandLineEvent,
};
use crate::runtime::counting_artifact_group_namer::CountingArtifactGroupNamer;
use crate::runtime::got_options_event::GotOptionsEvent;
use crate::runtime::runtime_events::{BuildMetadataEvent, ExecRootEvent};
use crate::runtime::test_summary_event::{TargetSummaryEvent, TestSummaryEvent};

type AnyArc = Arc<dyn Any + Send + Sync>;
type Downcaster = fn(AnyArc) -> Result<Arc<dyn BuildEvent>, AnyArc>;

fn downcast<T: BuildEvent + Any + Send + Sync>(a: AnyArc) -> Result<Arc<dyn BuildEvent>, AnyArc> {
    a.downcast::<T>().map(|t| t as Arc<dyn BuildEvent>)
}

fn registry() -> &'static RwLock<Vec<Downcaster>> {
    static REGISTRY: OnceLock<RwLock<Vec<Downcaster>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        RwLock::new(vec![
            downcast::<BuildStartingEvent>,
            downcast::<BuildCompletingEvent>,
            downcast::<NoBuildEvent>,
            downcast::<BuildMetadataEvent>,
            downcast::<ExecRootEvent>,
            downcast::<TargetParsingCompleteEvent>,
            downcast::<BuildInfoEvent>,
            downcast::<TargetConfiguredEvent>,
            downcast::<TargetCompleteEvent>,
            downcast::<TestAttempt>,
            downcast::<ActionExecutedEvent>,
            downcast::<ConvenienceSymlinksIdentifiedEvent>,
            downcast::<BuildToolLogs>,
            downcast::<FetchEvent>,
            downcast::<GenericBuildEvent>,
            downcast::<ProgressEvent>,
            downcast::<AbortedEvent>,
            downcast::<OriginalUnstructuredCommandLineEvent>,
            downcast::<OriginalCommandLineEvent>,
            downcast::<CanonicalCommandLineEvent>,
            downcast::<ToolCommandLineEvent>,
            downcast::<GotOptionsEvent>,
            downcast::<TestSummaryEvent>,
            downcast::<TargetSummaryEvent>,
            downcast::<BuildMetricsEvent>,
        ])
    })
}

/// Makes posts of `T` reach the streamer's `buildEvent` (Java: `T implements BuildEvent`, which
/// Guava's `EventBus` routes to `@Subscribe buildEvent(BuildEvent)`).
pub fn register_build_event_type<T: BuildEvent + Postable>() {
    let mut r = registry().write();
    let f: Downcaster = downcast::<T>;
    if !r.iter().any(|g| std::ptr::fn_addr_eq(*g, f)) {
        r.push(f);
    }
}

/// The build event a post carries, if it is one.
pub fn as_build_event(post: Arc<dyn Postable>) -> Option<Arc<dyn BuildEvent>> {
    let mut any: AnyArc = post;
    if let Some(wrapper) = any.downcast_ref::<BuildEventPost>() {
        return Some(wrapper.0.clone());
    }
    for f in registry().read().iter() {
        match f(any) {
            Ok(event) => return Some(event),
            Err(back) => any = back,
        }
    }
    None
}

/// A post carrying any build event, for events that are not `Postable` themselves (for example
/// `blaze_actions::BuildConfigurationEvent`).
#[derive(Debug, Clone)]
pub struct BuildEventPost(pub Arc<dyn BuildEvent>);

impl Postable for BuildEventPost {}

impl BuildEvent for BuildEventPost {
    fn get_event_id(&self) -> blaze_protos::build_event_stream::BuildEventId {
        self.0.get_event_id()
    }
    fn get_children_events(&self) -> Vec<blaze_protos::build_event_stream::BuildEventId> {
        self.0.get_children_events()
    }
    fn as_stream_proto(
        &self,
        context: &dyn crate::buildeventstream::build_event_context::BuildEventContext,
    ) -> Result<blaze_protos::build_event_stream::BuildEvent, crate::buildeventstream::Interrupted>
    {
        self.0.as_stream_proto(context)
    }
    fn referenced_local_files(&self) -> Vec<crate::buildeventstream::LocalFile> {
        self.0.referenced_local_files()
    }
    fn remote_uploads(&self) -> Vec<Option<String>> {
        self.0.remote_uploads()
    }
    fn posted_after(&self) -> Option<Vec<blaze_protos::build_event_stream::BuildEventId>> {
        self.0.posted_after()
    }
    fn get_configurations(&self) -> Option<Vec<Arc<dyn BuildEvent>>> {
        self.0.get_configurations()
    }
    fn reported_artifacts(
        &self,
        modes: &blaze_optdefs::misc::build_event_protocol_options::OutputGroupFileModes,
    ) -> Option<crate::buildeventstream::ReportedArtifacts> {
        self.0.reported_artifacts(modes)
    }
    fn replaceable(&self) -> Option<bool> {
        self.0.replaceable()
    }
    fn event_class(&self) -> crate::buildeventstream::BuildEventClass {
        self.0.event_class()
    }
}

/// Where the BEP files go and what the command environment contributes.
#[derive(Clone, Debug, Default)]
pub struct BepCommandContext {
    /// `DestinationRelativeTo.WORKSPACE_OR_HOME`: relative `--build_event_*_file` paths are
    /// resolved against the workspace (or the home directory outside a workspace).
    pub workspace_or_home: std::path::PathBuf,
    /// `CommonCommandOptions.oomMessage`.
    pub oom_message: String,
}

/// Builds the streamer of a command (the BEP part of `BuildEventServiceModule.beforeCommand`).
pub struct BuildEventStreamerBuilder;

impl BuildEventStreamerBuilder {
    /// Creates the file transports (`createBepTransports`) and the streamer. Returns `Ok(None)`
    /// when no file transport is requested; `Err` with Bazel's message when a file cannot be
    /// opened (`Unable to write to '<path>'. Omitting --build_event_json_file.`), which Bazel
    /// reports as an error (`BES_LOCAL_WRITE_ERROR`) while the command continues without it.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:825 BuildEventServiceModule.createBepTransports
    pub fn from_options(
        bep_options: &BuildEventProtocolOptions,
        bes_stream_options: &BuildEventStreamOptions,
        ctx: &BepCommandContext,
    ) -> (Option<BepStreamer>, Vec<String>) {
        let bep_options = Arc::new(bep_options.clone());
        let artifact_group_namer = Arc::new(CountingArtifactGroupNamer::new());
        let mut transports: Vec<Arc<dyn BuildEventTransport>> = Vec::new();
        let mut errors = Vec::new();

        if !bes_stream_options.build_event_text_file.is_empty() {
            blaze_util::unported!("TextFormatFileTransport (protobuf TextFormat is not ported)");
        }
        if !bes_stream_options.build_event_binary_file.is_empty() {
            match open_output(
                &ctx.workspace_or_home,
                &bes_stream_options.build_event_binary_file,
            ) {
                Ok(out) => transports.push(Arc::new(BinaryFormatFileTransport::new(
                    out,
                    bep_options.clone(),
                    local_file_uploader(bes_stream_options.build_event_binary_file_path_conversion),
                    artifact_group_namer.clone(),
                    bes_stream_options.build_event_binary_file_upload_mode,
                ))),
                Err(_) => errors.push(format!(
                    "Unable to write to '{}'. Omitting --build_event_binary_file.",
                    bes_stream_options.build_event_binary_file
                )),
            }
        }
        if !bes_stream_options.build_event_json_file.is_empty() {
            match open_output(
                &ctx.workspace_or_home,
                &bes_stream_options.build_event_json_file,
            ) {
                Ok(out) => transports.push(Arc::new(JsonFormatFileTransport::new(
                    out,
                    bep_options.clone(),
                    local_file_uploader(bes_stream_options.build_event_json_file_path_conversion),
                    artifact_group_namer.clone(),
                    JsonFormatFileTransport::bep_type_registry(),
                    bes_stream_options.build_event_json_file_upload_mode,
                ))),
                Err(_) => errors.push(format!(
                    "Unable to write to '{}'. Omitting --build_event_json_file.",
                    bes_stream_options.build_event_json_file
                )),
            }
        }
        if transports.is_empty() {
            return (None, errors);
        }
        let streamer = Builder::new()
            .build_event_transports(transports)
            .bes_stream_options(bes_stream_options.clone())
            .output_group_file_modes(bep_options.get_output_group_file_modes_mapping())
            .publish_target_summaries(bep_options.publish_target_summary)
            .artifact_group_namer(artifact_group_namer)
            .oom_message(ctx.oom_message.clone())
            .build();
        (
            Some(BepStreamer {
                streamer: Arc::new(streamer),
            }),
            errors,
        )
    }
}

/// `pathConversion` selects the configured uploader; Blaze's only uploader is the local one
/// (`--experimental_build_event_upload_strategy=local`).
fn local_file_uploader(_path_conversion: bool) -> Arc<dyn BuildEventArtifactUploader> {
    Arc::new(LocalFilesArtifactUploader)
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:1029 BuildEventOutputStreamFactoryImpl.create
fn open_output(base: &std::path::Path, file_path: &str) -> std::io::Result<Box<dyn Write + Send>> {
    let path = base.join(file_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let f = std::fs::File::create(&path)?;
    Ok(Box::new(std::io::BufWriter::new(f)))
}

/// The streamer as an event-bus subscriber.
#[derive(Clone)]
pub struct BepStreamer {
    streamer: Arc<BuildEventStreamer>,
}

impl BepStreamer {
    pub fn streamer(&self) -> &Arc<BuildEventStreamer> {
        &self.streamer
    }

    /// Dispatches one post like Guava's `EventBus` would to the streamer's `@Subscribe`
    /// methods.
    pub fn on_post(&self, post: Arc<dyn Postable>) {
        let any: &dyn Any = post.as_ref();
        if any.is::<BuildInterruptedEvent>() {
            self.streamer.build_interrupted();
            return;
        }
        if any.is::<NoAnalyzeEvent>() {
            self.streamer.no_analyze();
            return;
        }
        if any.is::<NoExecutionEvent>() {
            self.streamer.no_execution();
            return;
        }
        if let Some(release) = any.downcast_ref::<ReleaseReplaceableBuildEvent>() {
            self.streamer.release_replaceable_build_event(release);
            return;
        }
        if let Some(event) = as_build_event(post) {
            self.streamer.build_event(event);
        }
    }

    /// Posts a build event directly (Java: `streamer.buildEvent(event)`).
    pub fn build_event(&self, event: Arc<dyn BuildEvent>) {
        self.streamer.build_event(event);
    }

    pub fn register_out_err_provider(&self, provider: Arc<dyn OutErrProvider>) {
        self.streamer.register_out_err_provider(provider);
    }

    pub fn close(&self) {
        self.streamer.close();
    }

    pub fn close_on_abort(&self, reason: AbortReason) {
        self.streamer.close_on_abort(reason);
    }

    pub fn is_closed(&self) -> bool {
        self.streamer.is_closed()
    }
}

impl EventBus for BepStreamer {
    fn post(&self, event: Arc<dyn Postable>) {
        self.on_post(event);
    }
}

/// An in-memory output stream, shared with the caller.
#[derive(Clone, Default)]
pub struct SharedBuffer(pub Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The pure path: streams `events` (in posting order) through a streamer with one JSON file
/// transport writing to memory, closes it, and returns the file's bytes.
pub fn events_to_json_bytes(
    events: Vec<Arc<dyn BuildEvent>>,
    bep_options: &BuildEventProtocolOptions,
    bes_stream_options: &BuildEventStreamOptions,
) -> Vec<u8> {
    let buffer = SharedBuffer::default();
    let namer = Arc::new(CountingArtifactGroupNamer::new());
    let transport: Arc<dyn BuildEventTransport> = Arc::new(JsonFormatFileTransport::new(
        Box::new(buffer.clone()),
        Arc::new(bep_options.clone()),
        Arc::new(LocalFilesArtifactUploader),
        namer.clone(),
        JsonFormatFileTransport::bep_type_registry(),
        bes_stream_options.build_event_json_file_upload_mode,
    ));
    let streamer = Builder::new()
        .build_event_transports(vec![transport])
        .bes_stream_options(bes_stream_options.clone())
        .output_group_file_modes(bep_options.get_output_group_file_modes_mapping())
        .publish_target_summaries(bep_options.publish_target_summary)
        .artifact_group_namer(namer)
        .build();
    for event in events {
        streamer.build_event(event);
    }
    streamer.close();

    buffer.0.lock().clone()
}
