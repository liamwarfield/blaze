//! Port of the download half of `lib/remote/RemoteExecutionService.java` (lines 841-1725 at the
//! pin): parsing the metadata of an `ActionResult`, downloading the outputs of a remote action
//! into the output tree, and the reuse of the outputs of a concurrent local execution.
//!
//! The other half (`crate::remote_execution_service`) declares the struct; this file is a second
//! `impl RemoteExecutionService` block plus Java's nested classes of that line range.
//!
//! This code writes bytes fetched from a server into the output tree. Every check Java makes is
//! kept, in Java's order, and none is added. Where Java does not validate a server-provided value
//! the spot carries a `// SECURITY:` comment that says what Java relies on instead.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use blaze_exec::actions::{
    ActionInput, ActionInputRef, EnvironmentalExecException, ExecException, SpawnResult,
    Status as SpawnResultStatus,
};
use blaze_exec::exec::action_execution_context::SpawnCacheError;
use blaze_exec::util::FileOutErr;
use blaze_protos::JavaMessage;
use blaze_protos::failure_details::remote_execution;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DirentType, Path, PathFragment, Symlinks};
use futures::future::LocalBoxFuture;
use indexmap::IndexMap;
use parking_lot::{Condvar, Mutex};
use prost::Message as _;

use crate::combined_cache::{CombinedCache, DownloadProgressReporter};
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;
use crate::common::output_digest_mismatch_exception::OutputDigestMismatchException;
use crate::common::progress_status_listener::{ProgressStatusListener, no_action};
use crate::common::remote_action_execution_context::{CachePolicy, RemoteActionExecutionContext};
use crate::common::remote_cache_client::ActionKey;
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::{ActionResult, Digest, Directory, Tree};
use crate::remote_action::RemoteAction;
use crate::remote_execution_service::{RemoteActionResult, RemoteExecutionService};
use crate::runtime::get_from_future;
use crate::util::digest_util::DigestUtil;
use crate::util::utils::{InMemoryOutput, get_in_memory_output_path, wait_for_bulk_transfer};

/// A `java.util.HashMap<Path, V>`, as far as its iteration order goes.
///
/// PORT: java-hash-order. Java keeps the tree downloads, the output symlinks and the temporary
/// paths of a download in `HashMap`s keyed by `Path` and iterates them: the order decides in
/// which order symlinks are created and partial downloads are deleted, and so which error is
/// reported first. `Path.hashCode()` is the `String.hashCode()` of the path string, so the order
/// is deterministic and is reproduced here: the hash spread by `h ^ (h >>> 16)`, a power-of-two
/// table that doubles when the size exceeds 3/4 of the capacity, and per-bin insertion order
/// (which `HashMap.resize` preserves). A treeified bin (more than 8 entries in one bin of a table
/// of 64 or more slots) keeps insertion order here; Java's red-black bins link a new node after
/// its tree parent, so the order inside such a bin can differ.
// Java: java/util/HashMap.java HashMap
struct JavaHashMap<V> {
    /// `table`; empty until the first `put` (Java allocates it lazily).
    table: Vec<Vec<JavaHashMapNode<V>>>,
    /// `size`.
    size: usize,
    /// The capacity of the table that the first `put` allocates (Java keeps it in `threshold`).
    initial_capacity: usize,
}

struct JavaHashMapNode<V> {
    hash: u32,
    key: Path,
    value: V,
}

impl<V> JavaHashMap<V> {
    /// `HashMap.TREEIFY_THRESHOLD`.
    const TREEIFY_THRESHOLD: usize = 8;
    /// `HashMap.MIN_TREEIFY_CAPACITY`.
    const MIN_TREEIFY_CAPACITY: usize = 64;

    /// `new HashMap<>()`.
    // Java: java/util/HashMap.java HashMap.<init>
    fn new() -> JavaHashMap<V> {
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            // `DEFAULT_INITIAL_CAPACITY`
            initial_capacity: 16,
        }
    }

    /// `Maps.newHashMapWithExpectedSize(expectedSize)` (Guava 33.5.0, the version at the pin).
    // Java: com/google/common/collect/Maps.java Maps.newHashMapWithExpectedSize
    fn with_expected_size(expected_size: usize) -> JavaHashMap<V> {
        // Java: com/google/common/collect/Maps.java Maps.capacity
        let capacity = if expected_size < 3 {
            expected_size + 1
        } else {
            // `(int) Math.ceil(expectedSize / 0.75)`
            (expected_size as f64 / 0.75).ceil() as usize
        };
        JavaHashMap {
            table: Vec::new(),
            size: 0,
            // `new HashMap<>(capacity)`: `tableSizeFor(capacity)`, the smallest power of two
            // that is not less than the capacity.
            initial_capacity: capacity.next_power_of_two(),
        }
    }

    /// `HashMap.hash(key)` for a `Path`.
    // Java: java/util/HashMap.java HashMap.hash
    fn hash(key: &Path) -> u32 {
        // Java: com/google/devtools/build/lib/vfs/Path.java:229 Path.hashCode
        // (`pathFragment.hashCode()`, which is `normalizedPath.hashCode()`).
        // PORT: StringEncoding identity. The Java string is in the internal encoding (one char
        // per byte of the path), so `String.hashCode()` runs over the bytes.
        let mut h: i32 = 0;
        for b in key.get_path_string().bytes() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(b));
        }
        let h = h as u32;
        h ^ (h >> 16)
    }

    /// `threshold` of an allocated table: `(int) (capacity * 0.75f)`.
    fn threshold(&self) -> usize {
        self.table.len() * 3 / 4
    }

    // Java: java/util/HashMap.java HashMap.resize
    fn resize(&mut self) {
        let new_capacity = self.table.len() * 2;
        let mut new_table: Vec<Vec<JavaHashMapNode<V>>> = Vec::with_capacity(new_capacity);
        new_table.resize_with(new_capacity, Vec::new);
        // Each old bin is split into two new bins; the nodes keep their relative order.
        for bin in self.table.drain(..) {
            for node in bin {
                let index = (node.hash as usize) & (new_capacity - 1);
                new_table[index].push(node);
            }
        }
        self.table = new_table;
    }

    /// `put(key, value)`; returns the previous value.
    // Java: java/util/HashMap.java HashMap.putVal
    fn put(&mut self, key: Path, value: V) -> Option<V> {
        if self.table.is_empty() {
            self.table.resize_with(self.initial_capacity, Vec::new);
        }
        let hash = JavaHashMap::<V>::hash(&key);
        let index = (hash as usize) & (self.table.len() - 1);
        for node in &mut self.table[index] {
            if node.hash == hash && node.key == key {
                // An existing mapping keeps its place.
                return Some(std::mem::replace(&mut node.value, value));
            }
        }
        let bin_count = self.table[index].len();
        self.table[index].push(JavaHashMapNode { hash, key, value });
        // `if (binCount >= TREEIFY_THRESHOLD - 1) treeifyBin(tab, hash)`, where Java's
        // `binCount` is one less than the number of nodes that were in the bin.
        if bin_count >= Self::TREEIFY_THRESHOLD && self.table.len() < Self::MIN_TREEIFY_CAPACITY {
            // `treeifyBin` resizes a small table instead of treeifying the bin.
            self.resize();
        }
        self.size += 1;
        if self.size > self.threshold() {
            self.resize();
        }
        None
    }

    /// `get(key)`.
    // Java: java/util/HashMap.java HashMap.getNode
    fn get(&self, key: &Path) -> Option<&V> {
        if self.table.is_empty() {
            return None;
        }
        let hash = JavaHashMap::<V>::hash(key);
        let index = (hash as usize) & (self.table.len() - 1);
        self.table[index]
            .iter()
            .find(|node| node.hash == hash && &node.key == key)
            .map(|node| &node.value)
    }

    /// `containsKey(key)`.
    fn contains_key(&self, key: &Path) -> bool {
        self.get(key).is_some()
    }

    /// `size()`.
    fn len(&self) -> usize {
        self.size
    }

    /// `entrySet()`, in Java's iteration order.
    // Java: java/util/HashMap.java HashMap.HashIterator
    fn iter(&self) -> impl Iterator<Item = (&Path, &V)> {
        self.table
            .iter()
            .flat_map(|bin| bin.iter())
            .map(|node| (&node.key, &node.value))
    }

    /// `keySet()`, in Java's iteration order.
    fn keys(&self) -> impl Iterator<Item = &Path> {
        self.iter().map(|(key, _)| key)
    }

    /// `values()`, in Java's iteration order.
    fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }
}

/// Removes digests referenced by `metadata` from `known_missing_cas_digests` and returns whether
/// any were removed.
///
/// PORT: a module function rather than a `RemoteExecutionService` method so that it cannot clash
/// with the other half of the class, whose `lookupCache` calls it too.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:841 RemoteExecutionService.updateKnownMissingCasDigests
pub(crate) fn update_known_missing_cas_digests(
    known_missing_cas_digests: &mut HashSet<Digest>,
    metadata: &ActionResultMetadata,
) -> bool {
    // Using `remove` below because we assume the missing blob will be uploaded afterwards.
    let mut result = false;
    for file in metadata.files() {
        if known_missing_cas_digests.remove(file.digest()) {
            result = true;
        }
    }
    for (_, directory) in metadata.directories() {
        for file in directory.files() {
            if known_missing_cas_digests.remove(file.digest()) {
                result = true;
            }
        }
    }
    result
}

/// `ActionResultMetadata.SymlinkMetadata`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:977 ActionResultMetadata.SymlinkMetadata
#[derive(Debug, Clone)]
pub struct SymlinkMetadata {
    path: Path,
    target: PathFragment,
}

impl SymlinkMetadata {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:981 SymlinkMetadata.<init>
    fn new(path: Path, target: PathFragment) -> SymlinkMetadata {
        SymlinkMetadata { path, target }
    }

    /// `path()`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `target()`.
    pub fn target(&self) -> &PathFragment {
        &self.target
    }
}

/// `ActionResultMetadata.FileMetadata`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:995 ActionResultMetadata.FileMetadata
#[derive(Debug, Clone)]
pub struct FileMetadata {
    path: Path,
    digest: Digest,
    is_executable: bool,
    contents: bytes::Bytes,
}

impl FileMetadata {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1001 FileMetadata.<init>
    fn new(
        path: Path,
        digest: Digest,
        is_executable: bool,
        contents: bytes::Bytes,
    ) -> FileMetadata {
        FileMetadata {
            path,
            digest,
            is_executable,
            contents,
        }
    }

    /// `path()`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `digest()`.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    /// `isExecutable()`.
    pub fn is_executable(&self) -> bool {
        self.is_executable
    }

    /// `content()`.
    pub fn content(&self) -> &bytes::Bytes {
        &self.contents
    }
}

/// `ActionResultMetadata.DirectoryMetadata`.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1025 ActionResultMetadata.DirectoryMetadata
#[derive(Debug, Clone)]
pub struct DirectoryMetadata {
    files: Vec<FileMetadata>,
    symlinks: Vec<SymlinkMetadata>,
}

impl DirectoryMetadata {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1029 DirectoryMetadata.<init>
    fn new(files: Vec<FileMetadata>, symlinks: Vec<SymlinkMetadata>) -> DirectoryMetadata {
        DirectoryMetadata { files, symlinks }
    }

    /// `files()`.
    pub fn files(&self) -> &[FileMetadata] {
        &self.files
    }

    /// `symlinks()`.
    pub fn symlinks(&self) -> &[SymlinkMetadata] {
        &self.symlinks
    }
}

/// In-memory representation of action result metadata.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:975 RemoteExecutionService.ActionResultMetadata
#[derive(Debug, Clone)]
pub struct ActionResultMetadata {
    // Java: three `ImmutableMap`s (insertion order). `symlinks` and `directories` are filled from
    // the iteration of a `HashMap`, see `parse_action_result_metadata`.
    files: IndexMap<Path, FileMetadata>,
    symlinks: IndexMap<Path, SymlinkMetadata>,
    directories: IndexMap<Path, DirectoryMetadata>,
}

impl ActionResultMetadata {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1048 ActionResultMetadata.<init>
    fn new(
        files: IndexMap<Path, FileMetadata>,
        symlinks: IndexMap<Path, SymlinkMetadata>,
        directories: IndexMap<Path, DirectoryMetadata>,
    ) -> ActionResultMetadata {
        ActionResultMetadata {
            files,
            symlinks,
            directories,
        }
    }

    /// `file(path)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1058 ActionResultMetadata.file
    pub fn file(&self, path: &Path) -> Option<&FileMetadata> {
        self.files.get(path)
    }

    /// `directory(path)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1063 ActionResultMetadata.directory
    pub fn directory(&self, path: &Path) -> Option<&DirectoryMetadata> {
        self.directories.get(path)
    }

    /// `files()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1067 ActionResultMetadata.files
    pub fn files(&self) -> indexmap::map::Values<'_, Path, FileMetadata> {
        self.files.values()
    }

    /// `directories()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1071 ActionResultMetadata.directories
    pub fn directories(&self) -> indexmap::map::Iter<'_, Path, DirectoryMetadata> {
        self.directories.iter()
    }

    /// `symlinks()`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1075 ActionResultMetadata.symlinks
    pub fn symlinks(&self) -> indexmap::map::Values<'_, Path, SymlinkMetadata> {
        self.symlinks.values()
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1080 RemoteExecutionService.parseDirectory
fn parse_directory(
    parent: &Path,
    dir: &Directory,
    child_directories_map: &HashMap<Digest, &Directory>,
) -> DirectoryMetadata {
    // SECURITY: the names of a Tree's nodes (`FileNode.name`, `SymlinkNode.name`,
    // `DirectoryNode.name`) are not validated. `Path.getRelative` normalizes `..` segments and
    // returns an absolute name as it is, so a name can resolve outside `parent` and outside the
    // exec root. For files Java relies on `file.path.relativeTo(execRoot)` in `downloadOutputs`,
    // which throws `IllegalArgumentException` (a crash; a panic here) for a path that is not
    // under the exec root, before the download of that file is started. For symlinks Java relies
    // on nothing: `createSymlinks` takes the path as it is. Symlink targets are not validated
    // either (absolute targets and targets with `..` are accepted at the pin).
    let mut files_builder: Vec<FileMetadata> = Vec::new();
    for file in &dir.files {
        files_builder.push(FileMetadata::new(
            // PORT: StringEncoding identity (`unicodeToInternal`).
            parent.get_relative_str(&file.name),
            file.digest.clone().unwrap_or_default(),
            file.is_executable,
            bytes::Bytes::new(),
        ));
    }

    let mut symlinks_builder: Vec<SymlinkMetadata> = Vec::new();
    for symlink in &dir.symlinks {
        symlinks_builder.push(SymlinkMetadata::new(
            // PORT: StringEncoding identity (`unicodeToInternal`).
            parent.get_relative_str(&symlink.name),
            PathFragment::create(&symlink.target),
        ));
    }

    for directory_node in &dir.directories {
        // PORT: StringEncoding identity (`unicodeToInternal`).
        let child_path = parent.get_relative_str(&directory_node.name);
        // SECURITY: a Tree whose `children` lack a directory that a `DirectoryNode` refers to is
        // a `NullPointerException` in Java (`Preconditions.checkNotNull`), a crash; the same
        // panic here. The recursion is not bounded: a Tree in which each level refers to the
        // same child several times expands exponentially, as in Java. (A cycle is not possible,
        // the children are looked up by the digest computed from their contents.)
        let child_dir = *child_directories_map
            .get(&directory_node.digest.clone().unwrap_or_default())
            .expect("NullPointerException");
        let child_metadata = parse_directory(&child_path, child_dir, child_directories_map);
        files_builder.extend(child_metadata.files);
        symlinks_builder.extend(child_metadata.symlinks);
    }

    DirectoryMetadata::new(files_builder, symlinks_builder)
}

// The Tree message representing an empty directory.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1113 RemoteExecutionService.EMPTY_DIRECTORY
static EMPTY_DIRECTORY: LazyLock<Tree> = LazyLock::new(|| {
    let empty_directory = Tree {
        root: Some(Directory::default()),
        ..Default::default()
    };
    // See logic in parseActionResultMetadata below.
    blaze_util::check_state!(empty_directory.to_byte_array().len() == 2);
    empty_directory
});

/// A value of Java's `Map<Path, ListenableFuture<Tree>> dirMetadataDownloads`.
enum TreeDownload {
    /// `immediateFuture(EMPTY_DIRECTORY)`.
    EmptyDirectory,
    /// The download of the Tree blob (`combinedCache.downloadBlobAsByteString(context,
    /// outputPath, execPath, treeDigest)`) and its parsing.
    Blob {
        output_path: String,
        exec_path: PathFragment,
        tree_digest: Digest,
    },
}

/// `RemoteExecutionService.parseActionResultMetadata(combinedCache, digestUtil, context, result,
/// remotePathResolver)`.
///
/// PORT: the same function as [`RemoteExecutionService::parse_action_result_metadata`], for
/// callers that do not name the class.
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1121 RemoteExecutionService.parseActionResultMetadata
pub(crate) fn parse_action_result_metadata(
    combined_cache: &CombinedCache,
    digest_util: &DigestUtil,
    context: &RemoteActionExecutionContext<'_>,
    result: &ActionResult,
    remote_path_resolver: &dyn RemotePathResolver,
) -> io::Result<ActionResultMetadata> {
    // Java: checkNotNull(combinedCache, "combinedCache can't be null")
    // PORT: a reference cannot be null; the callers hold an `Option` and check it.

    // SECURITY: the paths of the ActionResult (`OutputDirectory.path`, `OutputFile.path`,
    // `OutputSymlink.path`) are resolved with `RemotePathResolver.outputPathToLocalPath`, which
    // is `base.getRelative(outputPath)`: `..` segments are normalized and an absolute path is
    // taken as it is, so a path can resolve outside the exec root. Nothing here compares the
    // paths with the outputs that the spawn declared: every file, directory and symlink that the
    // ActionResult lists is accepted. Java relies on `relativeTo(execRoot)` in `downloadOutputs`
    // for files and tree roots (see `parse_directory`), and on nothing for symlinks.
    let mut dir_metadata_downloads: JavaHashMap<TreeDownload> =
        JavaHashMap::with_expected_size(result.output_directories.len());
    for dir in &result.output_directories {
        let output_path = &dir.path;
        // PORT: StringEncoding identity (`unicodeToInternal`).
        let local_path = remote_path_resolver.output_path_to_local_path(output_path);
        // `dir.getTreeDigest()` is the default instance when the field is not set.
        let tree_digest = dir.tree_digest.clone().unwrap_or_default();
        if tree_digest.size_bytes == 2 {
            // A valid Tree message contains at least a non-empty root field. The only way for a
            // Tree message to have a size of 2 bytes is if the root field is the only non-empty
            // field and the Directory message in the root field is empty, which corresponds to
            // one byte for the LEN tag and field number and one byte for the zero-length varint.
            // Since empty tree artifacts are relatively common (e.g., as the undeclared test
            // output directory), we avoid downloading these messages here.
            dir_metadata_downloads.put(local_path, TreeDownload::EmptyDirectory);
        } else {
            let exec_path = remote_path_resolver.local_path_to_exec_path(local_path.as_fragment());
            dir_metadata_downloads.put(
                local_path,
                TreeDownload::Blob {
                    output_path: output_path.clone(),
                    exec_path,
                    tree_digest,
                },
            );
        }
    }

    // PORT: Java's downloads are already running and `waitForBulkTransfer` waits for them; the
    // Rust futures are made here, in the iteration order of the map, and
    // `wait_for_bulk_transfer` drives them together. It gives no results back, so each future
    // leaves its Tree in a slot (Java reads it with `getFromFuture` after the wait). A download
    // whose map entry was replaced by a later output directory with the same path is not started
    // (Java lets it run and never looks at it).
    // SECURITY: the Tree blob is checked against `tree_digest` by `CombinedCache` and the cache
    // clients behind it (`--remote_verify_downloads`), not here.
    let trees: Vec<Mutex<Option<Tree>>> = (0..dir_metadata_downloads.len())
        .map(|_| Mutex::new(None))
        .collect();
    let mut transfers: Vec<LocalBoxFuture<'_, io::Result<()>>> = Vec::new();
    for (download, slot) in dir_metadata_downloads.values().zip(&trees) {
        match download {
            TreeDownload::EmptyDirectory => {
                *slot.lock() = Some(EMPTY_DIRECTORY.clone());
            }
            TreeDownload::Blob {
                output_path,
                exec_path,
                tree_digest,
            } => {
                transfers.push(Box::pin(async move {
                    let tree_bytes = combined_cache
                        .download_blob_as_byte_string(
                            context,
                            output_path,
                            Some(exec_path),
                            tree_digest,
                        )
                        .await?;
                    // `Tree.parseFrom(treeBytes, ExtensionRegistry.getEmptyRegistry())`; an
                    // `InvalidProtocolBufferException` is an `IOException`.
                    let tree = Tree::decode(tree_bytes)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    *slot.lock() = Some(tree);
                    Ok(())
                }));
            }
        }
    }

    wait_for_bulk_transfer(transfers)?;

    let mut directories: IndexMap<Path, DirectoryMetadata> = IndexMap::new();
    for (path, slot) in dir_metadata_downloads.keys().zip(&trees) {
        // `getFromFuture(metadataDownload.getValue())`
        let directory_tree = slot
            .lock()
            .take()
            .expect("the transfer of the Tree has succeeded");
        // The children are keyed by the digest computed here, not by a digest the server names.
        let mut children_map: HashMap<Digest, &Directory> = HashMap::new();
        for child_dir in &directory_tree.children {
            children_map.insert(digest_util.compute_message(child_dir), child_dir);
        }

        // `directoryTree.getRoot()` is the default instance when the field is not set.
        let root = directory_tree.root.clone().unwrap_or_default();
        directories.insert(path.clone(), parse_directory(path, &root, &children_map));
    }

    let mut files: IndexMap<Path, FileMetadata> = IndexMap::new();
    // The first key that `files.buildOrThrow()` below finds twice.
    let mut duplicate_file: Option<Path> = None;
    for output_file in &result.output_files {
        // PORT: StringEncoding identity (`unicodeToInternal`).
        let local_path = remote_path_resolver.output_path_to_local_path(&output_file.path);
        let file_metadata = FileMetadata::new(
            local_path.clone(),
            output_file.digest.clone().unwrap_or_default(),
            output_file.is_executable,
            bytes::Bytes::from(output_file.contents.clone()),
        );
        if files.contains_key(&local_path) {
            duplicate_file.get_or_insert(local_path);
        } else {
            files.insert(local_path, file_metadata);
        }
    }

    let mut symlink_map: JavaHashMap<SymlinkMetadata> = JavaHashMap::new();
    let output_symlinks = result
        .output_file_symlinks
        .iter()
        .chain(&result.output_directory_symlinks)
        .chain(&result.output_symlinks);
    for symlink in output_symlinks {
        // PORT: StringEncoding identity (`unicodeToInternal`), here and for the target.
        let local_path = remote_path_resolver.output_path_to_local_path(&symlink.path);
        // SECURITY: the target of an output symlink is not validated at the pin: an absolute
        // target and a target with `..` segments are accepted (the "absolute symlink" check of
        // older Bazel versions is gone from the download path; only `UploadManifest` has one,
        // for uploads). Java relies on nothing here.
        let target = PathFragment::create(&symlink.target);
        let existing_metadata = symlink_map.get(&local_path);
        if let Some(existing_metadata) = existing_metadata {
            if &target != existing_metadata.target() {
                return Err(errors::io_exception(format!(
                    "Symlink path collision: '{}' is mapped to both '{}' and '{}'. Action Result should not contain multiple targets for the same symlink.",
                    local_path,
                    existing_metadata.target(),
                    target
                )));
            }
            continue;
        }

        symlink_map.put(local_path.clone(), SymlinkMetadata::new(local_path, target));
    }

    // `files.buildOrThrow()`: an `IllegalArgumentException` for two output files with the same
    // path.
    // PORT: Java's message also prints the two `FileMetadata` values (`Object.toString()`, with
    // identity hash codes).
    if let Some(duplicate_file) = duplicate_file {
        panic!("IllegalArgumentException: Multiple entries with same key: {duplicate_file}");
    }
    // `ImmutableMap.copyOf(symlinkMap)`: the entries in the iteration order of the `HashMap`.
    let symlinks: IndexMap<Path, SymlinkMetadata> = symlink_map
        .iter()
        .map(|(path, symlink)| (path.clone(), symlink.clone()))
        .collect();
    Ok(ActionResultMetadata::new(files, symlinks, directories))
}

/// The state of Java's `SettableFuture<SpawnResult> spawnResultFuture`.
enum SpawnResultFutureState {
    /// Not completed yet.
    Pending,
    /// `set(result)`.
    Set(Box<SpawnResult>),
    /// `cancel(true)`.
    Cancelled,
}

/// `SettableFuture<SpawnResult>`, as far as `LocalExecution` uses it (`set`, `cancel`, `get`).
// Java: com/google/common/util/concurrent/SettableFuture.java SettableFuture
struct SettableSpawnResultFuture {
    state: Mutex<SpawnResultFutureState>,
    done: Condvar,
}

impl SettableSpawnResultFuture {
    /// `SettableFuture.create()`.
    fn create() -> SettableSpawnResultFuture {
        SettableSpawnResultFuture {
            state: Mutex::new(SpawnResultFutureState::Pending),
            done: Condvar::new(),
        }
    }

    /// `set(value)`: completes the future unless it is already completed.
    fn set(&self, value: SpawnResult) -> bool {
        let mut state = self.state.lock();
        if !matches!(*state, SpawnResultFutureState::Pending) {
            return false;
        }
        *state = SpawnResultFutureState::Set(Box::new(value));
        self.done.notify_all();
        true
    }

    /// `cancel(true)`: cancels the future unless it is already completed.
    fn cancel(&self) -> bool {
        let mut state = self.state.lock();
        if !matches!(*state, SpawnResultFutureState::Pending) {
            return false;
        }
        *state = SpawnResultFutureState::Cancelled;
        self.done.notify_all();
        true
    }

    /// `get()`: waits for the future to complete; `None` is Java's `CancellationException`.
    ///
    /// PORT: Blaze has no thread interruption (see `crate::runtime`), so the wait cannot end
    /// with an `InterruptedException`.
    fn get(&self) -> Option<SpawnResult> {
        let mut state = self.state.lock();
        loop {
            match &*state {
                SpawnResultFutureState::Set(value) => return Some((**value).clone()),
                SpawnResultFutureState::Cancelled => return None,
                SpawnResultFutureState::Pending => {}
            }
            self.done.wait(&mut state);
        }
    }
}

/// The state of Java's `Phaser spawnResultConsumers`.
struct PhaserState {
    /// The number of registered parties.
    parties: i32,
    /// The number of registered parties that have not arrived at the (single) phase yet.
    unarrived: i32,
    /// Whether the phaser has terminated (`onAdvance` returns true: we only use a single phase).
    terminated: bool,
}

/// `new Phaser(1) { onAdvance(...) { return true; } }`, as far as `LocalExecution` uses it.
// Java: java/util/concurrent/Phaser.java Phaser
struct SinglePhasePhaser {
    state: Mutex<PhaserState>,
    advanced: Condvar,
}

impl SinglePhasePhaser {
    /// `new Phaser(1)`.
    fn new() -> SinglePhasePhaser {
        SinglePhasePhaser {
            state: Mutex::new(PhaserState {
                parties: 1,
                unarrived: 1,
                terminated: false,
            }),
            advanced: Condvar::new(),
        }
    }

    /// `register() == 0`: adds a party; false (Java: a negative phase) when the phaser has
    /// terminated.
    // Java: java/util/concurrent/Phaser.java Phaser.register
    fn register(&self) -> bool {
        let mut state = self.state.lock();
        if state.terminated {
            return false;
        }
        state.parties += 1;
        state.unarrived += 1;
        true
    }

    /// `arriveAndDeregister()`.
    // Java: java/util/concurrent/Phaser.java Phaser.arriveAndDeregister
    fn arrive_and_deregister(&self) {
        let mut state = self.state.lock();
        if state.terminated {
            return;
        }
        // Java: `throw new IllegalStateException(badArrive(s))`.
        blaze_util::check_state!(
            state.unarrived > 0,
            "Attempted arrival of unregistered party"
        );
        state.parties -= 1;
        state.unarrived -= 1;
        if state.unarrived == 0 {
            state.terminated = true;
            self.advanced.notify_all();
        }
    }

    /// `arriveAndAwaitAdvance()`.
    // Java: java/util/concurrent/Phaser.java Phaser.arriveAndAwaitAdvance
    fn arrive_and_await_advance(&self) {
        let mut state = self.state.lock();
        if state.terminated {
            return;
        }
        // Java: `throw new IllegalStateException(badArrive(s))`.
        blaze_util::check_state!(
            state.unarrived > 0,
            "Attempted arrival of unregistered party"
        );
        state.unarrived -= 1;
        if state.unarrived == 0 {
            state.terminated = true;
            self.advanced.notify_all();
            return;
        }
        while !state.terminated {
            self.advanced.wait(&mut state);
        }
    }
}

/// An ongoing local execution of a spawn.
///
/// PORT: Java keeps the `RemoteAction`. A `RemoteAction<'a>` borrows the spawn and its execution
/// context of the executing thread, while a `LocalExecution` is read by the threads of other
/// spawns, so this keeps (owned) what Java reads of the action instead: its action key, the
/// spawn's output files, its remote path resolver and the `FileOutErr` of its execution context.
/// `close()` is explicit (Java: `SilentCloseable` in a try-with-resources).
// Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1460 RemoteExecutionService.LocalExecution
pub struct LocalExecution {
    /// `action.getActionKey()`.
    action_key: ActionKey,
    /// `action.getSpawn().getOutputFiles()`.
    spawn_output_files: Vec<ActionInputRef>,
    /// `action.getRemotePathResolver()`.
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    /// `action.getSpawnExecutionContext().getFileOutErr()`.
    file_out_err: FileOutErr,
    spawn_result_future: SettableSpawnResultFuture,
    on_close: Box<dyn Fn() + Send + Sync>,
    close_manually: AtomicBool,
    spawn_result_consumers: SinglePhasePhaser,
}

impl LocalExecution {
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1474 LocalExecution.<init>
    fn new(action: &RemoteAction<'_>, on_close: Box<dyn Fn() + Send + Sync>) -> LocalExecution {
        LocalExecution {
            action_key: action.get_action_key().clone(),
            spawn_output_files: action.get_spawn().get_output_files().to_vec(),
            remote_path_resolver: action.get_remote_path_resolver().clone(),
            file_out_err: action
                .get_spawn_execution_context()
                .get_file_out_err()
                .clone(),
            spawn_result_future: SettableSpawnResultFuture::create(),
            on_close,
            close_manually: AtomicBool::new(false),
            spawn_result_consumers: SinglePhasePhaser::new(),
        }
    }

    /// Creates a new `LocalExecution` instance tracking the potential local execution of the
    /// given `RemoteAction` if there is a chance that the same action will be executed by a
    /// different Spawn.
    ///
    /// This is only done for local (as in, non-remote) execution as remote executors are expected
    /// to already have deduplication mechanisms for actions in place, perhaps even across
    /// different builds and clients.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1490 LocalExecution.createIfDeduplicatable
    pub fn create_if_deduplicatable(
        action: &RemoteAction<'_>,
        on_close: Box<dyn Fn() + Send + Sync>,
    ) -> Option<Arc<LocalExecution>> {
        if action.get_spawn().get_path_mapper().is_noop() {
            return None;
        }
        Some(Arc::new(LocalExecution::new(action, on_close)))
    }

    /// Attempts to register a thread waiting for the `spawnResultFuture` to become available and
    /// returns true if successful.
    ///
    /// Every call to this method must be matched by a call to [`LocalExecution::unregister`]
    /// (Java: via try-finally).
    ///
    /// This always returns true for actions that do not modify their spawns' outputs after
    /// execution.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1507 LocalExecution.registerForOutputReuse
    pub fn register_for_output_reuse(&self) -> bool {
        // We only use a single phase.
        self.spawn_result_consumers.register()
    }

    /// Unregisters a thread waiting for the `spawnResultFuture`, either after successful reuse of
    /// the outputs or upon failure.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1516 LocalExecution.unregister
    pub fn unregister(&self) {
        self.spawn_result_consumers.arrive_and_deregister();
    }

    /// Waits for all potential consumers of the `spawnResultFuture` to be done with their output
    /// reuse.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1524 LocalExecution.awaitAllOutputReuse
    pub fn await_all_output_reuse(&self) {
        self.spawn_result_consumers.arrive_and_await_advance();
    }

    /// Signals to all potential consumers of the `spawnResultFuture` that this execution has
    /// finished or been canceled and that the result will no longer be available.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1533 LocalExecution.close
    pub fn close(&self) {
        if !self.close_manually.load(Ordering::SeqCst) {
            self.do_close();
        }
    }

    /// Returns a closure that will close this `LocalExecution` instance when called. After this
    /// method is called, the `LocalExecution` instance will not be closed by the
    /// [`LocalExecution::close`] method.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1544 LocalExecution.delayClose
    pub fn delay_close(self: &Arc<Self>) -> impl Fn() + Send + Sync + 'static {
        if self
            .close_manually
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            panic!("IllegalStateException: delayClose has already been called");
        }
        let this = self.clone();
        move || this.do_close()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1551 LocalExecution.doClose
    fn do_close(&self) {
        self.spawn_result_future.cancel();
        (self.on_close)();
    }
}

/// The counter of [`child_out_err`].
static CHILD_OUT_ERR_COUNT: AtomicI32 = AtomicI32::new(0);

/// `outErr.childOutErr()`: `<out>-<n>` / `<err>-<n>`.
///
/// PORT: Java numbers the children per parent (`FileOutErr.childCount`). `blaze_exec`'s
/// `FileOutErr` is a pair of paths without that counter and without `childOutErr()`, so the
/// number comes from one counter for all the parents of this process: the names have Java's
/// form and are distinct among the downloads, but the numbers are not Java's. (`blaze_exec`'s
/// `StandaloneTestStrategy` keeps its own counter for the children it makes.)
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:232 FileOutErr.childOutErr
fn child_out_err(parent: &FileOutErr) -> FileOutErr {
    let index = CHILD_OUT_ERR_COUNT.fetch_add(1, Ordering::SeqCst);
    let out_path = parent.get_output_path();
    let err_path = parent.get_error_path();
    FileOutErr::new(
        out_path
            .get_parent_directory()
            .expect("NullPointerException")
            .get_relative_str(&format!("{}-{}", out_path.get_base_name(), index)),
        err_path
            .get_parent_directory()
            .expect("NullPointerException")
            .get_relative_str(&format!("{}-{}", err_path.get_base_name(), index)),
    )
}

/// `FileOutErr.dump(from, to)`: writes the captured content to the given `FileOutErr`.
///
/// PORT: `blaze_exec`'s `FileOutErr` has no `dump`. Java streams the files and never throws: a
/// failure to read or write is recorded in the stream (and printed with its output later);
/// `blaze_exec`'s `FileOutErr` has no recorded error, so such a failure is dropped here. Java
/// does not create the target file when there is nothing to write.
// Java: com/google/devtools/build/lib/util/io/FileOutErr.java:218 FileOutErr.dump
fn file_out_err_dump(from: &FileOutErr, to: &FileOutErr) {
    // `from.dumpOutAsLatin1(to.getOutputStream())`
    let out = from.out_as_bytes();
    if !out.is_empty() {
        let _ = to.write_out(&out);
    }
    // `from.dumpErrAsLatin1(to.getErrorStream())`
    let err = from.err_as_bytes();
    if !err.is_empty() {
        let _ = to.write_err(&err);
    }
}

/// `FileSystemUtils.copyTreesBelow(from, to)`.
///
/// PORT: `blaze_repo_fetch::fs_utils` (the port of `FileSystemUtils` this crate uses) lacks it.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:575 FileSystemUtils.copyTreesBelow
fn copy_trees_below(from: &Path, to: &Path) -> io::Result<()> {
    if to.starts_with(from) {
        panic!("IllegalArgumentException: {to} is a subdirectory of {from}");
    }

    for dirent in from.readdir(Symlinks::Nofollow)? {
        let from_child = from.get_child(dirent.get_name());
        let to_child = to.get_child(dirent.get_name());
        match dirent.get_type() {
            DirentType::File => fs_utils::copy_file(&from_child, &to_child)?,
            DirentType::Symlink => {
                fs_utils::ensure_symbolic_link(&to_child, &from_child.read_symbolic_link()?)?
            }
            DirentType::Directory => {
                to_child.create_directory()?;
                copy_trees_below(&from_child, &to_child)?;
            }
            _ => {
                return Err(errors::io_exception(format!(
                    "Don't know how to copy {from_child}"
                )));
            }
        }
    }
    Ok(())
}

impl RemoteExecutionService {
    /// PORT: Java returns a `ListenableFuture<FileMetadata>` that completes with `file`; the
    /// future here completes with `()` and the caller keeps `file`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:860 RemoteExecutionService.downloadFile
    fn download_file<'f>(
        &'f self,
        context: &'f RemoteActionExecutionContext<'_>,
        progress_status_listener: Arc<dyn ProgressStatusListener>,
        file: &'f FileMetadata,
        tmp_path: Path,
        remote_path_resolver: &dyn RemotePathResolver,
    ) -> LocalBoxFuture<'f, io::Result<()>> {
        let combined_cache: &'f CombinedCache = self
            .combined_cache
            .as_ref()
            .expect("NullPointerException: combinedCache can't be null");

        // PORT: StringEncoding identity (`internalToUnicode`).
        let output_path = remote_path_resolver.local_path_to_output_path(file.path());
        let exec_path = remote_path_resolver.local_path_to_exec_path(file.path().as_fragment());
        let reporter = DownloadProgressReporter::new(
            progress_status_listener,
            // PORT: StringEncoding identity (`internalToUnicode`).
            &remote_path_resolver.local_path_to_output_path(file.path()),
            file.digest().size_bytes,
        );
        // SECURITY: the digest of the downloaded file is checked by `CombinedCache.downloadFile`
        // and the cache clients behind it (`--remote_verify_downloads`); nothing here bypasses
        // it. The file is written to `tmp_path`, a path from the `TempPathGenerator`, never to a
        // path the server names.
        Box::pin(async move {
            combined_cache
                .download_file_with_reporter(
                    context,
                    &output_path,
                    Some(&exec_path),
                    &tmp_path,
                    file.digest(),
                    &reporter,
                )
                .await
        })
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:886 RemoteExecutionService.captureCorruptedOutputs
    fn capture_corrupted_outputs(&self, e: &mut io::Error) {
        if let Some(capture_corrupted_outputs_dir) = &self.capture_corrupted_outputs_dir {
            // `e instanceof BulkTransferException`
            // PORT: the exception is the payload of the `io::Error` that
            // `wait_for_bulk_transfer` built, which is not shared, so it can be changed in place.
            let Some(bulk_transfer_exception) = e
                .get_mut()
                .and_then(|inner| inner.downcast_mut::<BulkTransferException>())
            else {
                return;
            };
            // `e.getSuppressed()` returns a copy of the list.
            for suppressed in bulk_transfer_exception.get_suppressed().to_vec() {
                if let Some(output_digest_mismatch_exception) =
                    errors::downcast_ref::<OutputDigestMismatchException>(&suppressed)
                {
                    // Capture corrupted outputs
                    let captured = (|| -> io::Result<()> {
                        let output_path = output_digest_mismatch_exception.get_output_path();
                        let local_path = output_digest_mismatch_exception.get_local_path();
                        // `captureCorruptedOutputsDir.getRelative(outputPath)` throws a
                        // `NullPointerException` for a mismatch without an output path (a blob
                        // that was not downloaded to a file), which `catch (Exception ee)`
                        // catches.
                        // PORT: Java's message of that exception is the JVM's.
                        let output_path = output_path.ok_or_else(|| {
                            errors::io_exception("java.lang.NullPointerException")
                        })?;
                        // SECURITY: the directories are created before `dst` is checked to be
                        // under the capture directory, and the check is on the normalized path
                        // strings (symbolic links are not resolved), as in Java. The output path
                        // comes from `RemotePathResolver.localPathToOutputPath` of a file that
                        // `downloadOutputs` has already put under the exec root with
                        // `relativeTo(execRoot)`.
                        let dst = capture_corrupted_outputs_dir.get_relative_str(output_path);
                        dst.create_directory_and_parents()?;

                        // Make sure dst is still under captureCorruptedOutputsDir, otherwise
                        // IllegalArgumentException will be thrown.
                        dst.as_fragment()
                            .try_relative_to(capture_corrupted_outputs_dir.as_fragment())
                            .map_err(|message| errors::io_exception(message))?;

                        let local_path = local_path.ok_or_else(|| {
                            errors::io_exception("java.lang.NullPointerException")
                        })?;
                        fs_utils::copy_file(local_path, &dst)
                    })();
                    if let Err(ee) = captured {
                        // `e.addSuppressed(ee)`
                        // PORT: `BulkTransferException::add` also clears the "all caused by
                        // cache not found" flag, which Java's `addSuppressed` leaves alone; the
                        // flag is already false here, the exception holds an
                        // `OutputDigestMismatchException`.
                        bulk_transfer_exception.add(ee);
                    }
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:912 RemoteExecutionService.deletePartialDownloadedOutputs
    fn delete_partial_downloaded_outputs(
        &self,
        real_to_tmp_path: &JavaHashMap<Path>,
        tmp_out_err: &FileOutErr,
        _e: &io::Error,
    ) -> Result<(), ExecException> {
        let deleted = (|| -> io::Result<()> {
            // Delete any (partially) downloaded output files.
            for tmp_path in real_to_tmp_path.values() {
                tmp_path.delete()?;
            }

            tmp_out_err.clear_out()?;
            tmp_out_err.clear_err()?;
            Ok(())
        })();
        if let Err(io_ex) = deleted {
            // PORT: `ioEx.addSuppressed(e)` and `execEx.addSuppressed(e)` have no counterpart
            // (neither error type carries suppressed exceptions).

            // If deleting of output files failed, we abort the build with a decent error message
            // as any subsequent local execution failure would likely be incomprehensible.
            let exec_ex: ExecException = EnvironmentalExecException::with_cause(
                &io_ex,
                CombinedCache::create_failure_detail(
                    "Failed to delete output files after incomplete download",
                    remote_execution::Code::IncompleteOutputDownloadCleanupFailure,
                ),
            )
            .into();
            return Err(exec_ex);
        }
        Ok(())
    }

    /// Moves the locally created outputs from their temporary location to their declared
    /// location.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:939 RemoteExecutionService.moveOutputsToFinalLocation
    fn move_outputs_to_final_location<'p>(
        &self,
        local_outputs: impl IntoIterator<Item = &'p Path>,
        real_to_tmp_path: &JavaHashMap<Path>,
    ) -> io::Result<()> {
        // Move the output files from their temporary name to the actual output file name.
        // Executable bit is ignored since the file permission will be changed to 0555 after
        // execution.
        // SECURITY: no permission is set here and `FileMetadata.isExecutable` is not read; Java
        // relies on the action execution to set the permissions of all outputs afterwards. The
        // parent directories are created, and the move resolves, through whatever is in the
        // output tree already (symbolic links included); Java does not check that the parent is
        // a directory inside the exec root after resolution, nor whether `realPath` is a
        // directory or a file before it is replaced.
        for real_path in local_outputs {
            let tmp_path = real_to_tmp_path
                .get(real_path)
                .expect("NullPointerException");
            real_path
                .get_parent_directory()
                .expect("NullPointerException")
                .create_directory_and_parents()?;
            fs_utils::move_file(tmp_path, real_path)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:950 RemoteExecutionService.createSymlinks
    fn create_symlinks<'s>(
        &self,
        symlinks: impl IntoIterator<Item = &'s SymlinkMetadata>,
    ) -> io::Result<()> {
        // SECURITY: neither the path nor the target of a symlink is validated: the path is not
        // compared with the exec root (unlike the files, it does not go through
        // `relativeTo(execRoot)`) and the target may be absolute, contain `..` or dangle. Java
        // relies on nothing here; the symlink is created where the ActionResult (or the Tree)
        // says, with the parent directories it needs.
        for symlink in symlinks {
            symlink
                .path()
                .get_parent_directory()
                .unwrap_or_else(|| {
                    panic!(
                        "NullPointerException: Failed creating directory and parents for {}",
                        symlink.path()
                    )
                })
                .create_directory_and_parents()?;
            // If a directory output is being materialized as a symlink, creating the symlink
            // fails as we must first delete the preexisting empty directory. Since this is rare
            // (and in the future BwoB may no longer eagerly create these directories), we don't
            // delete the directory beforehand.
            if let Err(e) = symlink
                .path()
                .create_symbolic_link_to_fragment(symlink.target())
            {
                if !symlink.path().is_directory_with(Symlinks::Nofollow) {
                    return Err(e);
                }
                // Retry after deleting the directory.
                symlink.path().delete()?;
                symlink
                    .path()
                    .create_symbolic_link_to_fragment(symlink.target())?;
            }
        }
        Ok(())
    }

    /// `RemoteExecutionService.parseActionResultMetadata(combinedCache, digestUtil, context,
    /// result, remotePathResolver)`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1121 RemoteExecutionService.parseActionResultMetadata
    pub(crate) fn parse_action_result_metadata(
        combined_cache: &CombinedCache,
        digest_util: &DigestUtil,
        context: &RemoteActionExecutionContext<'_>,
        result: &ActionResult,
        remote_path_resolver: &dyn RemotePathResolver,
    ) -> io::Result<ActionResultMetadata> {
        parse_action_result_metadata(
            combined_cache,
            digest_util,
            context,
            result,
            remote_path_resolver,
        )
    }

    /// `action.getSpawnExecutionContext()::report` as a `ProgressStatusListener`.
    ///
    /// PORT: missing hook `SpawnExecutionContext.report(SpawnProgressEvent)`: `blaze_exec`'s
    /// `ProgressStatus` has no variant for the event (in Java it becomes an
    /// `ActionProgressEvent`, the "Downloading <file>, <n> / <m>" line of the progress bar), and
    /// the listener of a `DownloadProgressReporter` must not borrow the execution context. The
    /// progress of the downloads is not reported; nothing else depends on it.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1245 RemoteExecutionService.downloadOutputs
    fn progress_status_listener(_action: &RemoteAction<'_>) -> Arc<dyn ProgressStatusListener> {
        no_action()
    }

    /// `checkNotNull(remoteActionFileSystem).injectRemoteFile(path, digest, size, expirationTime,
    /// inMemoryOutput)`: the metadata of an output that is not downloaded is injected into the
    /// action file system of the spawn.
    ///
    /// PORT: two missing hooks. Java takes the file system at the top of `downloadOutputs`:
    /// `action.getSpawnExecutionContext().getActionFileSystem()`, with
    /// `checkState(actionFileSystem instanceof RemoteActionFileSystem, "expected the
    /// ActionFileSystem to be a RemoteActionFileSystem")`. `blaze_exec`'s `SpawnExecutionContext`
    /// has no action file system and `RemoteActionFileSystem` (the remote output tree) is not
    /// ported, so both are asked for here, where the file system is first used: a download in
    /// which every output is written to disk (`--remote_download_outputs=all` without an
    /// in-memory output) never gets here.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
    fn inject_remote_file(
        &self,
        _action: &RemoteAction<'_>,
        _path: &PathFragment,
        _digest: Vec<u8>,
        _size: i64,
        _expiration_time: SystemTime,
        _in_memory_output: bool,
    ) -> io::Result<()> {
        blaze_util::unported!(
            "RemoteExecutionService.downloadOutputs (missing hook: action.getSpawnExecutionContext().getActionFileSystem() as RemoteActionFileSystem, then remoteActionFileSystem.injectRemoteFile(path, digest, size, expirationTime, inMemoryOutput))"
        )
    }

    /// Downloads the outputs of a remotely executed action and injects their metadata.
    ///
    /// For a successful action, the `RemoteOutputChecker` is consulted to determine which of the
    /// outputs should be downloaded. For a failed action, all outputs are downloaded. The action
    /// stdout and stderr, as well as the in-memory output when present, are always downloaded
    /// even in the success case. Any outputs that are not downloaded have their metadata injected
    /// into the `RemoteActionFileSystem`.
    ///
    /// In case of download failure, all of the already downloaded outputs are deleted.
    ///
    /// Returns the in-memory output if the spawn had one, otherwise `None`.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1230 RemoteExecutionService.downloadOutputs
    pub fn download_outputs(
        &self,
        action: &RemoteAction<'_>,
        result: &RemoteActionResult,
    ) -> Result<Option<InMemoryOutput>, SpawnCacheError> {
        blaze_util::check_state!(!self.shutdown.load(Ordering::SeqCst), "shutdown");
        let combined_cache: &CombinedCache = self
            .combined_cache
            .as_ref()
            .expect("NullPointerException: combinedCache can't be null");

        // PORT: the output service is not ported: `hasBazelOutputService` is false, here and in
        // `should_download`, and the branches for it are left out. Java takes the
        // `RemoteActionFileSystem` of the spawn here; see `inject_remote_file`.

        let progress_status_listener = RemoteExecutionService::progress_status_listener(action);
        let mut context: RemoteActionExecutionContext<'_> =
            action.get_remote_action_execution_context().clone();
        if result.execute_response.is_some() {
            // Always read from remote cache for just remotely executed action.
            context =
                context.with_read_cache_policy(context.get_read_cache_policy().add_remote_cache());
        }
        let context = context;
        let remote_path_resolver: &dyn RemotePathResolver =
            action.get_remote_path_resolver().as_ref();

        let metadata = result.get_or_parse_action_result_metadata(
            combined_cache,
            &self.digest_util,
            &context,
            remote_path_resolver,
        )?;

        // The expiration time for remote cache entries.
        let expiration_time = SystemTime::now() + self.remote_options.remote_cache_ttl;

        let mut in_memory_output: Option<ActionInputRef> = None;
        // Java: `AtomicReference<ByteString>`.
        let in_memory_output_data: Mutex<Option<bytes::Bytes>> = Mutex::new(None);
        let in_memory_output_data = &in_memory_output_data;
        let in_memory_output_path: Option<PathFragment> =
            get_in_memory_output_path(action.get_spawn());
        let in_memory_output_path = in_memory_output_path.as_ref();
        if let Some(in_memory_output_path) = in_memory_output_path {
            for output in action.get_spawn().get_output_files() {
                if output.get_exec_path() == in_memory_output_path {
                    in_memory_output = Some(output.clone());
                    break;
                }
            }
        }

        // PORT: declared here and assigned where Java calls `outErr.childOutErr()`, below: the
        // downloads borrow it, so it has to outlive (be declared before) their list.
        let tmp_out_err: FileOutErr;

        // Collect the set of files to download.
        // PORT: Java collects `ListenableFuture<FileMetadata>`s that complete with the file of a
        // download and with null otherwise, and reads them back after the wait. The futures here
        // complete with `()`; `download_files` holds, for each of them, the file that Java's
        // future completes with.
        let mut downloads_builder: Vec<LocalBoxFuture<'_, io::Result<()>>> = Vec::new();
        let mut download_files: Vec<Option<&FileMetadata>> = Vec::new();

        // Download into temporary paths, then move everything at the end.
        // This avoids holding the output lock while downloading, which would prevent the local
        // branch from completing sooner under the dynamic execution strategy.
        let mut real_to_tmp_path: JavaHashMap<Path> = JavaHashMap::new();

        for file in metadata.files() {
            if real_to_tmp_path.contains_key(&file.path) {
                continue;
            }

            // SECURITY: this is the check that keeps a file inside the exec root: `relativeTo`
            // throws `IllegalArgumentException` (a crash in Java, a panic here) for a path that
            // is not under `execRoot`, before the download of the file is started. It compares
            // normalized path strings; symbolic links in the output tree are not resolved.
            let exec_path = file.path.relative_to(&self.exec_root);
            let is_in_memory_output_file =
                in_memory_output.is_some() && Some(&exec_path) == in_memory_output_path;
            if !is_in_memory_output_file
                && self.should_download(result, &exec_path, /* treeRootExecPath= */ None)
            {
                let tmp_path = self.temp_path_generator.generate_temp_path();
                real_to_tmp_path.put(file.path.clone(), tmp_path.clone());
                downloads_builder.push(self.download_file(
                    &context,
                    progress_status_listener.clone(),
                    file,
                    tmp_path,
                    remote_path_resolver,
                ));
                download_files.push(Some(file));
            } else {
                self.inject_remote_file(
                    action,
                    file.path().as_fragment(),
                    DigestUtil::to_binary_digest(file.digest()),
                    file.digest().size_bytes,
                    expiration_time,
                    is_in_memory_output_file,
                )?;

                if is_in_memory_output_file {
                    let in_memory_output_path =
                        in_memory_output_path.expect("inMemoryOutput is not null");
                    // Download into memory only; do not write to disk.
                    self.remote_output_checker
                        .as_ref()
                        .expect("NullPointerException: remoteOutputChecker")
                        .skip_download(in_memory_output_path);
                    if file.contents.is_empty() {
                        // As the contents field doesn't have presence information, we use the
                        // digest size to distinguish between an empty file and one that wasn't
                        // inlined.
                        if file.digest.size_bytes == 0 {
                            *in_memory_output_data.lock() = Some(bytes::Bytes::new());
                        } else {
                            let context = &context;
                            downloads_builder.push(Box::pin(async move {
                                let data = combined_cache
                                    .download_blob_as_byte_string(
                                        context,
                                        in_memory_output_path.get_path_string(),
                                        Some(in_memory_output_path),
                                        file.digest(),
                                    )
                                    .await?;
                                *in_memory_output_data.lock() = Some(data);
                                Ok(())
                            }));
                            download_files.push(None);
                        }
                    } else {
                        // SECURITY: contents that the server inlined in the ActionResult are
                        // taken as they are; Java does not compare them with `file.digest`.
                        *in_memory_output_data.lock() = Some(file.contents.clone());
                    }
                }
            }
        }

        for (path, directory) in metadata.directories() {
            // SECURITY: as for the files above, `relativeTo` is the check that the root of the
            // tree is under the exec root.
            let tree_root_exec_path = path.relative_to(&self.exec_root);

            for file in directory.files() {
                if real_to_tmp_path.contains_key(&file.path) {
                    continue;
                }

                // SECURITY: `relativeTo` is the check that the file is under the exec root (it
                // is not checked to be under the root of its tree).
                if self.should_download(
                    result,
                    &file.path.relative_to(&self.exec_root),
                    Some(&tree_root_exec_path),
                ) {
                    let tmp_path = self.temp_path_generator.generate_temp_path();
                    real_to_tmp_path.put(file.path.clone(), tmp_path.clone());
                    downloads_builder.push(self.download_file(
                        &context,
                        progress_status_listener.clone(),
                        file,
                        tmp_path,
                        remote_path_resolver,
                    ));
                    download_files.push(Some(file));
                } else {
                    self.inject_remote_file(
                        action,
                        file.path().as_fragment(),
                        DigestUtil::to_binary_digest(file.digest()),
                        file.digest().size_bytes,
                        expiration_time,
                        /* inMemoryOutput= */ false,
                    )?;
                }
            }
        }

        let out_err: &FileOutErr = action.get_spawn_execution_context().get_file_out_err();

        // Always download the action stdout/stderr.
        tmp_out_err = child_out_err(out_err);
        let out_err_downloads =
            combined_cache.download_out_err(&context, &result.action_result, &tmp_out_err);
        for future in out_err_downloads {
            downloads_builder.push(Box::pin(future));
            download_files.push(None);
        }

        let downloads = downloads_builder;
        // PORT: skipped, profiling ("Remote.download").
        if let Err(mut e) = wait_for_bulk_transfer(downloads) {
            // TODO(bazel-team): Consider adding better case-by-case exception handling instead of
            // just rethrowing
            self.capture_corrupted_outputs(&mut e);
            self.delete_partial_downloaded_outputs(&real_to_tmp_path, &tmp_out_err, &e)?;
            return Err(e.into());
        }

        file_out_err_dump(&tmp_out_err, out_err);

        // Ensure that we are the only ones writing to the output files when using the dynamic
        // spawn strategy.
        action.get_spawn_execution_context().lock_output_files(
            result.get_exit_code(),
            &result.get_message(),
            &tmp_out_err,
        )?;
        // Will these be properly garbage-collected if the above throws an exception?
        tmp_out_err.clear_out()?;
        tmp_out_err.clear_err()?;

        let mut finished_downloads: Vec<&FileMetadata> = Vec::with_capacity(download_files.len());
        for finished_download in &download_files {
            // `getFromFuture(finishedDownload)`: all the transfers have succeeded.
            let output_file = *finished_download;
            if let Some(output_file) = output_file {
                finished_downloads.push(output_file);
            }
        }

        self.move_outputs_to_final_location(
            finished_downloads
                .iter()
                .map(|output_file| output_file.path()),
            &real_to_tmp_path,
        )?;

        let mut symlinks_in_directories: Vec<&SymlinkMetadata> = Vec::new();
        for (_, directory) in metadata.directories() {
            for symlink in directory.symlinks() {
                symlinks_in_directories.push(symlink);
            }
        }

        let symlinks = metadata.symlinks().chain(symlinks_in_directories);

        // Create the symbolic links after all downloads are finished, because dangling symlinks
        // might not be supported on all platforms.
        self.create_symlinks(symlinks)?;

        if result.success() {
            // Check that all mandatory outputs are created.
            let missing_mandatory_output = result.maybe_get_missing_mandatory_output(action);
            if let Some(missing_mandatory_output) = missing_mandatory_output {
                return Err(errors::io_exception(format!(
                    "mandatory output {} was not created",
                    RemoteExecutionService::pretty_print(&*missing_mandatory_output)
                ))
                .into());
            }

            if result.execute_response.is_some() {
                let mut known_missing_cas_digests = self.known_missing_cas_digests.lock();
                if !known_missing_cas_digests.is_empty() {
                    // A succeeded execution uploads outputs to CAS. Refresh our knowledge about
                    // missing digests.
                    let _unused =
                        update_known_missing_cas_digests(&mut known_missing_cas_digests, &metadata);
                }
            }

            // When downloading outputs from just remotely executed action, the action result
            // comes from Execution response which means, if disk cache is enabled, action result
            // hasn't been uploaded to it. Upload action result to disk cache here so next build
            // could hit it.
            if self.use_disk_cache() && result.execute_response.is_some() {
                let action_key: ActionKey = action.get_action_key().clone();
                get_from_future(combined_cache.upload_action_result(
                    &context.with_write_cache_policy(CachePolicy::DiskCacheOnly),
                    &action_key,
                    &result.action_result,
                ))?;
            }
        }

        if let Some(in_memory_output) = in_memory_output {
            let in_memory_output_data = in_memory_output_data.lock().take();
            if let Some(in_memory_output_data) = in_memory_output_data {
                return Ok(Some(InMemoryOutput::new(
                    in_memory_output,
                    in_memory_output_data,
                )));
            }
        }

        Ok(None)
    }

    /// Makes the `SpawnResult` available to all parallel `Spawn`s for the same `RemoteAction`
    /// waiting for it or notifies them that the spawn failed.
    ///
    /// Returns whether the spawn result should be uploaded to the cache.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1563 RemoteExecutionService.commitResultAndDecideWhetherToUpload
    pub fn commit_result_and_decide_whether_to_upload(
        &self,
        result: &SpawnResult,
        execution: Option<&LocalExecution>,
    ) -> bool {
        if result.status() == SpawnResultStatus::Success && result.exit_code() == 0 {
            if let Some(execution) = execution {
                execution.spawn_result_future.set(result.clone());
            }
            true
        } else {
            if let Some(execution) = execution {
                execution.spawn_result_future.cancel();
            }
            false
        }
    }

    /// Reuses the outputs of a concurrent local execution of the same RemoteAction in a different
    /// spawn.
    ///
    /// Since each output file is generated by a unique action and actions generally take care to
    /// run a unique spawn for each output file, this method is only useful with path mapping
    /// enabled, which allows different spawns in a single build to have the same
    /// `RemoteAction.ActionKey`.
    ///
    /// Returns the `SpawnResult` of the previous execution if it was successful, otherwise
    /// `None`.
    ///
    /// PORT: Java throws `InterruptedException, IOException`; `lockOutputFiles` of `blaze_exec`
    /// returns an `ExecException` (which carries the interruption), so the error type is the one
    /// that has both.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1589 RemoteExecutionService.waitForAndReuseOutputs
    pub fn wait_for_and_reuse_outputs(
        &self,
        action: &RemoteAction<'_>,
        previous_execution: &LocalExecution,
    ) -> Result<Option<SpawnResult>, SpawnCacheError> {
        blaze_util::check_state!(!self.shutdown.load(Ordering::SeqCst), "shutdown");

        // PORT: the future is only ever set or cancelled, so `get()` has no `ExecutionException`
        // with a cause to rethrow.
        let Some(previous_spawn_result) = previous_execution.spawn_result_future.get() else {
            // The spawn this action was deduplicated against failed due to an exception or
            // non-zero exit code. Since it isn't possible to transparently replay its failure for
            // the current spawn, we rerun the action instead.
            return Ok(None);
        };

        let action_key: ActionKey = action.get_action_key().clone();
        blaze_util::check_argument!(action_key == previous_execution.action_key);

        // `toImmutableMap` throws an `IllegalArgumentException` for two outputs with one path.
        let mut previous_outputs: IndexMap<Path, ActionInputRef> = IndexMap::new();
        for output in &previous_execution.spawn_output_files {
            let path = self.exec_root.get_relative(output.get_exec_path());
            if previous_outputs.contains_key(&path) {
                panic!("IllegalArgumentException: Multiple entries with same key: {path}");
            }
            previous_outputs.insert(path, output.clone());
        }
        let mut real_to_tmp_path: JavaHashMap<Path> = JavaHashMap::new();
        let mut in_memory_output_content: Option<bytes::Bytes> = None;
        let mut in_memory_output_path: Option<String> = None;
        let command = action.get_command();
        let output_paths_list: Vec<&String> = if self.use_output_paths() {
            command.output_paths.iter().collect()
        } else {
            command
                .output_files
                .iter()
                .chain(command.output_directories.iter())
                .collect()
        };
        let reused = self.reuse_outputs(
            action,
            previous_execution,
            &previous_spawn_result,
            &previous_outputs,
            &output_paths_list,
            &mut real_to_tmp_path,
            &mut in_memory_output_content,
            &mut in_memory_output_path,
        );
        match reused {
            Ok(true) => {}
            // Java: `return null` from inside the `try` block.
            Ok(false) => return Ok(None),
            // Java: `catch (InterruptedException | IOException e)`.
            Err(e) => {
                // Delete any copied output files.
                for tmp_path in real_to_tmp_path.values() {
                    if tmp_path.delete().is_err() {
                        // Best effort, will be cleaned up at server restart.
                        break;
                    }
                }
                return Err(e);
            }
        }

        if let Some(in_memory_output_path) = in_memory_output_path {
            let final_in_memory_output_path = in_memory_output_path;
            let final_in_memory_output_content = in_memory_output_content;
            return Ok(Some(
                RemoteExecutionService::delegate_spawn_result_with_in_memory_output(
                    previous_spawn_result,
                    final_in_memory_output_path,
                    final_in_memory_output_content,
                ),
            ));
        }

        Ok(Some(previous_spawn_result))
    }

    /// The `try` block of `waitForAndReuseOutputs`. Returns false where Java returns null from
    /// inside the block.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1622 RemoteExecutionService.waitForAndReuseOutputs
    fn reuse_outputs(
        &self,
        action: &RemoteAction<'_>,
        previous_execution: &LocalExecution,
        previous_spawn_result: &SpawnResult,
        previous_outputs: &IndexMap<Path, ActionInputRef>,
        output_paths_list: &[&String],
        real_to_tmp_path: &mut JavaHashMap<Path>,
        in_memory_output_content: &mut Option<bytes::Bytes>,
        in_memory_output_path: &mut Option<String>,
    ) -> Result<bool, SpawnCacheError> {
        for output in output_paths_list {
            // PORT: StringEncoding identity (`unicodeToInternal`).
            let reencoded_output: &str = output;
            let source_path = previous_execution
                .remote_path_resolver
                .output_path_to_local_path(reencoded_output);
            let output_artifact: Option<&ActionInputRef> = previous_outputs.get(&source_path);
            let target_path = action
                .get_remote_path_resolver()
                .output_path_to_local_path(reencoded_output);
            // `previousSpawnResult.getInMemoryOutput(outputArtifact)` is null for a null
            // `outputArtifact`.
            *in_memory_output_content = output_artifact
                .and_then(|output_artifact| {
                    previous_spawn_result.get_in_memory_output(output_artifact)
                })
                .map(bytes::Bytes::copy_from_slice);
            if in_memory_output_content.is_some() {
                *in_memory_output_path = Some(
                    target_path
                        .relative_to(&self.exec_root)
                        .get_path_string()
                        .to_string(),
                );
                continue;
            }
            let tmp_path = self.temp_path_generator.generate_temp_path();
            tmp_path
                .get_parent_directory()
                .expect("NullPointerException")
                .create_directory_and_parents()?;
            let output_artifact = output_artifact.expect("NullPointerException");
            let copied = (|| -> io::Result<()> {
                if output_artifact.is_directory() {
                    tmp_path.create_directory()?;
                    copy_trees_below(&source_path, &tmp_path)?;
                } else if output_artifact.is_symlink() {
                    fs_utils::ensure_symbolic_link(&tmp_path, &source_path.read_symbolic_link()?)?;
                } else {
                    fs_utils::copy_file(&source_path, &tmp_path)?;
                }
                Ok(())
            })();
            match copied {
                Ok(()) => {
                    real_to_tmp_path.put(target_path, tmp_path);
                }
                // Java: `catch (FileNotFoundException e)`.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    // The spawn this action was deduplicated against failed to create an output
                    // file. If the output is mandatory, we cannot reuse the previous execution.
                    if action.get_spawn().is_mandatory_output(output_artifact) {
                        return Ok(false);
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        // TODO: FileOutErr is action-scoped, not spawn-scoped, but this is not a problem for the
        //  current use case of supporting deduplication of path mapped spawns:
        //  1. Starlark and C++ compilation actions always create a single spawn.
        //  2. Java compilation actions may run a fallback spawn, but reset the FileOutErr before
        //     running it.
        //  If this changes, we will need to introduce a spawn-scoped OutErr.
        file_out_err_dump(
            &previous_execution.file_out_err,
            action.get_spawn_execution_context().get_file_out_err(),
        );

        action.get_spawn_execution_context().lock_output_files(
            previous_spawn_result.exit_code(),
            previous_spawn_result.get_failure_message(),
            action.get_spawn_execution_context().get_file_out_err(),
        )?;
        // All outputs are created locally.
        self.move_outputs_to_final_location(real_to_tmp_path.keys(), real_to_tmp_path)?;
        Ok(true)
    }

    /// `new SpawnResult.DelegateSpawnResult(previousSpawnResult) { getInMemoryOutput(output) {
    /// return output.getExecPathString().equals(finalInMemoryOutputPath) ?
    /// finalInMemoryOutputContent : null; } }`.
    ///
    /// PORT: missing hook. `blaze_exec`'s `SpawnResult` is a plain struct; it has no
    /// `DelegateSpawnResult` and no way to give a built result another in-memory output.
    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1687 RemoteExecutionService.waitForAndReuseOutputs
    fn delegate_spawn_result_with_in_memory_output(
        _previous_spawn_result: SpawnResult,
        _final_in_memory_output_path: String,
        _final_in_memory_output_content: Option<bytes::Bytes>,
    ) -> SpawnResult {
        blaze_util::unported!(
            "RemoteExecutionService.waitForAndReuseOutputs (missing hook: SpawnResult.DelegateSpawnResult overriding getInMemoryOutput(ActionInput))"
        )
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1704 RemoteExecutionService.shouldDownload
    fn should_download(
        &self,
        result: &RemoteActionResult,
        exec_path: &PathFragment,
        tree_root_exec_path: Option<&PathFragment>,
    ) -> bool {
        // PORT: the output service is not ported, so `outputService instanceof
        // BazelOutputService` is false.

        // In case the action failed, download all outputs. It might be helpful for debugging and
        // there is no point in injecting output metadata of a failed action.
        if result.get_exit_code() != 0 {
            return true;
        }
        self.remote_output_checker
            .as_ref()
            .expect("NullPointerException: remoteOutputChecker")
            .should_download_output_path(exec_path, tree_root_exec_path)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:1718 RemoteExecutionService.prettyPrint
    pub(crate) fn pretty_print(action_input: &dyn ActionInput) -> String {
        if let Some(artifact) = action_input.as_artifact() {
            // `artifact.prettyPrint()`
            // Java: com/google/devtools/build/lib/actions/Artifact.java:669 Artifact.prettyPrint
            artifact
                .get_root_relative_path()
                .get_path_string()
                .to_string()
        } else {
            action_input.get_exec_path_string()
        }
    }
}
