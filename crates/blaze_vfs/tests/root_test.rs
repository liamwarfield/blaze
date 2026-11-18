//! Port of `src/test/java/com/google/devtools/build/lib/vfs/RootTest.java`.
//!
//! Java uses `InMemoryFileSystem`, which is not ported; these tests do no IO, so `UnixFileSystem`
//! instances stand in (each instance is a distinct file system identity).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, PathFragment, Root, UnixFileSystem,
};

fn new_fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn assert_panics<R>(f: impl FnOnce() -> R) {
    assert!(
        catch_unwind(AssertUnwindSafe(f)).is_err(),
        "expected a panic"
    );
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn test_equals_and_hash_code_contract() {
    let fs = new_fs();
    let other_fs = new_fs();
    let groups = [
        vec![Root::absolute_root(&fs), Root::absolute_root(&fs)],
        vec![
            Root::absolute_root(&other_fs),
            Root::absolute_root(&other_fs),
        ],
        vec![
            Root::from_path(fs.get_path("/foo")),
            Root::from_path(fs.get_path("/foo")),
        ],
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
fn test_path_root() {
    let fs = new_fs();
    let root = Root::from_path(fs.get_path("/foo"));
    assert_eq!(root.as_path(), Some(&fs.get_path("/foo")));
    assert!(root.contains(&fs.get_path("/foo/bar")));
    assert!(!root.contains(&fs.get_path("/boo/bar")));
    assert!(root.contains_fragment(&PathFragment::create("/foo/bar")));
    assert!(!root.contains_fragment(&PathFragment::create("foo/bar")));
    assert_eq!(
        root.get_relative(&PathFragment::create("bar")),
        fs.get_path("/foo/bar")
    );
    assert_eq!(root.get_relative_str("bar"), fs.get_path("/foo/bar"));
    assert_eq!(
        root.get_relative(&PathFragment::create("/bar")),
        fs.get_path("/bar")
    );
    assert_eq!(
        root.relativize(&fs.get_path("/foo/bar")),
        PathFragment::create("bar")
    );
    assert_eq!(
        root.relativize_fragment(&PathFragment::create("/foo/bar")),
        PathFragment::create("bar")
    );
    assert_panics(|| root.relativize_fragment(&PathFragment::create("foo")));
}

#[test]
fn test_filesystem_transform() {
    let fs = new_fs();
    let fs2 = new_fs();
    let root = Root::from_path(fs.get_path("/foo"));
    let root2 = Root::to_file_system(&root, &fs2);
    assert!(Arc::ptr_eq(
        root2.as_path().expect("path root").get_file_system(),
        &fs2
    ));
    assert_eq!(
        root2.as_path().expect("path root").as_fragment(),
        &PathFragment::create("/foo")
    );
    assert!(!root.is_absolute());
}

#[test]
fn test_file_system_absolute_root() {
    let fs = new_fs();
    let root = Root::absolute_root(&fs);
    assert!(root.as_path().is_none());
    assert!(root.contains(&fs.get_path("/foo")));
    assert!(root.contains_fragment(&PathFragment::create("/foo/bar")));
    assert!(!root.contains_fragment(&PathFragment::create("foo/bar")));
    assert_eq!(root.get_relative_str("/foo"), fs.get_path("/foo"));
    assert_eq!(
        root.relativize(&fs.get_path("/foo")),
        PathFragment::create("/foo")
    );
    assert_eq!(
        root.relativize_fragment(&PathFragment::create("/foo")),
        PathFragment::create("/foo")
    );

    assert_panics(|| root.get_relative(&PathFragment::create("foo")));
    assert_panics(|| root.get_relative(&PathFragment::create("foo")));
    assert_panics(|| root.relativize_fragment(&PathFragment::create("foo")));
}

#[test]
fn test_compare_to() {
    let fs = new_fs();
    let a = Root::from_path(fs.get_path("/a"));
    let b = Root::from_path(fs.get_path("/b"));
    let root = Root::absolute_root(&fs);
    let mut list = vec![a.clone(), root.clone(), b.clone()];
    list.sort();
    assert_eq!(list, vec![root, a, b]);
}

#[test]
#[ignore = "no serialization infrastructure (SerializationTester, RootCodec) yet"]
fn test_serialization_simple() {}

#[test]
#[ignore = "no serialization infrastructure (ObjectCodecs, RootCodec) yet"]
fn test_serialization_likely_popular_root_is_canonicalized() {}
