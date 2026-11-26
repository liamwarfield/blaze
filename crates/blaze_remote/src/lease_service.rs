//! Port of `lib/remote/LeaseService.java`.

use std::sync::atomic::{AtomicBool, Ordering};

use blaze_exec::actions::cache::ActionCache;

/// A lease service that manages the lease of remote blobs.
///
/// PORT: `memoizingEvaluator` is the caller's: [`LeaseService::finalize_execution`] gets
/// `memoizingEvaluator.delete(...)` of the Skyframe values with remote metadata as a callback
/// (the predicate needs `ActionExecutionValue`, which this crate does not know). The lease
/// extension (`--experimental_remote_cache_lease_extension`, `RemoteLeaseExtension`) is not
/// ported and fails loudly when requested.
// Java: com/google/devtools/build/lib/remote/LeaseService.java:30 LeaseService
pub struct LeaseService {
    lease_extension_started: AtomicBool,
    has_missing_action_inputs: AtomicBool,
}

impl LeaseService {
    /// `new LeaseService(memoizingEvaluator, actionCacheSupplier, leaseExtension)`;
    /// `lease_extension` is whether `RemoteModule.executorInit` created a `RemoteLeaseExtension`
    /// (`--experimental_remote_cache_lease_extension`).
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:37 LeaseService.<init>
    pub fn new(lease_extension: bool) -> LeaseService {
        if lease_extension {
            blaze_util::unported!(
                "RemoteLeaseExtension",
                "--experimental_remote_cache_lease_extension"
            );
        }
        LeaseService {
            lease_extension_started: AtomicBool::new(false),
            has_missing_action_inputs: AtomicBool::new(false),
        }
    }

    /// `finalizeAction()`: starts the lease extension once (there is none).
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:46 LeaseService.finalizeAction
    pub fn finalize_action(&self) {
        // if (leaseExtension != null) leaseExtension.start(): no lease extension.
        let _ = self.lease_extension_started.compare_exchange(
            false,
            true,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    /// `onLostInputs(LostInputsEvent event)`.
    // PORT: @Subscribe. The executor's lost-inputs handler calls this.
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:54 LeaseService.onLostInputs
    pub fn on_lost_inputs(&self) {
        self.has_missing_action_inputs.store(true, Ordering::SeqCst);
    }

    /// `finalizeExecution()` (on `ExecutionPhaseCompleteEvent`): after a build that lost inputs,
    /// forgets the remote metadata of the local action cache.
    ///
    /// PORT: Java runs this after `ExecutionTool` saved the action cache and removes the
    /// entries from the server's in-memory cache, which the next command (a retry of the
    /// build, too) uses; Blaze loads the action cache from disk for each command, so the caller
    /// runs this before the action cache is saved.
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:60 LeaseService.finalizeExecution
    ///
    /// `delete_remote_action_execution_values` is `memoizingEvaluator.delete(key ->
    /// ACTION_EXECUTION && isRemote(value))`.
    pub fn finalize_execution(
        &self,
        action_cache: Option<&dyn ActionCache>,
        delete_remote_action_execution_values: &dyn Fn(),
    ) {
        // if (leaseExtension != null) leaseExtension.stop(): no lease extension.
        if self.has_missing_action_inputs.swap(false, Ordering::SeqCst) {
            self.handle_missing_inputs(action_cache, delete_remote_action_execution_values);
        }
    }

    /// Clean up internal state when files are evicted from remote CAS.
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:79 LeaseService.handleMissingInputs
    fn handle_missing_inputs(
        &self,
        action_cache: Option<&dyn ActionCache>,
        delete_remote_action_execution_values: &dyn Fn(),
    ) {
        // If any outputs are evicted, remove all remote metadata from skyframe and local action
        // cache.
        //
        // With TTL based discarding and lease extension, remote cache eviction error won't happen
        // if remote cache can guarantee the TTL. However, if it happens, it usually means the
        // remote cache is under high load and it could possibly evict more blobs that Bazel
        // wouldn't aware of. Following builds could still fail for the same error (caused by
        // different blobs).
        delete_remote_action_execution_values();

        if let Some(action_cache) = action_cache {
            action_cache.remove_if(&|entry| {
                !entry.get_output_files().is_empty() || !entry.get_output_trees().is_empty()
            });
        }
    }
}
