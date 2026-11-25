//! `UnixFileSystem.getxattr` / `getFastDigest` (`--unix_digest_hash_attribute_name`). No Java
//! unit test exists for them at the pin; Bazel's integration tests
//! `unix_digest_hash_attribute_name_test` and `remote_execution_with_xattr_test` exercise them.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

use blaze_vfs::{DigestHashFunction, FileSystem, PathFragment, UnixFileSystem};

/// Sets `user.<name>` on `path`; false when the file system has no user xattrs.
fn setxattr(path: &std::path::Path, name: &str, value: &[u8]) -> bool {
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let c_name = CString::new(name).unwrap();
    // SAFETY: NUL-terminated strings and a valid buffer.
    let r = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if r == 0 {
        return true;
    }
    let e = std::io::Error::last_os_error();
    assert_eq!(e.raw_os_error(), Some(libc::ENOTSUP), "{e}");
    false
}

/// A fresh directory under the system temp dir, removed when dropped.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(test: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "blaze_vfs_UnixFileSystemXattrTest_{test}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fast_digest_comes_from_the_hash_attribute() {
    let dir = TempDir::new("fast_digest");
    let file = dir.path().join("f");
    std::fs::write(&file, b"hello").unwrap();
    let path = PathFragment::create(file.to_str().unwrap());

    let without = UnixFileSystem::new(DigestHashFunction::Sha256);
    assert_eq!(without.get_fast_digest(&path).unwrap(), None);

    let fs = UnixFileSystem::with_hash_attribute_name(
        DigestHashFunction::Sha256,
        "user.checksum.sha256".to_string(),
    );
    // No attribute: ENODATA is null.
    assert_eq!(fs.get_fast_digest(&path).unwrap(), None);
    if !setxattr(&file, "user.checksum.sha256", &[1, 2, 3]) {
        return;
    }
    assert_eq!(fs.get_fast_digest(&path).unwrap(), Some(vec![1, 2, 3]));
    assert_eq!(
        fs.getxattr(&path, "user.checksum.sha256", false).unwrap(),
        Some(vec![1, 2, 3])
    );
    // getDigest still reads the file.
    assert_eq!(
        fs.get_digest(&path).unwrap(),
        DigestHashFunction::Sha256
            .hash_reader(&mut &b"hello"[..])
            .unwrap()
    );
}

#[test]
fn getxattr_of_a_missing_file_is_the_native_error() {
    let dir = TempDir::new("missing");
    let missing = dir.path().join("missing");
    let path = PathFragment::create(missing.to_str().unwrap());
    let fs = UnixFileSystem::with_hash_attribute_name(
        DigestHashFunction::Sha256,
        "user.checksum.sha256".to_string(),
    );
    let e = fs.get_fast_digest(&path).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!("{} (No such file or directory)", missing.display())
    );
}
