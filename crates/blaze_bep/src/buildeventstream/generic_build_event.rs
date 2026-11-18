//! Port of `GenericBuildEvent`, `ProgressEvent`, `AbortedEvent`, `LastBuildEvent` and
//! `NullConfiguration`.

use std::sync::Arc;

use blaze_cmdline::label::Label;
use blaze_protos::build_event_stream::aborted::AbortReason;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};

use super::build_event::{BuildEvent, Interrupted, LocalFile};
use super::build_event_context::BuildEventContext;
use super::build_event_id_util;

/// Starts a `BuildEvent` proto with the id and children of `event`.
// Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:45 GenericBuildEvent.protoChaining
pub fn proto_chaining(event: &(impl BuildEvent + ?Sized)) -> bes::BuildEvent {
    bes::BuildEvent {
        id: Some(event.get_event_id()),
        children: event.get_children_events(),
        ..Default::default()
    }
}

/// Builds `protoChaining(event).setXxx(payload).build()`.
pub fn proto_chaining_with(
    event: &(impl BuildEvent + ?Sized),
    payload: Payload,
) -> bes::BuildEvent {
    let mut proto = proto_chaining(event);
    proto.payload = Some(payload);
    proto
}

/// Class of build events that have no specific payload.
// Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:26 GenericBuildEvent
#[derive(Clone, Debug)]
pub struct GenericBuildEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
}

impl GenericBuildEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:30 GenericBuildEvent.<init>
    pub fn new(id: BuildEventId, children: Vec<BuildEventId>) -> Self {
        GenericBuildEvent { id, children }
    }
}

impl BuildEvent for GenericBuildEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:36 GenericBuildEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:41 GenericBuildEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/GenericBuildEvent.java:56 GenericBuildEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining(self))
    }
}

/// A build event reporting progress (stdout/stderr) in the stream.
// Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:29 ProgressEvent
#[derive(Clone, Debug)]
pub struct ProgressEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
    out: Option<String>,
    err: Option<String>,
}

impl ProgressEvent {
    /// `ProgressEvent.INITIAL_PROGRESS_UPDATE`.
    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:35 ProgressEvent.INITIAL_PROGRESS_UPDATE
    pub fn initial_progress_update() -> BuildEventId {
        build_event_id_util::progress_id(0)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:37 ProgressEvent.<init>
    fn new(
        id: BuildEventId,
        children: Vec<BuildEventId>,
        out: Option<String>,
        err: Option<String>,
    ) -> Self {
        ProgressEvent {
            id,
            children,
            out,
            err,
        }
    }

    /// Create a regular progress update with the given running number.
    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:57 ProgressEvent.progressUpdate
    pub fn progress_update(
        number: i32,
        out: Option<String>,
        err: Option<String>,
    ) -> Arc<dyn BuildEvent> {
        let id = build_event_id_util::progress_id(number);
        let next = build_event_id_util::progress_id(number + 1);
        Arc::new(ProgressEvent::new(id, vec![next], out, err))
    }

    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:57 ProgressEvent.progressUpdate
    pub fn progress_update_empty(number: i32) -> Arc<dyn BuildEvent> {
        Self::progress_update(number, None, None)
    }

    /// Create a progress update event also chaining in a given id.
    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:68 ProgressEvent.progressChainIn
    pub fn progress_chain_in(
        number: i32,
        chain_in: BuildEventId,
        out: Option<String>,
        err: Option<String>,
    ) -> Arc<dyn BuildEvent> {
        let id = build_event_id_util::progress_id(number);
        let next = build_event_id_util::progress_id(number + 1);
        Arc::new(ProgressEvent::new(id, vec![next, chain_in], out, err))
    }

    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:68 ProgressEvent.progressChainIn
    pub fn progress_chain_in_empty(number: i32, chain_in: BuildEventId) -> Arc<dyn BuildEvent> {
        Self::progress_chain_in(number, chain_in, None, None)
    }

    /// A progress update event with no further progress events announced.
    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:83 ProgressEvent.finalProgressUpdate
    pub fn final_progress_update(
        number: i32,
        out: Option<String>,
        err: Option<String>,
    ) -> Arc<dyn BuildEvent> {
        let id = build_event_id_util::progress_id(number);
        Arc::new(ProgressEvent::new(id, Vec::new(), out, err))
    }

    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:83 ProgressEvent.finalProgressUpdate
    pub fn final_progress_update_empty(number: i32) -> Arc<dyn BuildEvent> {
        Self::final_progress_update(number, None, None)
    }
}

impl BuildEvent for ProgressEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/ProgressEvent.java:45 ProgressEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut builder = bes::Progress::default();
        if let Some(out) = &self.out {
            builder.stdout = out.clone();
        }
        if let Some(err) = &self.err {
            builder.stderr = err.clone();
        }
        Ok(proto_chaining_with(self, Payload::Progress(builder)))
    }
}

/// A build event reporting that an expected event will never be posted.
// Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:26 AbortedEvent
#[derive(Clone, Debug)]
pub struct AbortedEvent {
    id: BuildEventId,
    children: Vec<BuildEventId>,
    reason: AbortReason,
    description: String,
    label: Option<Label>,
}

impl AbortedEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:32 AbortedEvent.<init>
    pub fn new(id: BuildEventId, reason: AbortReason, description: String) -> Self {
        Self::with_label(id, reason, description, None)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:32 AbortedEvent.<init>
    pub fn with_label(
        id: BuildEventId,
        reason: AbortReason,
        description: String,
        label: Option<Label>,
    ) -> Self {
        Self::create(id, Vec::new(), reason, description, label)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:32 AbortedEvent.<init>
    pub fn with_children(
        id: BuildEventId,
        children: Vec<BuildEventId>,
        reason: AbortReason,
        description: String,
    ) -> Self {
        Self::create(id, children, reason, description, None)
    }

    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:32 AbortedEvent.<init>
    fn create(
        id: BuildEventId,
        children: Vec<BuildEventId>,
        reason: AbortReason,
        description: String,
        label: Option<Label>,
    ) -> Self {
        AbortedEvent {
            id,
            children,
            reason,
            description,
            label,
        }
    }

    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:59 AbortedEvent.getLabel
    pub fn get_label(&self) -> Option<&Label> {
        self.label.as_ref()
    }
}

impl BuildEvent for AbortedEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.id.clone()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.children.clone()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/AbortedEvent.java:64 AbortedEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::Aborted(bes::Aborted {
                reason: self.reason as i32,
                description: self.description.clone(),
            }),
        ))
    }
}

/// Wrapper class for a build event marking it as the final event in the protocol.
// Java: com/google/devtools/build/lib/buildeventstream/LastBuildEvent.java:22 LastBuildEvent
#[derive(Clone, Debug)]
pub struct LastBuildEvent {
    event: Arc<dyn BuildEvent>,
}

impl LastBuildEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/LastBuildEvent.java:25 LastBuildEvent.<init>
    pub fn new(event: Arc<dyn BuildEvent>) -> Self {
        LastBuildEvent { event }
    }
}

impl BuildEvent for LastBuildEvent {
    fn get_event_id(&self) -> BuildEventId {
        self.event.get_event_id()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        self.event.get_children_events()
    }
    fn referenced_local_files(&self) -> Vec<LocalFile> {
        self.event.referenced_local_files()
    }
    fn remote_uploads(&self) -> Vec<Option<String>> {
        self.event.remote_uploads()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/LastBuildEvent.java:50 LastBuildEvent.asStreamProto
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let mut proto = self.event.as_stream_proto(converters)?;
        proto.last_message = true;
        Ok(proto)
    }
}

/// Class to represent the trivial configuration.
// Java: com/google/devtools/build/lib/buildeventstream/NullConfiguration.java:25 NullConfiguration
#[derive(Clone, Copy, Debug)]
pub struct NullConfiguration;

impl NullConfiguration {
    // Java: com/google/devtools/build/lib/buildeventstream/NullConfiguration.java:26 NullConfiguration.INSTANCE
    pub fn instance() -> Arc<dyn BuildEvent> {
        Arc::new(NullConfiguration)
    }
}

impl BuildEvent for NullConfiguration {
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::null_configuration_id()
    }
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }
    // Java: com/google/devtools/build/lib/buildeventstream/NullConfiguration.java:39 NullConfiguration.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::Configuration(bes::Configuration::default()),
        ))
    }
}
