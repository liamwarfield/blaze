//! Port of `lib/remote/common/RemoteActionExecutionContext.java`.

use std::sync::Arc;

use blaze_exec::actions::{ActionExecutionMetadata, Spawn};
use blaze_exec::exec::SpawnExecutionContext;

use super::network_time::NetworkTime;
use crate::reapi::RequestMetadata;

/// Determines whether to read/write remote cache, disk cache or both.
// Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:32 RemoteActionExecutionContext.CachePolicy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CachePolicy {
    /// `NO_CACHE`.
    NoCache,
    /// `REMOTE_CACHE_ONLY`.
    RemoteCacheOnly,
    /// `DISK_CACHE_ONLY`.
    DiskCacheOnly,
    /// `ANY_CACHE`.
    AnyCache,
}

impl CachePolicy {
    /// `CachePolicy.create(allowRemoteCache, allowDiskCache)`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:38 CachePolicy.create
    pub fn create(allow_remote_cache: bool, allow_disk_cache: bool) -> CachePolicy {
        if allow_remote_cache && allow_disk_cache {
            CachePolicy::AnyCache
        } else if allow_remote_cache {
            CachePolicy::RemoteCacheOnly
        } else if allow_disk_cache {
            CachePolicy::DiskCacheOnly
        } else {
            CachePolicy::NoCache
        }
    }

    /// `allowAnyCache()`.
    pub fn allow_any_cache(self) -> bool {
        self != CachePolicy::NoCache
    }

    /// `allowRemoteCache()`.
    pub fn allow_remote_cache(self) -> bool {
        self == CachePolicy::RemoteCacheOnly || self == CachePolicy::AnyCache
    }

    /// `allowDiskCache()`.
    pub fn allow_disk_cache(self) -> bool {
        self == CachePolicy::DiskCacheOnly || self == CachePolicy::AnyCache
    }

    /// `addRemoteCache()`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:62 CachePolicy.addRemoteCache
    pub fn add_remote_cache(self) -> CachePolicy {
        if self == CachePolicy::DiskCacheOnly || self == CachePolicy::AnyCache {
            return CachePolicy::AnyCache;
        }
        CachePolicy::RemoteCacheOnly
    }
}

/// A context providing remote execution related information for executing a `RemoteAction`.
///
/// Terminology note: "action" is used here in the remote execution protocol sense, which is
/// equivalent to a Bazel "spawn" (a Bazel "action" being a higher-level concept).
///
/// PORT: the spawn and its execution context are borrowed for `'a` (Java holds references);
/// [`RemoteActionExecutionContext::detached`] gives the `'static` copy that background tasks
/// (uploads that outlive the spawn's execution) keep.
// Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:30 RemoteActionExecutionContext
#[derive(Clone)]
pub struct RemoteActionExecutionContext<'a> {
    spawn: Option<&'a dyn Spawn>,
    spawn_execution_context: Option<&'a dyn SpawnExecutionContext>,
    request_metadata: RequestMetadata,
    network_time: Arc<NetworkTime>,
    write_cache_policy: CachePolicy,
    read_cache_policy: CachePolicy,
}

impl std::fmt::Debug for RemoteActionExecutionContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionExecutionContext")
            .field("request_metadata", &self.request_metadata)
            .field("write_cache_policy", &self.write_cache_policy)
            .field("read_cache_policy", &self.read_cache_policy)
            .finish_non_exhaustive()
    }
}

impl<'a> RemoteActionExecutionContext<'a> {
    /// `withWriteCachePolicy(writeCachePolicy)`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:107 RemoteActionExecutionContext.withWriteCachePolicy
    pub fn with_write_cache_policy(&self, write_cache_policy: CachePolicy) -> Self {
        let mut c = self.clone();
        c.write_cache_policy = write_cache_policy;
        c
    }

    /// `withReadCachePolicy(readCachePolicy)`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:117 RemoteActionExecutionContext.withReadCachePolicy
    pub fn with_read_cache_policy(&self, read_cache_policy: CachePolicy) -> Self {
        let mut c = self.clone();
        c.read_cache_policy = read_cache_policy;
        c
    }

    /// Returns the `Spawn` of the `RemoteAction` being executed, or `None` if it has no
    /// associated `Spawn`.
    pub fn get_spawn(&self) -> Option<&'a dyn Spawn> {
        self.spawn
    }

    /// Returns the `SpawnExecutionContext` of the `RemoteAction` being executed, or `None` if it
    /// has no associated `Spawn`.
    pub fn get_spawn_execution_context(&self) -> Option<&'a dyn SpawnExecutionContext> {
        self.spawn_execution_context
    }

    /// Returns the `RequestMetadata` for the action being executed.
    pub fn get_request_metadata(&self) -> &RequestMetadata {
        &self.request_metadata
    }

    /// Returns the `NetworkTime` instance used to measure the network time during the action
    /// execution.
    pub fn get_network_time(&self) -> &Arc<NetworkTime> {
        &self.network_time
    }

    /// `getSpawnOwner()`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:162 RemoteActionExecutionContext.getSpawnOwner
    pub fn get_spawn_owner(&self) -> Option<&'a dyn ActionExecutionMetadata> {
        self.spawn.map(|s| s.get_resource_owner())
    }

    /// `getWriteCachePolicy()`.
    pub fn get_write_cache_policy(&self) -> CachePolicy {
        self.write_cache_policy
    }

    /// `getReadCachePolicy()`.
    pub fn get_read_cache_policy(&self) -> CachePolicy {
        self.read_cache_policy
    }

    /// Creates a `RemoteActionExecutionContext` with given `RequestMetadata`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:179 RemoteActionExecutionContext.create
    pub fn create(metadata: RequestMetadata) -> RemoteActionExecutionContext<'static> {
        RemoteActionExecutionContext {
            spawn: None,
            spawn_execution_context: None,
            request_metadata: metadata,
            network_time: Arc::new(NetworkTime::new()),
            write_cache_policy: CachePolicy::AnyCache,
            read_cache_policy: CachePolicy::AnyCache,
        }
    }

    /// Creates a `RemoteActionExecutionContext` with given `Spawn` and `RequestMetadata`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:188 RemoteActionExecutionContext.create
    pub fn create_for_spawn(
        spawn: &'a dyn Spawn,
        spawn_execution_context: &'a dyn SpawnExecutionContext,
        metadata: RequestMetadata,
    ) -> RemoteActionExecutionContext<'a> {
        RemoteActionExecutionContext::create_with_policies(
            spawn,
            spawn_execution_context,
            metadata,
            CachePolicy::AnyCache,
            CachePolicy::AnyCache,
        )
    }

    /// `create(spawn, spawnExecutionContext, requestMetadata, writeCachePolicy, readCachePolicy)`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteActionExecutionContext.java:194 RemoteActionExecutionContext.create
    pub fn create_with_policies(
        spawn: &'a dyn Spawn,
        spawn_execution_context: &'a dyn SpawnExecutionContext,
        request_metadata: RequestMetadata,
        write_cache_policy: CachePolicy,
        read_cache_policy: CachePolicy,
    ) -> RemoteActionExecutionContext<'a> {
        RemoteActionExecutionContext {
            spawn: Some(spawn),
            spawn_execution_context: Some(spawn_execution_context),
            request_metadata,
            network_time: Arc::new(NetworkTime::new()),
            write_cache_policy,
            read_cache_policy,
        }
    }

    /// The same context without the borrowed spawn (same metadata, network time and policies),
    /// for work that outlives the spawn's execution.
    pub fn detached(&self) -> RemoteActionExecutionContext<'static> {
        RemoteActionExecutionContext {
            spawn: None,
            spawn_execution_context: None,
            request_metadata: self.request_metadata.clone(),
            network_time: self.network_time.clone(),
            write_cache_policy: self.write_cache_policy,
            read_cache_policy: self.read_cache_policy,
        }
    }
}
