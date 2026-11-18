//! Port of `src/test/java/com/google/devtools/build/lib/vfs/PathGetParentTest.java`.
//!
//! Uses the native `UnixFileSystem` in a unique directory under the system temp dir.

use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};

struct Fixture {
    fs: FileSystemRef,
    test_root: Path,
}

impl Fixture {
    fn create_test_root() -> Fixture {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let tmp = std::env::temp_dir();
        let unique = format!(
            "blaze_vfs_UnixPathGetParentTest_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let test_root = fs
            .get_path(tmp.to_str().expect("utf-8 temp dir"))
            .get_relative_str(&unique)
            .get_relative_str("UnixPathGetParentTest");
        test_root.create_directory_and_parents().expect("mkdir");
        Fixture { fs, test_root }
    }

    fn get_parent(&self, path: &str) -> Option<Path> {
        self.fs.get_path(path).get_parent_directory()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // deleteTestRoot; also remove the unique parent we created.
        let _ = self.test_root.delete_tree();
        if let Some(parent) = self.test_root.get_parent_directory() {
            let _ = parent.delete_tree();
        }
    }
}

#[test]
fn test_absolute_root_has_no_parent() {
    let f = Fixture::create_test_root();
    assert!(f.get_parent("/").is_none());
}

#[test]
fn test_parent_of_simple_directory() {
    let f = Fixture::create_test_root();
    assert_eq!(f.get_parent("/foo/bar").unwrap().get_path_string(), "/foo");
}

#[test]
fn test_parent_of_dot_dot_in_middle_of_pathname() {
    let f = Fixture::create_test_root();
    assert_eq!(f.get_parent("/foo/../bar").unwrap().get_path_string(), "/");
}

#[test]
fn test_get_path_does_normalization_without_io() {
    let f = Fixture::create_test_root();
    let tmp = f.test_root.get_child("tmp");
    let tmp_wiz = tmp.get_child("wiz");

    tmp.create_directory().expect("mkdir tmp");

    // ln -sf /tmp /tmp/wiz
    tmp_wiz.create_symbolic_link(&tmp).expect("symlink");

    assert_eq!(tmp.get_parent_directory(), Some(f.test_root.clone()));

    assert_eq!(tmp_wiz.get_parent_directory(), Some(tmp.clone()));

    // Under UNIX, inode(/tmp/wiz/..) == inode(/).  However getPath() does not
    // perform I/O, only string operations, so it disagrees:
    assert_eq!(tmp.get_relative(&PathFragment::create("wiz/..")), tmp);
}
