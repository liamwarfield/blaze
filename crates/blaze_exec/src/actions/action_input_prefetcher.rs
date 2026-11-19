//! Port of `lib/actions/ActionInputPrefetcher.java`, with the two other entry points the remote
//! module's prefetcher (`RemoteActionInputFetcher`, in `blaze_remote`) is reached through from
//! the execution layer: `OutputService.finalizeAction` (`RemoteOutputService` delegates it to
//! the fetcher) and the `BulkTransferException` handler of
//! `SpawnExecutionContextImpl.prefetchInputs`, whose exception type lives in `blaze_remote`.

use std::io;

use crate::actions::action_execution_metadata::ActionExecutionMetadata;
use crate::actions::action_input::ActionInputRef;
use crate::actions::cache::OutputMetadataStore;
use crate::actions::exec_exception::ExecException;
use crate::actions::input_metadata_provider::InputMetadataProvider;

/// Priority for the staging task.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:31 ActionInputPrefetcher.Priority
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Critical priority tasks are tasks that are critical to the execution time e.g. staging
    /// files for in-process actions.
    CRITICAL,
    /// High priority tasks are tasks that may have impact on the execution time e.g. staging
    /// outputs that are inputs to local actions which will be executed later.
    HIGH,
    /// Medium priority tasks are tasks that may or may not have the impact on the execution time
    /// e.g. staging inputs for local branch of dynamically scheduled actions.
    MEDIUM,
    /// Low priority tasks are tasks that don't have impact on the execution time e.g. staging
    /// outputs of toplevel targets/aspects.
    LOW,
}

/// Why the files are staged.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:57 ActionInputPrefetcher.Reason
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    INPUTS,
    OUTPUTS,
}

/// Prefetches files to local disk.
///
/// PORT: Java's `prefetchFiles` returns a future; every Blaze caller waits for it at once
/// (`prefetchInputsAndWait`, `getFromFuture`), so the method blocks.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:24 ActionInputPrefetcher
pub trait ActionInputPrefetcher: Send + Sync {
    /// Initiates best-effort prefetching of all given inputs and waits for it.
    ///
    /// Inputs must be expanded (no tree artifacts or runfiles trees).
    // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:72 ActionInputPrefetcher.prefetchFiles
    fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()>;

    /// The `catchingAsync(BulkTransferException.class, ...)` of
    /// `SpawnExecutionContextImpl.prefetchInputs`: `Some` with the exception to throw when `e` is
    /// a `BulkTransferException`, `None` for any other `IOException`.
    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:274 SpawnExecutionContextImpl.prefetchInputs
    fn bulk_transfer_exception_to_exec_exception(
        &self,
        e: &io::Error,
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Option<ExecException>;

    /// `OutputService.finalizeAction(action, outputMetadataStore)` of the output service that
    /// owns this prefetcher (`RemoteOutputService.finalizeAction`: the action input fetcher's
    /// `finalizeAction`). `outputs` is `action.getOutputs()`.
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:178 OutputService.finalizeAction
    fn finalize_action(
        &self,
        action: &dyn ActionExecutionMetadata,
        outputs: &[ActionInputRef],
        output_metadata_store: &dyn OutputMetadataStore,
    ) -> io::Result<()>;
}
