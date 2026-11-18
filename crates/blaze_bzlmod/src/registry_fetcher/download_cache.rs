//! The slice of `DownloadCache` that `DownloadManager.downloadAndReadOneUrlForBzlmod` uses:
//! `getBytes` and the two in-memory `put`s, with Bazel's on-disk layout
//! (`<repository_cache>/content_addressable/<hash name>/<key>/file`, written through a
//! `tmp-<uuid>` file and a rename), so the cache is shared with real Bazel.
//!
// PORT: move to blaze_repo_fetch — temporary stand-in for DownloadManager while blaze_repo_fetch
// cannot build next to starlark (download_cache.rs:56).

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use blaze_bzlmod_model::KeyType;
use blaze_bzlmod_model::checksum::hash_with;

use super::{IoException, java_io_message};

// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:92 DownloadCache.DEFAULT_CACHE_FILENAME
/// Rename cached files to this value to simplify lookup.
pub const DEFAULT_CACHE_FILENAME: &str = "file";
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:93 DownloadCache.TMP_PREFIX
pub const TMP_PREFIX: &str = "tmp-";

/// `KeyType.getCachePath(parentDirectory)`.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:67 KeyType.getCachePath
pub fn get_cache_path(key_type: KeyType, parent_directory: &Path) -> PathBuf {
    parent_directory.join(key_type.get_hash_name())
}

/// The cache implementation to store download artifacts from external repositories. Port of
/// `DownloadCache` (the in-memory `getBytes` / `put` slice).
///
/// Operations performed by this class are atomic on the file system level under the assumption
/// that the cache directory is not subject to concurrent file deletion.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:39 DownloadCache
#[derive(Clone, Debug, Default)]
pub struct LocalDownloadCache {
    path: Option<PathBuf>,
}

impl LocalDownloadCache {
    /// A cache rooted at `path` (the `content_addressable` directory); `None` disables it.
    pub fn new(path: Option<PathBuf>) -> Self {
        LocalDownloadCache { path }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:99 DownloadCache.setPath
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns true iff the cache path is set.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:108 DownloadCache.isEnabled
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    fn root(&self) -> &Path {
        blaze_util::check_state!(self.is_enabled());
        self.path.as_deref().unwrap()
    }

    /// Determine if a cache entry exist, given a cache key.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:119 DownloadCache.exists
    pub fn exists(&self, cache_key: &str, key_type: KeyType) -> bool {
        get_cache_path(key_type, self.root())
            .join(cache_key)
            .join(DEFAULT_CACHE_FILENAME)
            .exists()
    }

    /// Get the content of a cached value, if it exists. Returns `None` if it does not.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:172 DownloadCache.getBytes
    pub fn get_bytes(
        &self,
        cache_key: &str,
        key_type: KeyType,
    ) -> Result<Option<Vec<u8>>, IoException> {
        let Some(cache_value) = self.find_cache_value(cache_key, key_type)? else {
            return Ok(None);
        };
        std::fs::read(&cache_value)
            .map(Some)
            .map_err(|e| IoException::from_io(&cache_value, &e))
    }

    // PORT: the canonical-id parameter is always null on the getBytes path, so it is dropped.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:183 DownloadCache.findCacheValue
    fn find_cache_value(
        &self,
        cache_key: &str,
        key_type: KeyType,
    ) -> Result<Option<PathBuf>, IoException> {
        blaze_util::check_state!(self.is_enabled());

        assert_key_is_valid(cache_key, key_type)?;
        if !self.exists(cache_key, key_type) {
            return Ok(None);
        }

        let cache_entry = get_cache_path(key_type, self.root()).join(cache_key);
        let cache_value = cache_entry.join(DEFAULT_CACHE_FILENAME);

        if let Err(e) = assert_file_checksum(cache_key, &cache_value, key_type) {
            // New lines because this error message gets large printing multiple absolute
            // filepaths.
            return Err(IoException::new(format!(
                "{}\n\nPlease delete the directory {} and try again.",
                e.get_message(),
                cache_entry.display()
            )));
        }

        // Ignore errors, because the cache might be on a read-only volume.
        let _ = touch_file(&cache_value);

        Ok(Some(cache_value))
    }

    /// Writes `content` into the cache under `cache_key` (`storeCacheValue` with
    /// `FileSystemUtils.writeContent` as the writer and no canonical id).
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:235 DownloadCache.storeCacheValue
    fn store_cache_value(
        &self,
        cache_key: &str,
        content: &[u8],
        key_type: KeyType,
    ) -> Result<(), IoException> {
        blaze_util::check_state!(self.is_enabled());

        assert_key_is_valid(cache_key, key_type)?;
        self.ensure_cache_directory_exists(key_type)?;

        let cache_entry = get_cache_path(key_type, self.root()).join(cache_key);
        let cache_value = cache_entry.join(DEFAULT_CACHE_FILENAME);
        let tmp_name = cache_entry.join(format!("{TMP_PREFIX}{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cache_entry)
            .map_err(|e| IoException::from_io(&cache_entry, &e))?;
        std::fs::write(&tmp_name, content).map_err(|e| IoException::from_io(&tmp_name, &e))?;
        // Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:509 FileSystemUtils.renameToleratingConcurrentCreation
        // PORT: the tolerated failure is Windows-only; on Linux it is a plain rename.
        std::fs::rename(&tmp_name, &cache_value).map_err(|e| {
            IoException::new(format!(
                "{} -> {} ({})",
                tmp_name.display(),
                cache_value.display(),
                strerror(&e)
            ))
        })?;
        Ok(())
    }

    /// Adds an in-memory value to the cache.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:277 DownloadCache.put
    pub fn put_with_key(
        &self,
        cache_key: &str,
        content: &[u8],
        key_type: KeyType,
    ) -> Result<(), IoException> {
        self.store_cache_value(cache_key, content, key_type)
    }

    /// Adds an in-memory value to the cache, keyed by its own hash. Returns the key.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:291 DownloadCache.put
    pub fn put(&self, content: &[u8], key_type: KeyType) -> Result<String, IoException> {
        let cache_key = hex::encode(hash_with(key_type, content));
        self.put_with_key(&cache_key, content, key_type)?;
        Ok(cache_key)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:312 DownloadCache.ensureCacheDirectoryExists
    fn ensure_cache_directory_exists(&self, key_type: KeyType) -> Result<(), IoException> {
        let directory_path = get_cache_path(key_type, self.root());
        if !directory_path.exists() {
            std::fs::create_dir_all(&directory_path)
                .map_err(|e| IoException::from_io(&directory_path, &e))?;
        }
        Ok(())
    }
}

/// Assert that a file has an expected checksum.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:328 DownloadCache.assertFileChecksum
pub fn assert_file_checksum(
    expected_checksum: &str,
    file_path: &Path,
    key_type: KeyType,
) -> Result<(), IoException> {
    blaze_util::check_argument!(!expected_checksum.is_empty());

    let actual_checksum = match get_checksum(key_type, file_path) {
        Ok(c) => c,
        Err(e) => {
            return Err(IoException::new(format!(
                "Could not hash file {}: {}, expected {} of {}. ",
                file_path.display(),
                e.get_message(),
                key_type,
                expected_checksum
            )));
        }
    };
    if !actual_checksum.eq_ignore_ascii_case(expected_checksum) {
        return Err(IoException::new(format!(
            "Downloaded file at {} has {} of {}, does not match expected {} ({})",
            file_path.display(),
            key_type,
            actual_checksum,
            key_type,
            expected_checksum
        )));
    }
    Ok(())
}

/// Obtain the checksum of a file, as lowercase hex.
// PORT: Bazel's file systems have no fast digest on Linux (`getFastDigest` returns null for
// UnixFileSystem), so the file is always hashed.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:369 DownloadCache.getChecksum
pub fn get_checksum(key_type: KeyType, path: &Path) -> Result<String, IoException> {
    let content = std::fs::read(path).map_err(|e| IoException::from_io(path, &e))?;
    Ok(hex::encode(hash_with(key_type, &content)))
}

// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:398 DownloadCache.assertKeyIsValid
fn assert_key_is_valid(key: &str, key_type: KeyType) -> Result<(), IoException> {
    if !key_type.is_valid(Some(key)) {
        return Err(IoException::new(format!(
            "Invalid key \"{key}\" of type {key_type}. "
        )));
    }
    Ok(())
}

/// `FileSystemUtils.touchFile`: set the modification time to now, or create an empty file.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:269 FileSystemUtils.touchFile
fn touch_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        std::fs::File::open(path)?.set_modified(SystemTime::now())
    } else {
        std::fs::File::create(path).map(|_| ())
    }
}

fn strerror(e: &io::Error) -> String {
    java_io_message(e)
}
