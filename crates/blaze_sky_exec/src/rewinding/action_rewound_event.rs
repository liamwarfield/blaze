//! Ports of `lib/skyframe/rewinding/ActionRewoundEvent.java`,
//! `PostableActionRewindingStats.java` and the messages of `skyframe/proto/action_rewind_event.proto`.

use std::fmt;
use std::time::Instant;

use blaze_actions::actions::action::ActionRef;
use blaze_events::Postable;

/// This event is fired during the build for an action that failed because of lost inputs, and
/// that will try to recover through rewinding (see `ActionRewindStrategy`).
// Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewoundEvent.java:23 ActionRewoundEvent
#[derive(Clone)]
pub struct ActionRewoundEvent {
    /// A nanotime taken before action execution began.
    pub relative_action_start_time: Instant,
    /// A nanotime taken after action execution finished.
    pub relative_action_finish_time: Instant,
    /// The failed action.
    pub failed_rewound_action: ActionRef,
}

impl fmt::Debug for ActionRewoundEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActionRewoundEvent")
    }
}

impl Postable for ActionRewoundEvent {}

/// `skyframe.ActionDescription`.
// Java: com/google/devtools/build/lib/skyframe/proto/action_rewind_event.proto:49 ActionDescription
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionDescription {
    pub r#type: String,
    pub rule_label: String,
}

/// `skyframe.LostInput`.
// Java: com/google/devtools/build/lib/skyframe/proto/action_rewind_event.proto:41 LostInput
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LostInput {
    pub path: String,
    pub digest: String,
}

/// `skyframe.ActionRewindEvent`: a sample of an action rewinding (`oneof` of the action and the
/// top-level key description).
// Java: com/google/devtools/build/lib/skyframe/proto/action_rewind_event.proto:23 ActionRewindEvent
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionRewindEvent {
    pub action_description: Option<ActionDescription>,
    pub top_level_action_lookup_key_description: Option<String>,
    pub lost_inputs: Vec<LostInput>,
    pub total_lost_inputs_count: i32,
    pub invalidated_nodes_count: i32,
}

/// Event that encapsulates data about action rewinding during a build.
///
/// PORT: Bazel has no subscriber for it (Google-internal logging).
// Java: com/google/devtools/build/lib/skyframe/rewinding/PostableActionRewindingStats.java:24 PostableActionRewindingStats
#[derive(Clone, Debug)]
pub struct PostableActionRewindingStats {
    pub lost_inputs_count: usize,
    pub lost_outputs_count: usize,
    pub rewound_action_count: usize,
    pub same_action_lost_inputs_count: usize,
    pub same_top_level_key_lost_outputs_count: usize,
    pub action_rewind_events: Vec<ActionRewindEvent>,
}

impl Postable for PostableActionRewindingStats {}
