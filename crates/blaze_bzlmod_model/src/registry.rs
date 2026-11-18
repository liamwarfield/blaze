//! Ports of `Registry`, `ModuleFile`, `RegistryFileDownloadEvent`, `YankedVersionsValue`, and the
//! `RegistryFetcher` seam that stands in for `DownloadManager.downloadAndReadOneUrlForBzlmod`.

use std::fmt;
use std::sync::Arc;

use blaze_events::{ExtendedEventHandler, Postable};
use indexmap::IndexMap;

use crate::checksum::Checksum;
use crate::java_uri::JavaUri;
use crate::module_key::ModuleKey;
use crate::repo_spec::RepoSpec;
use crate::version::Version;

/// Errors a registry operation can raise: Java's `IOException` (and subclasses
/// `FileNotFoundException`, `MissingChecksumException`), `Registry.NotFoundException` and
/// `InterruptedException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryException {
    /// `IOException` with its message.
    Io(String),
    /// `Registry.NotFoundException` with its message.
    NotFound(String),
    Interrupted,
}

impl RegistryException {
    pub fn get_message(&self) -> &str {
        match self {
            RegistryException::Io(m) | RegistryException::NotFound(m) => m,
            RegistryException::Interrupted => "interrupted",
        }
    }
}

impl fmt::Display for RegistryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_message())
    }
}

impl std::error::Error for RegistryException {}

/// The raw bytes of a module file and where it came from. Port of `ModuleFile`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFile.java:22 ModuleFile
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFile {
    pub content: Vec<u8>,
    pub location: String,
}

impl ModuleFile {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFile.java:29 ModuleFile.create
    pub fn create(content: impl Into<Vec<u8>>, location: impl Into<String>) -> ModuleFile {
        ModuleFile {
            content: content.into(),
            location: location.into(),
        }
    }
}

/// The result of yanked-version lookup for one module. Port of `YankedVersionsValue`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:28 YankedVersionsValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YankedVersionsValue {
    pub yanked_versions: Option<IndexMap<Version, String>>,
}

impl YankedVersionsValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:34 YankedVersionsValue.NONE_YANKED
    pub fn none_yanked() -> YankedVersionsValue {
        YankedVersionsValue {
            yanked_versions: Some(IndexMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:36 YankedVersionsValue.create
    pub fn create(yanked_versions: Option<IndexMap<Version, String>>) -> YankedVersionsValue {
        YankedVersionsValue { yanked_versions }
    }
}

/// Posted for every registry file fetched (or found missing) with a checksum. Port of
/// `RegistryFileDownloadEvent`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFileDownloadEvent.java:31 RegistryFileDownloadEvent
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryFileDownloadEvent {
    pub uri: String,
    pub checksum: Option<Checksum>,
}

impl Postable for RegistryFileDownloadEvent {}

impl RegistryFileDownloadEvent {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFileDownloadEvent.java:33 RegistryFileDownloadEvent.create
    pub fn create(uri: impl Into<String>, content: Option<&[u8]>) -> RegistryFileDownloadEvent {
        RegistryFileDownloadEvent {
            uri: uri.into(),
            checksum: content.map(Checksum::sha256_of),
        }
    }

    /// Collects the events among `postables` into a map (later entries win, first position kept).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFileDownloadEvent.java:37 RegistryFileDownloadEvent.collectToMap
    pub fn collect_to_map(postables: &[Arc<dyn Postable>]) -> IndexMap<String, Option<Checksum>> {
        let mut builder = IndexMap::new();
        for postable in postables {
            let any: &dyn std::any::Any = postable.as_ref();
            if let Some(e) = any.downcast_ref::<RegistryFileDownloadEvent>() {
                builder.insert(e.uri.clone(), e.checksum.clone());
            }
        }
        builder
    }
}

/// Fetches one registry file (`DownloadManager.downloadAndReadOneUrlForBzlmod`): the repository
/// cache, URL rewriting, credentials and retries live behind this trait.
pub trait RegistryFetcher: Send + Sync {
    /// Returns the bytes at `url`. A missing file is [`RegistryException::NotFound`] (Java
    /// `FileNotFoundException`); other failures are [`RegistryException::Io`]. When `checksum`
    /// is given, content not matching it fails with `Checksum was <actual> but wanted <expected>`.
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException>;
}

/// Reads `file:` URLs from the local file system. Other schemes are not ported here.
// PORT: a minimal stand-in for DownloadManager + HttpDownloader on `file:` URLs, without the
// download cache; the downloader crate provides the real implementation.
#[derive(Debug, Default)]
pub struct FileRegistryFetcher;

impl RegistryFetcher for FileRegistryFetcher {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        _client_env: &IndexMap<String, String>,
        checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        if url.get_scheme() != Some("file") {
            blaze_util::unported!("DownloadManager.downloadAndReadOneUrlForBzlmod", "{url}");
        }
        let path = url.get_path().unwrap_or_default();
        let content = match std::fs::read(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RegistryException::NotFound(format!(
                    "{path} (No such file or directory)"
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {
                return Err(RegistryException::NotFound(format!(
                    "{path} (Is a directory)"
                )));
            }
            Err(e) => return Err(RegistryException::Io(e.to_string())),
        };
        verify_checksum(&content, checksum)?;
        Ok(content)
    }
}

/// Checks downloaded content against an expected checksum, with `HashInputStream`'s message.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/HashInputStream.java:85 HashInputStream.check
pub fn verify_checksum(
    content: &[u8],
    checksum: Option<&Checksum>,
) -> Result<(), RegistryException> {
    if let Some(expected) = checksum {
        let actual = crate::checksum::hash_with(expected.get_key_type(), content);
        if actual.as_slice() != expected.get_hash_code() {
            return Err(RegistryException::Io(format!(
                "Checksum was {} but wanted {}",
                expected.emit_other_hash_in_same_format(&actual),
                expected.emit_other_hash_in_same_format(expected.get_hash_code())
            )));
        }
    }
    Ok(())
}

/// The HTTP(S) fetcher. Not ported yet: every call fails loudly.
#[derive(Debug, Default)]
pub struct HttpRegistryFetcher;

impl RegistryFetcher for HttpRegistryFetcher {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &JavaUri,
        _client_env: &IndexMap<String, String>,
        _checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        blaze_util::unported!("DownloadManager.downloadAndReadOneUrlForBzlmod", "{url}")
    }
}

/// A Bazel module registry. Port of `Registry`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Registry.java:30 Registry
pub trait Registry: Send + Sync + fmt::Debug {
    /// The URL uniquely identifying the registry.
    fn get_url(&self) -> String;

    /// Retrieves the contents of the module file of the module identified by `key`.
    fn get_module_file(
        &self,
        key: &ModuleKey,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<ModuleFile, RegistryException>;

    /// Retrieves the `RepoSpec` object that indicates how the contents of the module identified
    /// by `key` should be materialized as a repo.
    fn get_repo_spec(
        &self,
        key: &ModuleKey,
        module_file_hashes: &IndexMap<String, Option<Checksum>>,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<RepoSpec, RegistryException>;

    /// Retrieves the yanked versions of the module `module_name`, or `None` if they can't be
    /// determined.
    fn get_yanked_versions(
        &self,
        module_name: &str,
        event_handler: &dyn ExtendedEventHandler,
        download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<IndexMap<Version, String>>, RegistryException>;

    /// Returns yanked-version information known from the lockfile, if any.
    fn try_get_yanked_versions_from_lockfile(
        &self,
        selected_module_key: &ModuleKey,
    ) -> Option<YankedVersionsValue>;
}
