//! Port of `lib/remote/RemoteActionInputFetcher.java`.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use blaze_events::Reporter;
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::actions::cache::OutputPermissions;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, FileArtifactValue, FileStateType,
    InputMetadataProvider, VirtualActionInput,
};
use blaze_vfs::{Path, PathFragment, Symlinks};

use crate::abstract_action_input_prefetcher::{
    AbstractActionInputPrefetcher, AbstractActionInputPrefetcherBase, ActionInputPrefetcher,
    Priority, Reason,
};
use crate::combined_cache::{CombinedCache, DownloadProgressReporter};
use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::errors;
use crate::common::progress_status_listener::no_action;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::util::digest_util::DigestUtil;
use crate::util::temp_path_generator::TempPathGenerator;
use crate::util::tracing_metadata_utils;

/// Stages output files that are stored remotely to the local filesystem.
///
/// This is used to ensure that the inputs to a local action are present, even when they are
/// provided by a remote action when building without the bytes, or by an external repository
/// when building with a remote repository cache enabled.
// Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:52 RemoteActionInputFetcher
pub struct RemoteActionInputFetcher {
    base: AbstractActionInputPrefetcherBase,
    build_request_id: String,
    command_id: String,
    combined_cache: Arc<CombinedCache>,
    // PORT: `ConcurrentArtifactPathTrie rewoundActionOutputs` is only filled by
    // `handleRewoundActionOutputs`, i.e. by action rewinding, which is not ported: the trie is
    // always empty (see `rewound_action_outputs_contains`).
}

impl std::fmt::Debug for RemoteActionInputFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionInputFetcher")
            .field("base", &self.base)
            .field("build_request_id", &self.build_request_id)
            .field("command_id", &self.command_id)
            .finish_non_exhaustive()
    }
}

impl RemoteActionInputFetcher {
    /// `new RemoteActionInputFetcher(reporter, buildRequestId, commandId, combinedCache,
    /// execRoot, tempPathGenerator, remoteOutputChecker, outputDirectoryHelper,
    /// outputPermissions)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:59 RemoteActionInputFetcher.<init>
    pub fn new(
        reporter: Arc<Reporter>,
        build_request_id: &str,
        command_id: &str,
        combined_cache: Arc<CombinedCache>,
        exec_root: Path,
        temp_path_generator: Arc<TempPathGenerator>,
        remote_output_checker: Arc<RemoteOutputChecker>,
        output_directory_helper: Option<Arc<ActionOutputDirectoryHelper>>,
        output_permissions: OutputPermissions,
    ) -> RemoteActionInputFetcher {
        RemoteActionInputFetcher {
            base: AbstractActionInputPrefetcherBase::new(
                reporter,
                exec_root,
                temp_path_generator,
                remote_output_checker,
                output_directory_helper,
                output_permissions,
            ),
            // `Preconditions.checkNotNull`: the parameters are not nullable here.
            build_request_id: build_request_id.to_string(),
            command_id: command_id.to_string(),
            combined_cache,
        }
    }

    /// `rewoundActionOutputs.contains(execPath)`.
    ///
    /// PORT: rewinding is not ported; `handleRewoundActionOutputs` (the only writer of the trie)
    /// fails loudly, so the trie is empty and contains nothing.
    // Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:50 ConcurrentArtifactPathTrie.contains
    fn rewound_action_outputs_contains(&self, _exec_path: &PathFragment) -> bool {
        false
    }

    /// PORT: action rewinding is not ported (docs/remote.md, "Not ported is loud"). Java
    /// invalidates the cached creation of the outputs' tree artifact directories in
    /// `outputDirectoryHelper` and records the outputs (the parent of a `TreeFileArtifact`) in
    /// `rewoundActionOutputs`, so that `forceRefetch` makes their downloads and directory
    /// bookkeeping start over.
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:169 RemoteActionInputFetcher.handleRewoundActionOutputs
    pub fn handle_rewound_action_outputs(&self, _outputs: &[ActionInputRef]) {
        blaze_util::unported!(
            "RemoteActionInputFetcher.handleRewoundActionOutputs (action rewinding)"
        )
    }

    /// `this` as the abstract class.
    fn as_abstract(&self) -> &dyn AbstractActionInputPrefetcher {
        self
    }
}

#[async_trait]
impl AbstractActionInputPrefetcher for RemoteActionInputFetcher {
    fn base(&self) -> &AbstractActionInputPrefetcherBase {
        &self.base
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:82 RemoteActionInputFetcher.prefetchVirtualActionInput
    fn prefetch_virtual_action_input(&self, input: &dyn VirtualActionInput) -> io::Result<()> {
        input.atomically_write_relative_to(self.base.exec_root())?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:87 RemoteActionInputFetcher.canDownloadFile
    fn can_download_file(&self, path: &Path, metadata: &FileArtifactValue) -> bool {
        // Only files and directories have remote-only content that can be downloaded.
        if metadata.get_type() != FileStateType::RegularFile
            && metadata.get_type() != FileStateType::Directory
        {
            return false;
        }
        // When action rewinding is enabled, an action that had remote metadata at some point
        // during the build may have been re-executed locally to regenerate lost inputs, but may
        // then be rewound again and thus have its (now local) outputs deleted. In this case, we
        // need to download the outputs again, even if they are now considered local.
        metadata.is_remote() || (self.force_refetch(path) && !path.exists_with(Symlinks::Nofollow))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:101 RemoteActionInputFetcher.forceRefetch
    fn force_refetch(&self, path: &Path) -> bool {
        // Caches for download operations and output directory creation need to be disregarded
        // for the outputs of rewound actions as they may have been deleted after they were first
        // created. Compare as fragments since execRoot may be located on a file system overlaying
        // the host file system where downloads are written to.
        let exec_root_fragment = self.base.exec_root().as_fragment();
        let path_fragment = path.as_fragment();
        path_fragment.starts_with(exec_root_fragment)
            && self.rewound_action_outputs_contains(&path_fragment.relative_to(exec_root_fragment))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:113 RemoteActionInputFetcher.doDownloadFile
    async fn do_download_file(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        _reporter: &Reporter,
        input: &ActionInputRef,
        temp_path: &Path,
        metadata: &FileArtifactValue,
        _priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        let request_metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            match reason {
                Reason::INPUTS => "input",
                Reason::OUTPUTS => "output",
            },
            action,
        );
        let context = RemoteActionExecutionContext::create(request_metadata);

        let digest = DigestUtil::build_digest_from_bytes(
            metadata
                .get_digest()
                .expect("NullPointerException: metadata.getDigest()"),
            metadata.get_size(),
        );

        // `progress -> { if (action != null) { progress.postTo(reporter, action); } }`
        // PORT: missing hook. `SpawnProgressEvent.postTo` (an `ActionProgressEvent`) is not
        // ported in blaze_exec, and the listener of a `DownloadProgressReporter` cannot borrow the
        // action; as in `RemoteExecutionService.downloadOutputs`, the progress of the download is
        // not reported. Nothing else depends on it.
        let reporter = DownloadProgressReporter::new(
            no_action(),
            &input.get_exec_path_string(),
            digest.size_bytes,
        );

        // Treat other download error as CacheNotFoundException so that Bazel can
        // correctly rewind the action/build.
        // Intentionally, do not transform IOExceptions directly thrown by downloadFile rather
        // than in the returned future, as those are likely to be caused by local FS issues.
        // PORT: blaze's `CombinedCache::download_file_with_reporter` is one `async fn`: the
        // synchronous part of Java's `downloadFile` (creating the parent directory of the
        // temporary path, writing an empty file, the `--experimental_remote_downloader` symlink
        // template) fails the same future as the transfer, so those local errors are transformed
        // too.
        // SECURITY: the digest of the downloaded bytes is verified by `CombinedCache` and the
        // cache clients behind it (`--remote_verify_downloads`); the bytes go to `temp_path`, a
        // `TempPathGenerator` path, never to the output path.
        let result = self
            .combined_cache
            .download_file_with_reporter(
                &context,
                &input.get_exec_path_string(),
                Some(input.get_exec_path()),
                &temp_path.for_host_file_system(),
                &digest,
                &reporter,
            )
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(e) if errors::downcast_ref::<CacheNotFoundException>(&e).is_some() => Err(e),
            Err(_e) => {
                // PORT: `cacheNotFoundException.addSuppressed(e)`: an `io::Error` has no
                // suppressed list; the original error is dropped.
                let cache_not_found_exception =
                    CacheNotFoundException::with_exec_path(digest, input.get_exec_path().clone());
                Err(cache_not_found_exception.into())
            }
        }
    }
}

#[async_trait]
impl ActionInputPrefetcher for RemoteActionInputFetcher {
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:325 AbstractActionInputPrefetcher.prefetchFiles
    async fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        self.as_abstract()
            .prefetch_files(action, inputs, metadata_provider, priority, reason)
            .await
    }
}

/// The execution phase's view of the fetcher (`ExecutorBuilder.setActionInputPrefetcher`): the
/// blocking `prefetchFiles(action, inputs, metadataProvider, MEDIUM, INPUTS)` of
/// `SpawnExecutionContext.prefetchInputsAndWait`.
// Java: com/google/devtools/build/lib/exec/SpawnRunner.java:170 SpawnRunner.SpawnExecutionContext.prefetchInputsAndWait
impl blaze_exec::exec::action_execution_context::ActionInputPrefetcher
    for RemoteActionInputFetcher
{
    fn prefetch_files(
        &self,
        action: &dyn ActionExecutionMetadata,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> io::Result<()> {
        crate::abstract_action_input_prefetcher::prefetch_files_blocking(
            self,
            Some(action),
            inputs,
            metadata_provider,
            Priority::MEDIUM,
            Reason::INPUTS,
        )
        .map_err(to_execution_bulk_transfer_error)
    }
}

/// A `BulkTransferException` of a prefetch as the execution layer reads it
/// (`blaze_exec::exec::action_execution_context::BulkTransferError`: its message and the
/// `CacheNotFoundException`s `getLostArtifacts` looks at); any other error is returned as it is.
// Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:83 BulkTransferException.getLostArtifacts
fn to_execution_bulk_transfer_error(e: io::Error) -> io::Error {
    use crate::common::bulk_transfer_exception::BulkTransferException;
    use crate::common::cache_not_found_exception::CacheNotFoundException;
    let Some(bulk) = crate::common::errors::downcast_ref::<BulkTransferException>(&e) else {
        return e;
    };
    let causes = if bulk.all_caused_by_cache_not_found_exception() {
        let mut causes = Vec::new();
        for suppressed in bulk.get_suppressed() {
            let cache_not_found =
                crate::common::errors::downcast_ref::<CacheNotFoundException>(suppressed)
                    .expect("allCausedByCacheNotFoundException");
            if cache_not_found.get_exec_path().is_none() && cache_not_found.get_filename().is_none()
            {
                panic!(
                    "IllegalArgumentException: CacheNotFoundException that may represent a lost artifact should have been annotated with a filename: {cache_not_found}"
                );
            }
            causes.push((
                crate::util::digest_util::DigestUtil::to_string(
                    cache_not_found.get_missing_digest(),
                ),
                cache_not_found.get_exec_path().cloned(),
            ));
        }
        Some(causes)
    } else {
        None
    };
    io::Error::new(
        e.kind(),
        blaze_exec::exec::action_execution_context::BulkTransferError {
            message: bulk.get_message(),
            causes,
        },
    )
}

/// `RemoteOutputService.finalizeAction`: `actionInputFetcher.finalizeAction(action,
/// outputMetadataStore)`. `leaseService.finalizeAction()` only starts the lease extension
/// (`RemoteLeaseExtension`), which is not ported and fails loudly where it would be created
/// (`LeaseService::new`), so it is left out here.
// Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:170 RemoteOutputService.finalizeAction
impl blaze_exec::exec::action_execution_context::ActionFinalizer for RemoteActionInputFetcher {
    fn finalize_action(
        &self,
        action: &dyn blaze_exec::actions::ActionExecutionMetadata,
        outputs: &[blaze_exec::actions::ActionInputRef],
        output_metadata_store: &dyn blaze_exec::actions::cache::output_metadata_store::OutputMetadataStore,
    ) -> Result<(), blaze_exec::exec::action_execution_context::SpawnCacheError> {
        (self as &dyn AbstractActionInputPrefetcher).finalize_action(
            action,
            outputs,
            output_metadata_store,
        )
    }
}
