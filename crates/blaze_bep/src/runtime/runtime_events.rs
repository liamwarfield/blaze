//! Ports of build events from `lib/runtime`: `BuildMetadataEvent` and `ExecRootEvent`.

use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::{self as bes, BuildEventId};
use blaze_vfs::Path;
use indexmap::IndexMap;

use crate::buildeventstream::build_event::{BuildEvent, Interrupted};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;

/// A build event reporting data from the `--build_metadata` flag.
// Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:31 BuildMetadataEvent
#[derive(Clone, Debug)]
pub struct BuildMetadataEvent {
    build_metadata: IndexMap<String, String>,
}

impl BuildMetadataEvent {
    // Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:40 BuildMetadataEvent.<init>
    pub fn new(build_metadata: IndexMap<String, String>) -> Self {
        BuildMetadataEvent { build_metadata }
    }
}

impl BuildEvent for BuildMetadataEvent {
    // Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:45 BuildMetadataEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_metadata_id()
    }

    // Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:50 BuildMetadataEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:55 BuildMetadataEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        let mut metadata_builder = bes::BuildMetadata::default();
        for (key, value) in &self.build_metadata {
            // putMetadata: a later put replaces the value in place.
            match metadata_builder.metadata.iter_mut().find(|e| &e.key == key) {
                Some(e) => e.value = value.clone(),
                None => metadata_builder
                    .metadata
                    .push(bes::build_metadata::MetadataEntry {
                        key: key.clone(),
                        value: value.clone(),
                    }),
            }
        }
        Ok(proto_chaining_with(
            self,
            Payload::BuildMetadata(metadata_builder),
        ))
    }

    // Java: com/google/devtools/build/lib/runtime/BuildMetadataEvent.java:65 BuildMetadataEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_started_id()])
    }
}

impl blaze_events::Postable for BuildMetadataEvent {}

/// An event that informs about the execution root.
// Java: com/google/devtools/build/lib/runtime/ExecRootEvent.java:27 ExecRootEvent
#[derive(Clone, Debug)]
pub struct ExecRootEvent {
    exec_root: Path,
}

impl ExecRootEvent {
    // Java: com/google/devtools/build/lib/runtime/ExecRootEvent.java:31 ExecRootEvent.<init>
    pub fn new(exec_root: Path) -> Self {
        ExecRootEvent { exec_root }
    }
}

impl BuildEvent for ExecRootEvent {
    // Java: com/google/devtools/build/lib/runtime/ExecRootEvent.java:48 ExecRootEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::workspace_config_id()
    }

    // Java: com/google/devtools/build/lib/runtime/ExecRootEvent.java:53 ExecRootEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/ExecRootEvent.java:36 ExecRootEvent.asStreamProto
    fn as_stream_proto(&self, _: &dyn BuildEventContext) -> Result<bes::BuildEvent, Interrupted> {
        // Java builds the event without protoChaining (no children).
        Ok(bes::BuildEvent {
            id: Some(self.get_event_id()),
            payload: Some(Payload::WorkspaceInfo(bes::WorkspaceConfig {
                local_exec_root: self.exec_root.get_path_string().to_string(),
            })),
            ..Default::default()
        })
    }
}

impl blaze_events::Postable for ExecRootEvent {}
