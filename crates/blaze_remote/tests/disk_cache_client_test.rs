// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `lib/remote/disk/DiskCacheClientTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem` rooted at `/`; these tests use the real file system
//! (`UnixFileSystem`) under a fresh temporary directory, which stands for `/`.

use std::io::{self, Read, Write};
use std::sync::{Arc, Barrier, Mutex};

use blaze_protos::JavaMessage;
use blaze_remote::common::cache_not_found_exception::CacheNotFoundException;
use blaze_remote::common::errors;
use blaze_remote::common::lazy_file_output_stream::LazyFileOutputStream;
use blaze_remote::common::remote_cache_client::ActionKey;
use blaze_remote::disk::disk_cache_client::DiskCacheClient;
use blaze_remote::reapi::{
    ActionResult, Digest, Directory, FileNode, OutputDirectory, OutputFile, Tree,
};
use blaze_remote::runtime::get_from_future;
use blaze_remote::store::Store;
use blaze_remote::util::digest_util::DigestUtil;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{
    DigestHashFunction, Dirent, FileStatus, FileSystem, FileSystemExt, FileSystemRef, Path,
    PathFragment, UnixFileSystem,
};

// Java: DiskCacheClientTest.DIGEST_UTIL
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// The fields of the Java test class.
struct Fixture {
    _tmp: tempfile::TempDir,
    /// The directory standing for `/` of the Java test's file system.
    fs_root: Path,
    root: Path,
    client: DiskCacheClient,
}

impl Fixture {
    // Java: DiskCacheClientTest.setUp
    fn new() -> Fixture {
        Fixture::with_file_system(UnixFileSystem::create(DigestHashFunction::Sha256))
    }

    fn with_file_system(fs: FileSystemRef) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs_root = fs.get_path(tmp.path().to_str().unwrap());
        let root = fs_root.get_relative_str("disk_cache");
        let client = DiskCacheClient::new(
            root.clone(),
            &digest_util(),
            /* check_action_result_integrity= */ true,
        )
        .unwrap();
        Fixture {
            _tmp: tmp,
            fs_root,
            root,
            client,
        }
    }

    /// `fs.getPath("/" + name)`.
    fn get_path(&self, name: &str) -> Path {
        self.fs_root.get_relative_str(name)
    }

    // Java: DiskCacheClientTest.getCasPath
    fn get_cas_path(&self, digest: &Digest) -> Path {
        self.client.to_path(digest, Store::Cas)
    }

    // Java: DiskCacheClientTest.populateCas
    fn populate_cas(&self, digest: &Digest, contents: &[u8]) -> Path {
        populate_cas(&self.client, digest, contents)
    }

    // Java: DiskCacheClientTest.getAcPath
    fn get_ac_path(&self, action_key: &ActionKey) -> Path {
        self.client.to_path(action_key.digest(), Store::Ac)
    }

    // Java: DiskCacheClientTest.populateAc
    fn populate_ac(&self, action_key: &ActionKey, action_result: &ActionResult) -> Path {
        populate_ac(&self.client, action_key, action_result)
    }
}

impl Drop for Fixture {
    // Java: DiskCacheClientTest.tearDown
    fn drop(&mut self) {
        self.client.close();
    }
}

// Java: DiskCacheClientTest.populateCas
fn populate_cas(client: &DiskCacheClient, digest: &Digest, contents: &[u8]) -> Path {
    let path = client.to_path(digest, Store::Cas);
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    fs_utils::write_content(&path, contents).unwrap();
    path.set_last_modified_time(0).unwrap();
    path
}

// Java: DiskCacheClientTest.populateAc
fn populate_ac(
    client: &DiskCacheClient,
    action_key: &ActionKey,
    action_result: &ActionResult,
) -> Path {
    let path = client.to_path(action_key.digest(), Store::Ac);
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    fs_utils::write_content(&path, &action_result.to_byte_array()).unwrap();
    path.set_last_modified_time(0).unwrap();
    path
}

// Java: DiskCacheClientTest.getTreeWithFile
fn get_tree_with_file(file_digest: &Digest) -> Tree {
    Tree {
        children: vec![Directory {
            files: vec![FileNode {
                digest: Some(file_digest.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

// Java: DiskCacheClientTest.getDigest(String)
fn get_digest(contents: &str) -> Digest {
    digest_util().compute_as_utf8(contents)
}

// Java: DiskCacheClientTest.getDigest(Message)
fn get_digest_of_message<M: JavaMessage>(m: &M) -> Digest {
    digest_util().compute(&m.to_byte_array())
}

fn read_content_utf8(path: &Path) -> String {
    String::from_utf8(fs_utils::read_content(path).unwrap()).unwrap()
}

fn is_cache_not_found_exception(e: &io::Error) -> bool {
    errors::downcast_ref::<CacheNotFoundException>(e).is_some()
}

/// A filesystem that deletes a designated path as soon as its mtime is updated, simulating a
/// concurrent garbage collection that removes a cache entry in the window between
/// `DiskCacheClient.refresh` and the read that follows it.
// Java: DiskCacheClientTest.DeleteOnMtimeUpdateFileSystem
struct DeleteOnMtimeUpdateFileSystem {
    inner: UnixFileSystem,
    path_to_delete: Mutex<Option<PathFragment>>,
}

impl std::fmt::Debug for DeleteOnMtimeUpdateFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeleteOnMtimeUpdateFileSystem")
    }
}

impl DeleteOnMtimeUpdateFileSystem {
    fn new() -> DeleteOnMtimeUpdateFileSystem {
        DeleteOnMtimeUpdateFileSystem {
            inner: UnixFileSystem::new(DigestHashFunction::Sha256),
            path_to_delete: Mutex::new(None),
        }
    }

    fn delete_on_next_mtime_update(&self, path: &Path) {
        *self.path_to_delete.lock().unwrap() = Some(path.as_fragment().clone());
    }
}

impl FileSystem for DeleteOnMtimeUpdateFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.inner.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.inner.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.inner.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.inner.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.inner.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.inner.create_directory_and_parents(path)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.inner.get_file_size(path, follow_symlinks)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.delete(path)
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.inner.get_last_modified_time(path, follow_symlinks)
    }
    // Java: DiskCacheClientTest.DeleteOnMtimeUpdateFileSystem.setLastModifiedTime
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        let mut path_to_delete = self.path_to_delete.lock().unwrap();
        self.inner.set_last_modified_time(path, new_time)?;
        if path_to_delete.as_ref() == Some(path) {
            *path_to_delete = None;
            let _ = self.inner.delete(path)?;
        }
        Ok(())
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        self.inner.resolve_one_link(path)
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.inner.stat(path, follow_symlinks)
    }
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn blaze_vfs::StatMetadata>>> {
        self.inner.stat_metadata(path, follow_symlinks)
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        self.inner.stat_if_found(path, follow_symlinks)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.inner.create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.inner.read_symbolic_link(path)
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.inner.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.inner.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.inner.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.inner.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.inner.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.inner.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.inner.set_executable(path, executable)
    }
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.inner.chmod(path, mode)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.inner.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.inner.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        self.inner.rename_to(source_path, target_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.inner
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

/// `new DeleteOnMtimeUpdateFileSystem()` and the client and paths the race tests build on it.
struct RaceFixture {
    race_fs: Arc<DeleteOnMtimeUpdateFileSystem>,
    fixture: Fixture,
}

impl RaceFixture {
    fn new() -> RaceFixture {
        let race_fs = Arc::new(DeleteOnMtimeUpdateFileSystem::new());
        let fs: FileSystemRef = race_fs.clone();
        RaceFixture {
            race_fs,
            fixture: Fixture::with_file_system(fs),
        }
    }
}

#[test]
fn find_missing_digests_returns_all_digests() {
    let f = Fixture::new();
    let digests = vec![get_digest("foo"), get_digest("bar"), get_digest("baz")];
    let missing = get_from_future(f.client.find_missing_digests(&digests)).unwrap();
    assert_eq!(missing.len(), digests.len());
    for digest in &digests {
        assert!(missing.contains(digest));
    }
}

#[test]
fn to_path_for_cas_for_old_style_hash_function() {
    let f = Fixture::new();
    let digest = Digest {
        hash: "0123456789abcdef".to_string(),
        size_bytes: 42,
    };

    let path = f.client.to_path(&digest, Store::Cas);

    assert_eq!(path, f.root.get_relative_str("cas/01/0123456789abcdef"));
}

#[test]
fn to_path_for_ac_for_old_style_hash_function() {
    let f = Fixture::new();
    let digest = Digest {
        hash: "0123456789abcdef".to_string(),
        size_bytes: 42,
    };

    let path = f.client.to_path(&digest, Store::Ac);

    assert_eq!(path, f.root.get_relative_str("ac/01/0123456789abcdef"));
}

#[test]
fn to_path_for_cas_for_new_style_hash_function() {
    let f = Fixture::new();
    let client = DiskCacheClient::new(
        f.root.clone(),
        &DigestUtil::new(DigestHashFunction::Blake3),
        /* check_action_result_integrity= */ true,
    )
    .unwrap();
    let digest = Digest {
        hash: "0123456789abcdef".to_string(),
        size_bytes: 42,
    };

    let path = client.to_path(&digest, Store::Cas);

    assert_eq!(
        path,
        f.root.get_relative_str("blake3/cas/01/0123456789abcdef")
    );
}

#[test]
fn to_path_for_ac_for_new_style_hash_function() {
    let f = Fixture::new();
    let client = DiskCacheClient::new(
        f.root.clone(),
        &DigestUtil::new(DigestHashFunction::Blake3),
        /* check_action_result_integrity= */ true,
    )
    .unwrap();
    let digest = Digest {
        hash: "0123456789abcdef".to_string(),
        size_bytes: 42,
    };

    let path = client.to_path(&digest, Store::Ac);

    assert_eq!(
        path,
        f.root.get_relative_str("blake3/ac/01/0123456789abcdef")
    );
}

#[test]
fn upload_file_when_missing_populates_cas() {
    let f = Fixture::new();
    assert!(f.root.exists());
    let file = f.get_path("file");
    fs_utils::write_content(&file, b"contents").unwrap();
    let digest = get_digest("contents");

    get_from_future(f.client.upload_file(&digest, &file)).unwrap();

    assert_eq!(read_content_utf8(&f.get_cas_path(&digest)), "contents");
}

#[test]
fn upload_file_when_present_updates_mtime() {
    let f = Fixture::new();
    let file = f.get_path("file");
    fs_utils::write_content(&file, b"contents").unwrap();
    let digest = get_digest("contents");

    // The contents would match under normal operation. This serves to check that we don't
    // unnecessarily overwrite the file.
    let path = f.populate_cas(&digest, b"existing contents");

    get_from_future(f.client.upload_file(&digest, &file)).unwrap();

    assert_eq!(read_content_utf8(&path), "existing contents");
    assert_ne!(path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn upload_file_when_missing_does_not_inherit_source_mtime_or_permissions() {
    let f = Fixture::new();
    let file = f.get_path("file");
    fs_utils::write_content(&file, b"contents").unwrap();
    // A build output is read-only, and may have been written long before it is uploaded. Neither
    // property may leak into the cache entry: the mtime records when the entry was last stored
    // or retrieved, and the entry must remain readable by every user of a shared cache.
    file.chmod(0o555).unwrap();
    file.set_last_modified_time(1000).unwrap();
    let digest = get_digest("contents");

    get_from_future(f.client.upload_file(&digest, &file)).unwrap();

    let path = f.get_cas_path(&digest);
    assert_eq!(read_content_utf8(&path), "contents");
    assert_ne!(path.get_last_modified_time().unwrap(), 1000);
    assert!(path.is_readable().unwrap());
    assert!(path.is_writable().unwrap());
}

#[test]
fn upload_blob_when_missing_populates_cas() {
    let f = Fixture::new();
    let blob = bytes::Bytes::from_static(b"contents");
    let digest = get_digest("contents");

    get_from_future(f.client.upload_bytes(&digest, blob)).unwrap();

    assert_eq!(read_content_utf8(&f.get_cas_path(&digest)), "contents");
}

#[test]
fn upload_blob_when_present_updates_mtime() {
    let f = Fixture::new();
    let blob = bytes::Bytes::from_static(b"contents");
    let digest = get_digest("contents");

    // The contents would match under normal operation. This serves to check that we don't
    // unnecessarily overwrite the file.
    let path = f.populate_cas(&digest, b"existing contents");

    get_from_future(f.client.upload_bytes(&digest, blob)).unwrap();

    assert_eq!(read_content_utf8(&path), "existing contents");
    assert_ne!(path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn upload_action_result_when_missing_populates_ac() {
    let f = Fixture::new();
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        exit_code: 42,
        ..Default::default()
    };
    let path = f.get_ac_path(&action_key);

    get_from_future(f.client.upload_action_result(&action_key, &action_result)).unwrap();

    assert_eq!(
        fs_utils::read_content(&path).unwrap(),
        action_result.to_byte_array()
    );
}

#[test]
fn upload_action_result_when_present_updates_content() {
    let f = Fixture::new();
    let action_key = ActionKey::new(get_digest("key"));
    let action_result1 = ActionResult {
        exit_code: 42,
        ..Default::default()
    };

    let path = f.populate_ac(&action_key, &action_result1);

    let action_result2 = ActionResult {
        exit_code: 43,
        ..Default::default()
    };
    get_from_future(f.client.upload_action_result(&action_key, &action_result2)).unwrap();

    assert_eq!(
        fs_utils::read_content(&path).unwrap(),
        action_result2.to_byte_array()
    );
    assert_ne!(path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn download_blob_when_present_returns_contents() {
    let f = Fixture::new();
    let digest = get_digest("contents");
    f.populate_cas(&digest, b"contents");
    let out = f.get_path("out");

    {
        let mut out_stream = out.get_output_stream().unwrap();
        get_from_future(f.client.download_blob(&digest, &mut *out_stream)).unwrap();
    }

    assert_eq!(read_content_utf8(&out), "contents");
}

#[test]
fn download_blob_when_missing_throws_cache_not_found_exception() {
    let f = Fixture::new();
    let out = f.get_path("out");

    let mut out_stream = out.get_output_stream().unwrap();
    let e = get_from_future(
        f.client
            .download_blob(&get_digest("contents"), &mut *out_stream),
    )
    .unwrap_err();
    assert!(is_cache_not_found_exception(&e), "{e:?}");
}

#[test]
fn download_blob_when_deleted_after_refresh_throws_cache_not_found_exception() {
    let race = RaceFixture::new();
    let race_client = &race.fixture.client;
    let digest = get_digest("contents");
    let cas_path = populate_cas(race_client, &digest, b"contents");
    race.race_fs.delete_on_next_mtime_update(&cas_path);

    let mut out: Vec<u8> = Vec::new();
    let e = get_from_future(race_client.download_blob(&digest, &mut out)).unwrap_err();
    assert!(is_cache_not_found_exception(&e), "{e:?}");
}

#[test]
fn download_blob_to_path_backed_stream_when_deleted_after_refresh_throws_cache_not_found_exception()
{
    let race = RaceFixture::new();
    let race_client = &race.fixture.client;
    let digest = get_digest("contents");
    let cas_path = populate_cas(race_client, &digest, b"contents");
    race.race_fs.delete_on_next_mtime_update(&cas_path);
    let mut out = LazyFileOutputStream::new(race.fixture.get_path("out.tmp"));
    // `out instanceof MaybePathBacked ? out.maybeGetPath() : null`
    let out_path = out.maybe_get_path().cloned();

    let e = get_from_future(race_client.download_blob_maybe_path_backed(
        &digest,
        &mut out,
        out_path.as_ref(),
    ))
    .unwrap_err();
    assert!(is_cache_not_found_exception(&e), "{e:?}");
}

#[test]
fn download_blob_when_destination_is_unwritable_does_not_report_cache_miss() {
    let f = Fixture::new();
    let digest = get_digest("contents");
    f.populate_cas(&digest, b"contents");
    // The parent directory of the destination doesn't exist. This is a genuine local filesystem
    // error, not a cache miss, so it must not be masked as one.
    let mut out = LazyFileOutputStream::new(f.get_path("does_not_exist/out.tmp"));
    // `out instanceof MaybePathBacked ? out.maybeGetPath() : null`
    let out_path = out.maybe_get_path().cloned();

    let e = get_from_future(f.client.download_blob_maybe_path_backed(
        &digest,
        &mut out,
        out_path.as_ref(),
    ))
    .unwrap_err();

    assert!(!is_cache_not_found_exception(&e), "{e:?}");
}

#[test]
fn download_action_result_when_present_returns_cached_action_result() {
    let f = Fixture::new();
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        exit_code: 42,
        ..Default::default()
    };

    let path = f.populate_ac(&action_key, &action_result);

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, Some(action_result));
    assert_ne!(path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn download_action_result_when_missing_returns_null() {
    let f = Fixture::new();
    let action_key = ActionKey::new(get_digest("key"));

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, None);
}

#[test]
fn download_action_result_when_tree_deleted_after_refresh_returns_null() {
    let race = RaceFixture::new();
    let race_client = &race.fixture.client;
    let tree_file_digest = get_digest("tree file contents");
    let tree = get_tree_with_file(&tree_file_digest);
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        output_directories: vec![OutputDirectory {
            tree_digest: Some(tree_digest.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    populate_ac(race_client, &action_key, &action_result);
    populate_cas(race_client, &tree_file_digest, b"tree file contents");
    let tree_cas_path = populate_cas(race_client, &tree_digest, &tree.to_byte_array());
    race.race_fs.delete_on_next_mtime_update(&tree_cas_path);

    // The action result must be reported as stale rather than failing the lookup outright.
    assert_eq!(
        get_from_future(race_client.download_action_result(&action_key)).unwrap(),
        None
    );
}

#[test]
fn download_action_result_with_referenced_blobs_present_updates_mtime_on_blobs() {
    let f = Fixture::new();
    let stdout_digest = get_digest("stdout contents");
    let stderr_digest = get_digest("stderr contents");
    let file_digest = get_digest("file contents");
    let tree_file_digest = get_digest("tree file contents");
    let tree = get_tree_with_file(&tree_file_digest);
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        stdout_digest: Some(stdout_digest.clone()),
        stderr_digest: Some(stderr_digest.clone()),
        output_files: vec![OutputFile {
            digest: Some(file_digest.clone()),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            tree_digest: Some(get_digest_of_message(&tree)),
            ..Default::default()
        }],
        ..Default::default()
    };

    let ac_path = f.populate_ac(&action_key, &action_result);
    let stdout_cas_path = f.populate_cas(&stdout_digest, b"stdout contents");
    let stderr_cas_path = f.populate_cas(&stderr_digest, b"stderr contents");
    let file_cas_path = f.populate_cas(&file_digest, b"file contents");
    let tree_cas_path = f.populate_cas(&tree_digest, &tree.to_byte_array());
    let tree_file_cas_path = f.populate_cas(&tree_file_digest, b"tree file contents");

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, Some(action_result));
    assert_ne!(ac_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(stdout_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(stderr_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(file_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(tree_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(tree_file_cas_path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn download_action_result_with_referenced_file_missing_returns_null() {
    let f = Fixture::new();
    let file_digest = get_digest("contents");
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        output_files: vec![OutputFile {
            digest: Some(file_digest),
            ..Default::default()
        }],
        ..Default::default()
    };

    f.populate_ac(&action_key, &action_result);

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, None);
}

#[test]
fn download_action_result_with_referenced_tree_missing_returns_null() {
    let f = Fixture::new();
    let file_digest = get_digest("contents");
    let tree = get_tree_with_file(&file_digest);
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        output_directories: vec![OutputDirectory {
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };

    f.populate_ac(&action_key, &action_result);
    f.populate_cas(&file_digest, b"contents");

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, None);
}

#[test]
fn download_action_result_with_referenced_tree_file_missing_returns_null() {
    let f = Fixture::new();
    let file_digest = get_digest("contents");
    let tree = get_tree_with_file(&file_digest);
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        output_directories: vec![OutputDirectory {
            tree_digest: Some(tree_digest.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };

    f.populate_ac(&action_key, &action_result);
    f.populate_cas(&tree_digest, &tree.to_byte_array());

    let result = get_from_future(f.client.download_action_result(&action_key)).unwrap();

    assert_eq!(result, None);
}

#[test]
fn download_action_result_without_integrity_check_with_referenced_file_missing_returns_result() {
    let f = Fixture::new();
    let client_without_integrity_check = DiskCacheClient::new(
        f.root.clone(),
        &digest_util(),
        /* check_action_result_integrity= */ false,
    )
    .unwrap();
    let stdout_digest = get_digest("stdout contents");
    let stderr_digest = get_digest("stderr contents");
    let missing_file_digest = get_digest("missing file contents");
    let tree_file_digest = get_digest("tree file contents");
    let tree = get_tree_with_file(&tree_file_digest);
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        stdout_digest: Some(stdout_digest.clone()),
        stderr_digest: Some(stderr_digest.clone()),
        output_files: vec![OutputFile {
            digest: Some(missing_file_digest.clone()),
            ..Default::default()
        }],
        output_directories: vec![OutputDirectory {
            tree_digest: Some(tree_digest.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let ac_path = f.populate_ac(&action_key, &action_result);
    let stdout_cas_path = f.populate_cas(&stdout_digest, b"stdout contents");
    let stderr_cas_path = f.populate_cas(&stderr_digest, b"stderr contents");
    let tree_cas_path = f.populate_cas(&tree_digest, &tree.to_byte_array());
    let tree_file_cas_path = f.populate_cas(&tree_file_digest, b"tree file contents");

    let result =
        get_from_future(client_without_integrity_check.download_action_result(&action_key))
            .unwrap();

    assert_eq!(result, Some(action_result));
    assert!(!f.get_cas_path(&missing_file_digest).exists());
    // The blobs that do exist are marked as recently used, including the ones that follow the
    // missing blob.
    assert_ne!(ac_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(stdout_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(stderr_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(tree_cas_path.get_last_modified_time().unwrap(), 0);
    assert_ne!(tree_file_cas_path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn download_action_result_without_integrity_check_with_referenced_tree_missing_returns_result() {
    let f = Fixture::new();
    let client_without_integrity_check = DiskCacheClient::new(
        f.root.clone(),
        &digest_util(),
        /* check_action_result_integrity= */ false,
    )
    .unwrap();
    let stdout_digest = get_digest("stdout contents");
    let tree = get_tree_with_file(&get_digest("tree file contents"));
    let tree_digest = get_digest_of_message(&tree);
    let action_key = ActionKey::new(get_digest("key"));
    let action_result = ActionResult {
        stdout_digest: Some(stdout_digest.clone()),
        output_directories: vec![OutputDirectory {
            tree_digest: Some(tree_digest),
            ..Default::default()
        }],
        ..Default::default()
    };

    f.populate_ac(&action_key, &action_result);
    let stdout_cas_path = f.populate_cas(&stdout_digest, b"stdout contents");

    let result =
        get_from_future(client_without_integrity_check.download_action_result(&action_key))
            .unwrap();

    assert_eq!(result, Some(action_result));
    assert_ne!(stdout_cas_path.get_last_modified_time().unwrap(), 0);
}

#[test]
fn download_action_result_without_integrity_check_when_missing_returns_null() {
    let f = Fixture::new();
    let client_without_integrity_check = DiskCacheClient::new(
        f.root.clone(),
        &digest_util(),
        /* check_action_result_integrity= */ false,
    )
    .unwrap();
    let action_key = ActionKey::new(get_digest("key"));

    let result =
        get_from_future(client_without_integrity_check.download_action_result(&action_key))
            .unwrap();

    assert_eq!(result, None);
}

/// A slow output stream that pauses periodically to keep the file open longer during download.
struct SlowOutputStream {
    bytes_written: usize,
}

impl SlowOutputStream {
    fn maybe_sleep(&self) {
        // Sleep every 64KB to slow down the download.
        if self.bytes_written % (64 * 1024) < 100 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

impl Write for SlowOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes_written += buf.len();
        self.maybe_sleep();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn concurrent_upload_download() {
    // PORT: every test of this file already runs on the native file system.
    let f = Fixture::new();
    let native_client = &f.client;
    // Use 1 MB blobs to increase the window for concurrent access during write/rename.
    let content_size = 1024 * 1024;
    let num_concurrent_ops = 10;
    for attempt in 0..100usize {
        let mut content_array = vec![0u8; content_size];
        // Fill with a pattern based on the attempt number.
        for (i, b) in content_array.iter_mut().enumerate() {
            *b = (attempt + i) as u8;
        }
        let content_digest = digest_util().compute(&content_array);
        let content_bytes = bytes::Bytes::from(content_array);
        // Use a barrier to ensure all concurrent tasks start at roughly the same time.
        let start_latch = Barrier::new(num_concurrent_ops);
        // Half the tasks do uploads, half do downloads with a slow OutputStream to keep the file
        // open longer. This maximizes the chance of a rename failing because a download has the
        // file open.
        std::thread::scope(|scope| {
            let mut tasks = Vec::new();
            for concurrent_op in 0..num_concurrent_ops {
                let is_uploader = concurrent_op % 2 == 0;
                let content_digest = &content_digest;
                let content_bytes = content_bytes.clone();
                let start_latch = &start_latch;
                tasks.push(scope.spawn(move || {
                    // Signal ready and wait for all tasks to be ready.
                    start_latch.wait();
                    if is_uploader {
                        get_from_future(native_client.upload_bytes(content_digest, content_bytes))
                            .unwrap();
                    } else {
                        // Use a slow OutputStream that pauses periodically to keep the file open
                        // longer during download.
                        let mut out = SlowOutputStream { bytes_written: 0 };
                        match get_from_future(native_client.download_blob(content_digest, &mut out))
                        {
                            Ok(()) => {}
                            // File not yet uploaded by another task.
                            Err(e) if is_cache_not_found_exception(&e) => {}
                            Err(e) => panic!("{e:?}"),
                        }
                    }
                }));
            }
            for task in tasks {
                task.join().unwrap();
            }
        });
    }
}
