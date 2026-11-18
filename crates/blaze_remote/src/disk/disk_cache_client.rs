//! Port of `lib/remote/disk/DiskCacheClient.java`.

use std::io::{self, Read, Write};

use blaze_protos::JavaMessage;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{NOW_SENTINEL_TIME, Path};
use indexmap::IndexSet;
use prost::Message as _;

use crate::common::cache_not_found_exception::CacheNotFoundException;
use crate::common::remote_cache_client::{ActionKey, Blob, BytesBlob};
use crate::reapi::{ActionResult, Digest, Directory, Tree};
use crate::store::Store;
use crate::util::digest_util::DigestUtil;
use crate::util::utils;

const AC_DIR: &str = "ac";
const CAS_DIR: &str = "cas";
const TMP_DIR: &str = "tmp";

/// An on-disk store for the remote action cache.
///
/// Concurrent Bazel processes can safely retrieve and store entries in a shared disk cache, even
/// when they collide.
///
/// The mtime of an entry reflects the most recent time the entry was stored *or* retrieved. This
/// property may be used to garbage collect the disk cache by deleting the least recently accessed
/// entries. This may be done by Bazel itself (see `DiskCacheGarbageCollectorIdleTask`), by
/// another Bazel process sharing the disk cache, or by an external process. Although we could
/// have arranged for an ongoing garbage collection to block a concurrent build, we judge it to
/// not be worth the extra complexity; assuming that the collection policy is not overly
/// aggressive, the likelihood of a race condition is fairly small, and an affected build is able
/// to automatically recover by retrying.
///
/// PORT: Java submits every operation to a virtual-thread executor and returns a
/// `ListenableFuture`; here the operations are `async fn`s that do their file I/O inline
/// (docs/remote.md, convention 1).
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:67 DiskCacheClient
#[derive(Debug, Clone)]
pub struct DiskCacheClient {
    /// `storeRootMap.get(Store.AC)`.
    ac_root: Path,
    /// `storeRootMap.get(Store.CAS)`.
    cas_root: Path,
    tmp_root: Path,
    check_action_result_integrity: bool,
}

impl DiskCacheClient {
    /// Creates a new disk cache client.
    ///
    /// `check_action_result_integrity`: whether [`DiskCacheClient::download_action_result`]
    /// should only return an action result whose referenced blobs are all present in the disk
    /// cache.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:90 DiskCacheClient.<init>
    pub fn new(
        root: Path,
        digest_util: &DigestUtil,
        check_action_result_integrity: bool,
    ) -> io::Result<DiskCacheClient> {
        let fn_root = if DigestUtil::is_old_style_digest_function(digest_util.get_digest_function())
        {
            root.clone()
        } else {
            root.get_child(
                &digest_util
                    .get_digest_function()
                    .as_str_name()
                    .to_ascii_lowercase(),
            )
        };
        let ac_root = fn_root.get_child(AC_DIR);
        let cas_root = fn_root.get_child(CAS_DIR);

        let tmp_root = root.get_child(TMP_DIR);

        fn_root.create_directory_and_parents()?;
        tmp_root.create_directory_and_parents()?;

        Ok(DiskCacheClient {
            ac_root,
            cas_root,
            tmp_root,
            check_action_result_integrity,
        })
    }

    /// If the given path exists, updates its mtime and returns true. Otherwise, returns false.
    ///
    /// This provides a cheap way to identify candidates for deletion when trimming the cache. We
    /// deliberately use the mtime because the atime is more likely to be externally modified and
    /// may be unavailable on some filesystems.
    ///
    /// Prefer calling [`DiskCacheClient::download_blob`] instead, which will automatically update
    /// the mtime. This method should only be called by the remote worker implementation.
    ///
    /// Fails if an I/O error other than a missing file occurs.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:119 DiskCacheClient.refresh
    pub fn refresh(&self, path: &Path) -> io::Result<bool> {
        // Use NOW_SENTINEL_TIME instead of obtaining the current time so that the operation
        // succeeds even when the file has a different owner, as might be the case for a shared
        // cache.
        match path.set_last_modified_time(NOW_SENTINEL_TIME) {
            Ok(()) => Ok(true),
            Err(e) if is_file_not_found(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Moves an existing file into the cache.
    ///
    /// The caller must ensure that the digest is correct and the file has been recently modified.
    /// This method should only be called by the combined cache implementation.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:136 DiskCacheClient.captureFile
    pub fn capture_file(&self, src: &Path, digest: &Digest, store: Store) -> io::Result<()> {
        let target = self.to_path(digest, store);

        if self.refresh(&target)? {
            src.delete()?;
            return Ok(());
        }

        parent_directory(&target).create_directory_and_parents()?;
        fs_utils::rename_tolerating_concurrent_creation(src, &target)
    }

    /// `download(digest, out, store)`.
    ///
    /// PORT: Java asks `out instanceof MaybePathBacked` and takes `maybeGetPath()`. A
    /// `dyn Write` cannot be asked for an interface, so the caller passes that path as
    /// `out_path` (`None` when the stream is not path-backed).
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:148 DiskCacheClient.download
    async fn download(
        &self,
        digest: &Digest,
        out: &mut (dyn Write + Send),
        out_path: Option<&Path>,
        store: Store,
    ) -> io::Result<()> {
        // PORT: Java runs this on an executor.
        let path = self.to_path(digest, store);
        if !self.refresh(&path)? {
            return Err(CacheNotFoundException::new(digest.clone()).into());
        }

        let copied: io::Result<()> = match out_path {
            // If the output stream is path-backed, the filesystem may be able to avoid copying
            // the file.
            Some(out_path) => fs_utils::copy_file(&path, out_path),
            None => match path.get_input_stream() {
                Ok(mut input) => io::copy(&mut input, out).map(|_| ()),
                Err(e) => Err(e),
            },
        };
        match copied {
            Ok(()) => Ok(()),
            Err(e) if is_file_not_found(&e) => {
                // The entry may have been deleted between the refresh above and the copy, for
                // example due to a concurrent garbage collection. Report this case as a cache
                // miss rather than a real I/O error.
                //
                // Note that a FileNotFoundException could also be thrown if a parent directory of
                // the destination doesn't exist, so we try to preserve the error reporting in
                // that case.
                if let Some(out_path) = out_path
                    && let Some(parent) = out_path.get_parent_directory()
                    && !parent.exists()
                {
                    return Err(e);
                }
                // PORT: `cacheNotFoundException.addSuppressed(e)` has no counterpart (an
                // `io::Error` carries no suppressed exceptions).
                Err(CacheNotFoundException::new(digest.clone()).into())
            }
            Err(e) => Err(e),
        }
    }

    /// `downloadBlob(digest, out)` for a stream that is not path-backed.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:192 DiskCacheClient.downloadBlob
    pub async fn download_blob(
        &self,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        self.download_blob_maybe_path_backed(digest, out, None)
            .await
    }

    /// `downloadBlob(digest, out)` for a stream that may be path-backed: `out_path` is
    /// `out instanceof MaybePathBacked ? out.maybeGetPath() : null` (see
    /// [`DiskCacheClient::download`] for why it is a parameter). With a path, the entry is copied
    /// to that path directly and nothing is written to `out`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:192 DiskCacheClient.downloadBlob
    pub async fn download_blob_maybe_path_backed(
        &self,
        digest: &Digest,
        out: &mut (dyn Write + Send),
        out_path: Option<&Path>,
    ) -> io::Result<()> {
        self.download(digest, out, out_path, Store::Cas).await?;
        out.flush()
    }

    /// If the blob with the given digest exists, marks it as recently used.
    ///
    /// Returns whether the blob exists. Fails if an I/O error other than a missing file occurs.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:212 DiskCacheClient.refreshDigest
    fn refresh_digest(&self, digest: &Digest) -> io::Result<bool> {
        if digest.size_bytes == 0 {
            return Ok(true);
        }

        self.refresh(&self.to_path(digest, Store::Cas))
    }

    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:220 DiskCacheClient.refreshOutputDirectory
    fn refresh_output_directory(
        &self,
        dir: &Directory,
        stop_at_first_missing: bool,
    ) -> io::Result<bool> {
        let mut all_present = true;
        for file in &dir.files {
            all_present &= self.refresh_digest(&digest_or_default(&file.digest))?;
            if !all_present && stop_at_first_missing {
                return Ok(false);
            }
        }
        Ok(all_present)
    }

    /// Marks all of the blobs referenced by the `ActionResult` that exist as recently used.
    ///
    /// `stop_at_first_missing`: whether to return as soon as a referenced blob is found to be
    /// missing, leaving the mtime of the remaining blobs untouched.
    ///
    /// Returns whether all of the referenced blobs exist. Fails if an I/O error other than a
    /// missing file occurs.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:240 DiskCacheClient.refreshActionResult
    fn refresh_action_result(
        &self,
        action_result: &ActionResult,
        stop_at_first_missing: bool,
    ) -> io::Result<bool> {
        let mut all_present = true;

        for output_file in &action_result.output_files {
            all_present &= self.refresh_digest(&digest_or_default(&output_file.digest))?;
            if !all_present && stop_at_first_missing {
                return Ok(false);
            }
        }

        for output_directory in &action_result.output_directories {
            let tree_digest = digest_or_default(&output_directory.tree_digest);
            if !self.refresh_digest(&tree_digest)? {
                // Without the Tree, the blobs it references can't be determined.
                if stop_at_first_missing {
                    return Ok(false);
                }
                all_present = false;
                continue;
            }

            let tree = match read_fully(&self.to_path(&tree_digest, Store::Cas)) {
                Ok(bytes) => Tree::decode(&bytes[..]).map_err(invalid_protocol_buffer_exception)?,
                Err(e) if is_file_not_found(&e) => {
                    // The tree was deleted between the refresh above and the read, most likely by
                    // a concurrent garbage collection. Treat it as missing rather than as a real
                    // I/O error.
                    if stop_at_first_missing {
                        return Ok(false);
                    }
                    all_present = false;
                    continue;
                }
                Err(e) => return Err(e),
            };
            // `tree.getRoot()` is the default instance when the field is not set.
            let root = tree.root.clone().unwrap_or_default();
            all_present &= self.refresh_output_directory(&root, stop_at_first_missing)?;
            if !all_present && stop_at_first_missing {
                return Ok(false);
            }
            for dir in &tree.children {
                all_present &= self.refresh_output_directory(dir, stop_at_first_missing)?;
                if !all_present && stop_at_first_missing {
                    return Ok(false);
                }
            }
        }

        if let Some(stdout_digest) = &action_result.stdout_digest {
            all_present &= self.refresh_digest(stdout_digest)?;
            if !all_present && stop_at_first_missing {
                return Ok(false);
            }
        }

        if let Some(stderr_digest) = &action_result.stderr_digest {
            all_present &= self.refresh_digest(stderr_digest)?;
        }

        Ok(all_present)
    }

    /// `downloadActionResult(actionKey)`: `None` when the action result is not cached (Java: a
    /// future completing with null), or when it is stale and integrity checking is on.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:300 DiskCacheClient.downloadActionResult
    pub async fn download_action_result(
        &self,
        action_key: &ActionKey,
    ) -> io::Result<Option<ActionResult>> {
        // Update the mtime on the action result itself before any of the blobs it references.
        // This ensures that the blobs are always newer than the action result, so that trimming
        // the cache in LRU order cannot create dangling references.
        let action_result = utils::download_as_action_result(
            action_key,
            move |digest: Digest, mut out: Vec<u8>| async move {
                self.download(&digest, &mut out, None, Store::Ac).await?;
                Ok::<Vec<u8>, io::Error>(out)
            },
        )
        .await?;
        let Some(action_result) = action_result else {
            return Ok(None);
        };

        let all_blobs_present = self.refresh_action_result(
            &action_result,
            /* stop_at_first_missing= */ self.check_action_result_integrity,
        )?;

        if self.check_action_result_integrity && !all_blobs_present {
            // If at least one of the referenced blobs is missing, consider the action result to
            // be stale.
            return Ok(None);
        }

        Ok(Some(action_result))
    }

    /// `uploadActionResult(actionKey, actionResult)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:326 DiskCacheClient.uploadActionResult
    pub async fn upload_action_result(
        &self,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        // PORT: Java runs this on an executor.
        let data = action_result.to_byte_array();
        self.save_file(action_key.digest(), Store::Ac, &mut &data[..])
    }

    /// `close()`.
    ///
    /// PORT: Java closes the executor service; there is none here.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:336 DiskCacheClient.close
    pub fn close(&self) {}

    /// `uploadFile(digest, file)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:340 DiskCacheClient.uploadFile
    pub async fn upload_file(&self, digest: &Digest, file: &Path) -> io::Result<()> {
        // PORT: Java runs this on an executor.
        self.save_file_from_path(digest, Store::Cas, file)
    }

    /// `uploadBlob(digest, ByteString data)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:348 DiskCacheClient.uploadBlob
    pub async fn upload_bytes(&self, digest: &Digest, data: bytes::Bytes) -> io::Result<()> {
        self.upload_blob(digest, &BytesBlob(data)).await
    }

    /// Uploads a blob from a stream supplier.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:353 DiskCacheClient.uploadBlob
    pub async fn upload_blob(&self, digest: &Digest, blob: &dyn Blob) -> io::Result<()> {
        // PORT: Java runs this on an executor.
        let mut input = blob.get()?;
        self.save_file(digest, Store::Cas, &mut *input)
    }

    /// `findMissingDigests(digests)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:363 DiskCacheClient.findMissingDigests
    pub async fn find_missing_digests(&self, digests: &[Digest]) -> io::Result<IndexSet<Digest>> {
        // Both upload and download check if the file exists before doing I/O. So we don't
        // have to do it here.
        Ok(digests.iter().cloned().collect())
    }

    /// `getTempPath()`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:369 DiskCacheClient.getTempPath
    pub fn get_temp_path(&self) -> Path {
        self.tmp_root.get_child(&uuid::Uuid::new_v4().to_string())
    }

    /// `toPath(Digest digest, Store store)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:373 DiskCacheClient.toPath
    pub fn to_path(&self, digest: &Digest, store: Store) -> Path {
        let hash = &digest.hash;
        self.to_path_from_hash(hash, store)
    }

    /// `toPath(String hash, Store store)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:378 DiskCacheClient.toPath
    pub fn to_path_from_hash(&self, hash: &str, store: Store) -> Path {
        // Create the file in a subfolder to bypass possible folder file count limits.
        let store_root = match store {
            Store::Ac => &self.ac_root,
            Store::Cas => &self.cas_root,
        };
        store_root.get_child(&hash[0..2]).get_child(hash)
    }

    /// `saveFile(Digest digest, Store store, InputStream in)`.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:383 DiskCacheClient.saveFile
    pub fn save_file(&self, digest: &Digest, store: Store, input: &mut dyn Read) -> io::Result<()> {
        self.save(digest, store, |temp| {
            {
                let mut out = temp.get_output_stream()?;
                io::copy(input, &mut out)?;
                out.flush()?;
            }
            // Fsync temp before we rename it to avoid data loss in the case of machine
            // crashes (the OS may reorder the writes and the rename).
            // PORT: Java syncs through the output stream's descriptor before closing it; the
            // VFS output stream does not expose one, so the file is synced after it is written.
            sync_file(temp)
        })
    }

    /// Saves an existing file into the cache.
    ///
    /// The contents are copied through `FileSystemUtils.copyFile`, so a filesystem with
    /// copy-on-write support (clonefile on macOS, copy_file_range on Linux) can serve the copy as
    /// a clone, leaving the entry sharing its blocks with the file it was saved from.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:406 DiskCacheClient.saveFile
    fn save_file_from_path(&self, digest: &Digest, store: Store, file: &Path) -> io::Result<()> {
        self.save(digest, store, |temp| {
            fs_utils::copy_file(file, temp)?;
            // copyFile preserves the source's permissions and mtime, neither of which suits a
            // cache entry: an entry must remain readable by every user of a shared cache, and its
            // mtime records when it was last stored or retrieved.
            temp.chmod(0o644)?;
            temp.set_last_modified_time(NOW_SENTINEL_TIME)?;
            // Fsync temp before we rename it to avoid data loss in the case of machine
            // crashes (the OS may reorder the writes and the rename).
            sync_file(temp)
        })
    }

    /// `save(digest, store, writer)`; `writer` (Java `TempFileWriter`) writes the contents of a
    /// cache entry into a temporary file.
    // Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:428 DiskCacheClient.save
    fn save(
        &self,
        digest: &Digest,
        store: Store,
        writer: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let is_cas = matches!(store, Store::Cas);
        let path = self.to_path(digest, store);

        // CAS entries are content-addressed and thus automatically have the correct content if
        // they exist.
        if is_cas && self.refresh(&path)? {
            return Ok(());
        }

        // Write a temporary file first, and then rename, to avoid data corruption in case of a
        // crash.
        let temp = self.get_temp_path();

        let saved = (|| {
            writer(&temp)?;
            parent_directory(&path).create_directory_and_parents()?;
            fs_utils::rename_tolerating_concurrent_creation(&temp, &path)
        })();
        if let Err(e) = saved {
            // PORT: `e.addSuppressed(deleteErr)` has no counterpart (an `io::Error` carries no
            // suppressed exceptions).
            let _ = temp.delete();
            return Err(e);
        }
        Ok(())
    }
}

/// Flushes a file's contents to stable storage, where the filesystem supports it.
///
/// PORT: Java opens `path.getInputStream()` and syncs its descriptor when the stream is a
/// `FileInputStream`. The VFS stream does not expose its file, so the file is opened by its path
/// string; a path that the operating system cannot open (a file system that is not disk-backed,
/// Java's "not a FileInputStream") is not synced.
// Java: com/google/devtools/build/lib/remote/disk/DiskCacheClient.java:455 DiskCacheClient.syncFile
fn sync_file(path: &Path) -> io::Result<()> {
    match std::fs::File::open(path.get_path_string()) {
        Ok(file) => file.sync_all(),
        Err(_) => Ok(()),
    }
}

/// `e instanceof FileNotFoundException` (the native layer raises it for ENOENT).
fn is_file_not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound
}

/// `path.getParentDirectory()` of a cache entry, which is never the root.
fn parent_directory(path: &Path) -> Path {
    path.get_parent_directory()
        .expect("NullPointerException: getParentDirectory()")
}

/// `message.getDigest()`: the default instance when the field is not set.
fn digest_or_default(digest: &Option<Digest>) -> Digest {
    digest.clone().unwrap_or_default()
}

/// The whole contents of `path` (`Tree.parseFrom(InputStream)` reads the stream to its end).
fn read_fully(path: &Path) -> io::Result<Vec<u8>> {
    let mut input = path.get_input_stream()?;
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// `InvalidProtocolBufferException` (an `IOException`).
fn invalid_protocol_buffer_exception(e: prost::DecodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}
