//! Port of `lib/remote/LeaseService.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_exec::actions::cache::ActionCache;

/// A lease service that manages the lease of remote blobs.
///
/// PORT: `memoizingEvaluator` is the caller's: [`LeaseService::finalize_execution`] gets
/// `memoizingEvaluator.delete(...)` of the Skyframe values with remote metadata as a callback
/// (the predicate needs `ActionExecutionValue`, which this crate does not know).
// Java: com/google/devtools/build/lib/remote/LeaseService.java:30 LeaseService
pub struct LeaseService {
    lease_extension_started: AtomicBool,
    /// `leaseExtension`.
    ///
    /// PORT: Java creates it with the service in `executorInit`; Blaze creates the
    /// `SkyframeExecutor` and loads the action cache it needs later, so it is set once those
    /// exist ([`LeaseService::set_lease_extension`]), before the first action is finalized.
    lease_extension: std::sync::OnceLock<Arc<dyn LeaseExtension>>,
    has_missing_action_inputs: AtomicBool,
}

/// An interface whose implementations extend the leases of remote outputs referenced by
/// skyframe.
// Java: com/google/devtools/build/lib/remote/LeaseService.java:73 LeaseService.LeaseExtension
pub trait LeaseExtension: Send + Sync {
    fn start(&self);

    fn stop(&self);
}

impl LeaseService {
    /// `new LeaseService(memoizingEvaluator, actionCacheSupplier, leaseExtension)`.
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:37 LeaseService.<init>
    pub fn new(lease_extension: Option<Arc<dyn LeaseExtension>>) -> LeaseService {
        let slot = std::sync::OnceLock::new();
        if let Some(lease_extension) = lease_extension {
            let _ = slot.set(lease_extension);
        }
        LeaseService {
            lease_extension_started: AtomicBool::new(false),
            lease_extension: slot,
            has_missing_action_inputs: AtomicBool::new(false),
        }
    }

    /// Sets the lease extension of a service created without one (see `lease_extension`).
    pub fn set_lease_extension(&self, lease_extension: Arc<dyn LeaseExtension>) {
        if self.lease_extension.set(lease_extension).is_err() {
            panic!("IllegalStateException: the lease extension is already set");
        }
    }

    /// `finalizeAction()`: starts the lease extension once.
    // Java: com/google/devtools/build/lib/remote/LeaseService.java:46 LeaseService.finalizeAction
    pub fn finalize_action(&self) {
        if self
            .lease_extension_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            && let Some(lease_extension) = self.lease_extension.get()
        {
            lease_extension.start();
        }
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
        if let Some(lease_extension) = self.lease_extension.get() {
            lease_extension.stop();
        }
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
