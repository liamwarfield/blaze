//! Port of `BuildEventStreamer`: the streamer in charge of listening to build events and posting
//! them to each of the transports.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use blaze_optdefs::misc::build_event_protocol_options::OutputGroupFileModes;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_protos::build_event_stream::BuildEventId;
use blaze_protos::build_event_stream::aborted::AbortReason;
use indexmap::{IndexMap, IndexSet};
use parking_lot::ReentrantMutex;

use crate::buildeventstream::build_event::{BuildCompleteResultFacts, BuildEvent, BuildEventClass};
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::{
    AbortedEvent, LastBuildEvent, NullConfiguration, ProgressEvent,
};
use crate::buildeventstream::transport::{BuildEventTransport, TransportCloseError};
use crate::events::completion_context::CompletionContext;
use crate::runtime::counting_artifact_group_namer::CountingArtifactGroupNamer;
use crate::runtime::named_artifact_group::{ArtifactSet, NamedArtifactGroup};

/// Return value for `routeBuildEvent`.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:92 BuildEventStreamer.RetentionDecision
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetentionDecision {
    /// Delay posting this event until other events post.
    Buffered,
    /// Only post this event if the build ends before an event that replaces it is posted.
    BufferedForReplacement,
    /// Don't post this event.
    Discard,
    /// Post this event immediately.
    Post,
}

/// The current state of buffered progress events.
///
/// The typical case in which stdout and stderr alternate is output of the form:
///
/// ```text
/// INFO: From Executing genrule //:genrule:
/// <genrule stdout output>
/// [123 / 1234] 16 actions running
/// ```
///
/// We want the relative order of the stdout and stderr output to be preserved on a best-effort
/// basis and thus flush the two streams into a progress event before we are seeing a second
/// type transition. We are free to declare either stdout or stderr to come first. We choose
/// stderr here as that provides the more natural split in the example above: The INFO line and
/// the stdout of the action it precedes both end up in the same progress event.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:168 BuildEventStreamer.ProgressBufferState
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProgressBufferState {
    AcceptStderrAndStdout = 0,
    AcceptStdout = 1,
    RequireFlush = 2,
}

impl ProgressBufferState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => ProgressBufferState::AcceptStderrAndStdout,
            1 => ProgressBufferState::AcceptStdout,
            _ => ProgressBufferState::RequireFlush,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:171 ProgressBufferState.nextStateOnStderr
    fn next_state_on_stderr(self) -> Self {
        match self {
            ProgressBufferState::AcceptStderrAndStdout => {
                ProgressBufferState::AcceptStderrAndStdout
            }
            ProgressBufferState::AcceptStdout => ProgressBufferState::RequireFlush,
            ProgressBufferState::RequireFlush => ProgressBufferState::RequireFlush,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:176 ProgressBufferState.nextStateOnStdout
    fn next_state_on_stdout(self) -> Self {
        match self {
            ProgressBufferState::AcceptStderrAndStdout => ProgressBufferState::AcceptStdout,
            ProgressBufferState::AcceptStdout => ProgressBufferState::AcceptStdout,
            ProgressBufferState::RequireFlush => ProgressBufferState::RequireFlush,
        }
    }
}

/// Provider for stdout and stderr output.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:225 BuildEventStreamer.OutErrProvider
pub trait OutErrProvider: Send + Sync {
    /// Return the chunks of stdout that were produced since the last call to this function (or
    /// the beginning of the build, for the first call). It is the responsibility of the class
    /// implementing this interface to properly synchronize with simultaneously written output.
    fn get_out(&self) -> Option<Vec<String>>;

    /// Return the chunks of stderr that were produced since the last call to this function (or
    /// the beginning of the build, for the first call).
    fn get_err(&self) -> Option<Vec<String>>;
}

/// The mutable state guarded by `synchronized (this)` in Java.
struct State {
    announced_events: Option<IndexSet<BuildEventId>>,
    posted_events: HashSet<BuildEventId>,
    configurations_posted: HashSet<BuildEventId>,
    buffered_stdout_stderr_pairs: Option<Vec<(String, String)>>,
    /// `LinkedHashMultimap`: FIFO keys, FIFO values per key, no duplicate (key, value) pairs.
    pending_events: IndexMap<BuildEventId, Vec<Arc<dyn BuildEvent>>>,
    progress_count: i32,
    abort_reasons: IndexSet<AbortReason>,
    is_command_to_skip_build_complete_event: bool,
    final_events_to_come: Option<HashSet<BuildEventId>>,
    closed: bool,
    out_err_provider: Option<Arc<dyn OutErrProvider>>,
    close_results: Vec<(String, Result<(), TransportCloseError>)>,
}

/// Streamer in charge of listening to build events and post them to each of the transports.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:90 BuildEventStreamer
pub struct BuildEventStreamer {
    transports: Vec<Arc<dyn BuildEventTransport>>,
    bes_options: BuildEventStreamOptions,
    output_group_file_modes: OutputGroupFileModes,
    publish_target_summaries: bool,
    artifact_group_namer: Arc<CountingArtifactGroupNamer>,
    oom_message: String,
    progress_buffer_state: AtomicU8,
    /// Java's monitor: reentrant, like `synchronized`.
    state: ReentrantMutex<RefCell<State>>,
}

fn pending_put(
    pending: &mut IndexMap<BuildEventId, Vec<Arc<dyn BuildEvent>>>,
    id: BuildEventId,
    event: Arc<dyn BuildEvent>,
) {
    let values = pending.entry(id).or_default();
    if !values.iter().any(|e| Arc::ptr_eq(e, &event)) {
        values.push(event);
    }
}

impl BuildEventStreamer {
    /// Creates a new build event streamer.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:242 BuildEventStreamer.<init>
    fn new(
        transports: Vec<Arc<dyn BuildEventTransport>>,
        options: BuildEventStreamOptions,
        output_group_file_modes: OutputGroupFileModes,
        publish_target_summaries: bool,
        artifact_group_namer: Arc<CountingArtifactGroupNamer>,
        oom_message: String,
    ) -> Self {
        BuildEventStreamer {
            transports,
            bes_options: options,
            output_group_file_modes,
            publish_target_summaries,
            artifact_group_namer,
            oom_message,
            progress_buffer_state: AtomicU8::new(ProgressBufferState::AcceptStderrAndStdout as u8),
            state: ReentrantMutex::new(RefCell::new(State {
                announced_events: None,
                posted_events: HashSet::new(),
                configurations_posted: HashSet::new(),
                buffered_stdout_stderr_pairs: Some(Vec::new()),
                pending_events: IndexMap::new(),
                progress_count: 0,
                abort_reasons: IndexSet::new(),
                is_command_to_skip_build_complete_event: false,
                final_events_to_come: None,
                closed: false,
                out_err_provider: None,
                close_results: Vec::new(),
            })),
        }
    }

    /// The transports of this streamer.
    pub fn get_transports(&self) -> &[Arc<dyn BuildEventTransport>] {
        &self.transports
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:260 BuildEventStreamer.registerOutErrProvider
    pub fn register_out_err_provider(&self, out_err_provider: Arc<dyn OutErrProvider>) {
        let guard = self.state.lock();
        guard.borrow_mut().out_err_provider = Some(out_err_provider);
    }

    fn set_progress_buffer_state(&self, s: ProgressBufferState) {
        self.progress_buffer_state.store(s as u8, Ordering::SeqCst);
    }

    /// Reads `outErrProvider.getOut()/getErr()` (empty if absent) and resets the progress buffer
    /// state, as every caller does.
    fn take_out_err(&self, st: &State) -> Option<(Vec<String>, Vec<String>)> {
        let provider = st.out_err_provider.clone()?;
        let all_out = provider.get_out().unwrap_or_default();
        let all_err = provider.get_err().unwrap_or_default();
        self.set_progress_buffer_state(ProgressBufferState::AcceptStderrAndStdout);
        Some((all_out, all_err))
    }

    // This exists to nop out the announcement of new events after #buildComplete
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:265 BuildEventStreamer.maybeRegisterAnnouncedEvent
    fn maybe_register_announced_event(st: &mut State, id: BuildEventId) {
        if st.final_events_to_come.is_some() {
            return;
        }
        st.announced_events.as_mut().unwrap().insert(id);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:274 BuildEventStreamer.maybeRegisterAnnouncedEvents
    fn maybe_register_announced_events(st: &mut State, ids: Vec<BuildEventId>) {
        if st.final_events_to_come.is_some() {
            return;
        }
        st.announced_events.as_mut().unwrap().extend(ids);
    }

    /// Post a new event to all transports; simultaneously keep track of the events we announce
    /// to still come.
    ///
    /// Moreover, link unannounced events to the progress stream; we only expect failure events
    /// to come before their parents.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:292 BuildEventStreamer.post
    fn post(&self, event: Arc<dyn BuildEvent>) {
        let mut link_events: Option<Vec<Arc<dyn BuildEvent>>> = None;
        let id = event.get_event_id();
        let mut flush_events: Option<Vec<Arc<dyn BuildEvent>>> = None;
        let mut last_event = false;

        {
            let guard = self.state.lock();
            let mut st = guard.borrow_mut();
            if st.announced_events.is_none() {
                st.announced_events = Some(IndexSet::new());
                // The very first event of a stream is implicitly announced by the convention that
                // a complete stream has to have at least one entry. In this way we keep the
                // invariant that the set of posted events is always a subset of the set of
                // announced events.
                Self::maybe_register_announced_event(&mut st, id.clone());
                if !event
                    .get_children_events()
                    .contains(&ProgressEvent::initial_progress_update())
                {
                    let progress = ProgressEvent::progress_chain_in_empty(
                        st.progress_count,
                        event.get_event_id(),
                    );
                    link_events = Some(vec![progress.clone()]);
                    st.progress_count += 1;
                    Self::maybe_register_announced_events(&mut st, progress.get_children_events());
                    // the new first event in the stream, implicitly announced by the fact that
                    // complete stream may not be empty.
                    Self::maybe_register_announced_event(&mut st, progress.get_event_id());
                    st.posted_events.insert(progress.get_event_id());
                }

                let buffered = st.buffered_stdout_stderr_pairs.take().unwrap_or_default();
                if !buffered.is_empty() {
                    let mut flush = Vec::with_capacity(buffered.len());
                    for (out, err) in buffered {
                        flush.push(Self::flush_stdout_stderr_event(&mut st, out, err));
                    }
                    flush_events = Some(flush);
                }
            } else if !st.announced_events.as_ref().unwrap().contains(&id) {
                let (all_out, all_err) = self.take_out_err(&st).unwrap_or_default();
                let mut final_link_events = Vec::new();
                consume_as_pairs_of_strings(&all_out, &all_err, &mut |out, err| {
                    let progress_event = ProgressEvent::progress_chain_in(
                        st.progress_count,
                        id.clone(),
                        Some(out),
                        Some(err),
                    );
                    final_link_events.push(progress_event.clone());
                    st.progress_count += 1;
                    Self::maybe_register_announced_events(
                        &mut st,
                        progress_event.get_children_events(),
                    );
                    st.posted_events.insert(progress_event.get_event_id());
                });
                link_events = Some(final_link_events);
            }

            if event.event_class() == BuildEventClass::BuildInfo {
                // The specification for BuildInfoEvent says that there may be many such events,
                // but all except the first one should be ignored.
                if st.posted_events.contains(&id) {
                    return;
                }
            }

            st.posted_events.insert(id.clone());
            Self::maybe_register_announced_events(&mut st, event.get_children_events());
            // We keep as an invariant that postedEvents is a subset of announced events, so this
            // is a cheaper test for equality
            if st.announced_events.as_ref().unwrap().len() == st.posted_events.len() {
                last_event = true;
            }
        }

        let main_event: Arc<dyn BuildEvent> = if last_event {
            Arc::new(LastBuildEvent::new(event))
        } else {
            event
        };

        for transport in &self.transports {
            if let Some(link_events) = &link_events {
                for link_event in link_events {
                    transport.send_build_event(link_event);
                }
            }
            transport.send_build_event(&main_event);
        }

        if let Some(flush_events) = flush_events {
            for flush_event in &flush_events {
                for transport in &self.transports {
                    transport.send_build_event(flush_event);
                }
            }
        }
    }

    /// If some events are blocked on the absence of a build_started event, generate such an
    /// event; moreover, make that artificial start event announce all events blocked on it, as
    /// well as the `BuildCompletingEvent` that caused the early end of the stream.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:393 BuildEventStreamer.clearMissingStartEvent
    fn clear_missing_start_event(&self, id: BuildEventId) {
        let guard = self.state.lock();
        let children = {
            let st = guard.borrow();
            let Some(blocked) = st
                .pending_events
                .get(&build_event_id_util::build_started_id())
            else {
                return;
            };
            let mut children: IndexSet<BuildEventId> = IndexSet::new();
            children.insert(ProgressEvent::initial_progress_update());
            children.insert(id);
            children.extend(blocked.iter().map(|e| e.get_event_id()));
            children
        };
        let reason = self.get_last_abort_reason();
        let details = self.get_abort_reason_details();
        self.build_event(Arc::new(AbortedEvent::with_children(
            build_event_id_util::build_started_id(),
            children.into_iter().collect(),
            reason,
            details,
        )));
        drop(guard);
    }

    /// Clear pending events by generating aborted events for all their requests.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:412 BuildEventStreamer.clearPendingEvents
    fn clear_pending_events(&self) {
        let guard = self.state.lock();
        loop {
            let id = {
                let st = guard.borrow();
                match st.pending_events.keys().next() {
                    None => break,
                    Some(id) => id.clone(),
                }
            };
            let buffered_events_pending_on_this_type = self
                .release_replaceable_build_event(&ReleaseReplaceableBuildEvent::new(id.clone()));
            if buffered_events_pending_on_this_type {
                // Replaceable (BUFERED_FOR_REPLACEMENT) events finally trigger on build abort, so
                // we don't need a distinct AbortedEvent to acknowledge them. Normal buffered events
                // don't trigger because their trigger event never happened, so they need an
                // AbortedEvent.
                let reason = self.get_last_abort_reason();
                let details = self.get_abort_reason_details();
                self.build_event(Arc::new(AbortedEvent::new(id, reason, details)));
            }
        }
        drop(guard);
    }

    /// Clear all events that are still announced; events not naturally closed by the expected
    /// event normally only occur if the build is aborted.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:431 BuildEventStreamer.clearAnnouncedEvents
    fn clear_announced_events(&self, dontclear: &[BuildEventId]) {
        let guard = self.state.lock();
        let ids: Vec<BuildEventId> = {
            let st = guard.borrow();
            let Some(announced) = &st.announced_events else {
                return;
            };
            // PORT: java-hash-order. Java iterates Sets.difference(announcedEvents,
            // postedEvents) in HashSet order; this port uses announcement order. Only the
            // order of aborted events of an aborted build depends on it.
            announced
                .iter()
                .filter(|id| !st.posted_events.contains(*id))
                .cloned()
                .collect()
        };
        for id in ids {
            if !dontclear.contains(&id) {
                let reason = self.get_last_abort_reason();
                let details = self.get_abort_reason_details();
                self.post(Arc::new(AbortedEvent::new(id, reason, details)));
            }
        }
        drop(guard);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:447 BuildEventStreamer.isClosed
    pub fn is_closed(&self) -> bool {
        let guard = self.state.lock();
        guard.borrow().closed
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:451 BuildEventStreamer.closeOnAbort
    pub fn close_on_abort(&self, reason: AbortReason) {
        self.close_with(Some(reason));
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:452 BuildEventStreamer.close
    pub fn close(&self) {
        self.close_with(None);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:452 BuildEventStreamer.close
    fn close_with(&self, reason: Option<AbortReason>) {
        let guard = self.state.lock();
        {
            let mut st = guard.borrow_mut();
            if st.closed {
                return;
            }
            st.closed = true;
            if let Some(reason) = reason {
                st.abort_reasons.insert(reason);
            }
        }

        let final_events_to_come_is_null = guard.borrow().final_events_to_come.is_none();
        if final_events_to_come_is_null {
            // This should only happen if there's a crash. Try to clean up as best we can.
            self.clear_events_and_post_final_progress(None);
        }

        let mut results = Vec::new();
        for transport in &self.transports {
            results.push((transport.name(), transport.close()));
        }
        guard.borrow_mut().close_results = results;
        drop(guard);
    }

    /// The results of closing each transport (`getCloseFuturesMap`); empty before `close`.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:992 BuildEventStreamer.getCloseFuturesMap
    pub fn get_close_futures_map(&self) -> Vec<(String, Result<(), TransportCloseError>)> {
        let guard = self.state.lock();
        guard.borrow().close_results.clone()
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:488 BuildEventStreamer.maybeReportArtifactSet
    fn maybe_report_artifact_set(&self, ctx: &Arc<CompletionContext>, set: &ArtifactSet) {
        let Some(locked_name) = self.artifact_group_namer.maybe_name(set.to_node()) else {
            return;
        };
        let mut set = NamedArtifactGroup::expand_set(ctx, set);
        // Invariant: all leaf successors ("direct elements") of set are ExpandedArtifacts.

        // We only split if the max number of entries is at least 2 (it must be at least a binary
        // tree). The method throws for smaller values.
        if self.bes_options.max_named_set_entries >= 2 {
            // We only split the event after naming it to avoid splitting the same node multiple
            // times. Note that the artifactGroupNames keeps references to the individual pieces,
            // so this can double the memory consumption of large nested sets.
            set = set.split_if_exceeds_maximum_size(self.bes_options.max_named_set_entries);
        }

        for succ in set.get_non_leaves() {
            self.maybe_report_artifact_set(ctx, &succ);
        }
        self.post(Arc::new(NamedArtifactGroup::new(
            locked_name.get_name(),
            ctx.clone(),
            set,
        )));
        drop(locked_name);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:512 BuildEventStreamer.maybeReportConfiguration
    fn maybe_report_configuration(&self, configuration: Option<Arc<dyn BuildEvent>>) {
        let event = configuration.unwrap_or_else(NullConfiguration::instance);
        let id = event.get_event_id();
        let guard = self.state.lock();
        let added = guard.borrow_mut().configurations_posted.insert(id);
        if added {
            self.post(event);
        }
        drop(guard);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:523 BuildEventStreamer.buildInterrupted
    pub fn build_interrupted(&self) {
        self.add_abort_reason(AbortReason::UserInterrupted);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:528 BuildEventStreamer.noAnalyze
    pub fn no_analyze(&self) {
        self.add_abort_reason(AbortReason::NoAnalyze);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:533 BuildEventStreamer.noExecution
    pub fn no_execution(&self) {
        self.add_abort_reason(AbortReason::NoBuild);
    }

    /// Posts a `BUFFERED_FOR_REPLACEMENT` build event without waiting for its replacement.
    ///
    /// Does nothing if no replaceable event is pending for this event type. Note there can be at
    /// most one pending replaceable event for any build type.
    ///
    /// Returns true iff normal buffered events are also waiting on this event id. This is useful
    /// to distinguish from replaceable events because when builds abort, pending replaceable
    /// events trigger while normal buffered events are dropped and noted with a matching
    /// AbortedEvent.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:416 BuildEventStreamer.releaseReplaceableBuildEvent
    pub fn release_replaceable_build_event(&self, event: &ReleaseReplaceableBuildEvent) -> bool {
        let mut replaceable: Option<Arc<dyn BuildEvent>> = None;
        let mut buffered_events_also_pending = false;
        {
            let guard = self.state.lock();
            let mut st = guard.borrow_mut();
            if let Some(pending) = st.pending_events.get_mut(&event.event_id) {
                let mut kept = Vec::with_capacity(pending.len());
                for pending_event in pending.drain(..) {
                    if pending_event.replaceable().is_some() {
                        assert!(
                            replaceable.is_none(),
                            "Multiple replaceable events not supported for {:?} ",
                            event.event_id
                        );
                        replaceable = Some(pending_event);
                    } else {
                        buffered_events_also_pending = true;
                        kept.push(pending_event);
                    }
                }
                if kept.is_empty() {
                    st.pending_events.shift_remove(&event.event_id);
                } else {
                    *st.pending_events.get_mut(&event.event_id).unwrap() = kept;
                }
            }
        }
        if let Some(replaceable) = replaceable {
            self.post(replaceable);
        }
        buffered_events_also_pending
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:402 BuildEventStreamer.buildEvent
    pub fn build_event(&self, event: Arc<dyn BuildEvent>) {
        {
            let guard = self.state.lock();
            let mut st = guard.borrow_mut();
            if let Some(final_events_to_come) = st.final_events_to_come.as_mut() {
                let id = event.get_event_id();
                if !final_events_to_come.remove(&id) {
                    return;
                }
            }
        }

        match self.route_build_event(&event) {
            RetentionDecision::Discard => {
                // Check if there are pending events waiting on this event
                self.maybe_post_pending_events_before_discarding(&event);
                return; // bail: we're dropping this event
            }
            // Bail: the event was buffered and the BuildEventStreamer is now responsible for
            // eventually posting it.
            RetentionDecision::Buffered => return,
            // Bail: the event was buffered to possibly be replaced with an updated version. The
            // BuildEventStreamer is now responsible for eventually posting or discarding it.
            RetentionDecision::BufferedForReplacement => return,
            RetentionDecision::Post => {} // proceed
        }

        let class = event.event_class();
        if let BuildEventClass::BuildStarting { command_name } = &class {
            let guard = self.state.lock();
            guard.borrow_mut().is_command_to_skip_build_complete_event =
                command_name == "test" || command_name == "coverage" || command_name == "run";
        }

        if let Some(configurations) = event.get_configurations() {
            for configuration in configurations {
                self.maybe_report_configuration(Some(configuration));
            }
        }

        if let Some(reported_artifacts) = event.reported_artifacts(&self.output_group_file_modes) {
            for artifact_set in &reported_artifacts.artifacts {
                self.maybe_report_artifact_set(
                    &reported_artifacts.completion_context,
                    &ArtifactSet::Artifacts(artifact_set.clone()),
                );
            }
        }

        let is_build_completing = matches!(class, BuildEventClass::BuildCompleting { .. });
        if is_build_completing && event.get_event_id() != build_event_id_util::build_started_id() {
            self.clear_missing_start_event(event.get_event_id());
        }

        if class == BuildEventClass::BuildConfiguration {
            self.maybe_report_configuration(Some(event.clone()));
        } else {
            self.post(event.clone());
        }

        // Reconsider all events blocked by the event just posted.
        let blocked_events_fifo = {
            let guard = self.state.lock();
            let mut st = guard.borrow_mut();
            st.pending_events
                .shift_remove(&event.get_event_id())
                .unwrap_or_default()
        };
        for freed_event in blocked_events_fifo {
            // Replaceable events have been replaced, so can be silently dropped.
            if freed_event.replaceable().is_none() {
                self.build_event(freed_event);
            }
        }

        // Special-case handling for subclasses of `BuildCompletingEvent`.
        //
        // For most commands, exactly one `BuildCompletingEvent` will be posted to the EventBus.
        // If the command is "run" or "test", a non-crashing/catastrophic `BuildCompleteEvent`
        // will be followed by a RunBuildCompleteEvent/TestingCompleteEvent.
        if let BuildEventClass::BuildCompleting {
            build_complete: Some(result),
        } = &class
        {
            if is_crash(result) || is_catastrophe(result) {
                if is_oom(result) {
                    self.add_abort_reason(AbortReason::OutOfMemory);
                } else {
                    self.add_abort_reason(AbortReason::Internal);
                }
            } else if is_incomplete(result) {
                self.add_abort_reason(AbortReason::Incomplete);
            }
        }

        if is_build_completing {
            self.build_complete(&event);
        }

        if let BuildEventClass::NoBuild {
            separate_finished_event,
        } = class
            && !separate_finished_event
        {
            self.build_complete(&event);
        }

        let should_close = {
            let guard = self.state.lock();
            let st = guard.borrow();
            st.final_events_to_come
                .as_ref()
                .is_some_and(HashSet::is_empty)
        };
        if should_close {
            self.close();
        }
    }

    /// Given an event that will be discarded (not buffered), publishes any events waiting on
    /// the given event.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:592 BuildEventStreamer.maybePostPendingEventsBeforeDiscarding
    fn maybe_post_pending_events_before_discarding(&self, event: &Arc<dyn BuildEvent>) {
        if self.publish_target_summaries && is_vacuous_test_summary(event.as_ref()) {
            // Target summaries should "post after" test summaries, but we can't a priori know
            // whether test summaries will be vacuous (as that depends on test execution
            // progress). So check for and publish any pending (target summary) events here. If
            // we don't do this then clearPendingEvents() will publish "aborted" test_summary
            // events for the very events we're discarding here (b/184580877), followed by the
            // pending target_summary events, which is not only confusing but also delays
            // target_summary events until the end of the build.
            //
            // Technically it seems we should do this with all events we're dropping but that
            // would be a lot of extra locking e.g. for every ActionExecutedEvent and it's only
            // necessary to check for this where events are configured to "post after" events
            // that may be discarded.
            let event_id = event.get_event_id();
            let blocked_events_fifo = {
                let guard = self.state.lock();
                let mut st = guard.borrow_mut();
                let blocked = st
                    .pending_events
                    .shift_remove(&event_id)
                    .unwrap_or_default();
                // Pretend we posted this event so a target summary arriving after this test
                // summary (which is common) doesn't get erroneously buffered in
                // bufferUntilPrerequisitesReceived().
                st.posted_events.insert(event_id);
                blocked
            };
            for freed_event in blocked_events_fifo {
                self.build_event(freed_event);
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:733 BuildEventStreamer.flushStdoutStderrEvent
    fn flush_stdout_stderr_event(st: &mut State, out: String, err: String) -> Arc<dyn BuildEvent> {
        let update_event = ProgressEvent::progress_update(st.progress_count, Some(out), Some(err));
        st.progress_count += 1;
        Self::maybe_register_announced_events(st, update_event.get_children_events());
        st.posted_events.insert(update_event.get_event_id());
        update_event
    }

    /// Whether the given output type can be written without first flushing the streamer.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:742 BuildEventStreamer.canBufferProgressWrite
    pub fn can_buffer_progress_write(&self, is_stderr: bool) -> bool {
        let mut current = self.progress_buffer_state.load(Ordering::SeqCst);
        loop {
            let s = ProgressBufferState::from_u8(current);
            let new_state = if is_stderr {
                s.next_state_on_stderr()
            } else {
                s.next_state_on_stdout()
            };
            match self.progress_buffer_state.compare_exchange(
                current,
                new_state as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return new_state != ProgressBufferState::RequireFlush,
                Err(actual) => current = actual,
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:754 BuildEventStreamer.flush
    pub fn flush(&self) {
        let mut update_events: Option<Vec<Arc<dyn BuildEvent>>> = None;
        {
            let guard = self.state.lock();
            let mut st = guard.borrow_mut();
            let (all_out, all_err) = self.take_out_err(&st).unwrap_or_default();
            if all_out.is_empty() && all_err.is_empty() {
                // Nothing to flush; avoid generating an unneeded progress event.
                return;
            }
            if st.final_events_to_come.is_some() {
                // If we've already announced the final events, we cannot add more progress
                // events. Stdout and stderr are truncated from the event log.
                consume_as_pairs_of_strings(&all_out, &all_err, &mut |_, _| {});
            } else if st.announced_events.is_some() {
                let mut final_update_events = Vec::new();
                consume_as_pairs_of_strings(&all_out, &all_err, &mut |s1, s2| {
                    final_update_events.push(Self::flush_stdout_stderr_event(&mut st, s1, s2));
                });
                update_events = Some(final_update_events);
            } else {
                let mut pairs = Vec::new();
                consume_as_pairs_of_strings(&all_out, &all_err, &mut |s1, s2| pairs.push((s1, s2)));
                st.buffered_stdout_stderr_pairs
                    .get_or_insert_with(Vec::new)
                    .extend(pairs);
            }
        }
        if let Some(update_events) = update_events {
            for update_event in &update_events {
                for transport in &self.transports {
                    transport.send_build_event(update_event);
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:470 BuildEventStreamer.clearEventsAndPostFinalProgress
    fn clear_events_and_post_final_progress(&self, event: Option<&Arc<dyn BuildEvent>>) {
        let guard = self.state.lock();
        self.clear_pending_events();
        let (all_out, all_err) = {
            let st = guard.borrow();
            self.take_out_err(&st).unwrap_or_default()
        };
        let pairs = as_pairs(&all_out, &all_err);
        let n = pairs.len();
        for (i, (s1, s2)) in pairs.into_iter().enumerate() {
            let e = {
                let mut st = guard.borrow_mut();
                if i + 1 < n {
                    Self::flush_stdout_stderr_event(&mut st, s1, s2)
                } else {
                    let count = st.progress_count;
                    st.progress_count += 1;
                    ProgressEvent::final_progress_update(count, Some(s1), Some(s2))
                }
            };
            self.post(e);
        }
        let dontclear = event.map(|e| e.get_children_events()).unwrap_or_default();
        self.clear_announced_events(&dontclear);
        drop(guard);
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:669 BuildEventStreamer.buildComplete
    fn build_complete(&self, event: &Arc<dyn BuildEvent>) {
        let guard = self.state.lock();
        self.clear_events_and_post_final_progress(Some(event));

        let is_empty = {
            let mut st = guard.borrow_mut();
            let st = &mut *st;
            let mut final_events_to_come: HashSet<BuildEventId> = st
                .announced_events
                .as_ref()
                .map(|a| a.iter().cloned().collect())
                .unwrap_or_default();
            final_events_to_come.retain(|id| !st.posted_events.contains(id));
            let empty = final_events_to_come.is_empty();
            st.final_events_to_come = Some(final_events_to_come);
            empty
        };
        if is_empty {
            self.close();
        }
        drop(guard);
    }

    /// Returns whether a build event should be ignored or was buffered.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:887 BuildEventStreamer.routeBuildEvent
    fn route_build_event(&self, event: &Arc<dyn BuildEvent>) -> RetentionDecision {
        let class = event.event_class();
        if let BuildEventClass::ActionExecuted {
            has_exception,
            is_extra_action,
        } = class
            && !self.should_publish_action_executed_event(has_exception, is_extra_action)
        {
            return RetentionDecision::Discard;
        }

        if let Some(replaceable_decision) = self.decide_buffered_for_replacement_event(event) {
            return replaceable_decision;
        }

        if self.buffer_until_prerequisites_received(event) {
            return RetentionDecision::Buffered;
        }

        if is_vacuous_test_summary(event.as_ref()) {
            return RetentionDecision::Discard;
        }

        let skip = {
            let guard = self.state.lock();

            guard.borrow().is_command_to_skip_build_complete_event
        };
        if skip
            && let BuildEventClass::BuildCompleting {
                build_complete: Some(result),
            } = &class
        {
            // In case of "bazel test" or "bazel run" ignore the BuildCompleteEvent, as it will be
            // followed by a TestingCompleteEvent (or RunBuildCompleteEvent) that contains the
            // correct exit code.
            return if is_crash(result) {
                RetentionDecision::Post
            } else {
                RetentionDecision::Discard
            };
        }

        if let BuildEventClass::TargetParsingComplete {
            original_target_pattern_count,
            failed_target_patterns_empty,
        } = class
        {
            // If there is only one pattern and we have one failed pattern, then we already
            // posted a pattern expanded error, so we don't post the completion event.
            // TODO(b/109727414): This is brittle. It would be better to always post one
            // PatternExpanded event for each pattern given on the command line instead of one
            // event for all of them combined.
            let discard = original_target_pattern_count == 1 && !failed_target_patterns_empty;
            return if discard {
                RetentionDecision::Discard
            } else {
                RetentionDecision::Post
            };
        }

        RetentionDecision::Post
    }

    /// Returns whether an `ActionExecutedEvent` should be published.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:930 BuildEventStreamer.shouldPublishActionExecutedEvent
    fn should_publish_action_executed_event(
        &self,
        has_exception: bool,
        is_extra_action: bool,
    ) -> bool {
        if self.bes_options.publish_all_actions {
            return true;
        }
        if has_exception {
            // Publish failed actions
            return true;
        }
        is_extra_action
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:942 BuildEventStreamer.decideBufferedForReplacementEvent
    fn decide_buffered_for_replacement_event(
        &self,
        event: &Arc<dyn BuildEvent>,
    ) -> Option<RetentionDecision> {
        let replaceable = event.replaceable()?;
        if !replaceable {
            // The event's class is replaceable but this instance isn't. Treat it normally.
            return Some(RetentionDecision::Post);
        }
        let guard = self.state.lock();
        let mut st = guard.borrow_mut();
        let id = event.get_event_id();
        if st.posted_events.contains(&id) {
            // This event type has already been posted, so the replaceable event is outdated.
            return Some(RetentionDecision::Discard);
        }
        assert!(
            !st.pending_events
                .get(&id)
                .is_some_and(|v| v.iter().any(|e| e.replaceable().is_some())),
            "Multiple replaceable events not supported for {id:?}"
        );
        pending_put(&mut st.pending_events, id, event.clone());
        Some(RetentionDecision::BufferedForReplacement)
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:967 BuildEventStreamer.bufferUntilPrerequisitesReceived
    fn buffer_until_prerequisites_received(&self, event: &Arc<dyn BuildEvent>) -> bool {
        let Some(posted_after) = event.posted_after() else {
            return false;
        };
        let guard = self.state.lock();
        let mut st = guard.borrow_mut();
        // Check if all prerequisite events are posted already.
        for prerequisite_id in posted_after {
            if !st.posted_events.contains(&prerequisite_id) {
                pending_put(&mut st.pending_events, prerequisite_id, event.clone());
                return true;
            }
        }
        false
    }

    /// Stores the `AbortReason` for later reporting on BEP pending events.
    ///
    /// In case of multiple abort reasons:
    /// - Only the most recent reason will be reported as the main `AbortReason` in BEP.
    /// - All previous reasons will appear in the `Aborted.description` message.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:465 BuildEventStreamer.addAbortReason
    fn add_abort_reason(&self, reason: AbortReason) {
        let guard = self.state.lock();
        guard.borrow_mut().abort_reasons.insert(reason);
    }

    /// Returns the most recent `AbortReason` or `UNKNOWN` if no reason was set.
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:406 BuildEventStreamer.getLastAbortReason
    fn get_last_abort_reason(&self) -> AbortReason {
        let guard = self.state.lock();
        let st = guard.borrow();
        st.abort_reasons
            .last()
            .copied()
            .unwrap_or(AbortReason::Unknown)
    }

    /// Returns a detailed message explaining the most recent `AbortReason` (and possibly
    /// previous reasons).
    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:407 BuildEventStreamer.getAbortReasonDetails
    fn get_abort_reason_details(&self) -> String {
        let guard = self.state.lock();
        let st = guard.borrow();
        if st.abort_reasons.len() == 1
            && st.abort_reasons.first() == Some(&AbortReason::OutOfMemory)
        {
            return construct_oom_exit_message(&self.oom_message);
        }
        if st.abort_reasons.len() > 1 {
            // Java: "Multiple abort reasons reported: " + abortReasons (AbstractCollection
            // .toString of the enum names).
            let names: Vec<&str> = st.abort_reasons.iter().map(|r| r.as_str_name()).collect();
            format!("Multiple abort reasons reported: [{}]", names.join(", "))
        } else {
            String::new()
        }
    }
}

/// `BugReport.constructOomExitMessage`.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:400 BugReport.constructOomExitMessage
pub fn construct_oom_exit_message(extra_info: &str) -> String {
    let msg = "ERROR: Bazel ran out of memory and crashed.";
    if extra_info.is_empty() {
        msg.to_string()
    } else {
        format!("{msg} {extra_info}")
    }
}

// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:683 BuildEventStreamer.isCrash
fn is_crash(event: &BuildCompleteResultFacts) -> bool {
    event.has_unhandled_throwable || is_oom(event)
}

// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:687 BuildEventStreamer.isCatastrophe
fn is_catastrophe(event: &BuildCompleteResultFacts) -> bool {
    event.was_catastrophe
}

// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:691 BuildEventStreamer.isIncomplete
fn is_incomplete(event: &BuildCompleteResultFacts) -> bool {
    !event.success && !event.was_catastrophe && event.stop_on_first_failure
}

// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:697 BuildEventStreamer.isOom
fn is_oom(event: &BuildCompleteResultFacts) -> bool {
    event.is_oom
}

/// Return true if the test summary contains no actual test runs.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:982 BuildEventStreamer.isVacuousTestSummary
fn is_vacuous_test_summary(event: &dyn BuildEvent) -> bool {
    matches!(event.event_class(), BuildEventClass::TestSummary { total_runs } if total_runs == 0)
}

/// Given a pair of iterables and consumers, emit a sequence of pairs to the consumers. Given
/// the left iterables [L1, L2, ... LN], and the right iterable [R1, R2, ... RM], the consumers
/// will see this sequence of calls:
///
/// ```text
///  biConsumer.accept(L1, null);
///  ...
///  biConsumer.accept(L(N-1), null);
///  biConsumer.accept(LN, R1);
///  biConsumer.accept(null, R2);
///  ...
///  biConsumer.accept(null, R(M-1));
///  lastConsumer.accept(null, RM);
/// ```
///
/// The lastConsumer is always called exactly once, even if both iterables are empty.
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:811 BuildEventStreamer.consumeAsPairs
pub fn consume_as_pairs<T: Clone>(
    left_iterable: &[T],
    right_iterable: &[T],
    bi_consumer: &mut dyn FnMut(Option<T>, Option<T>),
    last_consumer: &mut dyn FnMut(Option<T>, Option<T>),
) {
    if left_iterable.is_empty() && right_iterable.is_empty() {
        last_consumer(None, None);
        return;
    }

    let mut left_iterator = left_iterable.iter().peekable();
    let mut right_iterator = right_iterable.iter().peekable();
    while let Some(left) = left_iterator.next() {
        let last_t = left_iterator.peek().is_none();
        let right = if last_t && right_iterator.peek().is_some() {
            right_iterator.next().cloned()
        } else {
            None
        };
        let last_item = last_t && right_iterator.peek().is_none();
        if last_item {
            last_consumer(Some(left.clone()), right);
        } else {
            bi_consumer(Some(left.clone()), right);
        }
    }

    while let Some(right) = right_iterator.next() {
        if right_iterator.peek().is_none() {
            last_consumer(None, Some(right.clone()));
        } else {
            bi_consumer(None, Some(right.clone()));
        }
    }
}

/// The pairs `consumeAsPairsofStrings` produces (nulls as empty strings), in order.
fn as_pairs(left: &[String], right: &[String]) -> Vec<(String, String)> {
    let pairs = RefCell::new(Vec::new());
    let push = |a: Option<String>, b: Option<String>| {
        pairs
            .borrow_mut()
            .push((a.unwrap_or_default(), b.unwrap_or_default()))
    };
    consume_as_pairs(left, right, &mut { push }, &mut { push });
    pairs.into_inner()
}

// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:334 BuildEventStreamer.consumeAsPairsofStrings
fn consume_as_pairs_of_strings(
    left: &[String],
    right: &[String],
    bi_consumer: &mut dyn FnMut(String, String),
) {
    for (a, b) in as_pairs(left, right) {
        bi_consumer(a, b);
    }
}

/// Posted to release a buffered replaceable event (`ReleaseReplaceableBuildEvent`).
// Java: com/google/devtools/build/lib/buildeventstream/ReleaseReplaceableBuildEvent.java:22 ReleaseReplaceableBuildEvent
#[derive(Clone, Debug)]
pub struct ReleaseReplaceableBuildEvent {
    event_id: BuildEventId,
}

impl ReleaseReplaceableBuildEvent {
    pub fn new(event_id: BuildEventId) -> Self {
        ReleaseReplaceableBuildEvent { event_id }
    }

    pub fn get_event_id(&self) -> &BuildEventId {
        &self.event_id
    }
}

impl blaze_events::Postable for ReleaseReplaceableBuildEvent {}

/// A builder for [`BuildEventStreamer`].
// Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:1044 BuildEventStreamer.Builder
#[derive(Default)]
pub struct Builder {
    build_event_transports: Option<Vec<Arc<dyn BuildEventTransport>>>,
    bes_stream_options: Option<BuildEventStreamOptions>,
    output_group_file_modes: Option<OutputGroupFileModes>,
    publish_target_summaries: bool,
    artifact_group_namer: Option<Arc<CountingArtifactGroupNamer>>,
    oom_message: Option<String>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build_event_transports(mut self, value: Vec<Arc<dyn BuildEventTransport>>) -> Self {
        self.build_event_transports = Some(value);
        self
    }

    pub fn bes_stream_options(mut self, value: BuildEventStreamOptions) -> Self {
        self.bes_stream_options = Some(value);
        self
    }

    pub fn output_group_file_modes(mut self, value: OutputGroupFileModes) -> Self {
        self.output_group_file_modes = Some(value);
        self
    }

    pub fn publish_target_summaries(mut self, value: bool) -> Self {
        self.publish_target_summaries = value;
        self
    }

    pub fn artifact_group_namer(mut self, value: Arc<CountingArtifactGroupNamer>) -> Self {
        self.artifact_group_namer = Some(value);
        self
    }

    pub fn oom_message(mut self, value: String) -> Self {
        self.oom_message = Some(value);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:1088 Builder.build
    pub fn build(self) -> BuildEventStreamer {
        BuildEventStreamer::new(
            self.build_event_transports
                .expect("NullPointerException: buildEventTransports"),
            self.bes_stream_options
                .expect("NullPointerException: besStreamOptions"),
            self.output_group_file_modes
                .unwrap_or_else(OutputGroupFileModes::default_modes),
            self.publish_target_summaries,
            self.artifact_group_namer
                .expect("NullPointerException: artifactGroupNamer"),
            self.oom_message.unwrap_or_default(),
        )
    }
}
