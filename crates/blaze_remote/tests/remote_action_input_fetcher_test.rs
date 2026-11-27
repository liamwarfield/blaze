#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java`
//! together with the tests it inherits from `ActionInputPrefetcherTestBase.java`, which run
//! against `RemoteActionInputFetcher` as the Java subclass does.
//!
//! PORT notes that apply to the whole file:
//! - Java runs the tests on an `InMemoryFileSystem` rooted at `/`; here they run on the real file
//!   system under a temporary directory, so Java's absolute paths (`/exec`, `/tmp`, `/dev`) are
//!   prefixed with that directory. The tests assume they do not run as root (permission checks).
//! - Java's `fs` is a Mockito spy of a `DelayedChmodFileSystem`: [`SpiedFileSystem`] delegates
//!   to the Unix file system, records the calls the tests verify (`createDirectory`, `chmod`,
//!   `statIfFound`), delays `chmod` and lets a test hook into `renameTo`.
//! - Java's `spy(prefetcher)` with `doAnswer(...).doDownloadFile(...)` is [`SpyPrefetcher`]: it
//!   records the `doDownloadFile` calls and answers them with the mock when one is installed.
//! - Thread interruption is cancellation: Java interrupts the waiting thread, `wait` cancels the
//!   future; here the waiting thread drops the prefetch future when the test signals it
//!   ([`wait_or_cancel`]).
//! - `TreeArtifactValue.getMetadata()` of a tree with remote children is a
//!   `TreeArtifactCompositeFileArtifactValue`, which blaze_exec's `TreeArtifactValue::build` does
//!   not port; [`tree_metadata`] builds a directory value with the tree's resolved path, the only
//!   parts of it the prefetcher reads.

use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Once, mpsc};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use blaze_actions::{
    ActionLookupData, ActionLookupKey, ActionLookupKeyRef, Artifact as BlazeArtifact, ArtifactRoot,
    RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_events::Reporter;
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::actions::cache::OutputPermissions;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, BasicVirtualActionInput, FakeOwner, FileArtifactValue,
    VirtualActionInput, empty_marker,
};
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_remote::abstract_action_input_prefetcher::{
    AbstractActionInputPrefetcher, AbstractActionInputPrefetcherBase, Priority, Reason,
    register_prefetcher,
};
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::RemoteCacheClient;
use blaze_remote::common::bulk_transfer_exception::BulkTransferException;
use blaze_remote::common::errors;
use blaze_remote::reapi::Digest;
use blaze_remote::remote_action_input_fetcher::RemoteActionInputFetcher;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_remote::runtime::{block_on, get_from_future};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemRef, Path,
    PathFragment, Symlinks, UnixFileSystem,
};
use futures::future::{BoxFuture, Either};
use indexmap::IndexMap;
use parking_lot::Mutex;

mod util;

use util::in_memory_cache_client::InMemoryCacheClient;

static SET_TOOL_VERSION: Once = Once::new();

/// `ActionInputPrefetcherTestBase.HASH_FUNCTION`.
const HASH_FUNCTION: DigestHashFunction = DigestHashFunction::Sha256;

// ---------------------------------------------------------------------------------------------
// SpiedFileSystem (DelayedChmodFileSystem + Mockito spy)
// ---------------------------------------------------------------------------------------------

/// A file system call the tests verify.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FsCall {
    CreateDirectory(PathFragment),
    Chmod(PathFragment, u32),
    StatIfFound(PathFragment, bool),
}

type RenameHook = Arc<dyn Fn(&PathFragment, &PathFragment) + Send + Sync>;

/// `SpiedFileSystem.createSpy(new DelayedChmodFileSystem(new InMemoryFileSystem(SHA256)))`.
// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:98 ActionInputPrefetcherTestBase.DelayedChmodFileSystem
struct SpiedFileSystem {
    delegate: FileSystemRef,
    chmod_delay: Mutex<Duration>,
    calls: Mutex<Vec<FsCall>>,
    rename_hook: Mutex<Option<RenameHook>>,
}

impl fmt::Debug for SpiedFileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SpiedFileSystem")
    }
}

impl SpiedFileSystem {
    fn new(delegate: FileSystemRef) -> SpiedFileSystem {
        SpiedFileSystem {
            delegate,
            chmod_delay: Mutex::new(Duration::ZERO),
            calls: Mutex::new(Vec::new()),
            rename_hook: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:121 DelayedChmodFileSystem.setChmodDelay
    fn set_chmod_delay(&self, chmod_delay: Duration) {
        *self.chmod_delay.lock() = chmod_delay;
    }

    /// `reset(fs)`.
    fn reset(&self) {
        self.calls.lock().clear();
    }

    /// The number of recorded calls equal to `call` (`verify(fs, times(n))`).
    fn count(&self, call: &FsCall) -> usize {
        self.calls.lock().iter().filter(|c| *c == call).count()
    }

    fn record(&self, call: FsCall) {
        self.calls.lock().push(call);
    }
}

impl FileSystem for SpiedFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.delegate.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.delegate.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.delegate.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.record(FsCall::CreateDirectory(path.clone()));
        self.delegate.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.create_directory_and_parents(path)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.delegate.get_file_size(path, follow_symlinks)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.delete(path)
    }
    fn delete_tree(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.delete_tree(path)
    }
    fn delete_trees_below(&self, dir: &PathFragment) -> io::Result<()> {
        self.delegate.delete_trees_below(dir)
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.delegate.get_last_modified_time(path, follow_symlinks)
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.delegate.set_last_modified_time(path, new_time)
    }
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.delegate.get_fast_digest(path)
    }
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        self.delegate.get_digest(path)
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        self.delegate.resolve_one_link(path)
    }
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.resolve_symbolic_links(path)
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.delegate.stat(path, follow_symlinks)
    }
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        self.delegate.stat_nullable(path, follow_symlinks)
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        self.record(FsCall::StatIfFound(path.clone(), follow_symlinks));
        self.delegate.stat_if_found(path, follow_symlinks)
    }
    fn is_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.is_file(path, follow_symlinks)
    }
    fn is_special_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.is_special_file(path, follow_symlinks)
    }
    fn is_symbolic_link(&self, path: &PathFragment) -> bool {
        self.delegate.is_symbolic_link(path)
    }
    fn is_directory(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.is_directory(path, follow_symlinks)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.read_symbolic_link(path)
    }
    fn read_symbolic_link_unchecked(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.read_symbolic_link_unchecked(path)
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.delegate.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.delegate.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.delegate.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.delegate.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.delegate.set_executable(path, executable)
    }
    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:108 DelayedChmodFileSystem.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.record(FsCall::Chmod(path.clone(), mode));
        let chmod_delay = *self.chmod_delay.lock();
        if !chmod_delay.is_zero() {
            std::thread::sleep(chmod_delay);
        }
        self.delegate.chmod(path, mode)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn io::Read + Send>> {
        self.delegate.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn io::Write + Send>> {
        self.delegate.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        let hook = self.rename_hook.lock().clone();
        if let Some(hook) = hook {
            hook(source_path, target_path);
        }
        self.delegate.rename_to(source_path, target_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

// ---------------------------------------------------------------------------------------------
// ActionsTestUtil
// ---------------------------------------------------------------------------------------------

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Debug)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        // `ActionsTestUtil.NULL_LABEL`.
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        1
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `ActionsTestUtil.NULL_TEMPLATE_EXPANSION_ARTIFACT_OWNER`
/// (`ActionTemplateExpansionKey.of(NULL_ARTIFACT_OWNER, 0)`).
#[derive(Debug)]
struct NullTemplateExpansionArtifactOwner;

impl fmt::Display for NullTemplateExpansionArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_TEMPLATE_EXPANSION_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullTemplateExpansionArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        // `SkyFunctions.ACTION_TEMPLATE_EXPANSION`.
        "ACTION_TEMPLATE_EXPANSION"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullTemplateExpansionArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        3
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn null_artifact_owner() -> ActionLookupKeyRef {
    ActionLookupKeyRef::new(NullArtifactOwner)
}

/// An artifact as an `ActionInput`.
fn input(artifact: &BlazeArtifact) -> ActionInputRef {
    ActionInputRef::new(artifact.clone())
}

/// `ActionsTestUtil.createArtifact(root, path)` for a `Path` under an output root.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:260 ActionsTestUtil.createArtifact
fn create_artifact(root: &ArtifactRoot, path: &Path) -> BlazeArtifact {
    let exec_path = root
        .get_exec_path()
        .get_relative(&root.get_root().relativize(path));
    BlazeArtifact::create_derived_artifact(root.clone(), exec_path, null_artifact_owner())
}

/// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(root, rootRelativePath)`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:298 ActionsTestUtil.createTreeArtifactWithGeneratingAction
fn create_tree_artifact_with_generating_action(
    root: &ArtifactRoot,
    root_relative_path: &str,
) -> BlazeArtifact {
    let tree_artifact = BlazeArtifact::create_special_artifact(
        root.clone(),
        root.get_exec_path()
            .get_relative(&PathFragment::create(root_relative_path)),
        null_artifact_owner(),
        SpecialArtifactType::TREE,
    );
    tree_artifact.set_generating_action_key(ActionLookupData::create(null_artifact_owner(), 0));
    tree_artifact
}

/// `TreeArtifactValue.getMetadata()` of a tree (see the file comment).
fn tree_metadata(resolved_path: Option<PathFragment>) -> FileArtifactValue {
    let metadata = FileArtifactValue::HashedDirectory { digest: Vec::new() };
    match resolved_path {
        Some(resolved_path) => {
            FileArtifactValue::create_from_existing_with_resolved_path(metadata, resolved_path)
        }
        None => metadata,
    }
}

// ---------------------------------------------------------------------------------------------
// The prefetcher spy
// ---------------------------------------------------------------------------------------------

type DownloadResult = BoxFuture<'static, io::Result<()>>;
type ResultSupplier = Arc<dyn Fn() -> DownloadResult + Send + Sync>;
type Cas = HashMap<Vec<u8>, Vec<u8>>;

/// `spy(createPrefetcher(cas))`.
struct SpyPrefetcher {
    inner: RemoteActionInputFetcher,
    /// The exec paths of the inputs `doDownloadFile` was called with.
    do_download_file_calls: Mutex<Vec<PathFragment>>,
    /// `mockDownload(prefetcher, cas, resultSupplier)`.
    mock: Mutex<Option<(Arc<Cas>, ResultSupplier)>>,
}

impl SpyPrefetcher {
    /// `verify(prefetcher, times(n)).doDownloadFile(eq(action), any(), eq(input), ...)`.
    fn do_download_file_count(&self, input: &BlazeArtifact) -> usize {
        self.do_download_file_calls
            .lock()
            .iter()
            .filter(|p| *p == input.get_exec_path())
            .count()
    }

    fn do_download_file_calls(&self) -> usize {
        self.do_download_file_calls.lock().len()
    }
}

#[async_trait]
impl AbstractActionInputPrefetcher for SpyPrefetcher {
    fn base(&self) -> &AbstractActionInputPrefetcherBase {
        self.inner.base()
    }
    fn can_download_file(&self, path: &Path, metadata: &FileArtifactValue) -> bool {
        self.inner.can_download_file(path, metadata)
    }
    fn force_refetch(&self, path: &Path) -> bool {
        self.inner.force_refetch(path)
    }
    fn prefetch_virtual_action_input(&self, input: &dyn VirtualActionInput) -> io::Result<()> {
        self.inner.prefetch_virtual_action_input(input)
    }
    async fn do_download_file(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        reporter: &Reporter,
        input: &ActionInputRef,
        temp_path: &Path,
        metadata: &FileArtifactValue,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        self.do_download_file_calls
            .lock()
            .push(input.get_exec_path().clone());
        let mock = self.mock.lock().clone();
        match mock {
            None => {
                self.inner
                    .do_download_file(
                        action, reporter, input, temp_path, metadata, priority, reason,
                    )
                    .await
            }
            // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1185 ActionInputPrefetcherTestBase.mockDownload
            Some((cas, result_supplier)) => {
                let content = cas.get(metadata.get_digest().unwrap());
                let Some(content) = content else {
                    return Err(io::Error::other("Not found"));
                };
                fs_utils::write_content(temp_path, content)?;
                result_supplier().await
            }
        }
    }
}

/// `mockDownload(prefetcher, cas, resultSupplier)`.
// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1181 ActionInputPrefetcherTestBase.mockDownload
fn mock_download(prefetcher: &SpyPrefetcher, cas: &Cas, result_supplier: ResultSupplier) {
    *prefetcher.mock.lock() = Some((Arc::new(cas.clone()), result_supplier));
}

/// A download that never finishes (`SettableFuture.create()` never set); records its
/// cancellation (the drop of the future).
struct NeverFinishes {
    cancelled: Arc<AtomicBool>,
}

impl Future for NeverFinishes {
    type Output = io::Result<()>;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl Drop for NeverFinishes {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------------------------

type MetadataMap = IndexMap<ActionInputRef, FileArtifactValue>;

/// `metadata::get` as a `MetadataSupplier`.
fn supplier(
    metadata: &MetadataMap,
) -> impl Fn(&ActionInputRef) -> io::Result<Option<FileArtifactValue>> + Send + Sync + '_ {
    move |input: &ActionInputRef| Ok(metadata.get(input).cloned())
}

/// `metadata.keySet()`.
fn keys(metadata: &MetadataMap) -> Vec<ActionInputRef> {
    metadata.keys().cloned().collect()
}

/// `wait(future)`.
// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1163 ActionInputPrefetcherTestBase.wait
fn wait(future: impl Future<Output = io::Result<()>>) -> io::Result<()> {
    get_from_future(future)
}

/// `wait(future)` on a thread that the test interrupts: `None` when `cancel` fired first (the
/// future is dropped, Java's `future.cancel(true)`).
fn wait_or_cancel(
    future: impl Future<Output = io::Result<()>>,
    cancel: futures::channel::oneshot::Receiver<()>,
) -> Option<io::Result<()>> {
    match block_on(futures::future::select(Box::pin(future), cancel)) {
        Either::Left((result, _)) => Some(result),
        Either::Right(_) => None,
    }
}

/// The fields of the Java test classes, as `setUp` leaves them.
struct Fixture {
    dir: tempfile::TempDir,
    fs: Arc<SpiedFileSystem>,
    fs_ref: FileSystemRef,
    exec_root: Path,
    artifact_root: ArtifactRoot,
    temp_path_generator: Arc<TempPathGenerator>,
    action: Arc<dyn ActionExecutionMetadata>,
    digest_util: DigestUtil,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The prefetcher leaves read-only directories behind; make them deletable.
        make_writable(self.dir.path());
    }
}

fn make_writable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        let mode = metadata.permissions().mode();
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o700));
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                make_writable(&entry.path());
            }
        }
    }
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:135 ActionInputPrefetcherTestBase.setUp
    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:66 RemoteActionInputFetcherTest.setUp
    fn set_up() -> Fixture {
        SET_TOOL_VERSION.call_once(|| tracing_metadata_utils::set_tool_version(""));

        // `action = mock(ActionExecutionMetadata.class)` with mnemonic "DummyAction" and owner
        // `NULL_ACTION_OWNER`.
        let action: Arc<dyn ActionExecutionMetadata> = Arc::new(FakeOwner::new(
            "DummyAction",
            None,
            Some(Label::parse_canonical("//null/action:owner").unwrap()),
        ));

        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let fs = Arc::new(SpiedFileSystem::new(UnixFileSystem::create(HASH_FUNCTION)));
        let fs_ref: FileSystemRef = fs.clone();
        let exec_root = Path::create_from_str(&format!("{root}/exec"), &fs_ref);
        exec_root.create_directory_and_parents().unwrap();
        let artifact_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["root"]);
        artifact_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let temp_dir = Path::create_from_str(&format!("{root}/tmp"), &fs_ref);
        temp_dir.create_directory_and_parents().unwrap();
        let temp_path_generator = Arc::new(TempPathGenerator::new(temp_dir));

        // RemoteActionInputFetcherTest.setUp
        let dev = Path::create_from_str(&format!("{root}/dev"), &fs_ref);
        dev.create_directory().unwrap();
        dev.set_writable(false).unwrap();

        Fixture {
            dir,
            fs,
            fs_ref,
            exec_root,
            artifact_root,
            temp_path_generator,
            action,
            digest_util: DigestUtil::new(HASH_FUNCTION),
        }
    }

    fn action(&self) -> Option<&dyn ActionExecutionMetadata> {
        Some(&*self.action)
    }

    /// `fs.getPath(fragment)`.
    fn get_path(&self, fragment: &PathFragment) -> Path {
        Path::create(fragment.clone(), &self.fs_ref)
    }

    fn artifact_root_path(&self) -> Path {
        self.artifact_root.get_root().as_path().unwrap().clone()
    }

    /// `HASH_FUNCTION.getHashFunction().hashBytes(contents).asBytes()`.
    fn hash(&self, contents: &[u8]) -> Vec<u8> {
        self.digest_util.hash(contents)
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:157 ActionInputPrefetcherTestBase.createRemoteArtifact
    fn create_remote_artifact_with_resolved_path(
        &self,
        path_fragment: &str,
        contents: &str,
        resolved_path: Option<PathFragment>,
        metadata: &mut MetadataMap,
        cas: Option<&mut Cas>,
    ) -> BlazeArtifact {
        let p = self
            .artifact_root
            .get_root()
            .get_relative(&PathFragment::create(path_fragment));
        let a = create_artifact(&self.artifact_root, &p);
        let contents_bytes = contents.as_bytes();
        let hash_code = self.hash(contents_bytes);
        let mut f = FileArtifactValue::create_for_remote_file_with_materialization_data(
            hash_code.clone(),
            contents_bytes.len() as i64,
            /* locationIndex= */ 1,
            /* expirationTime= */ None,
            /* inMemoryOutput= */ false,
        );
        if let Some(resolved_path) = resolved_path {
            f = FileArtifactValue::create_from_existing_with_resolved_path(f, resolved_path);
        }
        metadata.insert(input(&a), f);
        if let Some(cas) = cas {
            cas.insert(hash_code, contents_bytes.to_vec());
        }
        a
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:183 ActionInputPrefetcherTestBase.createRemoteArtifact
    fn create_remote_artifact(
        &self,
        path_fragment: &str,
        contents: &str,
        metadata: &mut MetadataMap,
        cas: Option<&mut Cas>,
    ) -> BlazeArtifact {
        self.create_remote_artifact_with_resolved_path(
            path_fragment,
            contents,
            /* resolvedPath= */ None,
            metadata,
            cas,
        )
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:191 ActionInputPrefetcherTestBase.createRemoteTreeArtifact
    #[allow(clippy::too_many_arguments)]
    fn create_remote_tree_artifact_full(
        &self,
        path_fragment: &str,
        local_content_map: &[(&str, &str)],
        remote_content_map: &[(&str, &str)],
        resolved_path: Option<PathFragment>,
        metadata: &mut MetadataMap,
        cas: &mut Cas,
        is_action_template_expansion: bool,
    ) -> (BlazeArtifact, Vec<BlazeArtifact>) {
        let parent =
            create_tree_artifact_with_generating_action(&self.artifact_root, path_fragment);

        let mut children: Vec<BlazeArtifact> = Vec::new();
        for (key, value) in local_content_map {
            let child = BlazeArtifact::create_tree_output(&parent, PathFragment::create(key));
            let contents = value.as_bytes();
            let hash_code = self.hash(contents);
            let child_value = FileArtifactValue::create_for_normal_file(
                Some(hash_code.clone()),
                /* proxy= */ None,
                contents.len() as i64,
            );
            metadata.insert(input(&child), child_value);
            children.push(child);
            cas.insert(hash_code, contents.to_vec());
        }
        for (key, value) in remote_content_map {
            let parent_relative_path = PathFragment::create(key);
            let child = if is_action_template_expansion {
                BlazeArtifact::create_template_expansion_output(
                    &parent,
                    parent_relative_path,
                    ActionLookupKeyRef::new(NullTemplateExpansionArtifactOwner),
                )
            } else {
                BlazeArtifact::create_tree_output(&parent, parent_relative_path)
            };
            let contents = value.as_bytes();
            let hash_code = self.hash(contents);
            let child_value = FileArtifactValue::create_for_remote_file_with_materialization_data(
                hash_code.clone(),
                contents.len() as i64,
                /* locationIndex= */ 1,
                /* expirationTime= */ None,
                /* inMemoryOutput= */ false,
            );
            metadata.insert(input(&child), child_value);
            children.push(child);
            cas.insert(hash_code, contents.to_vec());
        }

        metadata.insert(input(&parent), tree_metadata(resolved_path));

        // `treeValue.getChildren().asList()`: sorted by exec path.
        children.sort_by(|a, b| a.get_exec_path().cmp(b.get_exec_path()));
        (parent, children)
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:248 ActionInputPrefetcherTestBase.createRemoteTreeArtifact
    fn create_remote_tree_artifact(
        &self,
        path_fragment: &str,
        local_content_map: &[(&str, &str)],
        remote_content_map: &[(&str, &str)],
        metadata: &mut MetadataMap,
        cas: &mut Cas,
    ) -> (BlazeArtifact, Vec<BlazeArtifact>) {
        self.create_remote_tree_artifact_full(
            path_fragment,
            local_content_map,
            remote_content_map,
            /* resolvedPath= */ None,
            metadata,
            cas,
            /* isActionTemplateExpansion= */ false,
        )
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:265 ActionInputPrefetcherTestBase.createRemoteTreeArtifact
    fn create_remote_tree_artifact_with_resolved_path(
        &self,
        path_fragment: &str,
        local_content_map: &[(&str, &str)],
        remote_content_map: &[(&str, &str)],
        resolved_path: PathFragment,
        metadata: &mut MetadataMap,
        cas: &mut Cas,
    ) -> (BlazeArtifact, Vec<BlazeArtifact>) {
        self.create_remote_tree_artifact_full(
            path_fragment,
            local_content_map,
            remote_content_map,
            Some(resolved_path),
            metadata,
            cas,
            /* isActionTemplateExpansion= */ false,
        )
    }

    // Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:282 ActionInputPrefetcherTestBase.createRemoteTreeArtifactForActionTemplateExpansion
    fn create_remote_tree_artifact_for_action_template_expansion(
        &self,
        path_fragment: &str,
        local_content_map: &[(&str, &str)],
        remote_content_map: &[(&str, &str)],
        metadata: &mut MetadataMap,
        cas: &mut Cas,
    ) -> (BlazeArtifact, Vec<BlazeArtifact>) {
        self.create_remote_tree_artifact_full(
            path_fragment,
            local_content_map,
            remote_content_map,
            /* resolvedPath= */ None,
            metadata,
            cas,
            /* isActionTemplateExpansion= */ true,
        )
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:284 RemoteActionInputFetcherTest.newCombinedCache
    fn new_combined_cache(&self, cas: &Cas) -> Arc<CombinedCache> {
        let mut cache_entries: HashMap<Digest, Vec<u8>> = HashMap::with_capacity(cas.len());
        for (key, value) in cas {
            cache_entries.insert(
                DigestUtil::build_digest_from_bytes(key, value.len() as i64),
                value.clone(),
            );
        }
        let remote_cache_client: Arc<dyn RemoteCacheClient> =
            Arc::new(InMemoryCacheClient::new_with_cas_entries(cache_entries));
        Arc::new(CombinedCache::new(
            Some(remote_cache_client),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            self.digest_util.clone(),
            /* chunkingFunction= */ None,
        ))
    }

    /// `new RemoteActionInputFetcher(new Reporter(eventBus), "none", "none", combinedCache,
    /// execRoot, tempPathGenerator, DUMMY_REMOTE_OUTPUT_CHECKER,
    /// ActionOutputDirectoryHelper.createForTesting(), OutputPermissions.READONLY)`.
    fn new_fetcher(&self, cas: &Cas) -> RemoteActionInputFetcher {
        RemoteActionInputFetcher::new(
            Reporter::new(None),
            "none",
            "none",
            self.new_combined_cache(cas),
            self.exec_root.clone(),
            self.temp_path_generator.clone(),
            dummy_remote_output_checker(),
            Some(Arc::new(ActionOutputDirectoryHelper::new())),
            OutputPermissions::Readonly,
        )
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionInputFetcherTest.java:75 RemoteActionInputFetcherTest.createPrefetcher
    fn create_prefetcher(&self, cas: &Cas) -> Arc<dyn AbstractActionInputPrefetcher> {
        register_prefetcher(self.new_fetcher(cas))
    }

    /// `spy(createPrefetcher(cas))`.
    fn create_spy(&self, cas: &Cas) -> Arc<SpyPrefetcher> {
        register_prefetcher(SpyPrefetcher {
            inner: self.new_fetcher(cas),
            do_download_file_calls: Mutex::new(Vec::new()),
            mock: Mutex::new(None),
        })
    }

    /// `tempPathGenerator.getTempDir().getDirectoryEntries()`.
    fn temp_dir_entries(&self) -> Vec<Path> {
        self.temp_path_generator
            .get_temp_dir()
            .get_directory_entries()
            .unwrap()
    }
}

/// `DUMMY_REMOTE_OUTPUT_CHECKER = new RemoteOutputChecker("build", RemoteOutputsMode.MINIMAL,
/// ImmutableList.of())`.
fn dummy_remote_output_checker() -> Arc<RemoteOutputChecker> {
    Arc::new(RemoteOutputChecker::new(
        "build",
        RemoteOutputsMode::MINIMAL,
        Vec::new(),
    ))
}

fn read_content(path: &Path) -> String {
    String::from_utf8(fs_utils::read_content(path).unwrap()).unwrap()
}

fn set_of(paths: &[Path]) -> HashSet<Path> {
    paths.iter().cloned().collect()
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1202 ActionInputPrefetcherTestBase.assertReadableNonWritableAndExecutable
fn assert_readable_non_writable_and_executable(path: &Path) {
    assert!(path.is_readable().unwrap(), "{path} should be readable");
    assert!(
        !path.is_writable().unwrap(),
        "{path} should not be writable"
    );
    assert!(path.is_executable().unwrap(), "{path} should be executable");
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1208 ActionInputPrefetcherTestBase.assertTreeReadableNonWritableAndExecutable
fn assert_tree_readable_non_writable_and_executable(path: &Path) {
    assert!(path.is_directory(), "IllegalStateException");
    assert_readable_non_writable_and_executable(path);
    for dirent in path.readdir(Symlinks::Nofollow).unwrap() {
        if dirent.get_type() == DirentType::Directory {
            assert_tree_readable_non_writable_and_executable(&path.get_child(dirent.get_name()));
        }
    }
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1218 ActionInputPrefetcherTestBase.assertReadableWritableAndExecutable
fn assert_readable_writable_and_executable(path: &Path) {
    assert!(path.is_readable().unwrap(), "{path} should be readable");
    assert!(path.is_writable().unwrap(), "{path} should be writable");
    assert!(path.is_executable().unwrap(), "{path} should be executable");
}

// Java: com/google/devtools/build/lib/remote/ActionInputPrefetcherTestBase.java:1224 ActionInputPrefetcherTestBase.assertTreeReadableWritableAndExecutable
fn assert_tree_readable_writable_and_executable(path: &Path) {
    assert!(path.is_directory(), "IllegalStateException");
    assert_readable_writable_and_executable(path);
    for dirent in path.readdir(Symlinks::Nofollow).unwrap() {
        if dirent.get_type() == DirentType::Directory {
            assert_tree_readable_writable_and_executable(&path.get_child(dirent.get_name()));
        }
    }
}

/// Polls `condition` until it holds or `timeout` passes.
fn wait_until(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::yield_now();
    }
    condition()
}

// ---------------------------------------------------------------------------------------------
// ActionInputPrefetcherTestBase
// ---------------------------------------------------------------------------------------------

#[test]
fn prefetch_files_file_exists_do_not_download() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("file", "hello world", &mut metadata, Some(&mut cas));
    fs_utils::write_content(&a.get_path(), b"hello world").unwrap();
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(spy.do_download_file_count(&a), 0);
    assert_eq!(prefetcher.downloaded_files(), set_of(&[a.get_path()]));
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_file_exists_records_contents_proxy() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("file", "hello world", &mut metadata, Some(&mut cas));
    fs_utils::write_content(&a.get_path(), b"hello world").unwrap();
    let prefetcher = f.create_prefetcher(&cas);
    assert!(metadata[&input(&a)].get_contents_proxy().is_none());

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    // The already present file was verified to be up to date by digest and its contents proxy
    // was recorded to make future modification checks cheaper.
    assert_eq!(
        metadata[&input(&a)].get_contents_proxy(),
        Some(blaze_exec::actions::FileContentsProxy::create(
            &a.get_path().stat().unwrap()
        ))
    );
}

#[test]
fn prefetch_files_file_exists_but_content_mismatches_download() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("file", "hello world remote", &mut metadata, Some(&mut cas));
    fs_utils::write_content(&a.get_path(), b"hello world local").unwrap();
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(spy.do_download_file_count(&a), 1);
    assert_eq!(prefetcher.downloaded_files(), set_of(&[a.get_path()]));
    assert!(prefetcher.downloads_in_progress().is_empty());
    assert_eq!(read_content(&a.get_path()), "hello world remote");
}

#[test]
fn invalidate_downloads_redownloads_file_deleted_after_download() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let invalidated =
        f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));
    let not_invalidated =
        f.create_remote_artifact("file2", "fizz buzz", &mut metadata, Some(&mut cas));
    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_eq!(read_content(&invalidated.get_path()), "hello world");

    // Both files are deleted behind the prefetcher's back, but only one of them is invalidated.
    invalidated.get_path().delete().unwrap();
    not_invalidated.get_path().delete().unwrap();
    prefetcher.invalidate_downloads([invalidated.get_exec_path()]);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(read_content(&invalidated.get_path()), "hello world");
    // Downloads that weren't invalidated are still assumed to be in place.
    assert!(!not_invalidated.get_path().exists());
}

#[test]
fn invalidate_downloads_intact_file_not_redownloaded() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("file", "hello world", &mut metadata, Some(&mut cas));
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_eq!(read_content(&a.get_path()), "hello world");

    f.fs.reset();
    prefetcher.invalidate_downloads([a.get_exec_path()]);
    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    // The second prefetch checks the file against the file system rather than assuming that it
    // is still in place, but doesn't download it again as it is intact.
    assert_eq!(
        f.fs.count(&FsCall::StatIfFound(
            a.get_path().as_fragment().clone(),
            /* followSymlinks= */ true
        )),
        1
    );
    assert_eq!(spy.do_download_file_count(&a), 1);
    assert_eq!(read_content(&a.get_path()), "hello world");
}

#[test]
fn prefetch_files_download_remote_files() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a1 = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));
    let a2 = f.create_remote_artifact("file2", "fizz buzz", &mut metadata, Some(&mut cas));
    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(read_content(&a1.get_path()), "hello world");
    assert_readable_non_writable_and_executable(&a1.get_path());
    assert_eq!(read_content(&a2.get_path()), "fizz buzz");
    assert_readable_non_writable_and_executable(&a2.get_path());
    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[a1.get_path(), a2.get_path()])
    );
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_files_with_resolved_path() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let resolved_path = f
        .artifact_root_path()
        .get_child("target")
        .as_fragment()
        .clone();
    let a = f.create_remote_artifact_with_resolved_path(
        "file",
        "hello world",
        Some(resolved_path.clone()),
        &mut metadata,
        Some(&mut cas),
    );
    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert!(a.get_path().is_symbolic_link());
    assert_eq!(a.get_path().read_symbolic_link().unwrap(), resolved_path);
    assert_eq!(read_content(&a.get_path()), "hello world");
    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[a.get_path(), f.get_path(&resolved_path)])
    );
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_trees() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */ &[("file1", "content1"), ("nested_dir/file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    let inputs: Vec<ActionInputRef> = children.iter().map(input).collect();
    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &inputs,
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(read_content(&first_child.get_path()), "content1");
    assert_eq!(read_content(&second_child.get_path()), "content2");

    assert_tree_readable_non_writable_and_executable(&tree.get_path());

    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[first_child.get_path(), second_child.get_path()])
    );
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_trees_partial() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[("file1", "content1")],
        /* remoteContentMap= */ &[("file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(first_child), input(second_child)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert!(!first_child.get_path().exists());
    assert_eq!(read_content(&second_child.get_path()), "content2");
    assert_tree_readable_non_writable_and_executable(&tree.get_path());
    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[second_child.get_path()])
    );
}

#[test]
fn prefetch_files_download_remote_trees_with_resolved_path() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let resolved_path = f
        .artifact_root_path()
        .get_child("target")
        .as_fragment()
        .clone();
    let (tree, children) = f.create_remote_tree_artifact_with_resolved_path(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */ &[("file1", "content1"), ("nested_dir/file2", "content2")],
        resolved_path.clone(),
        &mut metadata,
        &mut cas,
    );
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    let inputs: Vec<ActionInputRef> = children.iter().map(input).collect();
    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &inputs,
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert!(tree.get_path().is_symbolic_link());
    assert_eq!(tree.get_path().read_symbolic_link().unwrap(), resolved_path);
    assert_eq!(read_content(&first_child.get_path()), "content1");
    assert_eq!(read_content(&second_child.get_path()), "content2");

    assert_tree_readable_non_writable_and_executable(&f.get_path(&resolved_path));

    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[
            tree.get_path(),
            f.get_path(&resolved_path)
                .get_relative(&first_child.get_parent_relative_path().unwrap()),
            f.get_path(&resolved_path)
                .get_relative(&second_child.get_parent_relative_path().unwrap()),
        ])
    );
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_download_remote_trees_for_action_template_expansion() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact_for_action_template_expansion(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */
        &[("subdir/file1", "content1"), ("subdir/file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(first_child)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_tree_readable_writable_and_executable(&tree.get_path());

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(second_child)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_tree_readable_writable_and_executable(&tree.get_path());
}

#[test]
fn prefetch_files_missing_files_fails() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let a = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut Cas::new()));
    let prefetcher = f.create_prefetcher(&Cas::new());

    let result = wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&a)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ));
    assert!(result.is_err());

    assert!(prefetcher.downloaded_files().is_empty());
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_ignore_non_remote_files() {
    // Test that non-remote files are not downloaded.
    let f = Fixture::set_up();

    let p = f
        .exec_root
        .get_relative(f.artifact_root.get_exec_path())
        .get_relative_str("file1");
    fs_utils::write_content(&p, b"hello world").unwrap();
    let a = create_artifact(&f.artifact_root, &p);
    let file_value = FileArtifactValue::create_for_testing(&a.get_path()).unwrap();
    let mut metadata = MetadataMap::new();
    metadata.insert(input(&a), file_value);
    let prefetcher = f.create_prefetcher(&Cas::new());

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&a)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert!(prefetcher.downloaded_files().is_empty());
    assert!(prefetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_ignore_non_remote_files_tree() {
    // Test that non-remote tree files are not downloaded, but other files in the tree are.
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        &[("file1", "content1")],
        &[("file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    let inputs: Vec<ActionInputRef> = children.iter().map(input).collect();
    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &inputs,
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert!(!first_child.get_path().exists());
    assert_eq!(read_content(&second_child.get_path()), "content2");
    assert_tree_readable_non_writable_and_executable(&tree.get_path());
    assert_eq!(
        prefetcher.downloaded_files(),
        set_of(&[second_child.get_path()])
    );
}

#[test]
fn prefetch_files_tree_files_minimize_filesystem_operations() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */
        &[("subdir/file1", "content1"), ("subdir/file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let root = tree.get_path().as_fragment().clone();
    let subdir = tree.get_path().get_child("subdir").as_fragment().clone();
    let first_child = &children[0];
    let second_child = &children[1];

    let prefetcher = f.create_prefetcher(&cas);

    f.fs.reset();

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(first_child), input(second_child)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(f.fs.count(&FsCall::CreateDirectory(root.clone())), 1);
    assert_eq!(f.fs.count(&FsCall::CreateDirectory(subdir.clone())), 1);
    assert_eq!(f.fs.count(&FsCall::Chmod(root.clone(), 0o555)), 1);
    assert_eq!(f.fs.count(&FsCall::Chmod(subdir.clone(), 0o555)), 1);

    f.fs.reset();

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(first_child), input(second_child)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(f.fs.count(&FsCall::CreateDirectory(root.clone())), 0);
    assert_eq!(f.fs.count(&FsCall::CreateDirectory(subdir.clone())), 0);
    assert_eq!(f.fs.count(&FsCall::Chmod(root, 0o555)), 0);
    assert_eq!(f.fs.count(&FsCall::Chmod(subdir, 0o555)), 0);
}

#[test]
fn prefetch_files_tree_files_sequential_calls_reopen_ancestors() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */
        &[("subdir1/file1", "content1"), ("subdir2/file2", "content2")],
        &mut metadata,
        &mut cas,
    );
    let prefetcher = f.create_prefetcher(&cas);

    // The first call leaves the directory helper's cache populated but the tree read-only. The
    // second call must explicitly reopen the cached root before creating a sibling directory,
    // then restore its output permissions.
    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&children[0])],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_tree_readable_non_writable_and_executable(&tree.get_path());

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&children[1])],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_eq!(read_content(&children[1].get_path()), "content2");
    assert_tree_readable_non_writable_and_executable(&tree.get_path());
}

#[test]
fn prefetch_files_tree_files_multiple_threads_wait_for_permissions_to_be_set() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */ &[("subdir/file", "content")],
        &mut metadata,
        &mut cas,
    );
    assert_eq!(children.len(), 1);
    let child = &children[0];

    let prefetcher = f.create_prefetcher(&cas);

    // Prefetch the same tree artifact in two concurrent calls.
    // Verify that the second waits until the download operation completes *and* sets the output
    // permissions on the entire tree artifact before returning.
    // Delay the chmod() calls to make it much more likely that we'd catch a bug where the second
    // call returns after the download completes but before the permissions have been set.
    // Regression test for b/299934607.

    f.fs.set_chmod_delay(Duration::from_millis(100));

    let prefetch = || {
        wait(prefetcher.prefetch_files_interruptibly(
            f.action(),
            &[input(child)],
            &supplier(&metadata),
            Priority::MEDIUM,
            Reason::INPUTS,
        ))
        .unwrap();
        assert_tree_readable_non_writable_and_executable(&tree.get_path());
    };

    std::thread::scope(|s| {
        let f1 = s.spawn(prefetch);
        let f2 = s.spawn(prefetch);
        f1.join().unwrap();
        f2.join().unwrap();
    });
}

/// Sends a signal when dropped (Java's `finally` blocks that release the paused threads).
struct SignalOnDrop(Option<mpsc::Sender<()>>);

impl Drop for SignalOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

#[test]
fn prefetch_files_tree_files_concurrent_finalizations_do_not_race_permissions() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let (tree, children) = f.create_remote_tree_artifact(
        "dir",
        /* localContentMap= */ &[],
        /* remoteContentMap= */
        &[("subdir1/file1", "content1"), ("subdir2/file2", "content2")],
        &mut metadata,
        &mut cas,
    );

    let (first_download_tx, first_download_rx) = futures::channel::oneshot::channel::<()>();
    let (second_download_tx, second_download_rx) = futures::channel::oneshot::channel::<()>();
    let downloads: Arc<Mutex<VecDeque<DownloadResult>>> = Arc::new(Mutex::new(VecDeque::new()));
    downloads.lock().push_back(Box::pin(async move {
        first_download_rx
            .await
            .map_err(|_| io::Error::other("cancelled"))
    }));
    downloads.lock().push_back(Box::pin(async move {
        second_download_rx
            .await
            .map_err(|_| io::Error::other("cancelled"))
    }));
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();
    {
        let downloads = downloads.clone();
        mock_download(
            &spy,
            &cas,
            Arc::new(move || -> DownloadResult {
                downloads
                    .lock()
                    .pop_front()
                    .expect("NoSuchElementException")
            }),
        );
    }

    let (second_finalization_started_tx, second_finalization_started_rx) = mpsc::channel::<()>();
    let (second_finalization_may_continue_tx, second_finalization_may_continue_rx) =
        mpsc::channel::<()>();
    let second_finalization_may_continue_rx = Mutex::new(second_finalization_may_continue_rx);
    let block_next_temp_file_move = Arc::new(AtomicBool::new(true));
    let first_prefetch_done = AtomicBool::new(false);

    std::thread::scope(|s| {
        // `firstPrefetch` and `secondPrefetch`: each prefetch runs (and finalizes its download)
        // on its own thread; the first one asks for its download first.
        let first_prefetch = s.spawn(|| {
            let result = wait(prefetcher.prefetch_files_interruptibly(
                f.action(),
                &[input(&children[0])],
                &supplier(&metadata),
                Priority::MEDIUM,
                Reason::INPUTS,
            ));
            first_prefetch_done.store(true, Ordering::SeqCst);
            result
        });
        assert!(wait_until(Duration::from_secs(10), || spy
            .do_download_file_calls()
            >= 1));
        let second_prefetch = s.spawn(|| {
            wait(prefetcher.prefetch_files_interruptibly(
                f.action(),
                &[input(&children[1])],
                &supplier(&metadata),
                Priority::MEDIUM,
                Reason::INPUTS,
            ))
        });
        assert!(wait_until(Duration::from_secs(10), || spy
            .do_download_file_calls()
            >= 2));

        // Pause the second finalization after it has made the tree writable, but before it moves
        // its temporary file into the tree.
        {
            let temp_dir = f.temp_path_generator.get_temp_dir().as_fragment().clone();
            let tree_path = tree.get_path().as_fragment().clone();
            let block_next_temp_file_move = block_next_temp_file_move.clone();
            let started_tx = Mutex::new(second_finalization_started_tx);
            let may_continue_rx = Arc::new(second_finalization_may_continue_rx);
            *f.fs.rename_hook.lock() = Some(Arc::new(
                move |source: &PathFragment, target: &PathFragment| {
                    if source.starts_with(&temp_dir)
                        && target.starts_with(&tree_path)
                        && block_next_temp_file_move.swap(false, Ordering::SeqCst)
                    {
                        let _ = started_tx.lock().send(());
                        let _ = may_continue_rx.lock().recv();
                    }
                },
            ));
        }

        let mut first_download_tx = Some(first_download_tx);
        {
            let _release = SignalOnDrop(Some(second_finalization_may_continue_tx));
            // `secondCompletion`
            let _ = second_download_tx.send(());
            assert!(
                second_finalization_started_rx
                    .recv_timeout(Duration::from_secs(10))
                    .is_ok()
            );
            // `firstCompletion`: completing the first download runs its finalization on its
            // prefetch thread. Its move may proceed concurrently with the paused move, but its
            // permission restoration must wait for the paused finalization to release its read
            // lock.
            let _ = first_download_tx.take().unwrap().send(());

            assert!(wait_until(Duration::from_secs(10), || children[0]
                .get_path()
                .exists()));
            assert!(children[0].get_path().exists());
            assert!(!first_prefetch_done.load(Ordering::SeqCst));
            assert!(tree.get_path().is_writable().unwrap());
        }

        first_prefetch.join().unwrap().unwrap();
        second_prefetch.join().unwrap().unwrap();
    });
    assert_tree_readable_non_writable_and_executable(&tree.get_path());
}

#[test]
fn prefetch_files_tree_files_many_concurrent_overlapping_calls() {
    let f = Fixture::set_up();
    let tree_count = 4;
    let children_per_tree = 16;
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let mut tree_list: Vec<BlazeArtifact> = Vec::new();
    let mut child_lists: Vec<Vec<BlazeArtifact>> = Vec::new();
    for tree_index in 0..tree_count {
        let names: Vec<String> = (0..children_per_tree)
            .map(|child_index| format!("subdir-{child_index}/file"))
            .collect();
        let remote_content: Vec<(&str, &str)> = names
            .iter()
            .map(|name| (name.as_str(), "content"))
            .collect();
        let (tree, children) = f.create_remote_tree_artifact(
            &format!("dir-{tree_index}"),
            /* localContentMap= */ &[],
            &remote_content,
            &mut metadata,
            &mut cas,
        );
        tree_list.push(tree);
        child_lists.push(children);
    }
    let prefetcher = f.create_prefetcher(&cas);

    let concurrency = 32;
    let request_count = 128;
    let start_requests = Barrier::new(concurrency + 1);
    let next_request = AtomicUsize::new(0);
    let failures: Mutex<Vec<String>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..concurrency {
            s.spawn(|| {
                start_requests.wait();
                loop {
                    let request_index = next_request.fetch_add(1, Ordering::SeqCst);
                    if request_index >= request_count {
                        break;
                    }
                    let tree_index = request_index % tree_count;
                    let child_index = (request_index / tree_count) % children_per_tree;
                    let children = &child_lists[tree_index];
                    let inputs = [
                        input(&children[child_index]),
                        input(&children[(child_index + 1) % children_per_tree]),
                    ];
                    if let Err(e) = wait(prefetcher.prefetch_files_interruptibly(
                        f.action(),
                        &inputs,
                        &supplier(&metadata),
                        Priority::MEDIUM,
                        Reason::INPUTS,
                    )) {
                        failures.lock().push(e.to_string());
                    }
                }
            });
        }
        start_requests.wait();
    });
    assert!(failures.lock().is_empty(), "{:?}", failures.lock());

    for (tree, children) in tree_list.iter().zip(&child_lists) {
        for child in children {
            assert_eq!(read_content(&child.get_path()), "content");
        }
        assert_tree_readable_non_writable_and_executable(&tree.get_path());
    }
}

#[test]
fn prefetch_files_multiple_threads_download_is_cancelled() {
    // Test shared downloads are cancelled if all threads/callers are interrupted

    // arrange
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let artifact = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));

    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();
    let download_that_never_finishes_cancelled = Arc::new(AtomicBool::new(false));
    {
        let cancelled = download_that_never_finishes_cancelled.clone();
        mock_download(
            &spy,
            &cas,
            Arc::new(move || -> DownloadResult {
                Box::pin(NeverFinishes {
                    cancelled: cancelled.clone(),
                })
            }),
        );
    }

    let (interrupt1, cancelled1) = futures::channel::oneshot::channel::<()>();
    let (interrupt2, cancelled2) = futures::channel::oneshot::channel::<()>();
    let prefetch = |cancel| {
        // do nothing with the result
        let _ = wait_or_cancel(
            prefetcher.prefetch_files_interruptibly(
                f.action(),
                &[input(&artifact)],
                &supplier(&metadata),
                Priority::MEDIUM,
                Reason::INPUTS,
            ),
            cancel,
        );
    };

    // act
    std::thread::scope(|s| {
        let cancelled_thread1 = s.spawn(|| prefetch(cancelled1));
        let cancelled_thread2 = s.spawn(|| prefetch(cancelled2));
        let _ = interrupt1.send(());
        let _ = interrupt2.send(());
        cancelled_thread1.join().unwrap();
        cancelled_thread2.join().unwrap();
    });

    // assert
    assert!(download_that_never_finishes_cancelled.load(Ordering::SeqCst));
    assert!(!artifact.get_path().exists());
    assert!(f.temp_dir_entries().is_empty());
}

#[test]
fn prefetch_files_multiple_threads_download_is_not_cancelled_by_other_threads() {
    // Test multiple threads can share downloads, but do not cancel each other when interrupted

    // arrange
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let artifact = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));
    // `SettableFuture<Void> download`.
    let (download_tx, download_rx) = tokio::sync::watch::channel(false);
    let download_cancelled = Arc::new(AtomicUsize::new(0));
    let download_cancelled_in_download = download_cancelled.clone();
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();
    mock_download(
        &spy,
        &cas,
        Arc::new(move || -> DownloadResult {
            let mut download = download_rx.clone();
            // `download.isCancelled()`: the future was dropped before it completed.
            let guard = CancelledGuard(download_cancelled_in_download.clone(), true);
            Box::pin(async move {
                let mut guard = guard;
                loop {
                    let done = *download.borrow();
                    if done {
                        break;
                    }
                    download
                        .changed()
                        .await
                        .map_err(|_| io::Error::other("closed"))?;
                }
                guard.1 = false;
                Ok::<(), io::Error>(())
            })
        }),
    );
    let (interrupt, cancelled) = futures::channel::oneshot::channel::<()>();
    let successful = AtomicBool::new(false);
    let path = f.exec_root.get_relative(artifact.get_exec_path());

    std::thread::scope(|s| {
        let cancelled_thread = s.spawn(|| {
            // do nothing with the result
            let _ = wait_or_cancel(
                prefetcher.prefetch_files_interruptibly(
                    f.action(),
                    &[input(&artifact)],
                    &supplier(&metadata),
                    Priority::MEDIUM,
                    Reason::INPUTS,
                ),
                cancelled,
            );
        });
        let successful_thread = s.spawn(|| {
            if wait(prefetcher.prefetch_files_interruptibly(
                f.action(),
                &[input(&artifact)],
                &supplier(&metadata),
                Priority::MEDIUM,
                Reason::INPUTS,
            ))
            .is_ok()
            {
                successful.store(true, Ordering::SeqCst);
            }
        });
        while prefetcher.get_download_cache().get_subscriber_count(&path) != 2 {
            std::thread::yield_now();
        }

        // act
        let _ = interrupt.send(());
        cancelled_thread.join().unwrap();
        // simulate the download finishing
        // `assertThat(download.isCancelled()).isFalse()`, checked after the download is
        // released so that a failure does not leave the other thread waiting.
        let download_is_cancelled = download_cancelled.load(Ordering::SeqCst) != 0;
        download_tx.send(true).unwrap();
        successful_thread.join().unwrap();
        assert!(!download_is_cancelled);
    });

    // assert
    assert!(successful.load(Ordering::SeqCst));
    assert_eq!(read_content(&artifact.get_path()), "hello world");
}

#[test]
fn prefetch_file_interrupting_metadata_supplier_interrupts_download() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a1 = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));
    let prefetcher = f.create_prefetcher(&cas);

    let interrupted_metadata_supplier =
        |_unused: &ActionInputRef| -> io::Result<Option<FileArtifactValue>> {
            Err(errors::interrupted())
        };

    let result = get_from_future(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&a1)],
        &interrupted_metadata_supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ));

    let e = result.unwrap_err();
    assert!(errors::is_interrupted(&e), "{e}");
}

#[test]
fn prefetch_files_on_interrupt_delete_partial_downloaded_file() {
    let f = Fixture::set_up();
    let (start_tx, start_rx) = mpsc::channel::<()>();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a1 = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut cas));
    let spy = f.create_spy(&cas);
    let prefetcher: Arc<dyn AbstractActionInputPrefetcher> = spy.clone();
    {
        let start_tx = Mutex::new(start_tx);
        mock_download(
            &spy,
            &cas,
            Arc::new(move || -> DownloadResult {
                let _ = start_tx.lock().send(());
                // A future that never complete so we can interrupt later
                Box::pin(futures::future::pending::<io::Result<()>>())
            }),
        );
    }

    let interrupted = AtomicBool::new(false);
    let (interrupt, cancelled) = futures::channel::oneshot::channel::<()>();
    std::thread::scope(|s| {
        let t = s.spawn(|| {
            match wait_or_cancel(
                prefetcher.prefetch_files_interruptibly(
                    f.action(),
                    &[input(&a1)],
                    &supplier(&metadata),
                    Priority::MEDIUM,
                    Reason::INPUTS,
                ),
                cancelled,
            ) {
                // `IOException`: Intentionally left empty
                Some(_) => {}
                // `InterruptedException`
                None => interrupted.store(true, Ordering::SeqCst),
            }
        });
        start_rx.recv().unwrap();
        let _ = interrupt.send(());
        t.join().unwrap();
    });

    assert!(interrupted.load(Ordering::SeqCst));
    assert!(!a1.get_path().exists());
    assert!(f.temp_dir_entries().is_empty());
}

#[test]
fn missing_inputs_exception_has_lost_inputs() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let cas = Cas::new();
    let input_artifact =
        f.create_remote_artifact("file", "hello world", &mut metadata, /* cas= */ None);
    let prefetcher = f.create_prefetcher(&cas);
    // `new StaticInputMetadataProvider(metadata)::getInput`.
    let get_input = |exec_path: &PathFragment| -> Option<ActionInputRef> {
        metadata
            .keys()
            .find(|input| input.get_exec_path() == exec_path)
            .cloned()
    };

    let e = wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap_err();
    let e = errors::downcast_ref::<BulkTransferException>(&e).expect("BulkTransferException");
    let by_digest = e.get_lost_artifacts(&get_input).by_digest;
    assert_eq!(by_digest.len(), 1);
    let expected_digest =
        DigestUtil::to_string(&DigestUtil::new(HASH_FUNCTION).compute_as_utf8("hello world"));
    let lost = by_digest
        .get(&expected_digest)
        .expect("lost artifacts keyed by the digest");
    assert_eq!(lost.len(), 1);
    assert!(lost.contains(&input(&input_artifact)));
}

// ---------------------------------------------------------------------------------------------
// RemoteActionInputFetcherTest
// ---------------------------------------------------------------------------------------------

#[test]
fn test_staging_virtual_action_input() {
    // arrange
    let f = Fixture::set_up();
    let action_input_fetcher = f.create_prefetcher(&Cas::new());
    // `ActionsTestUtil.createVirtualActionInput("file1", "hello world")`.
    let a = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("file1"),
        b"hello world".to_vec(),
    ));

    // act
    let null_supplier =
        |_unused: &ActionInputRef| -> io::Result<Option<FileArtifactValue>> { Ok(None) };
    wait(action_input_fetcher.prefetch_files_interruptibly(
        f.action(),
        std::slice::from_ref(&a),
        &null_supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    // assert
    let p = f.exec_root.get_relative(a.get_exec_path());
    assert_eq!(read_content(&p), "hello world");
    assert!(p.is_executable().unwrap());
    assert!(action_input_fetcher.downloaded_files().is_empty());
    assert!(action_input_fetcher.downloads_in_progress().is_empty());
}

#[test]
fn test_staging_empty_virtual_action_input() {
    // arrange
    let f = Fixture::set_up();
    let action_input_fetcher = f.create_prefetcher(&Cas::new());

    // act
    let null_supplier =
        |_unused: &ActionInputRef| -> io::Result<Option<FileArtifactValue>> { Ok(None) };
    wait(action_input_fetcher.prefetch_files_interruptibly(
        f.action(),
        &[empty_marker()],
        &null_supplier,
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    // assert that nothing happened
    assert!(action_input_fetcher.downloaded_files().is_empty());
    assert!(action_input_fetcher.downloads_in_progress().is_empty());
}

#[test]
fn prefetch_files_missing_files_fails_with_specific_message() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let a = f.create_remote_artifact("file1", "hello world", &mut metadata, Some(&mut Cas::new()));
    let prefetcher = f.create_prefetcher(&Cas::new());

    let error = wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &[input(&a)],
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap_err();
    assert!(errors::downcast_ref::<BulkTransferException>(&error).is_some());

    assert!(prefetcher.downloaded_files().is_empty());
    assert!(prefetcher.downloads_in_progress().is_empty());
    let m = &metadata[&input(&a)];
    let digest = DigestUtil::build_digest_from_bytes(m.get_digest().unwrap(), m.get_size());
    let message = error.to_string();
    assert!(
        message.contains(&format!("{}/{}", digest.hash, digest.size_bytes)),
        "{message}"
    );
}

#[test]
#[ignore = "needs RemoteExternalOverlayFileSystem (the remote repo contents cache), not ported"]
fn inject_remote_repo_invalid_path_throws_io_exception() {
    // Tests that RemoteExternalOverlayFileSystem and RemoteActionInputFetcher
    // maintain path containment within the repository directory.
    blaze_util::unported!("RemoteExternalOverlayFileSystem.injectRemoteRepo")
}

#[test]
#[ignore = "needs RemoteExternalOverlayFileSystem (the remote repo contents cache), not ported"]
fn rewound_action_output_exec_root_on_overlay_file_system_redownloaded() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("file", "hello world", &mut metadata, Some(&mut cas));
    // When the remote repo contents cache is enabled, the exec root lies on the file system
    // overlaying the host file system that downloads are written to.
    // PORT: `new RemoteExternalOverlayFileSystem(PathFragment.create("/output_base/external"),
    // fs)` is not ported; the exec root stays on the host file system.
    let action_input_fetcher = register_prefetcher(f.new_fetcher(&cas));
    let prefetcher: &dyn AbstractActionInputPrefetcher = &*action_input_fetcher;

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_eq!(read_content(&a.get_path()), "hello world");

    // Rewinding deletes the output and requires it to be downloaded again.
    a.get_path().delete().unwrap();
    action_input_fetcher.handle_rewound_action_outputs(std::slice::from_ref(&a));

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();
    assert_eq!(read_content(&a.get_path()), "hello world");
}

#[test]
#[ignore = "needs RemoteExternalOverlayFileSystem (the remote repo contents cache), not ported"]
fn prefetch_files_exec_root_on_overlay_file_system_replaces_parent_file() {
    let f = Fixture::set_up();
    let mut metadata = MetadataMap::new();
    let mut cas = Cas::new();
    let a = f.create_remote_artifact("dir/file", "hello world", &mut metadata, Some(&mut cas));
    fs_utils::write_content(&a.get_path().get_parent_directory().unwrap(), b"stale").unwrap();
    // PORT: `new RemoteExternalOverlayFileSystem(PathFragment.create("/output_base/external"),
    // fs)` is not ported; the exec root stays on the host file system.
    let prefetcher = f.create_prefetcher(&cas);

    wait(prefetcher.prefetch_files_interruptibly(
        f.action(),
        &keys(&metadata),
        &supplier(&metadata),
        Priority::MEDIUM,
        Reason::INPUTS,
    ))
    .unwrap();

    assert_eq!(read_content(&a.get_path()), "hello world");
}

/// Counts (in `.0`) the downloads dropped while `.1` (not completed) holds.
struct CancelledGuard(Arc<AtomicUsize>, bool);

impl Drop for CancelledGuard {
    fn drop(&mut self) {
        if self.1 {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}
