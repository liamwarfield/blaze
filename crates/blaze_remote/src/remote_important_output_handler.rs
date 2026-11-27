//! Port of `lib/remote/RemoteImportantOutputHandler.java`.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_exec::actions::{
    ActionInputRef, FileArtifactValue, InputMetadataProvider, LostArtifacts, OutputChecker,
};
use blaze_exec::exec::action_execution_context::{
    ImportantOutputError, ImportantOutputException, ImportantOutputHandler,
};
use blaze_protos::failure_details::{
    FailureDetail, RemoteExecution, failure_detail, remote_execution,
};
use futures::future::BoxFuture;

use crate::abstract_action_input_prefetcher::{ActionInputPrefetcher, Priority, Reason};
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;
use crate::remote_action_input_fetcher::RemoteActionInputFetcher;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::runtime::get_from_future;
use crate::util::utils::merge_bulk_transfer;

/// Ensures that top-level outputs the remote output checker wants local are downloaded (for
/// example the outputs of a target that became top-level after it was executed, and source files
/// of repositories that the remote repository contents cache only holds in memory).
// Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:53 RemoteImportantOutputHandler
pub struct RemoteImportantOutputHandler {
    remote_output_checker: Arc<RemoteOutputChecker>,
    action_input_prefetcher: Arc<RemoteActionInputFetcher>,
    /// See [`ImportantOutputHandler::is_registered`]: set once the build knows whether it is a
    /// Skymeld build (`env.withMergedAnalysisAndExecutionSourceOfTruth()`).
    registered: Arc<AtomicBool>,
    /// `rewoundActionSynchronizer` when it is a `RemoteRewoundActionSynchronizer`.
    rewound_action_synchronizer:
        Option<Arc<crate::remote_rewound_action_synchronizer::RemoteRewoundActionSynchronizer>>,
}

impl RemoteImportantOutputHandler {
    /// PORT: Java's `graph` (for `getGeneratingAction`) is not taken: see
    /// [`RemoteImportantOutputHandler::download_artifact`]. `rewoundActionSynchronizer` is the
    /// remote one or `None` (Java's `NOOP`).
    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:60 RemoteImportantOutputHandler.<init>
    pub fn new(
        remote_output_checker: Arc<RemoteOutputChecker>,
        action_input_prefetcher: Arc<RemoteActionInputFetcher>,
        registered: Arc<AtomicBool>,
        rewound_action_synchronizer: Option<
            Arc<crate::remote_rewound_action_synchronizer::RemoteRewoundActionSynchronizer>,
        >,
    ) -> RemoteImportantOutputHandler {
        RemoteImportantOutputHandler {
            remote_output_checker,
            action_input_prefetcher,
            registered,
            rewound_action_synchronizer,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:134 RemoteImportantOutputHandler.maybeEnterProcessOutputsAndGetLostArtifacts
    fn maybe_enter_process_outputs_and_get_lost_artifacts(
        &self,
        important_outputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<
        blaze_exec::exec::rewound_action_synchronizer::SilentCloseable,
        blaze_shell::InterruptedException,
    > {
        match &self.rewound_action_synchronizer {
            Some(synchronizer) => synchronizer
                .enter_process_outputs_and_get_lost_artifacts(important_outputs, metadata_provider),
            None => Ok(blaze_exec::exec::rewound_action_synchronizer::SilentCloseable::noop()),
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:146 RemoteImportantOutputHandler.ensureToplevelArtifacts
    fn ensure_toplevel_artifacts(
        &self,
        important_artifacts: &[ActionInputRef],
        runfiles_tree_artifacts: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> io::Result<()> {
        let mut downloads: Vec<Vec<ActionInputRef>> = Vec::new();
        let mut ensured_output_metadata: Vec<FileArtifactValue> = Vec::new();

        for artifact in important_artifacts {
            self.download_artifact(
                metadata_provider,
                artifact,
                &mut downloads,
                &mut ensured_output_metadata,
            )?;
        }

        for artifact in runfiles_tree_artifacts {
            self.download_artifact(
                metadata_provider,
                artifact,
                &mut downloads,
                &mut ensured_output_metadata,
            )?;
        }

        // TODO: Only wait for failed futures to complete as long as they can all be explained by
        // lost outputs.
        let prefetcher = &*self.action_input_prefetcher;
        let futures: Vec<BoxFuture<'_, io::Result<()>>> = downloads
            .iter()
            .map(|files| {
                Box::pin(prefetcher.prefetch_files(
                    // PORT: Java passes the generating action of a derived artifact
                    // (`Actions.getGeneratingAction(graph, artifact)`), which only names the
                    // action in the download's progress event; the completion function does not
                    // hold the actions, so every download is announced without one, as Java's
                    // download of a source artifact is.
                    None,
                    files,
                    metadata_provider,
                    Priority::LOW,
                    Reason::OUTPUTS,
                )) as BoxFuture<'_, io::Result<()>>
            })
            .collect();
        get_from_future(merge_bulk_transfer(futures))?;

        // These outputs are now present in the local filesystem, but their generating actions
        // were not reexecuted, so the metadata tracked for them in Skyframe remains remote.
        // Record the materialization on the metadata so that if such a file is deleted locally, a
        // later invocation reruns the generating action to restore it.
        // PORT: the record is a cell the clones of a metadata value share (the graph's value
        // and the copies the metadata provider hands out).
        // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:178 RemoteImportantOutputHandler.ensureToplevelArtifacts
        for metadata in &ensured_output_metadata {
            metadata.set_materialized_as_toplevel_output(true);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:196 RemoteImportantOutputHandler.downloadArtifact
    fn download_artifact(
        &self,
        metadata_provider: &dyn InputMetadataProvider,
        artifact: &ActionInputRef,
        downloads: &mut Vec<Vec<ActionInputRef>>,
        ensured_output_metadata: &mut Vec<FileArtifactValue>,
    ) -> io::Result<()> {
        if !RemoteOutputChecker::may_be_remote(&**artifact) {
            return Ok(());
        }

        // Metadata can be null during error bubbling, only download outputs that are already
        // generated. b/342188273
        if artifact
            .as_artifact()
            .is_some_and(|artifact| artifact.is_tree_artifact())
        {
            let Some(tree_artifact_value) = metadata_provider.get_tree_metadata(artifact) else {
                return Ok(());
            };

            let mut files_to_download: Vec<ActionInputRef> =
                Vec::with_capacity(tree_artifact_value.child_values.len());
            for (child, metadata) in &tree_artifact_value.child_values {
                if self
                    .remote_output_checker
                    .should_download_output(&**child, metadata)
                {
                    files_to_download.push(child.clone());
                }
            }
            if !files_to_download.is_empty() {
                downloads.push(files_to_download);
            }
        } else {
            let Some(metadata) = metadata_provider.get_input_metadata(artifact)? else {
                return Ok(());
            };

            if self
                .remote_output_checker
                .should_download_output(&**artifact, &metadata)
            {
                ensured_output_metadata.push(metadata);
                downloads.push(vec![artifact.clone()]);
            }
        }
        Ok(())
    }
}

impl ImportantOutputHandler for RemoteImportantOutputHandler {
    fn is_registered(&self) -> bool {
        self.registered.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:72 RemoteImportantOutputHandler.requiresHiddenOutputMetadata
    fn requires_hidden_output_metadata(&self) -> bool {
        // We want to process top-level runfiles in processOutputsAndGetLostArtifacts.
        true
    }

    // Java: com/google/devtools/build/lib/remote/RemoteImportantOutputHandler.java:78 RemoteImportantOutputHandler.processOutputsAndGetLostArtifacts
    fn process_outputs_and_get_lost_artifacts(
        &self,
        important_outputs: &[ActionInputRef],
        runfiles_tree_artifacts: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<LostArtifacts, ImportantOutputError> {
        let _lock = self
            .maybe_enter_process_outputs_and_get_lost_artifacts(important_outputs, metadata_provider)
            .map_err(|_| ImportantOutputError::Interrupted)?;
        match self.ensure_toplevel_artifacts(
            important_outputs,
            runfiles_tree_artifacts,
            metadata_provider,
        ) {
            Ok(()) => Ok(LostArtifacts::empty()),
            // InterruptedException leaves processOutputsAndGetLostArtifacts as it is.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                Err(ImportantOutputError::Interrupted)
            }
            Err(e) => {
                if let Some(bulk_transfer_exception) =
                    errors::downcast_ref::<BulkTransferException>(&e)
                {
                    let lost_artifacts = bulk_transfer_exception
                        .get_lost_artifacts(&|exec_path| metadata_provider.get_input(exec_path));
                    if !lost_artifacts.is_empty() {
                        return Ok(lost_artifacts);
                    }
                }
                // e.getMessage()
                let message = e.to_string();
                Err(ImportantOutputError::Exception(ImportantOutputException {
                    message: message.clone(),
                    failure_detail: FailureDetail {
                        message,
                        category: Some(failure_detail::Category::RemoteExecution(
                            RemoteExecution {
                                code: remote_execution::Code::ToplevelOutputsDownloadFailure as i32,
                            },
                        )),
                    },
                }))
            }
        }
    }
}
