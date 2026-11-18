//! Build events whose Java classes live outside `lib/buildeventstream` and `lib/runtime`
//! (`lib/analysis`, `lib/buildtool/buildevent`, `lib/pkgcache`, `lib/actions`).

pub mod actions_events;
pub mod analysis_events;
pub mod buildtool_events;
pub mod completion_context;
pub mod pkgcache_events;
pub mod target_complete_event;
