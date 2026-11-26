//! The identity of the Blaze executable a persistent server runs.
//!
//! PORT: Blaze-only. A Bazel install base holds the server (`A-server.jar`), so a Bazel binary
//! with another server has another install md5, another install base, and
//! `EnsureCorrectRunningVersion` replaces a running server that uses another install base. Blaze
//! is its own server (`/proc/self/exe`, see `ServerMode::Daemon`) and keeps one install base for
//! every build of it (the embedded files are Bazel's, decision of 2026-11-26), so a rebuilt
//! Blaze binary would otherwise keep talking to a server started by an older one. The client
//! therefore records the identity of its executable in the server directory when it starts a
//! server ([`IDENTITY_FILE`]), and `EnsureCorrectRunningVersion` treats a different identity like
//! a different install base: it kills the running server (restart reason `NEW_VERSION`, the
//! INFO line "Killing running server because it is using another version of ...") and a new one
//! starts.
//!
//! The identity is the executable's size, modification time and MD5 (the digest Bazel's install
//! md5 uses). The MD5 is computed once per size and modification time and cached in the install
//! base ([`CACHE_DIR`]), so a client does not read its whole executable on every command.

use std::io::Read;
use std::os::unix::fs::MetadataExt;

use blaze_util::string_encoding::internal_to_platform;
use md5::{Digest, Md5};

use crate::util::file::{read_file_path, write_file_path};
use crate::util::path::Path;

/// The file in the server directory that holds the identity of the executable the server runs.
pub const IDENTITY_FILE: &str = "blaze_binary";

/// The directory in the install base that caches executable digests by size and mtime.
pub const CACHE_DIR: &str = "blaze_binary_md5";

/// `<size> <mtime seconds>.<mtime nanoseconds> <md5 hex>` of the executable at `self_path`, or
/// `None` when it cannot be read (then no identity is recorded or compared).
pub fn identity(self_path: &str, install_base: &Path) -> Option<String> {
    let native = internal_to_platform(self_path);
    let metadata = std::fs::metadata(std::path::Path::new(&*native)).ok()?;
    let key = format!(
        "{}-{}.{:09}",
        metadata.size(),
        metadata.mtime(),
        metadata.mtime_nsec()
    );
    let cache = install_base.get_relative(CACHE_DIR).get_relative(&key);
    let digest = match read_file_path(&cache, 0) {
        Some(bytes) if bytes.len() == 32 && bytes.iter().all(u8::is_ascii_hexdigit) => {
            String::from_utf8(bytes).ok()?
        }
        _ => {
            let mut file = std::fs::File::open(std::path::Path::new(&*native)).ok()?;
            let mut hasher = Md5::new();
            let mut buffer = vec![0u8; 1 << 20];
            loop {
                let n = file.read(&mut buffer).ok()?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            let digest = hex::encode(hasher.finalize());
            // Best effort: an install base that is not writable just means no cache.
            let _ = std::fs::create_dir_all(std::path::Path::new(&*internal_to_platform(
                &install_base.get_relative(CACHE_DIR).as_native_path(),
            )));
            let _ = write_file_path(digest.as_bytes(), &cache, 0o644);
            digest
        }
    };
    Some(format!(
        "{} {}.{:09} {}",
        metadata.size(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        digest
    ))
}

/// Whether the server whose directory is `server_dir` was started by an executable with this
/// `identity` (a missing or unreadable record counts as different).
pub fn server_has_identity(server_dir: &Path, identity: &str) -> bool {
    read_file_path(&server_dir.get_relative(IDENTITY_FILE), 0)
        .is_some_and(|recorded| recorded == identity.as_bytes())
}

/// Records `identity` for a server about to start in `server_dir`.
pub fn record(server_dir: &Path, identity: &str) -> bool {
    write_file_path(
        identity.as_bytes(),
        &server_dir.get_relative(IDENTITY_FILE),
        0o644,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_cached_and_changes_with_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let exe = format!("{root}/exe");
        std::fs::write(&exe, b"one").unwrap();
        let install_base = Path::new(&format!("{root}/install"));
        std::fs::create_dir_all(format!("{root}/install")).unwrap();

        let first = identity(&exe, &install_base).unwrap();
        assert!(first.starts_with("3 "), "{first}");
        assert!(
            first.ends_with(" f97c5d29941bfb1b2fdab0874906ab82"),
            "{first}"
        );
        // The digest is cached by size and mtime.
        let cached: Vec<_> = std::fs::read_dir(format!("{root}/install/{CACHE_DIR}"))
            .unwrap()
            .collect();
        assert_eq!(cached.len(), 1);
        assert_eq!(identity(&exe, &install_base).unwrap(), first);

        std::fs::write(&exe, b"two!").unwrap();
        let second = identity(&exe, &install_base).unwrap();
        assert_ne!(first, second);

        let server_dir = Path::new(&format!("{root}/server"));
        std::fs::create_dir_all(format!("{root}/server")).unwrap();
        assert!(!server_has_identity(&server_dir, &first));
        assert!(record(&server_dir, &first));
        assert!(server_has_identity(&server_dir, &first));
        assert!(!server_has_identity(&server_dir, &second));
    }
}
