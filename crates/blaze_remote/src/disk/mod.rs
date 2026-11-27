//! Port of `com.google.devtools.build.lib.remote.disk`.

pub mod disk_cache_client;
pub mod disk_cache_garbage_collector;
pub mod disk_cache_garbage_collector_idle_task;

pub use disk_cache_client::DiskCacheClient;
pub use disk_cache_garbage_collector::{
    CollectionPolicy, CollectionStats, DiskCacheGarbageCollector, GcError,
};
pub use disk_cache_garbage_collector_idle_task::DiskCacheGarbageCollectorIdleTask;
