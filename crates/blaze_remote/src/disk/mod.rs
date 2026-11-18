//! Port of `com.google.devtools.build.lib.remote.disk`.
//!
//! PORT: `DiskCacheGarbageCollectorIdleTask` is not ported. It schedules a
//! [`DiskCacheGarbageCollector`] run while the server is idle (`IdleTask`,
//! `IdleTaskException`), and Blaze has no server idle loop yet.

pub mod disk_cache_client;
pub mod disk_cache_garbage_collector;

pub use disk_cache_client::DiskCacheClient;
pub use disk_cache_garbage_collector::{
    CollectionPolicy, CollectionStats, DiskCacheGarbageCollector,
};
