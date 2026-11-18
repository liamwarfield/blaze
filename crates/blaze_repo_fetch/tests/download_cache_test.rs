//! Port of `DownloadCacheTest` (parameterized over SHA-256 and BLAKE3).
// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/cache/DownloadCacheTest.java

use std::sync::Arc;

use blaze_repo_fetch::cache::download_cache::{
    self, DEFAULT_CACHE_FILENAME, DownloadCache, KeyType,
};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: Path,
    download_cache: DownloadCache,
    repository_cache_path: Path,
    downloaded_file: Path,
    key_type: KeyType,
    hash: &'static str,
}

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn set_up(key_type: KeyType, hash: &'static str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs = fs();
    let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);
    let repository_cache_path = root.get_relative_str("repository_cache");
    repository_cache_path
        .create_directory_and_parents()
        .unwrap();
    let mut download_cache = DownloadCache::new();
    download_cache.set_path(Some(repository_cache_path.clone()));
    let downloaded_file = root.get_relative_str("file.tmp");
    // Scratch.file writes the lines followed by a newline.
    std::fs::write(downloaded_file.get_path_string(), "contents\n").unwrap();
    Fixture {
        _tmp: tmp,
        root,
        download_cache,
        repository_cache_path,
        downloaded_file,
        key_type,
        hash,
    }
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p.get_path_string()).unwrap()
}

fn cache_value(f: &Fixture) -> Path {
    f.key_type
        .get_cache_path(&f.repository_cache_path)
        .get_child(f.hash)
        .get_child(DEFAULT_CACHE_FILENAME)
}

fn external_target(f: &Fixture) -> Path {
    let target_directory = f.root.get_relative_str("external");
    target_directory.create_directory_and_parents().unwrap();
    target_directory.get_child(f.downloaded_file.get_base_name())
}

fn test_non_existent_cache_value(f: Fixture) {
    let fake_hash = "a".repeat(64);
    assert!(!f.download_cache.exists(&fake_hash, f.key_type));
}

fn test_put_cache_value(f: Fixture) {
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, None)
        .unwrap();
    assert_eq!(read(&f.downloaded_file), read(&cache_value(&f)));
}

fn test_put_cache_value_without_hash(f: Fixture) {
    let cache_key = f
        .download_cache
        .put_computing_key(&f.downloaded_file, f.key_type, None)
        .unwrap();
    assert_eq!(cache_key, f.hash);
    assert_eq!(read(&f.downloaded_file), read(&cache_value(&f)));
}

fn test_put_cache_value_idempotent(f: Fixture) {
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, None)
        .unwrap();
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, None)
        .unwrap();
    assert_eq!(read(&f.downloaded_file), read(&cache_value(&f)));
}

fn test_put_cache_value_concurrent(f: Fixture) {
    let f = Arc::new(f);
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let f = f.clone();
            std::thread::spawn(move || {
                f.download_cache
                    .put(f.hash, &f.downloaded_file, f.key_type, None)
            })
        })
        .collect();
    let errors: Vec<_> = handles
        .into_iter()
        .filter_map(|h| h.join().unwrap().err())
        .collect();
    assert!(
        errors.is_empty(),
        "Exceptions occurred during concurrent puts: {:?}",
        errors
    );
    assert_eq!(read(&f.downloaded_file), read(&cache_value(&f)));
}

fn test_get_cache_value(f: Fixture) {
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, None)
        .unwrap();
    let target_path = external_target(&f);
    let actual_target_path = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, None, true)
        .unwrap()
        .unwrap();
    assert_eq!(read(&f.downloaded_file), read(&actual_target_path));
    assert_eq!(actual_target_path, target_path);
}

fn test_get_null_cache_value(f: Fixture) {
    let target_path = external_target(&f);
    let actual = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, None, true)
        .unwrap();
    assert!(actual.is_none());
}

fn test_invalid_sha256_throws(f: Fixture) {
    let e = f
        .download_cache
        .put("foo", &f.downloaded_file, f.key_type, None)
        .unwrap_err();
    assert!(
        e.get_message()
            .contains(&format!("Invalid key \"foo\" of type {}", f.key_type))
    );
}

fn test_poisoned_cache(f: Fixture) {
    let poisoned_value = cache_value(&f);
    poisoned_value
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    std::fs::write(poisoned_value.get_path_string(), "poisoned\n").unwrap();
    let target_path = external_target(&f);
    let e = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, None, true)
        .unwrap_err();
    assert!(e.get_message().contains("does not match expected"));
    assert!(e.get_message().contains("Please delete the directory"));
}

fn test_get_checksum(f: Fixture) {
    assert_eq!(
        download_cache::get_checksum(f.key_type, &f.downloaded_file).unwrap(),
        f.hash
    );
}

fn test_assert_file_checksum_pass(f: Fixture) {
    download_cache::assert_file_checksum(f.hash, &f.downloaded_file, f.key_type).unwrap();
}

fn test_assert_file_checksum_fail(f: Fixture) {
    let e = download_cache::assert_file_checksum(&"a".repeat(64), &f.downloaded_file, f.key_type)
        .unwrap_err();
    assert!(e.get_message().contains("does not match expected"));
}

fn test_canonical_id(f: Fixture) {
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, Some("fooid"))
        .unwrap();
    let target_path = external_target(&f);
    let lookup_with_same_id = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, Some("fooid"), true)
        .unwrap();
    assert_eq!(lookup_with_same_id, Some(target_path.clone()));
    let lookup_other_id = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, Some("barid"), true)
        .unwrap();
    assert_eq!(lookup_other_id, None);
    let lookup_no_id = f
        .download_cache
        .get(f.hash, &target_path, f.key_type, None, true)
        .unwrap();
    assert_eq!(lookup_no_id, Some(target_path));
}

const SHA256_HASH: &str = "bfe5ed57e6e323555b379c660aa8d35b70c2f8f07cf03ad6747266495ac13be0";
const BLAKE3_HASH: &str = "54e00265e2516f168096da17059a6109563d9ba64a0b77cdc4b33e44600c2a39";

macro_rules! parameterized {
    ($($name:ident),* $(,)?) => {
        mod sha256 {
            use super::*;
            $( #[test] fn $name() { super::$name(set_up(KeyType::SHA256, SHA256_HASH)); } )*
        }
        mod blake3 {
            use super::*;
            $( #[test] fn $name() { super::$name(set_up(KeyType::BLAKE3, BLAKE3_HASH)); } )*
        }
    };
}

parameterized!(
    test_non_existent_cache_value,
    test_put_cache_value,
    test_put_cache_value_without_hash,
    test_put_cache_value_idempotent,
    test_put_cache_value_concurrent,
    test_get_cache_value,
    test_get_null_cache_value,
    test_invalid_sha256_throws,
    test_poisoned_cache,
    test_get_checksum,
    test_assert_file_checksum_pass,
    test_assert_file_checksum_fail,
    test_canonical_id,
);

#[test]
#[ignore = "needs a Mockito spy of Path.getFastDigest; Blaze's UnixFileSystem has no fast digest"]
fn test_get_checksum_with_fast_digest() {}

/// The cache entry Bazel writes for a canonical id: `id-<hash of the id with the key type>`.
#[test]
fn canonical_id_marker_matches_bazel_layout() {
    let f = set_up(KeyType::SHA256, SHA256_HASH);
    f.download_cache
        .put(f.hash, &f.downloaded_file, f.key_type, Some("fooid"))
        .unwrap();
    let id_hash = KeyType::SHA256.hash_bytes_hex(b"fooid");
    let marker = f
        .repository_cache_path
        .get_relative_str(&format!("sha256/{}/id-{}", f.hash, id_hash));
    assert!(marker.exists());
}
