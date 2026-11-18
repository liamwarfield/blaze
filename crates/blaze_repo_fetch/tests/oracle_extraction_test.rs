//! Integration test against the oracle: takes the `platforms` 1.0.0 tarball from the repository
//! cache shared with Bazel, fetches it through `DownloadManager` (a repository-cache hit, so no
//! network), extracts it with `DecompressorValue.decompress`, and compares the tree with what
//! Bazel 9.3.2 extracted for `@platforms` under `~/.cache/blaze/oracle-out/ur*/*/external/`.
//!
//! The shared cache and the oracle output are only read; the cache entry is copied into a
//! temporary repository cache first (a cache hit touches the entry's mtime).

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use blaze_events::StoredEventHandler;
use blaze_repo_fetch::cache::{KeyType, RepositoryCache};
use blaze_repo_fetch::decompressor::{self, DecompressorDescriptor};
use blaze_repo_fetch::downloader::{Checksum, DownloadManager, HttpDownloader};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

const SHA256: &str = "3384eb1c30762704fbe38e440204e114154086c8fc8a8c2e3e28441028c019a8";
const URL: &str =
    "https://github.com/bazelbuild/platforms/releases/download/1.0.0/platforms-1.0.0.tar.gz";

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap())
}

fn find_oracle_platforms() -> Option<std::path::PathBuf> {
    let base = home().join(".cache/blaze/oracle-out");
    for ur in std::fs::read_dir(&base).ok()?.flatten() {
        if !ur.file_name().to_string_lossy().starts_with("ur") {
            continue;
        }
        let Ok(bases) = std::fs::read_dir(ur.path()) else {
            continue;
        };
        for b in bases.flatten() {
            let p = b.path().join("external/platforms");
            if p.join("BUILD").exists() {
                return Some(p);
            }
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    kind: char,
    mode: Option<u32>,
    mtime_ms: Option<i64>,
    content: Option<Vec<u8>>,
}

/// Relative path -> entry. Directory modes are not compared (they follow the umask).
/// `MODULE.bazel` is skipped: Bazel overwrites it with the registry's copy after extraction.
fn listing(root: &std::path::Path) -> BTreeMap<String, Entry> {
    let root = std::fs::canonicalize(root).unwrap();
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let m = std::fs::symlink_metadata(&p).unwrap();
            let entry = if m.is_dir() {
                stack.push(p.clone());
                Entry {
                    kind: 'd',
                    mode: None,
                    mtime_ms: None,
                    content: None,
                }
            } else if m.file_type().is_symlink() {
                Entry {
                    kind: 'l',
                    mode: None,
                    mtime_ms: None,
                    content: Some(
                        std::fs::read_link(&p)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned()
                            .into_bytes(),
                    ),
                }
            } else if rel == "MODULE.bazel" {
                Entry {
                    kind: 'f',
                    mode: None,
                    mtime_ms: None,
                    content: None,
                }
            } else {
                Entry {
                    kind: 'f',
                    mode: Some(m.mode() & 0o7777),
                    mtime_ms: Some(m.mtime() * 1000 + m.mtime_nsec() / 1_000_000),
                    content: Some(std::fs::read(&p).unwrap()),
                }
            };
            out.insert(rel, entry);
        }
    }
    out
}

#[test]
fn platforms_tarball_from_shared_cache_matches_oracle_extraction() {
    let cas_entry = home()
        .join(".cache/blaze/repo-cache/content_addressable/sha256")
        .join(SHA256);
    let Some(oracle) = find_oracle_platforms() else {
        eprintln!("skipping: no oracle extraction of @platforms found");
        return;
    };
    if !cas_entry.join("file").exists() {
        eprintln!("skipping: {} not in the shared repository cache", SHA256);
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);

    // A private copy of the shared cache entry, in Bazel's layout.
    let copied = tmp
        .path()
        .join("repository_cache/content_addressable/sha256")
        .join(SHA256);
    std::fs::create_dir_all(&copied).unwrap();
    for e in std::fs::read_dir(&cas_entry).unwrap().flatten() {
        std::fs::copy(e.path(), copied.join(e.file_name())).unwrap();
    }
    let mut repository_cache = RepositoryCache::new();
    repository_cache.set_path(Some(root.get_relative_str("repository_cache")));

    let events = Arc::new(StoredEventHandler::new());
    let manager = DownloadManager::new(
        repository_cache.get_download_cache().clone(),
        Arc::new(HttpDownloader::default_downloader()),
        Arc::new(HttpDownloader::default_downloader()),
        events.clone(),
    );
    let checksum = Checksum::from_string(KeyType::SHA256, SHA256).unwrap();
    let output_dir = root.get_relative_str("download");
    // http_archive's canonical id is the space-joined URL list; Bazel recorded it in the
    // cache as id-<sha256 of the id>.
    let archive = manager
        .download(
            &[JavaUri::create(URL).unwrap()],
            &Default::default(),
            &HashMap::new(),
            Some(&checksum),
            URL,
            Some("tar.gz"),
            &output_dir,
            &HashMap::new(),
            "repository @@platforms",
            false,
        )
        .unwrap();
    assert_eq!(archive.get_base_name(), "platforms-1.0.0.tar.gz");

    let destination = root.get_relative_str("external/platforms");
    let extracted = decompressor::decompress(
        &DecompressorDescriptor::builder()
            .set_context("repository @@platforms")
            .set_archive_path(archive)
            .set_destination_path(destination)
            .build(),
        None,
    )
    .unwrap();

    let ours = listing(std::path::Path::new(extracted.get_path_string()));
    let theirs = listing(&oracle);
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>(),
        "tree listing differs from the oracle"
    );
    for (k, v) in &ours {
        assert_eq!(v, &theirs[k], "entry {} differs from the oracle", k);
    }
}
