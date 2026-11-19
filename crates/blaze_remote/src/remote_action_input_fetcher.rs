//! Port of `lib/remote/RemoteActionInputFetcher.java`.

use std::io;
use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_exec::actions::action_input_prefetcher::{ActionInputPrefetcher, Priority, Reason};
use blaze_exec::actions::cache::OutputMetadataStore;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, EnvironmentalExecException, ExecException,
    FileArtifactValue, FileStateType, InputMetadataProvider,
};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{Path, Symlinks};

use crate::abstract_action_input_prefetcher::{
    AbstractActionInputPrefetcher, ActionInputPrefetcherHooks, OutputPermissions,
};
use crate::combined_cache::{CombinedCache, DownloadProgressReporter};
use crate::common::bulk_transfer_exception::BulkTransferException;
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
/// This is necessary when a locally executed action consumes outputs produced by a remotely
/// executed action and `--remote_download_outputs=minimal` is specified, or when building with a
/// remote repository cache enabled.
// Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:52 RemoteActionInputFetcher
pub struct RemoteActionInputFetcher {
    base: AbstractActionInputPrefetcher,
}

/// The overridden methods of `RemoteActionInputFetcher`.
struct Hooks {
    build_request_id: String,
    command_id: String,
    combined_cache: Arc<CombinedCache>,
    exec_root: Path,
}

impl RemoteActionInputFetcher {
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:59 RemoteActionInputFetcher.<init>
    pub fn new(
        reporter: Arc<dyn ExtendedEventHandler>,
        build_request_id: String,
        command_id: String,
        combined_cache: Arc<CombinedCache>,
        exec_root: Path,
        temp_path_generator: Arc<TempPathGenerator>,
        remote_output_checker: Arc<RemoteOutputChecker>,
        output_permissions: OutputPermissions,
    ) -> RemoteActionInputFetcher {
        let hooks = Hooks {
            build_request_id,
            command_id,
            combined_cache,
            exec_root: exec_root.clone(),
        };
        RemoteActionInputFetcher {
            base: AbstractActionInputPrefetcher::new(
                reporter,
                exec_root,
                temp_path_generator,
                remote_output_checker,
                output_permissions,
                Box::new(hooks),
            ),
        }
    }

    /// The `AbstractActionInputPrefetcher` part.
    pub fn base(&self) -> &AbstractActionInputPrefetcher {
        &self.base
    }
}

#[async_trait::async_trait]
impl ActionInputPrefetcherHooks for Hooks {
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:83 RemoteActionInputFetcher.prefetchVirtualActionInput
    fn prefetch_virtual_action_input(&self, input: &ActionInputRef) -> io::Result<()> {
        input
            .as_virtual_action_input()
            .expect("a VirtualActionInput")
            .atomically_write_relative_to(&self.exec_root)?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:88 RemoteActionInputFetcher.canDownloadFile
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

    /// Caches for download operations and output directory creation need to be disregarded for
    /// the outputs of rewound actions as they may have been deleted after they were first
    /// created.
    ///
    /// PORT: `rewoundActionOutputs` is filled by `handleRewoundActionOutputs`, which only action
    /// rewinding calls (not ported), so it is always empty.
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:103 RemoteActionInputFetcher.forceRefetch
    fn force_refetch(&self, _path: &Path) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcher.java:114 RemoteActionInputFetcher.doDownloadFile
    async fn do_download_file(
        &self,
        action: Option<&(dyn ActionExecutionMetadata + '_)>,
        _reporter: &(dyn ExtendedEventHandler + '_),
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
            metadata.get_digest().expect("a remote file has a digest"),
            metadata.get_size(),
        );

        // PORT: the progress of the download (`progress.postTo(reporter, action)`) feeds the UI
        // only; it is not posted.
        let reporter = DownloadProgressReporter::new(
            no_action(),
            &input.get_exec_path_string(),
            digest.size_bytes,
        );
        // Treat other download error as CacheNotFoundException so that Bazel can correctly
        // rewind the action/build.
        match self
            .combined_cache
            .download_file_with_reporter(
                &context,
                &input.get_exec_path_string(),
                Some(input.get_exec_path()),
                &temp_path.for_host_file_system(),
                &digest,
                &reporter,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if errors::downcast_ref::<CacheNotFoundException>(&e).is_some() => Err(e),
            Err(e) => {
                // PORT: `cacheNotFoundException.addSuppressed(e)` has no counterpart.
                let _ = e;
                Err(CacheNotFoundException::with_exec_path(
                    digest.clone(),
                    input.get_exec_path().clone(),
                )
                .into())
            }
        }
    }
}

impl ActionInputPrefetcher for RemoteActionInputFetcher {
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:325 AbstractActionInputPrefetcher.prefetchFiles
    fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        self.base
            .prefetch_files(action, inputs, metadata_provider, priority, reason)
    }

    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:284 SpawnExecutionContextImpl.prefetchInputs
    fn bulk_transfer_exception_to_exec_exception(
        &self,
        e: &io::Error,
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Option<ExecException> {
        let bulk = errors::downcast_ref::<BulkTransferException>(e)?;
        bulk.get_lost_artifacts(&|exec_path| metadata_provider.get_input(exec_path))
            .throw_if_not_empty();
        Some(
            EnvironmentalExecException::with_cause(
                e,
                FailureDetail {
                    message: "Failed to fetch blobs because of a remote cache error.".to_string(),
                    category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                        code: failure_details::spawn::Code::RemoteCacheEvicted as i32,
                        ..Default::default()
                    })),
                },
            )
            .into(),
        )
    }

    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:170 RemoteOutputService.finalizeAction
    fn finalize_action(
        &self,
        action: &dyn ActionExecutionMetadata,
        outputs: &[ActionInputRef],
        output_metadata_store: &dyn OutputMetadataStore,
    ) -> io::Result<()> {
        self.base
            .finalize_action(action, outputs, output_metadata_store)
        // PORT: `leaseService.finalizeAction()` only matters after lost inputs, which need
        // action rewinding (not ported); see `RemoteModule::executor_init`.
    }
}
