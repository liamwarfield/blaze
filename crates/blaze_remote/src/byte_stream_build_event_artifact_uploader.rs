//! Port of `lib/remote/ByteStreamBuildEventArtifactUploader.java` and
//! `ByteStreamBuildEventArtifactUploaderFactory.java`: uploads the files the build event
//! protocol refers to into the remote cache and names them with `bytestream://` URIs.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};

use blaze_bep::buildeventstream::{
    BuildEventArtifactUploader, LocalFile, LocalFileType, NoConversion, PathConverter,
    PathConverterFuture,
};
use blaze_bep::runtime::BuildEventArtifactUploaderFactory;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_exec::actions::FileStateType;
use blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem;
use blaze_optdefs::misc::remote_build_event_upload_mode::RemoteBuildEventUploadMode;
use blaze_util::check_state;
use blaze_vfs::Path;
use parking_lot::Mutex;
use regex::Regex;

use crate::combined_cache::CombinedCache;
use crate::common::{CachePolicy, RemoteActionExecutionContext};
use crate::reapi::{Digest, digest_function};
use crate::util::DigestUtil;
use crate::util::tracing_metadata_utils;
use crate::util::utils::grpc_aware_error_message;

// The patterns are matched against internal path strings (one Latin-1 char per byte). Java's
// `.` does not match the line terminators `\n`, `\r`, U+0085, U+2028, U+2029, and U+0085 is the
// second byte of common UTF-8 letters (`Å` is C3 85), so `.` is spelled out as the class of
// everything else (the `regex` crate's `.` only excludes `\n`). `matcher(..).matches()` is the
// pattern anchored at both ends.
// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:68 ByteStreamBuildEventArtifactUploader.TEST_LOG_PATTERN
static TEST_LOG_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[^\n\r\x{85}\x{2028}\x{2029}]*/bazel-out/[^/]*/testlogs/[^\n\r\x{85}\x{2028}\x{2029}]*)$",
    )
    .expect("valid regex")
});
// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:69 ByteStreamBuildEventArtifactUploader.BUILD_LOG_PATTERN
static BUILD_LOG_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[^\n\r\x{85}\x{2028}\x{2029}]*/bazel-out/_tmp/actions/std(err|out)-[^\n\r\x{85}\x{2028}\x{2029}]*)$",
    )
    .expect("valid regex")
});

/// A [`BuildEventArtifactUploader`] backed by [`CombinedCache`].
///
/// PORT: Java's uploader runs its uploads on the executor `RemoteModule` passes
/// (`subscribeOn(scheduler)`); here they run as tasks of the crate's tokio runtime
/// (`crate::runtime::runtime()`), and the returned future is completed from that task.
// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:66 ByteStreamBuildEventArtifactUploader
pub struct ByteStreamBuildEventArtifactUploader {
    inner: Arc<Inner>,
    /// `AbstractReferenceCounted.refCnt`.
    ref_cnt: AtomicI32,
    shutdown: AtomicBool,
}

/// The fields the upload tasks share.
struct Inner {
    reporter: Arc<dyn ExtendedEventHandler>,
    verbose_failures: bool,
    combined_cache: Arc<CombinedCache>,
    build_request_id: String,
    command_id: String,
    remote_instance_name: String,
    remote_bytestream_uri_prefix: Option<String>,
    remote_build_event_upload_mode: RemoteBuildEventUploadMode,
}

/// Returns `true` if Bazel knows that the file is stored on a remote system.
// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:112 ByteStreamBuildEventArtifactUploader.isRemoteFile
fn is_remote_file(file: &Path) -> io::Result<bool> {
    match file
        .get_file_system()
        .as_any()
        .downcast_ref::<RemoteActionFileSystem>()
    {
        // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:293 RemoteActionFileSystem.isRemote
        Some(remote_action_file_system) => remote_action_file_system.is_remote(file.as_fragment()),
        None => Ok(false),
    }
}

// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:117 ByteStreamBuildEventArtifactUploader.PathMetadata
#[derive(Clone, Debug)]
struct PathMetadata {
    path: Path,
    digest: Option<Digest>,
    directory: bool,
    symlink: bool,
    remote: bool,
    is_build_tool_log: bool,
    digest_function: digest_function::Value,
}

impl PathMetadata {
    fn new(
        path: Path,
        digest: Option<Digest>,
        directory: bool,
        symlink: bool,
        remote: bool,
        is_build_tool_log: bool,
        digest_function: digest_function::Value,
    ) -> PathMetadata {
        PathMetadata {
            path,
            digest,
            directory,
            symlink,
            remote,
            is_build_tool_log,
            digest_function,
        }
    }
}

impl ByteStreamBuildEventArtifactUploader {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:87 ByteStreamBuildEventArtifactUploader.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reporter: Arc<dyn ExtendedEventHandler>,
        verbose_failures: bool,
        combined_cache: Arc<CombinedCache>,
        remote_instance_name: String,
        remote_bytestream_uri_prefix: Option<String>,
        build_request_id: String,
        command_id: String,
        remote_build_event_upload_mode: RemoteBuildEventUploadMode,
    ) -> ByteStreamBuildEventArtifactUploader {
        ByteStreamBuildEventArtifactUploader {
            inner: Arc::new(Inner {
                reporter,
                verbose_failures,
                combined_cache,
                build_request_id,
                command_id,
                remote_instance_name,
                remote_bytestream_uri_prefix,
                remote_build_event_upload_mode,
            }),
            ref_cnt: AtomicI32::new(1),
            shutdown: AtomicBool::new(false),
        }
    }

    /// `refCnt()`.
    pub fn ref_cnt(&self) -> i32 {
        self.ref_cnt.load(Ordering::SeqCst)
    }

    /// `upload(files).get()`: uploads and waits for the path converter.
    pub fn upload_and_wait(
        &self,
        files: HashMap<Path, LocalFile>,
    ) -> io::Result<Box<dyn PathConverter>> {
        self.upload(files)
            .get()
            .map_err(|failure| io::Error::other(failure.get_message()))
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:448 ByteStreamBuildEventArtifactUploader.deallocate
    fn deallocate(&self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.combined_cache.release();
    }
}

impl Inner {
    /// Collects metadata for `file`. Depending on the underlying filesystem used this method
    /// might do I/O.
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:177 ByteStreamBuildEventArtifactUploader.readPathMetadata
    fn read_path_metadata(&self, path: &Path, file: &LocalFile) -> io::Result<PathMetadata> {
        // PORT: Java's DigestUtil also takes the xattr provider (`env.getXattrProvider()`).
        let digest_util = DigestUtil::new(path.get_file_system().get_digest_function());
        let digest_function = digest_util.get_digest_function();
        let is_build_tool_log =
            file.type_ == LocalFileType::Log || file.type_ == LocalFileType::PerformanceLog;
        if let Some(metadata) = &file.artifact_metadata {
            match metadata.get_type() {
                FileStateType::Directory => {
                    return Ok(PathMetadata::new(
                        path.clone(),
                        /* digest= */ None,
                        /* directory= */ true,
                        /* symlink= */ false,
                        /* remote= */ false,
                        /* is_build_tool_log= */ false,
                        digest_function,
                    ));
                }
                FileStateType::Symlink => {
                    return Ok(PathMetadata::new(
                        path.clone(),
                        /* digest= */ None,
                        /* directory= */ false,
                        /* symlink= */ true,
                        /* remote= */ false,
                        /* is_build_tool_log= */ false,
                        digest_function,
                    ));
                }
                FileStateType::RegularFile => {
                    if let Some(digest) = metadata.get_digest() {
                        return Ok(PathMetadata::new(
                            path.clone(),
                            Some(DigestUtil::build_digest_from_bytes(
                                digest,
                                metadata.get_size(),
                            )),
                            /* directory= */ false,
                            /* symlink= */ false,
                            /* remote= */ metadata.is_remote(),
                            is_build_tool_log,
                            digest_function,
                        ));
                    }
                }
                _ => {}
            }
        }

        if file.type_ == LocalFileType::OutputDirectory
            || ((file.type_ == LocalFileType::SuccessfulTestOutput
                || file.type_ == LocalFileType::FailedTestOutput)
                && path.is_directory())
        {
            return Ok(PathMetadata::new(
                path.clone(),
                /* digest= */ None,
                /* directory= */ true,
                /* symlink= */ false,
                /* remote= */ false,
                /* is_build_tool_log= */ false,
                digest_function,
            ));
        }
        if file.type_ == LocalFileType::OutputSymlink {
            return Ok(PathMetadata::new(
                path.clone(),
                /* digest= */ None,
                /* directory= */ false,
                /* symlink= */ true,
                /* remote= */ false,
                /* is_build_tool_log= */ false,
                digest_function,
            ));
        }

        let digest = digest_util.compute_path(path)?;
        Ok(PathMetadata::new(
            path.clone(),
            Some(digest),
            /* directory= */ false,
            /* symlink= */ false,
            is_remote_file(path)?,
            is_build_tool_log,
            digest_function,
        ))
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:256 ByteStreamBuildEventArtifactUploader.processQueryResult
    fn process_query_result(
        missing_digests: &HashSet<Digest>,
        files_to_query: Vec<PathMetadata>,
        known_remote_paths: &mut Vec<PathMetadata>,
    ) {
        for file in files_to_query {
            let missing = file
                .digest
                .as_ref()
                .is_some_and(|digest| missing_digests.contains(digest));
            if missing {
                known_remote_paths.push(file);
            } else {
                let remote_path_metadata = PathMetadata::new(
                    file.path,
                    file.digest,
                    file.directory,
                    file.symlink,
                    /* remote= */ true,
                    file.is_build_tool_log,
                    file.digest_function,
                );
                known_remote_paths.push(remote_path_metadata);
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:278 ByteStreamBuildEventArtifactUploader.shouldUpload
    fn should_upload(&self, path: &PathMetadata) -> bool {
        let mut result = path.digest.is_some() && !path.remote && !path.directory && !path.symlink;

        if self.remote_build_event_upload_mode == RemoteBuildEventUploadMode::MINIMAL {
            result = result && (path.is_build_tool_log || Self::is_build_or_test_log(path));
        }

        result
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:289 ByteStreamBuildEventArtifactUploader.isBuildOrTestLog
    fn is_build_or_test_log(path: &PathMetadata) -> bool {
        TEST_LOG_PATTERN.is_match(path.path.get_path_string())
            || BUILD_LOG_PATTERN.is_match(path.path.get_path_string())
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:294 ByteStreamBuildEventArtifactUploader.queryCombinedCache
    async fn query_combined_cache(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        paths: Vec<PathMetadata>,
    ) -> Vec<PathMetadata> {
        let mut known_paths: Vec<PathMetadata> = Vec::with_capacity(paths.len());
        let mut files_to_query: Vec<PathMetadata> = Vec::new();
        let mut digests_to_query: Vec<Digest> = Vec::new();
        let mut seen: HashSet<Digest> = HashSet::new();
        for path in paths {
            if self.should_upload(&path) {
                let digest = path
                    .digest
                    .clone()
                    .expect("shouldUpload checked the digest");
                if seen.insert(digest.clone()) {
                    digests_to_query.push(digest);
                }
                files_to_query.push(path);
            } else {
                known_paths.push(path);
            }
        }

        if digests_to_query.is_empty() {
            return known_paths;
        }
        let missing_digests: HashSet<Digest> = match self
            .combined_cache
            .find_missing_digests(context, &digests_to_query)
            .await
        {
            Ok(missing) => missing.into_iter().collect(),
            Err(error) => {
                self.report_upload_error(&error, None, None);
                // Assuming all digests are missing if failed to query
                seen
            }
        };
        Self::process_query_result(&missing_digests, files_to_query, &mut known_paths);
        known_paths
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:325 ByteStreamBuildEventArtifactUploader.reportUploadError
    fn report_upload_error(&self, error: &io::Error, path: Option<&Path>, digest: Option<&Digest>) {
        // `if (error instanceof CancellationException) return;`
        // PORT: the crate carries a `CancellationException` (an upload cancelled by
        // `CombinedCache.shutdownNow()` after an interrupted build, `AsyncTaskCache`) as an error
        // of kind `Interrupted`, the same kind as an `InterruptedException`; these uploads run
        // on the transport's writer, which is never the interrupted command thread, so the kind
        // means the cancellation here.
        if crate::common::errors::is_interrupted(error) {
            return;
        }

        let mut error_message = "Uploading BEP referenced local file".to_string();
        if let Some(path) = path {
            error_message.push(' ');
            error_message.push_str(path.get_path_string());
        }
        if let Some(digest) = digest {
            error_message.push(' ');
            error_message.push_str(&digest_to_string(digest));
        }
        error_message.push_str(": ");
        error_message.push_str(&grpc_aware_error_message(error, self.verbose_failures));

        self.reporter.handle(&Event::warn(error_message));
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:342 ByteStreamBuildEventArtifactUploader.uploadLocalFiles
    async fn upload_local_files(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        paths: Vec<PathMetadata>,
    ) -> Vec<PathMetadata> {
        let uploads = paths.into_iter().map(|path| async move {
            if !self.should_upload(&path) {
                return path;
            }
            let digest = path
                .digest
                .clone()
                .expect("shouldUpload checked the digest");
            match self
                .combined_cache
                .upload_file(context, &digest, &path.path)
                .await
            {
                Ok(()) => PathMetadata::new(
                    path.path,
                    path.digest,
                    path.directory,
                    path.symlink,
                    // set remote to true so the PathConverter will use bytestream://
                    // scheme to convert the URI for this file
                    /* remote= */
                    true,
                    path.is_build_tool_log,
                    path.digest_function,
                ),
                Err(error) => {
                    self.report_upload_error(&error, Some(&path.path), Some(&digest));
                    path
                }
            }
        });
        futures::future::join_all(uploads).await
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:375 ByteStreamBuildEventArtifactUploader.getRemoteServerInstanceName
    async fn get_remote_server_instance_name(&self) -> io::Result<String> {
        if let Some(prefix) = &self.remote_bytestream_uri_prefix
            && !prefix.is_empty()
        {
            return Ok(prefix.clone());
        }

        let a = self.combined_cache.get_remote_authority().await?;
        if !self.remote_instance_name.is_empty() {
            return Ok(format!("{a}/{}", self.remote_instance_name));
        }
        Ok(a)
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:390 ByteStreamBuildEventArtifactUploader.doUpload
    async fn do_upload(&self, files: HashMap<Path, LocalFile>) -> io::Result<PathConverterImpl> {
        let metadata = tracing_metadata_utils::build_metadata(
            &self.build_request_id,
            &self.command_id,
            "bes-upload",
            None,
        );
        let context = RemoteActionExecutionContext::create(metadata)
            .with_write_cache_policy(CachePolicy::RemoteCacheOnly);

        // Single.using(combinedCache::retain, ..., CombinedCache::release)
        self.combined_cache.retain();
        let result = async {
            let mut paths = Vec::with_capacity(files.len());
            for (path, file) in &files {
                // PORT: Java runs this on an executor.
                let metadata = match self.read_path_metadata(path, file) {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        self.report_upload_error(&e, Some(path), None);
                        PathMetadata::new(
                            path.clone(),
                            /* digest= */ None,
                            /* directory= */ false,
                            /* symlink= */ false,
                            /* remote= */ false,
                            /* is_build_tool_log= */ false,
                            digest_function::Value::Sha256,
                        )
                    }
                };
                paths.push(metadata);
            }
            let paths = self.query_combined_cache(&context, paths).await;
            let paths = self.upload_local_files(&context, paths).await;
            let remote_server_instance_name = self.get_remote_server_instance_name().await?;
            Ok(PathConverterImpl::new(
                remote_server_instance_name,
                paths,
                self.remote_build_event_upload_mode,
            ))
        }
        .await;
        self.combined_cache.release();
        result
    }
}

/// `Digest.toString()`: the protobuf text format of the message.
// PORT: com.google.protobuf.TextFormat.printer().printToString(digest) for the two fields of a
// Digest (a field with its default value is not printed).
fn digest_to_string(digest: &Digest) -> String {
    let mut s = String::new();
    if !digest.hash.is_empty() {
        s.push_str(&format!("hash: \"{}\"\n", digest.hash));
    }
    if digest.size_bytes != 0 {
        s.push_str(&format!("size_bytes: {}\n", digest.size_bytes));
    }
    s
}

impl BuildEventArtifactUploader for ByteStreamBuildEventArtifactUploader {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:438 ByteStreamBuildEventArtifactUploader.upload
    fn upload(&self, files: HashMap<Path, LocalFile>) -> PathConverterFuture {
        if files.is_empty() {
            return PathConverterFuture::immediate(Box::new(NoConversion));
        }
        let (promise, future) = PathConverterFuture::create();
        let inner = self.inner.clone();
        crate::runtime::runtime().spawn(async move {
            match inner.do_upload(files).await {
                Ok(converter) => promise.set(Box::new(converter)),
                // The Java class of the failure travels to the BEP transport, which prints the
                // exception's `toString()` (review 2026-11-19 N7).
                Err(e) => promise.set_exception(io::Error::new(
                    e.kind(),
                    blaze_bep::buildeventstream::JavaThrowable::new(
                        crate::common::errors::java_to_string(&e),
                    ),
                )),
            }
        });
        future
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:443 ByteStreamBuildEventArtifactUploader.mayBeSlow
    fn may_be_slow(&self) -> bool {
        true
    }

    // Java: io/netty/util/AbstractReferenceCounted.java AbstractReferenceCounted.retain
    fn retain(&self) {
        let mut ref_cnt = self.ref_cnt.load(Ordering::SeqCst);
        loop {
            if ref_cnt <= 0 {
                panic!("IllegalReferenceCountException: refCnt: {ref_cnt}, increment: 1");
            }
            match self.ref_cnt.compare_exchange(
                ref_cnt,
                ref_cnt + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(actual) => ref_cnt = actual,
            }
        }
    }

    // Java: io/netty/util/AbstractReferenceCounted.java AbstractReferenceCounted.release
    fn release(&self) {
        let mut ref_cnt = self.ref_cnt.load(Ordering::SeqCst);
        loop {
            if ref_cnt <= 0 {
                panic!("IllegalReferenceCountException: refCnt: {ref_cnt}, decrement: 1");
            }
            match self.ref_cnt.compare_exchange(
                ref_cnt,
                ref_cnt - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => ref_cnt = actual,
            }
        }
        if ref_cnt == 1 {
            self.deallocate();
        }
    }
}

// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:461 ByteStreamBuildEventArtifactUploader.PathConverterImpl
struct PathConverterImpl {
    remote_server_instance_name: String,
    path_to_metadata: HashMap<Path, PathMetadata>,
    skipped_paths: HashSet<Path>,
    local_paths: HashSet<Path>,
}

impl PathConverterImpl {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:468 PathConverterImpl.<init>
    fn new(
        remote_server_instance_name: String,
        uploads: Vec<PathMetadata>,
        remote_build_event_upload_mode: RemoteBuildEventUploadMode,
    ) -> PathConverterImpl {
        let mut path_to_metadata = HashMap::with_capacity(uploads.len());
        let mut skipped_paths = HashSet::new();
        let mut local_paths = HashSet::new();
        for metadata in uploads {
            let path = metadata.path.clone();
            if metadata.digest.is_some() {
                // Always use bytestream:// in MINIMAL mode
                if remote_build_event_upload_mode == RemoteBuildEventUploadMode::MINIMAL
                    || metadata.remote
                {
                    path_to_metadata.insert(path, metadata);
                } else {
                    local_paths.insert(path);
                }
            } else {
                skipped_paths.insert(path);
            }
        }
        PathConverterImpl {
            remote_server_instance_name,
            path_to_metadata,
            skipped_paths,
            local_paths,
        }
    }
}

impl PathConverter for PathConverterImpl {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploader.java:499 PathConverterImpl.apply
    fn apply(&self, path: &Path) -> Option<String> {
        if self.local_paths.contains(path) {
            return Some(format!("file://{}", path.get_path_string()));
        }

        let Some(metadata) = self.path_to_metadata.get(path) else {
            if self.skipped_paths.contains(path) {
                return None;
            }
            // It's a programming error to reference a file that has not been uploaded.
            check_state!(
                false,
                "Illegal file reference: '{}'",
                path.get_path_string()
            );
            unreachable!()
        };

        let digest = metadata.digest.as_ref().expect("only paths with a digest");
        let digest_function = metadata.digest_function;
        let out = if DigestUtil::is_old_style_digest_function(digest_function) {
            format!(
                "bytestream://{}/blobs/{}/{}",
                self.remote_server_instance_name, digest.hash, digest.size_bytes
            )
        } else {
            format!(
                "bytestream://{}/blobs/{}/{}/{}",
                self.remote_server_instance_name,
                digest_function.as_str_name().to_ascii_lowercase(),
                digest.hash,
                digest.size_bytes
            )
        };
        Some(out)
    }
}

/// A factory for [`ByteStreamBuildEventArtifactUploader`].
// Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderFactory.java:27 ByteStreamBuildEventArtifactUploaderFactory
pub struct ByteStreamBuildEventArtifactUploaderFactory {
    reporter: Arc<dyn ExtendedEventHandler>,
    verbose_failures: bool,
    combined_cache: Arc<CombinedCache>,
    remote_instance_name: String,
    remote_bytestream_uri_prefix: Option<String>,
    build_request_id: String,
    command_id: String,
    remote_build_event_upload_mode: RemoteBuildEventUploadMode,
    uploader: Mutex<Option<Arc<ByteStreamBuildEventArtifactUploader>>>,
}

impl ByteStreamBuildEventArtifactUploaderFactory {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderFactory.java:42 ByteStreamBuildEventArtifactUploaderFactory.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reporter: Arc<dyn ExtendedEventHandler>,
        verbose_failures: bool,
        combined_cache: Arc<CombinedCache>,
        remote_instance_name: String,
        remote_bytestream_uri_prefix: Option<String>,
        build_request_id: String,
        command_id: String,
        remote_build_event_upload_mode: RemoteBuildEventUploadMode,
    ) -> ByteStreamBuildEventArtifactUploaderFactory {
        ByteStreamBuildEventArtifactUploaderFactory {
            reporter,
            verbose_failures,
            combined_cache,
            remote_instance_name,
            remote_bytestream_uri_prefix,
            build_request_id,
            command_id,
            remote_build_event_upload_mode,
            uploader: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderFactory.java:81 ByteStreamBuildEventArtifactUploaderFactory.get
    pub fn get(&self) -> Option<Arc<ByteStreamBuildEventArtifactUploader>> {
        self.uploader.lock().clone()
    }
}

impl BuildEventArtifactUploaderFactory for ByteStreamBuildEventArtifactUploaderFactory {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderFactory.java:65 ByteStreamBuildEventArtifactUploaderFactory.create
    fn create(&self) -> Arc<dyn BuildEventArtifactUploader> {
        let mut uploader = self.uploader.lock();
        check_state!(uploader.is_none(), "Already created");
        self.combined_cache.retain();
        let created = Arc::new(ByteStreamBuildEventArtifactUploader::new(
            self.reporter.clone(),
            self.verbose_failures,
            self.combined_cache.clone(),
            self.remote_instance_name.clone(),
            self.remote_bytestream_uri_prefix.clone(),
            self.build_request_id.clone(),
            self.command_id.clone(),
            self.remote_build_event_upload_mode,
        ));
        // PORT: `env.getEventBus().register(uploader)`: the uploader has no subscriber methods.
        *uploader = Some(created.clone());
        created
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_vfs::{DigestHashFunction, UnixFileSystem};

    fn path(s: &str) -> Path {
        Path::create_from_str(s, &UnixFileSystem::create(DigestHashFunction::Sha256))
    }

    fn metadata(p: &Path, remote: bool, digest_function: digest_function::Value) -> PathMetadata {
        PathMetadata::new(
            p.clone(),
            Some(DigestUtil::build_digest("abcd", 7)),
            false,
            false,
            remote,
            false,
            digest_function,
        )
    }

    /// Not in the Java test: the log patterns on internal paths. Java's `.` does not match
    /// U+0085, the second char of the internal `Å` (C3 85), so such a path is not a log; `é`
    /// (C3 A9) is matched. Expectations from `java.util.regex.Pattern` (OpenJDK 25) on the same
    /// strings: false false / true false / false false / false true.
    #[test]
    fn log_patterns_on_internal_paths_follow_java_dot() {
        let cases = [
            (
                "/home/\u{c3}\u{85}sa/ws/bazel-out/k8-fastbuild/testlogs/t/test.log",
                false,
                false,
            ),
            (
                "/home/\u{c3}\u{a9}/ws/bazel-out/k8-fastbuild/testlogs/t/test.log",
                true,
                false,
            ),
            (
                "/home/x/bazel-out/_tmp/actions/stderr-\u{c3}\u{85}",
                false,
                false,
            ),
            (
                "/home/x/bazel-out/_tmp/actions/stdout-\u{c3}\u{a9}",
                false,
                true,
            ),
        ];
        for (p, test_log, build_log) in cases {
            assert_eq!(TEST_LOG_PATTERN.is_match(p), test_log, "{p:?}");
            assert_eq!(BUILD_LOG_PATTERN.is_match(p), build_log, "{p:?}");
        }
    }

    /// The `customHashFunction_uploaded` URI format (Java needs BLAKE3, which Blaze's file
    /// systems do not offer): a new-style digest function names itself in the URI.
    #[test]
    fn path_converter_names_new_style_digest_functions() {
        let p = path("/file");
        let converter = PathConverterImpl::new(
            "localhost/instance".to_string(),
            vec![metadata(&p, true, digest_function::Value::Blake3)],
            RemoteBuildEventUploadMode::ALL,
        );
        assert_eq!(
            converter.apply(&p).as_deref(),
            Some("bytestream://localhost/instance/blobs/blake3/abcd/7")
        );
    }

    #[test]
    fn path_converter_all_mode_keeps_local_files_as_file_uris() {
        let p = path("/a b");
        let all = PathConverterImpl::new(
            "host".to_string(),
            vec![metadata(&p, false, digest_function::Value::Sha256)],
            RemoteBuildEventUploadMode::ALL,
        );
        assert_eq!(all.apply(&p).as_deref(), Some("file:///a b"));
        let minimal = PathConverterImpl::new(
            "host".to_string(),
            vec![metadata(&p, false, digest_function::Value::Sha256)],
            RemoteBuildEventUploadMode::MINIMAL,
        );
        assert_eq!(
            minimal.apply(&p).as_deref(),
            Some("bytestream://host/blobs/abcd/7")
        );
    }

    #[test]
    #[should_panic(expected = "Illegal file reference: '/other'")]
    fn path_converter_rejects_unknown_paths() {
        let converter = PathConverterImpl::new(
            "host".to_string(),
            Vec::new(),
            RemoteBuildEventUploadMode::ALL,
        );
        converter.apply(&path("/other"));
    }

    #[test]
    fn log_patterns_match_java() {
        let m = |s: &str| {
            Inner::is_build_or_test_log(&metadata(&path(s), false, digest_function::Value::Sha256))
        };
        assert!(m(
            "/o/execroot/_main/bazel-out/k8-fastbuild/testlogs/a/test.log"
        ));
        assert!(m("/o/execroot/_main/bazel-out/_tmp/actions/stderr-12"));
        assert!(m("/o/execroot/_main/bazel-out/_tmp/actions/stdout-3"));
        assert!(!m("/o/execroot/_main/bazel-out/k8-fastbuild/bin/a"));
        assert!(!m("/o/bazel-out/k8/x/testlogs/y"));
    }
}
