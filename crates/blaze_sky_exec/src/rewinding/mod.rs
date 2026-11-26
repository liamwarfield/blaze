//! Port of `lib/skyframe/rewinding`: action rewinding (`--rewind_lost_inputs`), which recovers
//! from inputs lost by a remote cache by rewinding the Skyframe nodes that generated them.

pub mod action_rewind_exception;
pub mod action_rewind_strategy;
pub mod action_rewound_event;
pub mod rewindable_graph_inconsistency_receiver;
