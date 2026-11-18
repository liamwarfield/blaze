//! Port of `lib/remote/UploadManifest.java`, with the two events it posts
//! (`lib/actions/ActionUploadStartedEvent.java`, `lib/actions/ActionUploadFinishedEvent.java`).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_events::{ExtendedEventHandler, Postable};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ActionOwner, ExecException, UserExecException,
};
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_exec::util::FileOutErr;
use blaze_protos::JavaMessage;
use blaze_protos::failure_details::{
    FailureDetail, RemoteExecution, failure_detail, remote_execution,
};
use blaze_protos::google::protobuf::Timestamp;
use blaze_vfs::{DirentType, FileStatus, Path, PathFragment, Symlinks};
use bytes::Bytes;
use futures::future::BoxFuture;
use indexmap::{IndexMap, IndexSet};
use prost::encoding::{WireType, encode_key, encode_varint};

use crate::combined_cache::CombinedCache;
use crate::common::errors::io_exception;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_cache_client::{ActionKey, BytesBlob};
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::{
    Action, ActionResult, CacheCapabilities, Command, Digest, Directory, DirectoryNode, FileNode,
    OutputDirectory, OutputFile, OutputSymlink, SymlinkNode, symlink_absolute_path_strategy,
};
use crate::runtime::get_from_future;
use crate::store::Store;
use crate::util::digest_util::DigestUtil;
use crate::util::utils::wait_for_bulk_transfer;

/// The event fired when a resource is about to be uploaded to a remote or disk cache upon
/// completion of a local action.
///
/// PORT: neither `blaze_exec` nor `blaze_events` has this event. Java keeps a reference to the
/// action; a posted value must own its data here, so the event keeps the parts of the action
/// that consumers read (as `blaze_exec`'s `SpawnExecutedEvent` does).
// Java: com/google/devtools/build/lib/actions/ActionUploadStartedEvent.java:30 ActionUploadStartedEvent
#[derive(Debug, Clone)]
pub struct ActionUploadStartedEvent {
    action_owner: ActionOwner,
    action_mnemonic: String,
    action_progress_message: Option<String>,
    action_primary_output: ActionInputRef,
    store: Store,
    digest: Digest,
}

impl ActionUploadStartedEvent {
    /// `ActionUploadStartedEvent.create(action, store, digest)`.
    // Java: com/google/devtools/build/lib/actions/ActionUploadStartedEvent.java:38 ActionUploadStartedEvent.create
    pub fn create(
        action: &dyn ActionExecutionMetadata,
        store: Store,
        digest: Digest,
    ) -> ActionUploadStartedEvent {
        ActionUploadStartedEvent {
            action_owner: action.get_owner().clone(),
            action_mnemonic: action.get_mnemonic(),
            action_progress_message: action.get_progress_message(),
            action_primary_output: action.get_primary_output(),
            store,
            digest,
        }
    }

    /// `action().getOwner()`.
    pub fn get_action_owner(&self) -> &ActionOwner {
        &self.action_owner
    }

    /// `action().getMnemonic()`.
    pub fn get_action_mnemonic(&self) -> &str {
        &self.action_mnemonic
    }

    /// `action().getProgressMessage()`.
    pub fn get_action_progress_message(&self) -> Option<&str> {
        self.action_progress_message.as_deref()
    }

    /// `action().getPrimaryOutput()`.
    pub fn get_action_primary_output(&self) -> &ActionInputRef {
        &self.action_primary_output
    }

    /// Returns the store that the resource belongs to.
    pub fn store(&self) -> Store {
        self.store
    }

    /// Returns the digest that uniquely identifies the resource.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

impl Postable for ActionUploadStartedEvent {}

/// The event fired when a resource is done uploading to a remote or disk cache upon completion
/// of a local action.
///
/// PORT: see [`ActionUploadStartedEvent`].
// Java: com/google/devtools/build/lib/actions/ActionUploadFinishedEvent.java:30 ActionUploadFinishedEvent
#[derive(Debug, Clone)]
pub struct ActionUploadFinishedEvent {
    action_owner: ActionOwner,
    action_mnemonic: String,
    action_progress_message: Option<String>,
    action_primary_output: ActionInputRef,
    store: Store,
    digest: Digest,
}

impl ActionUploadFinishedEvent {
    /// `ActionUploadFinishedEvent.create(action, store, digest)`.
    // Java: com/google/devtools/build/lib/actions/ActionUploadFinishedEvent.java:38 ActionUploadFinishedEvent.create
    pub fn create(
        action: &dyn ActionExecutionMetadata,
        store: Store,
        digest: Digest,
    ) -> ActionUploadFinishedEvent {
        ActionUploadFinishedEvent {
            action_owner: action.get_owner().clone(),
            action_mnemonic: action.get_mnemonic(),
            action_progress_message: action.get_progress_message(),
            action_primary_output: action.get_primary_output(),
            store,
            digest,
        }
    }

    /// `action().getOwner()`.
    pub fn get_action_owner(&self) -> &ActionOwner {
        &self.action_owner
    }

    /// `action().getMnemonic()`.
    pub fn get_action_mnemonic(&self) -> &str {
        &self.action_mnemonic
    }

    /// `action().getProgressMessage()`.
    pub fn get_action_progress_message(&self) -> Option<&str> {
        self.action_progress_message.as_deref()
    }

    /// `action().getPrimaryOutput()`.
    pub fn get_action_primary_output(&self) -> &ActionInputRef {
        &self.action_primary_output
    }

    /// Returns the `Store` that the resource belongs to.
    pub fn store(&self) -> Store {
        self.store
    }

    /// Returns the `Digest` that uniquely identifies the resource.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

impl Postable for ActionUploadFinishedEvent {}

/// UploadManifest adds output metadata to a [`ActionResult`].
///
/// PORT: Java shares the `ActionResult.Builder` with the caller; the manifest owns it here
/// ([`UploadManifest::get_action_result`] is `result.build()`). `digestToFile` and
/// `digestToBlobs` are `ConcurrentHashMap`s in Java because the directory traversal is parallel;
/// the port traverses on the calling thread and keeps insertion order.
// Java: com/google/devtools/build/lib/remote/UploadManifest.java:92 UploadManifest
pub struct UploadManifest {
    digest_util: DigestUtil,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    result: ActionResult,
    allow_absolute_symlinks: bool,
    preserve_executable_bit: bool,
    digest_to_file: IndexMap<Digest, Path>,
    digest_to_blobs: IndexMap<Digest, Bytes>,
    action_key: Option<ActionKey>,
    stderr_digest: Option<Digest>,
    stdout_digest: Option<Digest>,
}

/// `StringEncoding.internalToUnicode(s)` of the internal string that `s` stands for.
///
/// Seam (decisions.md 2026-11-18 "Strings"): while
/// `blaze_util::string_encoding::STRINGS_ARE_INTERNAL` is false Blaze's strings are Unicode
/// already and this is the identity; once it is true it is `internal_to_unicode`.
// Java: com/google/devtools/build/lib/util/StringEncoding.java:126 StringEncoding.internalToUnicode
fn internal_to_unicode(s: &str) -> String {
    if blaze_util::string_encoding::strings_are_internal() {
        blaze_util::string_encoding::internal_to_unicode(s).into_owned()
    } else {
        s.to_string()
    }
}

/// The sort key of `comparing(FileNode::getName)` / `comparing(SymlinkNode::getName)`:
/// `String.compareTo` is the lexicographic order of UTF-16 code units.
fn java_string_key(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// Field numbers of the 'root' and 'directory' fields in the Tree message.
// Java: com/google/devtools/build/lib/remote/UploadManifest.java:552 UploadManifest.TREE_ROOT_FIELD_NUMBER
const TREE_ROOT_FIELD_NUMBER: u32 = 1;
// Java: com/google/devtools/build/lib/remote/UploadManifest.java:554 UploadManifest.TREE_CHILDREN_FIELD_NUMBER
const TREE_CHILDREN_FIELD_NUMBER: u32 = 2;

impl UploadManifest {
    /// `UploadManifest.create(cacheCapabilities, digestUtil, remotePathResolver, actionKey,
    /// action, command, outputFiles, outErr, exitCode, startTime, wallTimeInMs,
    /// preserveExecutableBit)`.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:106 UploadManifest.create
    pub fn create(
        cache_capabilities: &CacheCapabilities,
        digest_util: DigestUtil,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
        action_key: ActionKey,
        action: &Action,
        command: &Command,
        output_files: &[Path],
        out_err: Option<&FileOutErr>,
        exit_code: i32,
        start_time: Option<SystemTime>,
        wall_time_in_ms: i32,
        preserve_executable_bit: bool,
    ) -> Result<UploadManifest, SpawnCacheError> {
        let result = ActionResult {
            exit_code,
            ..Default::default()
        };

        let mut manifest = UploadManifest::new_with_preserve_executable_bit(
            digest_util,
            remote_path_resolver,
            result,
            /* allowAbsoluteSymlinks= */
            cache_capabilities.symlink_absolute_path_strategy
                == symlink_absolute_path_strategy::Value::Allowed as i32,
            preserve_executable_bit,
        );
        manifest.add_files(output_files)?;
        if let Some(out_err) = out_err {
            manifest.set_stdout_stderr(out_err)?;
        }
        manifest.add_action(action_key, action, command);
        if let Some(stderr_digest) = manifest.get_stderr_digest().cloned() {
            manifest.result.stderr_digest = Some(stderr_digest);
        }
        if let Some(stdout_digest) = manifest.get_stdout_digest().cloned() {
            manifest.result.stdout_digest = Some(stdout_digest);
        }

        // if wallTime is zero, than it's not set
        if let Some(start_time) = start_time
            && wall_time_in_ms != 0
        {
            let start_timestamp = instant_to_timestamp(start_time);
            let completed_timestamp =
                instant_to_timestamp(plus_millis(start_time, wall_time_in_ms));
            let execution_metadata = manifest
                .result
                .execution_metadata
                .get_or_insert_with(Default::default);
            execution_metadata.worker_start_timestamp = Some(start_timestamp);
            execution_metadata.execution_start_timestamp = Some(start_timestamp);
            execution_metadata.execution_completed_timestamp = Some(completed_timestamp);
            execution_metadata.worker_completed_timestamp = Some(completed_timestamp);
        }

        Ok(manifest)
    }

    /// Create an UploadManifest from an ActionResult builder and an exec root. The ActionResult
    /// builder is populated through a call to [`UploadManifest::add_files`].
    ///
    /// * `allow_absolute_symlinks`: whether the remote allows uploading absolute symlinks
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:173 UploadManifest.<init>
    pub fn new(
        digest_util: DigestUtil,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
        result: ActionResult,
        allow_absolute_symlinks: bool,
    ) -> UploadManifest {
        UploadManifest::new_with_preserve_executable_bit(
            digest_util,
            remote_path_resolver,
            result,
            allow_absolute_symlinks,
            /* preserveExecutableBit= */ false,
        )
    }

    /// `new UploadManifest(digestUtil, remotePathResolver, result, allowAbsoluteSymlinks,
    /// preserveExecutableBit)`.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:186 UploadManifest.<init>
    pub fn new_with_preserve_executable_bit(
        digest_util: DigestUtil,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
        result: ActionResult,
        allow_absolute_symlinks: bool,
        preserve_executable_bit: bool,
    ) -> UploadManifest {
        UploadManifest {
            digest_util,
            remote_path_resolver,
            result,
            allow_absolute_symlinks,
            preserve_executable_bit,
            digest_to_file: IndexMap::new(),
            digest_to_blobs: IndexMap::new(),
            action_key: None,
            stderr_digest: None,
            stdout_digest: None,
        }
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:199 UploadManifest.setStdoutStderr
    fn set_stdout_stderr(&mut self, out_err: &FileOutErr) -> io::Result<()> {
        if out_err.get_error_path().exists() {
            let stderr_digest = self.digest_util.compute_path(out_err.get_error_path())?;
            self.digest_to_file
                .insert(stderr_digest.clone(), out_err.get_error_path().clone());
            self.stderr_digest = Some(stderr_digest);
        }
        if out_err.get_output_path().exists() {
            let stdout_digest = self.digest_util.compute_path(out_err.get_output_path())?;
            self.digest_to_file
                .insert(stdout_digest.clone(), out_err.get_output_path().clone());
            self.stdout_digest = Some(stdout_digest);
        }
        Ok(())
    }

    /// Add a collection of files, directories or symlinks to the manifest.
    ///
    /// Adding a directory has the effect of:
    ///
    /// 1. uploading a `Tree` protobuf message from which the whole structure of the directory,
    ///    including the descendants, can be reconstructed.
    /// 2. uploading all of the non-directory descendant files.
    ///
    /// Note that the manifest describes the outcome of a spawn, not of an action. In particular,
    /// it's possible for an output to be missing or to have been created with an unsuitable file
    /// type for the corresponding `Artifact` (e.g., a directory where a file was expected, or a
    /// non-symlink where a symlink was expected). Except for the oddity noted in the next
    /// paragraph, outputs are always uploaded according to the filesystem state. A type mismatch
    /// may later cause execution to fail, but that's an action-level concern.
    ///
    /// For historical reasons, non-dangling absolute symlinks are uploaded as the file or
    /// directory they point to. This is inconsistent with the treatment of non-dangling relative
    /// symlinks, which are uploaded as such, but fixing it would now require an incompatible
    /// change. For the purposes of this check, a looping symlink is considered dangling.
    ///
    /// All files are uploaded with the executable bit set, in accordance with input Merkle
    /// trees. This does not affect correctness since we always set the output permissions to
    /// 0555 or 0755 after execution, both for cache hits and misses.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:238 UploadManifest.addFiles
    pub fn add_files(&mut self, files: &[Path]) -> Result<(), SpawnCacheError> {
        for file in files {
            // TODO(ulfjack): Maybe pass in a SpawnResult here, add a list of output files to
            // that, and rely on the local spawn runner to stat the files, instead of statting
            // here.
            let stat_no_follow = file.stat_if_found(Symlinks::Nofollow)?;
            // TODO(#6547): handle the case where the parent directory of the output file is an
            // output symlink.
            let Some(stat_no_follow) = stat_no_follow else {
                // Ignore missing outputs.
                continue;
            };
            if stat_no_follow.is_file() && !stat_no_follow.is_special_file() {
                let digest = self
                    .digest_util
                    .compute_path_with_status(file, &stat_no_follow)?;
                self.add_file(digest, file, &stat_no_follow);
                continue;
            }
            if stat_no_follow.is_directory() {
                self.add_directory(file)?;
                continue;
            }
            if stat_no_follow.is_symbolic_link() {
                let target = file.read_symbolic_link()?;
                // Need to resolve the symbolic link to know what to add, file or directory.
                let stat_follow = stat_follow_if_found(file)?;
                let Some(stat_follow) = stat_follow else {
                    // Symlink uploaded as a symlink. Report it as a file since we don't know any
                    // better.
                    if target.is_absolute() {
                        self.check_absolute_symlink_allowed(file, &target)?;
                    }
                    self.add_file_symbolic_link(file, &target);
                    continue;
                };
                if stat_follow.is_file() && !stat_follow.is_special_file() {
                    if target.is_absolute() {
                        // Symlink to file uploaded as a file.
                        let digest = self
                            .digest_util
                            .compute_path_with_status(file, &stat_follow)?;
                        self.add_file(digest, file, &stat_no_follow);
                    } else {
                        // Symlink to file uploaded as a symlink.
                        self.add_file_symbolic_link(file, &target);
                    }
                    continue;
                }
                if stat_follow.is_directory() {
                    if target.is_absolute() {
                        // Symlink to directory uploaded as a directory.
                        self.add_directory(file)?;
                    } else {
                        // Symlink to directory uploaded as a symlink.
                        self.add_directory_symbolic_link(file, &target);
                    }
                    continue;
                }
            }
            // Special file or dereferenced symlink to special file.
            return Err(reject_special_file(file).into());
        }
        Ok(())
    }

    /// Adds an action and command protos to upload. They need to be uploaded as part of the
    /// action result.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:305 UploadManifest.addAction
    fn add_action(&mut self, action_key: ActionKey, action: &Action, command: &Command) {
        blaze_util::check_state!(self.action_key.is_none(), "Already added an action");
        self.digest_to_blobs.insert(
            action_key.digest().clone(),
            Bytes::from(action.to_byte_array()),
        );
        self.digest_to_blobs.insert(
            action.command_digest.clone().unwrap_or_default(),
            Bytes::from(command.to_byte_array()),
        );
        self.action_key = Some(action_key);
    }

    /// Map of digests to file paths to upload.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:314 UploadManifest.getDigestToFile
    pub fn get_digest_to_file(&self) -> &IndexMap<Digest, Path> {
        &self.digest_to_file
    }

    /// `getStdoutDigest()`.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:319 UploadManifest.getStdoutDigest
    pub fn get_stdout_digest(&self) -> Option<&Digest> {
        self.stdout_digest.as_ref()
    }

    /// `getStderrDigest()`.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:324 UploadManifest.getStderrDigest
    pub fn get_stderr_digest(&self) -> Option<&Digest> {
        self.stderr_digest.as_ref()
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:328 UploadManifest.addFileSymbolicLink
    // The deprecated fields are still written, as Java does.
    #[allow(deprecated)]
    fn add_file_symbolic_link(&mut self, file: &Path, target: &PathFragment) {
        let output_symlink = OutputSymlink {
            path: internal_to_unicode(&self.remote_path_resolver.local_path_to_output_path(file)),
            target: internal_to_unicode(target.get_path_string()),
            ..Default::default()
        };
        self.result
            .output_file_symlinks
            .push(output_symlink.clone());
        self.result.output_symlinks.push(output_symlink);
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:338 UploadManifest.addDirectorySymbolicLink
    // The deprecated fields are still written, as Java does.
    #[allow(deprecated)]
    fn add_directory_symbolic_link(&mut self, file: &Path, target: &PathFragment) {
        let output_symlink = OutputSymlink {
            path: internal_to_unicode(&self.remote_path_resolver.local_path_to_output_path(file)),
            target: internal_to_unicode(target.get_path_string()),
            ..Default::default()
        };
        self.result
            .output_directory_symlinks
            .push(output_symlink.clone());
        self.result.output_symlinks.push(output_symlink);
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:348 UploadManifest.addFile
    fn add_file(&mut self, digest: Digest, file: &Path, stat_no_follow: &FileStatus) {
        self.result.output_files.push(OutputFile {
            path: internal_to_unicode(&self.remote_path_resolver.local_path_to_output_path(file)),
            digest: Some(digest.clone()),
            is_executable: !self.preserve_executable_bit
                || (stat_no_follow.get_permissions() & 0o100) != 0,
            ..Default::default()
        });

        self.digest_to_file.insert(digest, file.clone());
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:557 UploadManifest.addDirectory
    fn add_directory(&mut self, dir: &Path) -> Result<(), SpawnCacheError> {
        let tree_blob = DirectoryBuilder::new(self, dir.clone()).build()?;
        let tree_digest = self.digest_util.compute(&tree_blob);

        self.result.output_directories.push(OutputDirectory {
            path: internal_to_unicode(&self.remote_path_resolver.local_path_to_output_path(dir)),
            tree_digest: Some(tree_digest.clone()),
            is_topologically_sorted: true,
            ..Default::default()
        });

        self.digest_to_blobs
            .insert(tree_digest, Bytes::from(tree_blob));
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:570 UploadManifest.checkAbsoluteSymlinkAllowed
    fn check_absolute_symlink_allowed(&self, file: &Path, target: &PathFragment) -> io::Result<()> {
        if !self.allow_absolute_symlinks {
            return Err(io_exception(format!(
                "Spawn output {} is an absolute symbolic link to {}, which is not allowed by the remote cache",
                file, target
            )));
        }
        Ok(())
    }

    /// `getActionResult()`.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:599 UploadManifest.getActionResult
    pub fn get_action_result(&self) -> ActionResult {
        self.result.clone()
    }

    /// Uploads outputs and action result (if exit code is 0) to the remote and/or disk cache.
    ///
    /// * `force`: whether to upload blobs to the remote cache even if `combined_cache` has
    ///   already completed uploads of them, e.g. because they may have been evicted since
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:609 UploadManifest.upload
    pub fn upload(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        combined_cache: &CombinedCache,
        reporter: &dyn ExtendedEventHandler,
        force: bool,
    ) -> Result<ActionResult, SpawnCacheError> {
        let action = context.get_spawn_owner();
        // PORT: java-hash-order. `Sets.union(digestToBlobs.keySet(), digestToFile.keySet())`
        // iterates two `ConcurrentHashMap`s in Java; the order only decides the order of the
        // digests in the FindMissingBlobs request and the order in which the uploads start.
        let all_digests: Vec<Digest> = self
            .digest_to_blobs
            .keys()
            .chain(
                self.digest_to_file
                    .keys()
                    .filter(|digest| !self.digest_to_blobs.contains_key(*digest)),
            )
            .cloned()
            .collect();
        let missing_digests: IndexSet<Digest> =
            get_from_future(combined_cache.find_missing_digests(context, &all_digests))?;

        {
            let mut upload_futures: Vec<BoxFuture<'_, io::Result<()>>> =
                Vec::with_capacity(missing_digests.len());
            for digest in &missing_digests {
                upload_futures.push(decorate_upload_future(
                    self.upload_single_digest(context, combined_cache, digest, force),
                    reporter,
                    action,
                    Store::Cas,
                    digest,
                ));
            }
            wait_for_bulk_transfer(upload_futures)?;
        }

        // The action result must be uploaded after the Action and Command protos per the REAPI
        // protocol. We choose to upload it after all other blobs since this has historically been
        // the case and action results may fail to validate server-side if they are accessed
        // before all blobs they refer to are present.
        let action_result = self.result.clone();
        if action_result.exit_code == 0
            && let Some(action_key) = &self.action_key
        {
            get_from_future(decorate_upload_future(
                combined_cache.upload_action_result(context, action_key, &action_result),
                reporter,
                action,
                Store::Ac,
                action_key.digest(),
            ))?;
        }
        Ok(action_result)
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:658 UploadManifest.uploadSingleDigest
    async fn upload_single_digest(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        combined_cache: &CombinedCache,
        digest: &Digest,
        force: bool,
    ) -> io::Result<()> {
        if let Some(file) = self.digest_to_file.get(digest) {
            return combined_cache
                .upload_file_with_force(context, digest, file, force)
                .await;
        }

        let Some(blob) = self.digest_to_blobs.get(digest) else {
            return Err(io_exception(format!(
                "FindMissingBlobs call returned an unknown digest: {}",
                digest_to_text_format(digest)
            )));
        };

        combined_cache
            .upload_blob_with_force(context, digest, &BytesBlob(blob.clone()), force)
            .await
    }
}

/// `file.statIfFound(Symlinks.FOLLOW)`, treating a looping symlink as a dangling symlink.
fn stat_follow_if_found(file: &Path) -> io::Result<Option<FileStatus>> {
    match file.stat_if_found(Symlinks::Follow) {
        Ok(stat_follow) => Ok(stat_follow),
        // Treat a looping symlink as a dangling symlink.
        Err(e) if crate::common::errors::is_file_symlink_loop(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

// Java: com/google/devtools/build/lib/remote/UploadManifest.java:580 UploadManifest.rejectSpecialFile
fn reject_special_file(path: &Path) -> ExecException {
    // TODO(tjgq): Consider treating special files as regular, following Skyframe.
    // (On the other hand, they seem to be only useful for testing purposes, so we might instead
    // want to forbid them entirely.)
    let message = format!(
        "Spawn output {} is a special file. Only regular files, directories or symlinks may be uploaded to a remote cache.",
        path
    );

    let failure_detail = FailureDetail {
        message,
        category: Some(failure_detail::Category::RemoteExecution(RemoteExecution {
            code: remote_execution::Code::IllegalOutput as i32,
        })),
    };
    UserExecException::new(failure_detail).into()
}

// Java: com/google/devtools/build/lib/remote/UploadManifest.java:159 UploadManifest.instantToTimestamp
fn instant_to_timestamp(instant: SystemTime) -> Timestamp {
    match instant.duration_since(UNIX_EPOCH) {
        Ok(since_epoch) => Timestamp {
            seconds: since_epoch.as_secs() as i64,
            nanos: since_epoch.subsec_nanos() as i32,
        },
        // Before the epoch: `Instant.getEpochSecond()` rounds down, `getNano()` is positive.
        Err(e) => {
            let before_epoch = e.duration();
            let mut seconds = -(before_epoch.as_secs() as i64);
            let mut nanos = before_epoch.subsec_nanos() as i32;
            if nanos > 0 {
                seconds -= 1;
                nanos = 1_000_000_000 - nanos;
            }
            Timestamp { seconds, nanos }
        }
    }
}

/// `instant.plusMillis(millis)`.
fn plus_millis(instant: SystemTime, millis: i32) -> SystemTime {
    let duration = Duration::from_millis(u64::from(millis.unsigned_abs()));
    if millis >= 0 {
        instant + duration
    } else {
        instant - duration
    }
}

/// `Digest.toString()`: the protobuf text format of the message.
///
/// PORT: `blaze_protos` has no `TextFormat`; this is its output for the two scalar fields of a
/// `Digest`.
fn digest_to_text_format(digest: &Digest) -> String {
    let mut out = String::new();
    if !digest.hash.is_empty() {
        out.push_str("hash: \"");
        // Java: com/google/protobuf/TextFormatEscaper.java TextFormatEscaper.escapeBytes
        for b in digest.hash.bytes() {
            match b {
                0x07 => out.push_str("\\a"),
                0x08 => out.push_str("\\b"),
                0x0c => out.push_str("\\f"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                0x0b => out.push_str("\\v"),
                b'\\' => out.push_str("\\\\"),
                b'\'' => out.push_str("\\'"),
                b'"' => out.push_str("\\\""),
                0x20..=0x7e => out.push(b as char),
                _ => {
                    out.push('\\');
                    out.push((b'0' + ((b >> 6) & 3)) as char);
                    out.push((b'0' + ((b >> 3) & 7)) as char);
                    out.push((b'0' + (b & 7)) as char);
                }
            }
        }
        out.push_str("\"\n");
    }
    if digest.size_bytes != 0 {
        out.push_str(&format!("size_bytes: {}\n", digest.size_bytes));
    }
    out
}

/// Posts the `ActionUploadFinishedEvent` when the decorated upload completes or is cancelled
/// (Java: the listener added to the upload future).
struct PostActionUploadFinished<'a> {
    reporter: &'a dyn ExtendedEventHandler,
    event: Option<ActionUploadFinishedEvent>,
}

impl Drop for PostActionUploadFinished<'_> {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            self.reporter.post(Arc::new(event));
        }
    }
}

// Java: com/google/devtools/build/lib/remote/UploadManifest.java:678 UploadManifest.decorateUploadFuture
fn decorate_upload_future<'a>(
    future: impl Future<Output = io::Result<()>> + Send + 'a,
    reporter: &'a dyn ExtendedEventHandler,
    action: Option<&dyn ActionExecutionMetadata>,
    store: Store,
    digest: &Digest,
) -> BoxFuture<'a, io::Result<()>> {
    let Some(action) = action else {
        return Box::pin(future);
    };
    reporter.post(Arc::new(ActionUploadStartedEvent::create(
        action,
        store,
        digest.clone(),
    )));
    let finished = PostActionUploadFinished {
        reporter,
        event: Some(ActionUploadFinishedEvent::create(
            action,
            store,
            digest.clone(),
        )),
    };
    Box::pin(async move {
        let _finished = finished;
        future.await
    })
}

/// A `DirectoryBuilder` constructs a `Tree` message for an output directory.
///
/// PORT: Java extends `AbstractQueueVisitor` and visits the directory entries in parallel on the
/// shared `upload-manifest-directory-visitor` pool, failing fast. The port visits them on the
/// calling thread in the order they are queued; the result does not depend on the visiting
/// order (every collection is sorted), only which of several errors is reported does (in Java
/// that is a race).
// Java: com/google/devtools/build/lib/remote/UploadManifest.java:380 UploadManifest.DirectoryBuilder
struct DirectoryBuilder<'m> {
    manifest: &'m mut UploadManifest,
    root_dir: Path,

    /// The visits submitted with `execute(...)` that have not run yet.
    queue: VecDeque<(Path, DirentType)>,

    // Directories found during the traversal, including the root.
    // Sorted in reverse so that children iterate before parents.
    dirs: BTreeSet<Reverse<Path>>,

    // Maps each directory found during the traversal to its subdirectories.
    dir_to_subdirs: BTreeMap<Path, BTreeSet<Path>>,

    // Maps each directory found during the traversal to its files.
    dir_to_files: BTreeMap<Path, BTreeMap<Vec<u16>, FileNode>>,

    // Maps each directory found during the traversal to its symlinks.
    dir_to_symlinks: BTreeMap<Path, BTreeMap<Vec<u16>, SymlinkNode>>,
}

impl<'m> DirectoryBuilder<'m> {
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:403 DirectoryBuilder.<init>
    fn new(manifest: &'m mut UploadManifest, root_dir: Path) -> DirectoryBuilder<'m> {
        DirectoryBuilder {
            manifest,
            root_dir,
            queue: VecDeque::new(),
            dirs: BTreeSet::new(),
            dir_to_subdirs: BTreeMap::new(),
            dir_to_files: BTreeMap::new(),
            dir_to_symlinks: BTreeMap::new(),
        }
    }

    /// Returns a `Tree` message in wire format describing the directory contents, obeying the
    /// requirements of the `OutputDirectory.is_topologically_sorted` field.
    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:416 DirectoryBuilder.build
    fn build(mut self) -> Result<Vec<u8>, SpawnCacheError> {
        // Collect directory entries (subdirectories, files, symlinks).
        self.queue
            .push_back((self.root_dir.clone(), DirentType::Directory));
        while let Some((path, type_)) = self.queue.pop_front() {
            self.visit(&path, type_)?;
        }

        // Compute the Directory message for every node, including the root. Since each directory
        // references its subdirectories by their digest, the messages must be computed in
        // topological order (children before parents). In addition, the contents of each
        // Directory message must be sorted, which is already ensured by the use of sorted maps.

        let mut dir_to_digest: HashMap<Path, Digest> = HashMap::with_capacity(self.dirs.len());
        let mut dir_blobs: IndexSet<Vec<u8>> = IndexSet::with_capacity(self.dirs.len());

        for Reverse(dir) in &self.dirs {
            let mut builder = Directory::default();
            if let Some(files) = self.dir_to_files.get(dir) {
                builder.files.extend(files.values().cloned());
            }
            if let Some(symlinks) = self.dir_to_symlinks.get(dir) {
                builder.symlinks.extend(symlinks.values().cloned());
            }
            if let Some(subdirs) = self.dir_to_subdirs.get(dir) {
                for subdir in subdirs {
                    blaze_util::check_state!(subdir.get_parent_directory().as_ref() == Some(dir));
                    builder.directories.push(DirectoryNode {
                        name: internal_to_unicode(subdir.get_base_name()),
                        digest: Some(
                            dir_to_digest
                                .get(subdir)
                                .expect("NullPointerException")
                                .clone(),
                        ),
                    });
                }
            }
            let dir_blob = builder.to_byte_array();

            dir_to_digest.insert(dir.clone(), self.manifest.digest_util.compute(&dir_blob));
            dir_blobs.insert(dir_blob);
        }

        // Convert individual Directory messages to a Tree message. As we want the records to be
        // topologically sorted (parents before children), we iterate over the directories in
        // reverse insertion order. We construct the message through direct byte manipulation to
        // ensure that the strict requirements on the encoding are observed.

        let mut out: Vec<u8> = Vec::new();
        let mut field_number = TREE_ROOT_FIELD_NUMBER;
        for directory in dir_blobs.iter().rev() {
            // `codedOutputStream.writeBytes(fieldNumber, directory)`.
            encode_key(field_number, WireType::LengthDelimited, &mut out);
            encode_varint(directory.len() as u64, &mut out);
            out.extend_from_slice(directory);
            field_number = TREE_CHILDREN_FIELD_NUMBER;
        }

        Ok(out)
    }

    /// `execute(() -> visit(path, type))`.
    fn execute(&mut self, path: Path, type_: DirentType) {
        self.queue.push_back((path, type_));
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:470 DirectoryBuilder.visit
    fn visit(&mut self, path: &Path, type_: DirentType) -> Result<(), SpawnCacheError> {
        if type_ == DirentType::File {
            self.visit_as_file(path)?;
            return Ok(());
        }
        if type_ == DirentType::Directory {
            self.visit_as_directory(path);
            for dirent in path.readdir(Symlinks::Nofollow)? {
                let child_path = path.get_child(dirent.get_name());
                let child_type = dirent.get_type();
                self.execute(child_path, child_type);
            }
            return Ok(());
        }
        if type_ == DirentType::Symlink {
            let target = path.read_symbolic_link()?;
            let stat_follow = stat_follow_if_found(path)?;
            let stat_follow = match stat_follow {
                Some(stat_follow) if target.is_absolute() => stat_follow,
                _ => {
                    // Symlink uploaded as a symlink.
                    if target.is_absolute() {
                        self.manifest
                            .check_absolute_symlink_allowed(path, &target)?;
                    }
                    self.visit_as_symlink(path, &target);
                    return Ok(());
                }
            };
            if stat_follow.is_file() && !stat_follow.is_special_file() {
                // Symlink to file uploaded as a file.
                self.execute(path.clone(), DirentType::File);
                return Ok(());
            }
            if stat_follow.is_directory() {
                // Symlink to directory uploaded as a directory.
                self.execute(path.clone(), DirentType::Directory);
                return Ok(());
            }
        }
        Err(reject_special_file(path).into())
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:519 DirectoryBuilder.visitAsDirectory
    fn visit_as_directory(&mut self, path: &Path) {
        self.dirs.insert(Reverse(path.clone()));
        if *path != self.root_dir {
            self.dir_to_subdirs
                .entry(path.get_parent_directory().expect("NullPointerException"))
                .or_default()
                .insert(path.clone());
        }
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:526 DirectoryBuilder.visitAsFile
    fn visit_as_file(&mut self, path: &Path) -> io::Result<()> {
        let parent_path = path.get_parent_directory().expect("NullPointerException");
        let stat = path.stat_if_found(Symlinks::Nofollow)?;
        let digest = self.manifest.digest_util.compute_path(path)?;
        let name = internal_to_unicode(path.get_base_name());
        let node = FileNode {
            name: name.clone(),
            digest: Some(digest.clone()),
            is_executable: !self.manifest.preserve_executable_bit
                || (stat.expect("NullPointerException").get_permissions() & 0o100) != 0,
            ..Default::default()
        };
        self.manifest.digest_to_file.insert(digest, path.clone());
        // `TreeMultimap.put` keeps the node already there for a name.
        self.dir_to_files
            .entry(parent_path)
            .or_default()
            .entry(java_string_key(&name))
            .or_insert(node);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/UploadManifest.java:540 DirectoryBuilder.visitAsSymlink
    fn visit_as_symlink(&mut self, path: &Path, target: &PathFragment) {
        let parent_path = path.get_parent_directory().expect("NullPointerException");
        let name = internal_to_unicode(path.get_base_name());
        let node = SymlinkNode {
            name: name.clone(),
            target: internal_to_unicode(target.get_path_string()),
            ..Default::default()
        };
        // `TreeMultimap.put` keeps the node already there for a name.
        self.dir_to_symlinks
            .entry(parent_path)
            .or_default()
            .entry(java_string_key(&name))
            .or_insert(node);
    }
}
