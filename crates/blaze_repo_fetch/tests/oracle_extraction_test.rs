//! Integration test against the oracle: takes the `platforms` tarball that an oracle output base
//! extracted for `@platforms` from the repository cache shared with Bazel, fetches it through
//! `DownloadManager` (a repository-cache hit, so no network), extracts it with
//! `DecompressorValue.decompress`, and compares the tree with what Bazel 9.3.2 extracted under
//! that output base's `external/platforms`.
//!
//! Only output bases of the oracle are used (their `install` links to the oracle's install base).
//! Both sides come from one fact, the output base's own module resolution: its workspace
//! (`DO_NOT_BUILD_HERE`) has a `MODULE.bazel.lock` whose `registryFileHashes` name the
//! `platforms/<version>/source.json` it used; that file, from the shared cache, gives the archive's
//! URL and sha256. The extraction's `MODULE.bazel` must declare the same version. When a link of
//! the chain is missing (no such output base, no lockfile entry, the source.json or the tarball not
//! in the shared cache, a version mismatch), the test is skipped and says why.
//!
//! The shared cache and the oracle output are only read; the cache entry is copied into a
//! temporary repository cache first (a cache hit touches the entry's mtime).

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use base64::Engine as _;
use blaze_events::StoredEventHandler;
use blaze_repo_fetch::cache::{KeyType, RepositoryCache};
use blaze_repo_fetch::decompressor::{self, DecompressorDescriptor};
use blaze_repo_fetch::downloader::{Checksum, DownloadManager, HttpDownloader};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

/// The install base of the oracle binary (Bazel 9.3.2, AGENTS.md).
const ORACLE_INSTALL_MD5: &str = "58cf1ecbff441bac762b0050d6da21e9";

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap())
}

fn shared_cas() -> std::path::PathBuf {
    home().join(".cache/blaze/repo-cache/content_addressable/sha256")
}

/// The `@platforms` archive an oracle output base used, and where it extracted it.
struct Platforms {
    extraction: std::path::PathBuf,
    url: String,
    sha256: String,
}

/// The value of `"key": "value"` in a small JSON document (source.json, lockfile lines).
fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))?;
    let rest = &text[at + key.len() + 2..];
    let start = rest.find('"')? + 1;
    let end = start + rest[start..].find('"')?;
    Some(rest[start..end].to_owned())
}

/// The `version = "..."` of a `module(...)` call.
fn module_version(module_bazel: &str) -> Option<String> {
    let at = module_bazel.find("version")?;
    let rest = &module_bazel[at..];
    let start = rest.find('"')? + 1;
    let end = start + rest[start..].find('"')?;
    Some(rest[start..end].to_owned())
}

/// Follows one output base's chain (see the module comment); `Err` is why it cannot be used.
fn platforms_of(output_base: &std::path::Path) -> Result<Platforms, String> {
    let extraction = output_base.join("external/platforms");
    let module = std::fs::read_to_string(extraction.join("MODULE.bazel"))
        .map_err(|e| format!("{}: {e}", extraction.display()))?;
    let extracted_version = module_version(&module)
        .ok_or_else(|| format!("{}: no module version", extraction.display()))?;
    let workspace = std::fs::read_to_string(output_base.join("DO_NOT_BUILD_HERE"))
        .map_err(|e| format!("{}: no DO_NOT_BUILD_HERE: {e}", output_base.display()))?;
    let lockfile_path = std::path::Path::new(workspace.trim()).join("MODULE.bazel.lock");
    let lockfile = std::fs::read_to_string(&lockfile_path)
        .map_err(|e| format!("{}: {e}", lockfile_path.display()))?;
    let line = lockfile
        .lines()
        .find(|l| l.contains("/modules/platforms/") && l.contains("/source.json\""))
        .ok_or_else(|| format!("{}: no platforms source.json", lockfile_path.display()))?;
    let key = line.trim().split('"').nth(1).unwrap_or_default().to_owned();
    let locked_version = key
        .rsplit('/')
        .nth(1)
        .ok_or_else(|| format!("{}: bad key {key}", lockfile_path.display()))?
        .to_owned();
    if locked_version != extracted_version {
        return Err(format!(
            "{}: the lockfile names platforms {locked_version}, the extraction is {extracted_version}",
            output_base.display()
        ));
    }
    let source_hash = line.trim().split('"').nth(3).unwrap_or_default().to_owned();
    let source_file = shared_cas().join(&source_hash).join("file");
    let source = std::fs::read_to_string(&source_file).map_err(|e| {
        format!("source.json {source_hash} not in the shared repository cache: {e}")
    })?;
    let url = json_string(&source, "url").ok_or("source.json has no url")?;
    let integrity = json_string(&source, "integrity").ok_or("source.json has no integrity")?;
    let digest = base64::engine::general_purpose::STANDARD
        .decode(
            integrity
                .strip_prefix("sha256-")
                .ok_or_else(|| format!("integrity {integrity} is not sha256"))?,
        )
        .map_err(|e| format!("integrity {integrity}: {e}"))?;
    Ok(Platforms {
        extraction,
        url,
        sha256: hex::encode(digest),
    })
}

/// The first oracle output base (in name order) whose chain is complete, or the reasons none is.
fn find_oracle_platforms() -> Result<Platforms, Vec<String>> {
    let base = home().join(".cache/blaze/oracle-out");
    let mut output_bases = Vec::new();
    for ur in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if !ur.file_name().to_string_lossy().starts_with("ur") {
            continue;
        }
        for b in std::fs::read_dir(ur.path()).into_iter().flatten().flatten() {
            // Only the oracle's output bases (their `install` links to the oracle's install base);
            // Blaze runs keep their output bases in the same tree.
            let is_oracle = std::fs::read_link(b.path().join("install"))
                .ok()
                .and_then(|t| t.file_name().map(|n| n == ORACLE_INSTALL_MD5))
                .unwrap_or(false);
            if is_oracle && b.path().join("external/platforms/BUILD").exists() {
                output_bases.push(b.path());
            }
        }
    }
    output_bases.sort();
    let mut reasons = Vec::new();
    for output_base in output_bases {
        match platforms_of(&output_base) {
            Ok(p) if shared_cas().join(&p.sha256).join("file").exists() => return Ok(p),
            Ok(p) => reasons.push(format!(
                "{}: tarball {} not in the shared repository cache",
                output_base.display(),
                p.sha256
            )),
            Err(reason) => reasons.push(reason),
        }
    }
    if reasons.is_empty() {
        reasons.push("no oracle extraction of @platforms found".to_owned());
    }
    Err(reasons)
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
    let platforms = match find_oracle_platforms() {
        Ok(p) => p,
        Err(reasons) => {
            eprintln!("skipping: {}", reasons.join("; "));
            return;
        }
    };
    eprintln!(
        "comparing with {} ({}, sha256 {})",
        platforms.extraction.display(),
        platforms.url,
        platforms.sha256
    );
    let oracle = platforms.extraction.clone();
    let sha256 = platforms.sha256.as_str();
    let url = platforms.url.as_str();
    let cas_entry = shared_cas().join(sha256);

    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);

    // A private copy of the shared cache entry, in Bazel's layout.
    let copied = tmp
        .path()
        .join("repository_cache/content_addressable/sha256")
        .join(sha256);
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
    let checksum = Checksum::from_string(KeyType::SHA256, sha256).unwrap();
    let output_dir = root.get_relative_str("download");
    // http_archive's canonical id is the space-joined URL list; Bazel recorded it in the
    // cache as id-<sha256 of the id>.
    let archive = manager
        .download(
            &[JavaUri::create(url).unwrap()],
            &Default::default(),
            &HashMap::new(),
            Some(&checksum),
            url,
            Some("tar.gz"),
            &output_dir,
            &HashMap::new(),
            "repository @@platforms",
            false,
        )
        .unwrap();
    assert_eq!(archive.get_base_name(), url.rsplit('/').next().unwrap());

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
