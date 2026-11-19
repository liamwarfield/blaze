//! Ports from `lib/metrics/criticalpath`: the critical path of the executed actions, and
//! `buildtool/buildevent/CriticalPathEvent`.

pub mod aggregated_critical_path;
pub mod critical_path_component;
pub mod critical_path_computer;

use blaze_events::Postable;

pub use aggregated_critical_path::AggregatedCriticalPath;
pub use critical_path_component::CriticalPathComponent;
pub use critical_path_computer::{CanBeShared, CriticalPathComputer};

/// Event triggered after the critical path is computed.
// Java: com/google/devtools/build/lib/buildtool/buildevent/CriticalPathEvent.java:20 CriticalPathEvent
#[derive(Clone, Debug)]
pub struct CriticalPathEvent {
    critical_path: AggregatedCriticalPath,
}

impl CriticalPathEvent {
    // Java: com/google/devtools/build/lib/buildtool/buildevent/CriticalPathEvent.java:24 CriticalPathEvent.<init>
    pub fn new(critical_path: AggregatedCriticalPath) -> CriticalPathEvent {
        CriticalPathEvent { critical_path }
    }

    // Java: com/google/devtools/build/lib/buildtool/buildevent/CriticalPathEvent.java:28 CriticalPathEvent.getCriticalPath
    pub fn get_critical_path(&self) -> &AggregatedCriticalPath {
        &self.critical_path
    }
}

impl Postable for CriticalPathEvent {}
