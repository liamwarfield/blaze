//! Port of `lib/remote/RemoteRepoContentsCacheImpl.java`: a cache for the contents of external
//! repositories that is backed by an ordinary remote cache.
//!
//! Upon a cache hit, the metadata of the files comprising the repository is downloaded and
//! injected into a `RemoteExternalOverlayFileSystem`. Downloads of file contents only occur when
//! Bazel needs to read a file (e.g., a BUILD or .bzl file) or if a file is an input to an action
//! executed locally.
//!
//! Repositories are cached as AC entries for a synthetic command with a special hash as the salt.
//! The contents are represented as an output file for the marker file and an output directory
//! for the contents.
//!
//! If a repo rule does not record any `RepoRecordedInput`s during its execution, this hash is
//! just the predeclared inputs hash (`DigestWriter`). Otherwise, the AC entry for the predeclared
//! inputs hash will be an intermediate entry that lists one or more sets of `RepoRecordedInput`s
//! that a previously cached repo consumed during the evaluation of its rule. The cache requests
//! the current values of these inputs and computes the next hash to look up by a rolling
//! construction that combines the previous hash with the string representations of the
//! `RepoRecordedInput.WithValue`. This process is repeated until a final entry with the repo
//! contents is found or no matching entry exists.

use std::future::Future;
use std::io::{self, Read};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use blaze_cmdline::RepositoryName;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_protos::JavaMessage;
use blaze_repo_rules::repository::digest_writer::DigestWriter;
use blaze_repo_rules::repository::remote_helpers::RemoteRepoContentsCache;
use blaze_repo_rules::repository::repo_recorded_input::{
    MaybeValue, RepoRecordedInput, RepoRecordedInputWithValue,
};
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::io::IOException;
use blaze_skyframe::{Environment, Interrupted};
use blaze_util::fingerprint::Fingerprint;
use blaze_util::string_encoding::{latin1_decode, latin1_encode};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexSet, indexset};

use crate::combined_cache::CombinedCache;
use crate::common::errors;
use crate::common::remote_action_execution_context::{CachePolicy, RemoteActionExecutionContext};
use crate::common::remote_cache_client::{ActionKey, BytesBlob};
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::{
    Action, ActionResult, Command, Digest, Directory, OutputDirectory, OutputFile, Platform, Tree,
};
use crate::remote_external_overlay_file_system::RemoteExternalOverlayFileSystem;
use crate::upload_manifest::UploadManifest;
use crate::util::DigestUtil;
use crate::util::tracing_metadata_utils;
use crate::util::utils::wait_for_bulk_transfer;

/// Salts all cache keys; change it whenever previously cached entries may no longer be valid.
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:103 RemoteRepoContentsCacheImpl.GUID
const GUID: &str = "06a53d89-9f52-46ed-8064-f7af6ba27769";
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:104 RemoteRepoContentsCacheImpl.MARKER_FILE_PATH
const MARKER_FILE_PATH: &str = ".recorded_inputs";
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:105 RemoteRepoContentsCacheImpl.REPO_DIRECTORY_PATH
const REPO_DIRECTORY_PATH: &str = "repo_contents";

/// A unique but nonsensical command that is valid on all platforms. It is never executed, but
/// should pass all checks that an RE backend may apply to commands.
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:108 RemoteRepoContentsCacheImpl.COMMAND
#[allow(deprecated)] // Java sets the deprecated output_files, output_directories and platform.
static COMMAND: LazyLock<Command> = LazyLock::new(|| Command {
    arguments: vec![GUID.to_string()],
    output_paths: vec![
        MARKER_FILE_PATH.to_string(),
        REPO_DIRECTORY_PATH.to_string(),
    ],
    output_files: vec![MARKER_FILE_PATH.to_string()],
    output_directories: vec![REPO_DIRECTORY_PATH.to_string()],
    platform: Some(Platform::default()),
    ..Default::default()
});

/// A remote cache for the contents of external repositories.
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:101 RemoteRepoContentsCacheImpl
pub struct RemoteRepoContentsCacheImpl {
    directories: LoadingDirectories,
    cache: Arc<CombinedCache>,
    build_request_id: String,
    command_id: String,
    accept_cached: bool,
    upload_local_results: bool,
    verbose_failures: bool,
    digest_util: DigestUtil,
    base_action: Action,
    command_digest: Digest,
}

/// An error of the lookup: Java's `IOException` or `InterruptedException`.
enum LookupError {
    Io(io::Error),
    Interrupted(Interrupted),
}

impl From<io::Error> for LookupError {
    fn from(e: io::Error) -> LookupError {
        if errors::is_interrupted(&e) {
            LookupError::Interrupted(Interrupted)
        } else {
            LookupError::Io(e)
        }
    }
}

impl From<Interrupted> for LookupError {
    fn from(e: Interrupted) -> LookupError {
        LookupError::Interrupted(e)
    }
}

/// Represents a single AC entry in the internal format used by the remote repo contents cache.
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:469 RemoteRepoContentsCacheImpl.CacheEntry
enum CacheEntry {
    /// A final cache entry containing the contents of a repository: one output directory and one
    /// output file.
    Final {
        repo_directory: OutputDirectory,
        marker_file: OutputFile,
    },
    /// An intermediate cache entry that points to the keys of any number of further AC entries,
    /// which can themselves be intermediate or final entries. The remote repo contents cache
    /// will try them in order.
    Intermediate { next_input_hashes: Vec<String> },
    /// The cache entry didn't match any of the formats expected by this version of the remote
    /// repo contents cache for the given human-readable reason.
    Invalid { reason: String },
}

#[derive(Clone, Copy)]
enum CacheOp {
    Download,
    Upload,
}

impl RemoteRepoContentsCacheImpl {
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:134 RemoteRepoContentsCacheImpl.<init>
    pub fn new(
        directories: LoadingDirectories,
        cache: Arc<CombinedCache>,
        build_request_id: String,
        command_id: String,
        accept_cached: bool,
        upload_local_results: bool,
        verbose_failures: bool,
    ) -> RemoteRepoContentsCacheImpl {
        let digest_util = cache.digest_util().clone();
        let base_action = Action {
            command_digest: Some(digest_util.compute_message(&*COMMAND)),
            input_root_digest: Some(digest_util.compute_message(&Directory::default())),
            platform: Some(Platform::default()),
            ..Default::default()
        };
        let command_digest = digest_util.compute_message(&*COMMAND);
        RemoteRepoContentsCacheImpl {
            directories,
            cache,
            build_request_id,
            command_id,
            accept_cached,
            upload_local_results,
            verbose_failures,
            digest_util,
            base_action,
            command_digest,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:242 RemoteRepoContentsCacheImpl.doLookupCache
    fn do_lookup_cache(
        &self,
        repo_name: &RepositoryName,
        repo_dir: &Path,
        predeclared_input_hash: &str,
        env: &mut Environment<'_>,
    ) -> Result<bool, LookupError> {
        let Some(remote_fs) = RemoteExternalOverlayFileSystem::of(repo_dir.get_file_system())
        else {
            return Ok(false);
        };

        let context = self.build_context(repo_name, CacheOp::Download);
        if !context.get_read_cache_policy().allow_remote_cache() {
            return Ok(false);
        }
        let final_entry = self.fetch_final_cache_entry(env, &context, predeclared_input_hash)?;
        let Some((repo_directory, marker_file)) = final_entry else {
            return Ok(false);
        };
        if env.values_missing() {
            return Ok(false);
        }

        // Inlining is an optional feature, so we have to be prepared to download the marker file.
        let mut marker_file_content_bytes: Option<Vec<u8>> = None;
        let mut repo_directory_content: Option<Tree> = None;
        {
            let default_digest = Digest::default();
            let marker_future: Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> =
                if marker_file.contents.is_empty() {
                    let digest = marker_file.digest.as_ref().unwrap_or(&default_digest);
                    let cache = &self.cache;
                    let context = &context;
                    let slot = &mut marker_file_content_bytes;
                    Box::pin(async move {
                        *slot = Some(
                            cache
                                .download_blob_named(context, MARKER_FILE_PATH, None, digest)
                                .await?,
                        );
                        Ok(())
                    })
                } else {
                    marker_file_content_bytes = Some(marker_file.contents.to_vec());
                    Box::pin(async { Ok(()) })
                };
            let tree_digest = repo_directory
                .tree_digest
                .as_ref()
                .unwrap_or(&default_digest);
            let cache = &self.cache;
            let context = &context;
            let slot = &mut repo_directory_content;
            let tree_future: Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> =
                Box::pin(async move {
                    let tree_bytes = cache
                        .download_blob_as_byte_string(
                            context,
                            REPO_DIRECTORY_PATH,
                            None,
                            tree_digest,
                        )
                        .await?;
                    *slot = Some(
                        <Tree as prost::Message>::decode(tree_bytes)
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
                    );
                    Ok(())
                });
            wait_for_bulk_transfer([marker_future, tree_future])?;
        }

        let marker_file_content =
            latin1_decode(&marker_file_content_bytes.unwrap_or_default()).into_owned();
        let Some(recorded_inputs) =
            DigestWriter::read_marker_file(&marker_file_content, predeclared_input_hash)
        else {
            return Ok(false);
        };
        let outdated_reason =
            RepoRecordedInput::is_any_value_outdated(env, &self.directories, &recorded_inputs)?;
        if env.values_missing() || outdated_reason.is_some() {
            env.listener().handle(&Event::warn(format!(
                "Unexpectedly outdated cached repo {}: {}",
                repo_name,
                outdated_reason.as_deref().unwrap_or("unknown reason")
            )));
            return Ok(false);
        }

        Ok(remote_fs.inject_remote_repo(
            repo_name,
            &repo_directory_content.unwrap_or_default(),
            &marker_file_content,
        )?)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:303 RemoteRepoContentsCacheImpl.buildContext
    fn build_context(
        &self,
        repo_name: &RepositoryName,
        cache_op: CacheOp,
    ) -> RemoteActionExecutionContext<'static> {
        let metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            repo_name.get_name(),
            None,
        );
        // Don't upload local repo contents to the disk cache as the (local)
        // `--repo_contents_cache` is a better alternative for local caching. Do write through the
        // disk cache for downloads from the remote cache to speed up future usage.
        RemoteActionExecutionContext::create(metadata)
            .with_read_cache_policy(if self.accept_cached {
                CachePolicy::AnyCache
            } else {
                CachePolicy::NoCache
            })
            .with_write_cache_policy(match cache_op {
                CacheOp::Download => CachePolicy::AnyCache,
                CacheOp::Upload => {
                    if self.upload_local_results {
                        CachePolicy::RemoteCacheOnly
                    } else {
                        CachePolicy::NoCache
                    }
                }
            })
    }

    /// We choose to embed the hash into the salt simply because that results in a constant
    /// Command message.
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:321 RemoteRepoContentsCacheImpl.buildAction
    fn build_action(&self, input_hash: &str) -> Action {
        let mut action = self.base_action.clone();
        // ByteString.copyFrom(StringUnsafe.getByteArray(inputHash))
        action.salt = latin1_encode(input_hash).into_owned().into();
        action
    }

    /// Uploads the intermediate action results representing the inputs recorded at runtime and
    /// returns the input hash to use for the final action result.
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:332 RemoteRepoContentsCacheImpl.uploadIntermediateActionResults
    fn upload_intermediate_action_results(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        predeclared_input_hash: &str,
        recorded_input_values: &[RepoRecordedInputWithValue],
    ) -> io::Result<String> {
        // The command is shared by all action results and small enough that FindMissingBlobs is
        // not worthwhile. The REAPI spec requires the command to be uploaded before an action
        // result that references it.
        let command_bytes = COMMAND.to_byte_array();
        wait_for_bulk_transfer([self.cache.upload_blob(
            context,
            &self.command_digest,
            Arc::new(BytesBlob(command_bytes.into())),
        )])?;

        let mut rolling_hash = predeclared_input_hash.to_string();
        let batches = RepoRecordedInputWithValue::split_into_batches(recorded_input_values);
        let mut futures = Vec::with_capacity(batches.len());
        for batch in &batches {
            let action = self.build_action(&rolling_hash);
            let inputs: Vec<RepoRecordedInput> = batch.iter().map(|v| v.input.clone()).collect();
            futures.push(self.add_to_action_result(context, action, inputs));
            for recorded_input_value in batch {
                rolling_hash = self.roll_forward_hash(&rolling_hash, recorded_input_value);
            }
        }
        wait_for_bulk_transfer(futures)?;
        Ok(rolling_hash)
    }

    /// Adds the given set of recorded inputs as one of the alternative paths to the action result
    /// for the given action, if not already present.
    ///
    /// Most repo rule evaluations with a fixed previous batch of hashes (in particular, the same
    /// predeclared inputs hash) will request a fixed set of inputs in the next batch. Thus, most
    /// intermediate action results will only contain a single set of recorded inputs.
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:363 RemoteRepoContentsCacheImpl.addToActionResult
    async fn add_to_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action: Action,
        new_inputs: Vec<RepoRecordedInput>,
    ) -> io::Result<()> {
        let action_key = self.digest_util.compute_action_key(&action);
        let current_result = self
            .cache
            .download_action_result_async(
                context,
                &action_key,
                /* inline_out_err= */ true,
                &IndexSet::new(),
            )
            .await?;
        let current_inputs_string = match &current_result {
            Some(current_result)
                if current_result
                    .action_result()
                    .stdout_digest
                    .as_ref()
                    .is_some_and(|d| d.size_bytes != 0) =>
            {
                self.fetch_stdout(context, current_result.action_result())
                    .await?
            }
            _ => String::new(),
        };
        // RepoRecordedInput.toString() is guaranteed to return a string that doesn't contain
        // spaces or newlines. We can thus safely use spaces to separate inputs within a batch and
        // newlines to separate different batches.
        let new_input_string = new_inputs
            .iter()
            .map(|input| input.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        if java_lines(&current_inputs_string).any(|line| line == new_input_string) {
            // The current batch of inputs is already present, no need to update the action
            // result.
            return Ok(());
        }
        // Add the new inputs to the top so that the most recently added inputs stay at the top.
        // This could be used to implement a simple "least recently added" eviction strategy in
        // the future in case the size of action results becomes a concern.
        //
        // Note that this update is inherently racy: multiple clients may add inputs concurrently,
        // resulting in some added inputs being lost since the REAPI does not provide a way to
        // update action results atomically. However, since different batches of inputs are
        // already rare and them being added concurrently even more so, the temporary loss of a
        // cache entry is an acceptable trade-off for simplicity.
        let new_inputs_string = format!("{new_input_string}\n{current_inputs_string}");
        let stdout_bytes = latin1_encode(&new_inputs_string).into_owned();
        let stdout_digest = self.digest_util.compute(&stdout_bytes);
        let action_result = ActionResult {
            exit_code: 0,
            stdout_digest: Some(stdout_digest.clone()),
            ..Default::default()
        };
        // whenAllSucceed(upload action, upload stdout).callAsync(uploadActionResult)
        let action_upload = self.cache.upload_blob(
            context,
            action_key.digest(),
            Arc::new(BytesBlob(action.to_byte_array().into())),
        );
        let stdout_upload = self.cache.upload_blob(
            context,
            &stdout_digest,
            Arc::new(BytesBlob(stdout_bytes.into())),
        );
        let (a, b) = futures::join!(action_upload, stdout_upload);
        a?;
        b?;
        self.cache
            .upload_action_result(context, &action_key, &action_result)
            .await
    }

    /// Fetches a final cache entry for the given predeclared input hash by recursively following
    /// intermediate entries if needed or returns `None` if no final entry could be found or a
    /// Skyframe restart is needed.
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:504 RemoteRepoContentsCacheImpl.fetchFinalCacheEntry
    fn fetch_final_cache_entry(
        &self,
        env: &mut Environment<'_>,
        context: &RemoteActionExecutionContext<'_>,
        predeclared_input_hash: &str,
    ) -> Result<Option<(OutputDirectory, OutputFile)>, LookupError> {
        let mut current_hashes = vec![predeclared_input_hash.to_string()];
        while !current_hashes.is_empty() {
            let mut next_hashes = Vec::new();
            for hash in &current_hashes {
                match self.fetch_cache_entry(env, context, hash)? {
                    Some(CacheEntry::Final {
                        repo_directory,
                        marker_file,
                    }) => return Ok(Some((repo_directory, marker_file))),
                    Some(CacheEntry::Intermediate { next_input_hashes }) => {
                        next_hashes.extend(next_input_hashes)
                    }
                    Some(CacheEntry::Invalid { reason }) => {
                        env.listener().handle(&Event::warn(reason))
                    }
                    None => {
                        // Keep checking hashes to batch missing values in fewer restarts.
                        blaze_util::check_state!(env.values_missing());
                    }
                }
            }
            if env.values_missing() {
                return Ok(None);
            }
            current_hashes = next_hashes;
        }
        Ok(None)
    }

    /// Returns `None` if and only if values are missing.
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:538 RemoteRepoContentsCacheImpl.fetchCacheEntry
    fn fetch_cache_entry(
        &self,
        env: &mut Environment<'_>,
        context: &RemoteActionExecutionContext<'_>,
        input_hash: &str,
    ) -> Result<Option<CacheEntry>, LookupError> {
        let action_key = ActionKey::new(
            self.digest_util
                .compute_message(&self.build_action(input_hash)),
        );
        // The marker file is read right after and thus requested to be inlined. If the action
        // result is an intermediate node, the full result will be contained in the stdout, which
        // should thus also be inlined.
        let cached_action_result = self.cache.download_action_result(
            context,
            &action_key,
            /* inline_out_err= */ true,
            &indexset! {MARKER_FILE_PATH.to_string()},
        )?;
        let Some(cached_action_result) = cached_action_result else {
            return Ok(Some(CacheEntry::Intermediate {
                next_input_hashes: Vec::new(),
            }));
        };
        let action_result = cached_action_result.action_result();

        if action_result.exit_code != 0 {
            return Ok(Some(CacheEntry::Invalid {
                reason: format!(
                    "Unexpected exit code in action result for remotely cached repo {}:\n{}",
                    context.get_request_metadata().action_id,
                    action_result_to_string(action_result)
                ),
            }));
        }
        if action_result.output_files.len() == 1
            && action_result.output_directories.len() == 1
            && action_result.output_symlinks.is_empty()
        {
            return Ok(Some(CacheEntry::Final {
                repo_directory: action_result.output_directories[0].clone(),
                marker_file: action_result.output_files[0].clone(),
            }));
        }
        if !(action_result.output_files.is_empty()
            && action_result.output_directories.is_empty()
            && action_result.output_symlinks.is_empty()
            && action_result
                .stdout_digest
                .as_ref()
                .is_some_and(|d| d.size_bytes > 0))
        {
            return Ok(Some(CacheEntry::Invalid {
                reason: format!(
                    "Unexpected intermediate action result for remotely cached repo {}:\n{}",
                    context.get_request_metadata().action_id,
                    action_result_to_string(action_result)
                ),
            }));
        }
        let mut stdout = String::new();
        wait_for_bulk_transfer([async {
            stdout = self.fetch_stdout(context, action_result).await?;
            Ok::<(), io::Error>(())
        }])?;

        // The action result's stdout contains multiple lines, each representing a batch of
        // RepoRecordedInputs separated by spaces. A given batch is valid only if all inputs in
        // the batch are, but separate batches are tried independently.
        let next_input_batches: Vec<Vec<RepoRecordedInput>> = java_lines(&stdout)
            .map(|line| line.split(' ').map(RepoRecordedInput::parse).collect())
            .collect();
        let unique_next_inputs: Vec<RepoRecordedInput> = next_input_batches
            .iter()
            .flatten()
            .cloned()
            .collect::<IndexSet<_>>()
            .into_iter()
            .collect();
        RepoRecordedInput::prefetch(env, &self.directories, &unique_next_inputs)?;
        if env.values_missing() {
            return Ok(None);
        }
        let mut next_hashes = Vec::new();
        'next_batch: for batch in &next_input_batches {
            let mut rolling_hash = input_hash.to_string();
            for input in batch {
                let value = input.get_value(env, &self.directories)?;
                // Values have been prefetched above.
                blaze_util::check_state!(!env.values_missing());
                let MaybeValue::Valid(value_string) = value else {
                    continue 'next_batch;
                };
                rolling_hash = self.roll_forward_hash(
                    &rolling_hash,
                    &RepoRecordedInputWithValue::new(input.clone(), value_string),
                );
            }
            next_hashes.push(rolling_hash);
        }
        Ok(Some(CacheEntry::Intermediate {
            next_input_hashes: next_hashes,
        }))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:603 RemoteRepoContentsCacheImpl.rollForwardHash
    fn roll_forward_hash(
        &self,
        hash: &str,
        input_with_value: &RepoRecordedInputWithValue,
    ) -> String {
        Fingerprint::new()
            .add_string(hash)
            .add_string(&input_with_value.to_string())
            .hex_digest_and_reset()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:610 RemoteRepoContentsCacheImpl.fetchStdout
    async fn fetch_stdout(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_result: &ActionResult,
    ) -> io::Result<String> {
        if !action_result.stdout_raw.is_empty() {
            return Ok(latin1_decode(&action_result.stdout_raw).into_owned());
        }
        let default_digest = Digest::default();
        let stdout = self
            .cache
            .download_blob(
                context,
                action_result
                    .stdout_digest
                    .as_ref()
                    .unwrap_or(&default_digest),
            )
            .await?;
        Ok(latin1_decode(&stdout).into_owned())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:622 RemoteRepoContentsCacheImpl.maybeGetStackTrace
    fn maybe_get_stack_trace(&self, e: &dyn std::fmt::Display) -> String {
        // PORT: Rust errors have no stack trace; with --verbose_failures Java prints
        // `Throwables.getStackTraceAsString(e)`, whose first line is the exception's toString().
        let _ = self.verbose_failures;
        e.to_string()
    }
}

/// `String.lines()`: the lines of `s`, split at `\n`, `\r` or `\r\n`, without a trailing empty
/// line.
fn java_lines(s: &str) -> impl Iterator<Item = &str> {
    let mut lines: Vec<&str> = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&s[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&s[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&s[start..]);
    }
    lines.into_iter()
}

/// `actionResult.toString()`: the message in protobuf text format.
fn action_result_to_string(action_result: &ActionResult) -> String {
    blaze_protos::text_format::printer().print_to_string(action_result)
}

impl RemoteRepoContentsCache for RemoteRepoContentsCacheImpl {
    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:156 RemoteRepoContentsCacheImpl.addToCache
    fn add_to_cache(
        &self,
        repo_name: &RepositoryName,
        fetched_repo_dir: &Path,
        fetched_repo_marker_file: &Path,
        predeclared_input_hash: &str,
        reporter: &dyn ExtendedEventHandler,
    ) -> Result<(), Interrupted> {
        if RemoteExternalOverlayFileSystem::of(fetched_repo_dir.get_file_system()).is_none() {
            return Ok(());
        }
        let context = self.build_context(repo_name, CacheOp::Upload);
        if !context.get_write_cache_policy().allow_remote_cache() {
            return Ok(());
        }
        // DigestWriter.readMarkerFile(FileSystemUtils.readContent(fetchedRepoMarkerFile,
        // ISO_8859_1), predeclaredInputHash)
        let content = fetched_repo_marker_file
            .get_input_stream()
            .and_then(|mut input| {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                Ok(latin1_decode(&bytes).into_owned())
            });
        let recorded_input_values = match content {
            Ok(content) => match DigestWriter::read_marker_file(&content, predeclared_input_hash) {
                None => return Ok(()),
                Some(values) => values,
            },
            Err(e) if errors::is_interrupted(&e) => return Err(Interrupted),
            Err(e) => {
                reporter.handle(&Event::warn(format!(
                    "Failed to read marker file for repo {}, skipping: {}",
                    repo_name,
                    self.maybe_get_stack_trace(&e)
                )));
                return Ok(());
            }
        };
        let result: Result<(), SpawnCacheError> = (|| {
            // TODO: Consider uploading asynchronously.
            let final_hash = self.upload_intermediate_action_results(
                &context,
                predeclared_input_hash,
                &recorded_input_values,
            )?;
            let action = self.build_action(&final_hash);
            let action_key = ActionKey::new(self.digest_util.compute_message(&action));
            let remote_path_resolver: Arc<dyn RemotePathResolver> =
                Arc::new(RepoRemotePathResolver {
                    fetched_repo_marker_file: fetched_repo_marker_file.clone(),
                    fetched_repo_dir: fetched_repo_dir.clone(),
                });
            let manifest = UploadManifest::create(
                &self.cache.get_remote_cache_capabilities()?,
                self.digest_util.clone(),
                remote_path_resolver,
                action_key,
                &action,
                &COMMAND,
                &[fetched_repo_marker_file.clone(), fetched_repo_dir.clone()],
                /* out_err= */ None,
                /* exit_code= */ 0,
                /* start_time= */ Some(SystemTime::now()),
                /* wall_time_in_ms= */ 0,
                /* preserve_executable_bit= */ true,
            )?;
            manifest.upload(&context, &self.cache, reporter, /* force= */ false)?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(SpawnCacheError::Io(e)) if errors::is_interrupted(&e) => Err(Interrupted),
            Err(e) => {
                reporter.handle(&Event::warn(format!(
                    "Failed to upload repo contents to remote cache for repo {}: {}",
                    repo_name,
                    match &e {
                        SpawnCacheError::Exec(e) => self.maybe_get_stack_trace(e),
                        SpawnCacheError::Io(e) => self.maybe_get_stack_trace(e),
                    }
                )));
                Ok(())
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:225 RemoteRepoContentsCacheImpl.lookupCache
    fn lookup_cache(
        &self,
        repo_name: &RepositoryName,
        repo_dir: &Path,
        predeclared_input_hash: &str,
        env: &mut Environment<'_>,
    ) -> Result<Result<bool, IOException>, Interrupted> {
        match self.do_lookup_cache(repo_name, repo_dir, predeclared_input_hash, env) {
            Ok(hit) => Ok(Ok(hit)),
            Err(LookupError::Interrupted(e)) => Err(e),
            Err(LookupError::Io(e)) => Ok(Err(IOException::new(format!(
                "Failed to look up repo {} in the remote repo contents cache: {}",
                repo_name,
                self.maybe_get_stack_trace(&e)
            )))),
        }
    }
}

/// Maps the repo marker file and contents to fixed locations under the fake remote exec root.
// Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:626 RemoteRepoContentsCacheImpl.RepoRemotePathResolver
struct RepoRemotePathResolver {
    fetched_repo_marker_file: Path,
    fetched_repo_dir: Path,
}

impl RemotePathResolver for RepoRemotePathResolver {
    fn get_working_directory(&self) -> PathFragment {
        panic!("UnsupportedOperationException: Not used")
    }

    fn get_input_mapping(
        &self,
        _context: &dyn blaze_exec::exec::SpawnExecutionContext,
        _will_access_repeatedly: bool,
    ) -> Arc<blaze_exec::actions::action_input::InputMapping> {
        panic!("UnsupportedOperationException: Not used")
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRepoContentsCacheImpl.java:630 RepoRemotePathResolver.localPathToOutputPath
    fn local_path_to_output_path(&self, path: &Path) -> String {
        // Map repo marker file and contents to fixed locations under the fake remote exec root.
        if *path == self.fetched_repo_marker_file {
            return MARKER_FILE_PATH.to_string();
        }
        if *path == self.fetched_repo_dir {
            return REPO_DIRECTORY_PATH.to_string();
        }
        format!(
            "{}/{}",
            REPO_DIRECTORY_PATH,
            path.relative_to(&self.fetched_repo_dir).get_path_string()
        )
    }

    fn local_exec_path_to_output_path(&self, _exec_path: &PathFragment) -> String {
        panic!("UnsupportedOperationException: Not used")
    }

    fn output_path_to_local_path(&self, _output_path: &str) -> Path {
        panic!("UnsupportedOperationException: Not used")
    }

    fn local_path_to_exec_path(&self, _local_path: &PathFragment) -> PathFragment {
        panic!("UnsupportedOperationException: Not used")
    }
}
