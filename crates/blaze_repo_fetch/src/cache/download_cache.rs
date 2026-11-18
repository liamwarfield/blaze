//! Port of `DownloadCache`: the content-addressable store of downloaded files.
//!
//! On-disk layout (shared with Bazel; both directions must work):
//! `<repository_cache>/content_addressable/<hash name>/<lowercase hex>/file`, plus an empty marker
//! `id-<hex hash of canonical id>` per canonical id, and `tmp-<uuid>` files while writing.

use std::io::Read;

use blaze_util::{check_argument, check_state};
use blaze_vfs::{DigestHashFunction, Path};
use sha1::Digest as _;

use crate::errors::IoException;
use crate::fs_utils;

/// The types of cache keys used.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:43 DownloadCache.KeyType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyType {
    SHA1,
    SHA256,
    SHA384,
    SHA512,
    BLAKE3,
}

/// An incremental hasher for one [`KeyType`] (Guava `Hasher`).
pub enum KeyHasher {
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl KeyHasher {
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        match self {
            KeyHasher::Sha1(h) => h.update(bytes),
            KeyHasher::Sha256(h) => h.update(bytes),
            KeyHasher::Sha384(h) => h.update(bytes),
            KeyHasher::Sha512(h) => h.update(bytes),
            KeyHasher::Blake3(h) => {
                h.update(bytes);
            }
        }
    }

    /// `Hasher.hash().asBytes()`.
    pub fn hash(self) -> Vec<u8> {
        match self {
            KeyHasher::Sha1(h) => h.finalize().to_vec(),
            KeyHasher::Sha256(h) => h.finalize().to_vec(),
            KeyHasher::Sha384(h) => h.finalize().to_vec(),
            KeyHasher::Sha512(h) => h.finalize().to_vec(),
            KeyHasher::Blake3(h) => h.finalize().as_bytes().to_vec(),
        }
    }
}

impl KeyType {
    pub const ALL: [KeyType; 5] = [
        KeyType::SHA1,
        KeyType::SHA256,
        KeyType::SHA384,
        KeyType::SHA512,
        KeyType::BLAKE3,
    ];

    fn hex_len(self) -> usize {
        match self {
            KeyType::SHA1 => 40,
            KeyType::SHA256 | KeyType::BLAKE3 => 64,
            KeyType::SHA384 => 96,
            KeyType::SHA512 => 128,
        }
    }

    /// `KeyType.isValid`: non-empty and matching `\p{XDigit}{n}`.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:66 KeyType.isValid
    pub fn is_valid(self, checksum: Option<&str>) -> bool {
        match checksum {
            None => false,
            Some(c) => {
                !c.is_empty()
                    && c.len() == self.hex_len()
                    && c.chars().all(|ch| ch.is_ascii_hexdigit())
            }
        }
    }

    /// The directory of this key type below the CAS root.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:70 KeyType.getCachePath
    pub fn get_cache_path(self, parent_directory: &Path) -> Path {
        parent_directory.get_child(self.get_hash_name())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:74 KeyType.newHasher
    pub fn new_hasher(self) -> KeyHasher {
        match self {
            KeyType::SHA1 => KeyHasher::Sha1(sha1::Sha1::new()),
            KeyType::SHA256 => KeyHasher::Sha256(sha2::Sha256::new()),
            KeyType::SHA384 => KeyHasher::Sha384(sha2::Sha384::new()),
            KeyType::SHA512 => KeyHasher::Sha512(sha2::Sha512::new()),
            KeyType::BLAKE3 => KeyHasher::Blake3(Box::new(blake3::Hasher::new())),
        }
    }

    /// Hex digest of `bytes` (Guava `HashFunction.hashBytes(...).toString()`).
    pub fn hash_bytes_hex(self, bytes: &[u8]) -> String {
        let mut h = self.new_hasher();
        h.put_bytes(bytes);
        hex::encode(h.hash())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:82 KeyType.getHashName
    pub fn get_hash_name(self) -> &'static str {
        match self {
            KeyType::SHA1 => "sha1",
            KeyType::SHA256 => "sha256",
            KeyType::SHA384 => "sha384",
            KeyType::SHA512 => "sha512",
            KeyType::BLAKE3 => "blake3",
        }
    }

    /// `KeyType.toString()`: `SHA-256` etc.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:87 KeyType.toString
    pub fn string_repr(self) -> &'static str {
        match self {
            KeyType::SHA1 => "SHA-1",
            KeyType::SHA256 => "SHA-256",
            KeyType::SHA384 => "SHA-384",
            KeyType::SHA512 => "SHA-512",
            KeyType::BLAKE3 => "BLAKE3",
        }
    }

    fn matches_file_system_digest(self, f: DigestHashFunction) -> bool {
        match f {
            DigestHashFunction::Sha256 => self == KeyType::SHA256,
        }
    }
}

impl std::fmt::Display for KeyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.string_repr())
    }
}

const BUFFER_SIZE: usize = 32 * 1024;

/// Rename cached files to this value to simplify lookup.
pub const DEFAULT_CACHE_FILENAME: &str = "file";
pub const TMP_PREFIX: &str = "tmp-";
pub const ID_PREFIX: &str = "id-";

/// The cache implementation to store download artifacts from external repositories.
///
/// Operations performed by this class are atomic on the file system level under the assumption
/// that the cache directory is not subject to concurrent file deletion.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:41 DownloadCache
#[derive(Clone, Debug, Default)]
pub struct DownloadCache {
    path: Option<Path>,
    use_hardlinks: bool,
}

impl DownloadCache {
    pub fn new() -> DownloadCache {
        DownloadCache::default()
    }

    pub fn set_path(&mut self, path: Option<Path>) {
        self.path = path;
    }

    pub fn get_path(&self) -> Option<&Path> {
        self.path.as_ref()
    }

    pub fn set_hardlink(&mut self, use_hardlinks: bool) {
        self.use_hardlinks = use_hardlinks;
    }

    /// Returns true iff the cache path is set.
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    fn cache_root(&self) -> &Path {
        check_state!(self.is_enabled());
        self.path.as_ref().unwrap()
    }

    /// Determine if a cache entry exists, given a cache key.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:122 DownloadCache.exists
    pub fn exists(&self, cache_key: &str, key_type: KeyType) -> bool {
        key_type
            .get_cache_path(self.cache_root())
            .get_child(cache_key)
            .get_child(DEFAULT_CACHE_FILENAME)
            .exists()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:127 DownloadCache.hasCanonicalId
    fn has_canonical_id(&self, cache_key: &str, key_type: KeyType, canonical_id: &str) -> bool {
        let id_hash = key_type.hash_bytes_hex(canonical_id.as_bytes());
        key_type
            .get_cache_path(self.cache_root())
            .get_child(cache_key)
            .get_child(&format!("{}{}", ID_PREFIX, id_hash))
            .exists()
    }

    /// Copies or hardlinks the cached value to `target_path`, if it exists. Returns `None` on a
    /// cache miss. A non-empty `canonical_id` restricts hits to entries added with that id.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:150 DownloadCache.get
    pub fn get(
        &self,
        cache_key: &str,
        target_path: &Path,
        key_type: KeyType,
        canonical_id: Option<&str>,
        may_hardlink: bool,
    ) -> Result<Option<Path>, IoException> {
        let Some(cache_value) = self.find_cache_value(cache_key, key_type, canonical_id)? else {
            return Ok(None);
        };
        if let Some(parent) = target_path.get_parent_directory() {
            fs_utils::io(parent.create_directory_and_parents())?;
        }
        if self.use_hardlinks && may_hardlink {
            fs_utils::io(fs_utils::create_hard_link(target_path, &cache_value))?;
        } else {
            fs_utils::io(fs_utils::copy_file(&cache_value, target_path))?;
        }
        Ok(Some(target_path.clone()))
    }

    /// The content of a cached value, if it exists.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:174 DownloadCache.getBytes
    pub fn get_bytes(
        &self,
        cache_key: &str,
        key_type: KeyType,
    ) -> Result<Option<Vec<u8>>, IoException> {
        let Some(cache_value) = self.find_cache_value(cache_key, key_type, None)? else {
            return Ok(None);
        };
        Ok(Some(fs_utils::io(fs_utils::read_content(&cache_value))?))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:185 DownloadCache.findCacheValue
    fn find_cache_value(
        &self,
        cache_key: &str,
        key_type: KeyType,
        canonical_id: Option<&str>,
    ) -> Result<Option<Path>, IoException> {
        let root = self.cache_root();
        assert_key_is_valid(cache_key, key_type)?;
        if !self.exists(cache_key, key_type) {
            return Ok(None);
        }
        let cache_entry = key_type.get_cache_path(root).get_relative_str(cache_key);
        let cache_value = cache_entry.get_relative_str(DEFAULT_CACHE_FILENAME);
        if let Err(e) = assert_file_checksum(cache_key, &cache_value, key_type) {
            // New lines because this error message gets large printing multiple absolute
            // filepaths.
            return Err(IoException::new(format!(
                "{}\n\nPlease delete the directory {} and try again.",
                e.get_message(),
                cache_entry
            )));
        }
        if let Some(id) = canonical_id
            && !id.is_empty()
            && !self.has_canonical_id(cache_key, key_type, id)
        {
            return Ok(None);
        }
        // Ignore errors, because the cache might be on a read-only volume.
        let _ = fs_utils::touch_file(&cache_value);
        Ok(Some(cache_value))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:236 DownloadCache.storeCacheValue
    fn store_cache_value(
        &self,
        cache_key: &str,
        file_writer: &dyn Fn(&Path) -> std::io::Result<()>,
        key_type: KeyType,
        canonical_id: Option<&str>,
    ) -> Result<(), IoException> {
        let root = self.cache_root();
        assert_key_is_valid(cache_key, key_type)?;
        self.ensure_cache_directory_exists(key_type)?;
        let cache_entry = key_type.get_cache_path(root).get_relative_str(cache_key);
        let cache_value = cache_entry.get_relative_str(DEFAULT_CACHE_FILENAME);
        let tmp_name =
            cache_entry.get_relative_str(&format!("{}{}", TMP_PREFIX, uuid::Uuid::new_v4()));
        fs_utils::io(cache_entry.create_directory_and_parents())?;
        fs_utils::io(file_writer(&tmp_name))?;
        fs_utils::io(fs_utils::rename_tolerating_concurrent_creation(
            &tmp_name,
            &cache_value,
        ))?;
        if let Some(id) = canonical_id
            && !id.is_empty()
        {
            let id_hash = key_type.hash_bytes_hex(id.as_bytes());
            fs_utils::io(fs_utils::touch_file(
                &cache_entry.get_relative_str(&format!("{}{}", ID_PREFIX, id_hash)),
            ))?;
        }
        Ok(())
    }

    /// Copies a value from `source_path` into the cache under `cache_key`.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:265 DownloadCache.put
    pub fn put(
        &self,
        cache_key: &str,
        source_path: &Path,
        key_type: KeyType,
        canonical_id: Option<&str>,
    ) -> Result<(), IoException> {
        self.store_cache_value(
            cache_key,
            &|tmp_name| fs_utils::copy_file(source_path, tmp_name),
            key_type,
            canonical_id,
        )
    }

    /// Adds an in-memory value to the cache under `cache_key`.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:277 DownloadCache.put
    pub fn put_bytes_with_key(
        &self,
        cache_key: &str,
        content: &[u8],
        key_type: KeyType,
    ) -> Result<(), IoException> {
        self.store_cache_value(
            cache_key,
            &|tmp_name| fs_utils::write_content(tmp_name, content),
            key_type,
            None,
        )
    }

    /// Adds an in-memory value to the cache, computing its key.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:291 DownloadCache.put
    pub fn put_bytes(&self, content: &[u8], key_type: KeyType) -> Result<(), IoException> {
        let cache_key = key_type.hash_bytes_hex(content);
        self.put_bytes_with_key(&cache_key, content, key_type)
    }

    /// Copies a value from `source_path` into the cache, computing and returning the key.
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:306 DownloadCache.put
    pub fn put_computing_key(
        &self,
        source_path: &Path,
        key_type: KeyType,
        canonical_id: Option<&str>,
    ) -> Result<String, IoException> {
        let cache_key = get_checksum(key_type, source_path)?;
        self.put(&cache_key, source_path, key_type, canonical_id)?;
        Ok(cache_key)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:313 DownloadCache.ensureCacheDirectoryExists
    fn ensure_cache_directory_exists(&self, key_type: KeyType) -> Result<(), IoException> {
        let directory_path = key_type.get_cache_path(self.cache_root());
        if !directory_path.exists() {
            fs_utils::io(directory_path.create_directory_and_parents())?;
        }
        Ok(())
    }
}

/// Asserts that a file has an expected checksum.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:329 DownloadCache.assertFileChecksum
pub fn assert_file_checksum(
    expected_checksum: &str,
    file_path: &Path,
    key_type: KeyType,
) -> Result<(), IoException> {
    check_argument!(!expected_checksum.is_empty());
    let actual_checksum = match get_checksum(key_type, file_path) {
        Ok(c) => c,
        Err(e) => {
            return Err(IoException::new(format!(
                "Could not hash file {}: {}, expected {} of {}. ",
                file_path,
                e.get_message(),
                key_type,
                expected_checksum
            )));
        }
    };
    if !actual_checksum.eq_ignore_ascii_case(expected_checksum) {
        return Err(IoException::new(format!(
            "Downloaded file at {} has {} of {}, does not match expected {} ({})",
            file_path, key_type, actual_checksum, key_type, expected_checksum
        )));
    }
    Ok(())
}

/// Obtains the checksum of a file (lowercase hex).
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:371 DownloadCache.getChecksum
pub fn get_checksum(key_type: KeyType, path: &Path) -> Result<String, IoException> {
    // Attempt to use the fast digest if the hash function of the filesystem matches `keyType`
    // and it's available.
    if key_type.matches_file_system_digest(path.get_file_system().get_digest_function())
        && let Ok(Some(digest)) = path.get_fast_digest()
    {
        return Ok(hex::encode(digest));
    }
    let mut hasher = key_type.new_hasher();
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut stream = fs_utils::io(path.get_input_stream())?;
    loop {
        let n = fs_utils::io(stream.read(&mut buffer))?;
        if n == 0 {
            break;
        }
        hasher.put_bytes(&buffer[..n]);
    }
    Ok(hex::encode(hasher.hash()))
}

// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:396 DownloadCache.assertKeyIsValid
fn assert_key_is_valid(key: &str, key_type: KeyType) -> Result<(), IoException> {
    if !key_type.is_valid(Some(key)) {
        return Err(IoException::new(format!(
            "Invalid key \"{}\" of type {}. ",
            key, key_type
        )));
    }
    Ok(())
}
