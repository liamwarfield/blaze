//! Ports from `lib/runtime`: the streamer, the artifact group namer and the runtime's build
//! events.

pub mod build_event_streamer;
pub mod command_line_event;
pub mod counting_artifact_group_namer;
pub mod got_options_event;
pub mod named_artifact_group;
pub mod runtime_events;
pub mod test_summary_event;

pub use build_event_streamer::{
    BuildEventStreamer, Builder as BuildEventStreamerJavaBuilder, OutErrProvider,
    ReleaseReplaceableBuildEvent,
};
pub use counting_artifact_group_namer::CountingArtifactGroupNamer;
pub use named_artifact_group::{ArtifactSet, ExpandedArtifact, NamedArtifactGroup};
