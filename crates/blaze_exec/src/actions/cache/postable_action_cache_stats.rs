//! Port of `lib/actions/cache/PostableActionCacheStats.java`.

use blaze_events::Postable;

use super::action_cache::ActionCacheStatistics;

/// `Postable` wrapper around [`ActionCacheStatistics`].
// Java: com/google/devtools/build/lib/actions/cache/PostableActionCacheStats.java:21 PostableActionCacheStats
#[derive(Debug, Clone)]
pub struct PostableActionCacheStats {
    action_cache_statistics: ActionCacheStatistics,
}

impl PostableActionCacheStats {
    // Java: com/google/devtools/build/lib/actions/cache/PostableActionCacheStats.java:24 PostableActionCacheStats.<init>
    pub fn new(action_cache_statistics: ActionCacheStatistics) -> PostableActionCacheStats {
        PostableActionCacheStats {
            action_cache_statistics,
        }
    }

    // Java: com/google/devtools/build/lib/actions/cache/PostableActionCacheStats.java:28 PostableActionCacheStats.asProto
    pub fn as_proto(&self) -> &ActionCacheStatistics {
        &self.action_cache_statistics
    }
}

impl Postable for PostableActionCacheStats {}
