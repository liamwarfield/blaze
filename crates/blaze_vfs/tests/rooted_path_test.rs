//! Port of `src/test/java/com/google/devtools/build/lib/vfs/RootedPathTest.java`.
//!
//! Java uses `InMemoryFileSystem`, which is not ported; these tests do no IO, so a
//! `UnixFileSystem` instance stands in.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Root, RootedPath,
    UnixFileSystem,
};

struct Fixture {
    root: Path,
}

fn initialize_file_system() -> Fixture {
    let filesystem: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = filesystem.get_path("/");
    Fixture { root }
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn test_equals_and_hash_code_contract() {
    let f = initialize_file_system();
    let pkg_root1 = f.root.get_relative_str("pkgroot1");
    let pkg_root2 = f.root.get_relative_str("pkgroot2");
    let rooted_path_a1 = RootedPath::to_rooted_path(
        Root::from_path(pkg_root1.clone()),
        &PathFragment::create("foo/bar"),
    );
    let rooted_path_a2 = RootedPath::to_rooted_path(
        Root::from_path(pkg_root1.clone()),
        &PathFragment::create("foo/bar"),
    );
    let absolute_path1 = RootedPath::to_rooted_path(
        Root::from_path(f.root.clone()),
        &PathFragment::create("pkgroot1/foo/bar"),
    );
    let rooted_path_b1 = RootedPath::to_rooted_path(
        Root::from_path(pkg_root2.clone()),
        &PathFragment::create("foo/bar"),
    );
    let rooted_path_b2 = RootedPath::to_rooted_path(
        Root::from_path(pkg_root2.clone()),
        &PathFragment::create("foo/bar"),
    );
    let absolute_path2 = RootedPath::to_rooted_path(
        Root::from_path(f.root.clone()),
        &PathFragment::create("pkgroot2/foo/bar"),
    );
    let groups = [
        vec![rooted_path_a1, rooted_path_a2],
        vec![rooted_path_b1, rooted_path_b2],
        vec![absolute_path1],
        vec![absolute_path2],
    ];
    for (i, g) in groups.iter().enumerate() {
        for a in g {
            for b in g {
                assert_eq!(a, b);
                assert_eq!(hash_of(a), hash_of(b));
            }
            for (j, h) in groups.iter().enumerate() {
                if i != j {
                    for b in h {
                        assert_ne!(a, b);
                    }
                }
            }
        }
    }
}

#[test]
fn test_get_parent_directory() {
    let f = initialize_file_system();
    let path = create_rooted_path(&f, "root/folder", "folder1/folder2");
    let parent = path.get_parent_directory().expect("parent");
    assert_eq!(parent.as_path().get_path_string(), "/root/folder/folder1");
    assert_eq!(parent.get_root_relative_path().get_path_string(), "folder1");

    let parent = parent.get_parent_directory().expect("parent");
    assert_eq!(parent.as_path().get_path_string(), "/root/folder");
    assert!(parent.get_root_relative_path().get_path_string().is_empty());

    assert!(parent.get_parent_directory().is_none());
}

#[test]
fn test_get_parent_directory_of_root() {
    let f = initialize_file_system();
    let path = create_rooted_path(&f, "root", "");
    assert!(path.get_parent_directory().is_none());
}

fn create_rooted_path(f: &Fixture, relative_root_path: &str, relative_path: &str) -> RootedPath {
    RootedPath::to_rooted_path(
        Root::from_path(f.root.get_relative_str(relative_root_path)),
        &PathFragment::create(relative_path),
    )
}
