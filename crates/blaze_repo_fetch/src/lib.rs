//! Port of Bazel's Starlark-independent machinery for fetching external code:
//!
//! - [`cache`]: `RepositoryCache`, `DownloadCache` (`content_addressable/`) and
//!   `LocalRepoContentsCache` (`contents/`), byte-compatible with Bazel's on-disk layout.
//! - [`downloader`]: `DownloadManager`, `HttpDownloader`, `HttpConnector`, `UrlRewriter`,
//!   `Checksum`, proxies and progress events.
//! - [`authandtls`]: `.netrc` parsing and credentials.
//! - [`decompressor`]: `DecompressorValue` and the tar/zip/ar/7z/compressed-file extractors,
//!   plus `PatchUtil` (Bazel's native patch).
//! - [`repo_utils`]: Starlark-free helpers behind `repository_ctx` (`which`, file writing,
//!   environment handling).

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod authandtls;
pub mod cache;
pub mod decompressor;
pub mod downloader;
pub mod errors;
pub mod file_system_lock;
pub mod fs_utils;
pub mod java_hash;
pub mod repo_utils;
pub mod uri;

/// The Bazel release this port follows (`BlazeVersionInfo.getVersion()`).
pub const BAZEL_VERSION: &str = "9.3.2";
