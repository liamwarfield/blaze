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
//! The identity is the MD5 of the executable's content, as Bazel's install md5 is a digest of
//! content only: two copies of one binary share a server, and a different binary never does. The
//! digest is computed once per file and cached, keyed by what identifies the file and its
//! version (device, inode, size, mtime, ctime), so a different file with the same size and mtime
//! cannot reuse a stale digest. The cache lives in Blaze's own directory of the output user root
//! ([`CACHE_DIR`]), not in the install base that Blaze shares with the oracle.

use std::io::Read;
use std::os::unix::fs::MetadataExt;

use blaze_util::string_encoding::internal_to_platform;
use md5::{Digest, Md5};

use crate::util::file::{read_file_path, write_file_path};
use crate::util::path::Path;

/// The file in the server directory that holds the identity of the executable the server runs.
pub const IDENTITY_FILE: &str = "blaze_binary";

/// The directory in the output user root that caches executable digests.
pub const CACHE_DIR: &str = "blaze/binary_md5";

/// The cache key of a file: device, inode, size, mtime and ctime (both with nanoseconds).
fn cache_key(metadata: &std::fs::Metadata) -> String {
    format!(
        "{}-{}-{}-{}.{:09}-{}.{:09}",
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    )
}

/// The MD5 (hex) of the executable at `self_path`, or `None` when it cannot be read (then no
/// identity is recorded or compared).
pub fn identity(self_path: &str, output_user_root: &Path) -> Option<String> {
    let native = internal_to_platform(self_path);
    // The key comes from the open file, so the digest is of the file the key describes.
    let mut file = std::fs::File::open(std::path::Path::new(&*native)).ok()?;
    let metadata = file.metadata().ok()?;
    let cache_dir = output_user_root.get_relative(CACHE_DIR);
    let cache = cache_dir.get_relative(&cache_key(&metadata));
    if let Some(bytes) = read_file_path(&cache, 0)
        && bytes.len() == 32
        && bytes.iter().all(u8::is_ascii_hexdigit)
    {
        return String::from_utf8(bytes).ok();
    }
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
    // Best effort: an output user root that is not writable just means no cache.
    let _ = std::fs::create_dir_all(std::path::Path::new(&*internal_to_platform(
        &cache_dir.as_native_path(),
    )));
    let _ = write_file_path(digest.as_bytes(), &cache, 0o644);
    Some(digest)
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
    fn identity_is_the_content_digest_and_is_cached() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let exe = format!("{root}/exe");
        std::fs::write(&exe, b"one").unwrap();
        let output_user_root = Path::new(&format!("{root}/root"));

        let first = identity(&exe, &output_user_root).unwrap();
        assert_eq!(first, "f97c5d29941bfb1b2fdab0874906ab82");
        let cached: Vec<_> = std::fs::read_dir(format!("{root}/root/{CACHE_DIR}"))
            .unwrap()
            .collect();
        assert_eq!(cached.len(), 1);
        assert_eq!(identity(&exe, &output_user_root).unwrap(), first);

        // A copy of the same content (another inode, another mtime) has the same identity.
        let copy = format!("{root}/copy");
        std::fs::write(&copy, b"one").unwrap();
        assert_eq!(identity(&copy, &output_user_root).unwrap(), first);

        // Other content of the same size gets its own digest, never a cached one.
        std::fs::write(&exe, b"two").unwrap();
        let second = identity(&exe, &output_user_root).unwrap();
        assert_eq!(second, "b8a9f715dbb64fd5c56e7783c6820a61");

        let server_dir = Path::new(&format!("{root}/server"));
        std::fs::create_dir_all(format!("{root}/server")).unwrap();
        assert!(!server_has_identity(&server_dir, &first));
        assert!(record(&server_dir, &first));
        assert!(server_has_identity(&server_dir, &first));
        assert!(!server_has_identity(&server_dir, &second));
    }
}
