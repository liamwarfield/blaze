//! Port of `src/test/java/com/google/devtools/build/lib/vfs/UnixPathTest.java` together with the
//! tests it inherits from `PathAbstractTest.java`.
//!
//! Java uses `InMemoryFileSystem`, which is not ported; these tests do no IO, so a
//! `UnixFileSystem` instance stands in.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};

struct Fixture {
    file_system: FileSystemRef,
}

fn setup() -> Fixture {
    Fixture {
        file_system: UnixFileSystem::create(DigestHashFunction::Sha256),
    }
}

impl Fixture {
    fn create(&self, path: &str) -> Path {
        self.file_system.get_path(path)
    }
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

// ---- PathAbstractTest ----

#[test]
fn test_strips_trailing_slash() {
    let f = setup();
    // compare string forms
    assert_eq!(f.create("/foo/bar/").get_path_string(), "/foo/bar");
    // compare fragment forms
    assert_eq!(f.create("/foo/bar/"), f.create("/foo/bar"));
}

#[test]
fn test_basename() {
    let f = setup();
    assert_eq!(f.create("/foo/bar").get_base_name(), "bar");
    assert_eq!(f.create("/foo/").get_base_name(), "foo");
    assert_eq!(f.create("/foo").get_base_name(), "foo");
    assert!(f.create("/").get_base_name().is_empty());
}

#[test]
#[ignore = "Java String identity (isSameInstanceAs on the input string) has no Rust analogue; \
            PathFragment always stores its own Arc<str>"]
fn test_normal_strings_do_not_allocate() {}

#[test]
fn test_comparable_sort_order() {
    let f = setup();
    let mut list = [
        f.create("/zzz"),
        f.create("/ZZZ"),
        f.create("/ABC"),
        f.create("/aBc"),
        f.create("/AbC"),
        f.create("/abc"),
    ];
    list.sort();
    let result: Vec<&str> = list.iter().map(|p| p.get_path_string()).collect();
    assert_eq!(result, vec!["/ABC", "/AbC", "/ZZZ", "/aBc", "/abc", "/zzz"]);
}

// ---- UnixPathTest ----

#[test]
fn test_equals_and_hash_code_unix() {
    let f = setup();
    let groups = [
        vec![f.create("/something/else")],
        vec![f.create("/"), f.create("//////")],
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
fn test_relative_to_unix() {
    let f = setup();
    assert!(
        f.create("/")
            .relative_to(&f.create("/"))
            .get_path_string()
            .is_empty()
    );
    assert!(
        f.create("/foo")
            .relative_to(&f.create("/foo"))
            .get_path_string()
            .is_empty()
    );
    assert_eq!(
        f.create("/foo/bar/baz")
            .relative_to(&f.create("/foo"))
            .get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        f.create("/foo/bar/baz")
            .relative_to(&f.create("/foo/bar"))
            .get_path_string(),
        "baz"
    );
    assert_eq!(
        f.create("/foo")
            .relative_to(&f.create("/"))
            .get_path_string(),
        "foo"
    );
    // Java: create("foo") throws for a relative Path.
    assert_panics(|| f.create("/foo/bar/baz").relative_to(&f.create("foo")));
    assert_panics(|| f.create("/foo").relative_to(&f.create("/foo/bar/baz")));
}

#[test]
fn test_get_relative_unix() {
    let f = setup();
    assert_eq!(
        f.create("/a").get_relative_str("b").get_path_string(),
        "/a/b"
    );
    assert_eq!(
        f.create("/a/b").get_relative_str("c/d").get_path_string(),
        "/a/b/c/d"
    );
    assert_eq!(
        f.create("/c/d").get_relative_str("/a/b").get_path_string(),
        "/a/b"
    );
    assert_eq!(f.create("/a").get_relative_str("").get_path_string(), "/a");
    assert_eq!(f.create("/").get_relative_str("").get_path_string(), "/");
    assert_eq!(
        f.create("/a/b")
            .get_relative_str("../foo")
            .get_path_string(),
        "/a/foo"
    );

    // Make sure any fast path of Path#getRelative(PathFragment) works
    assert_eq!(
        f.create("/a/b")
            .get_relative(&PathFragment::create("../foo"))
            .get_path_string(),
        "/a/foo"
    );

    // Make sure any fast path of Path#getRelative(PathFragment) works
    assert_eq!(
        f.create("/c/d")
            .get_relative(&PathFragment::create("/a/b"))
            .get_path_string(),
        "/a/b"
    );

    // Test normalization
    assert_eq!(f.create("/a").get_relative_str(".").get_path_string(), "/a");
}

#[test]
fn test_empty_path_to_empty_path_unix() {
    let f = setup();
    // compare string forms
    assert_eq!(f.create("/").get_path_string(), "/");
    // compare fragment forms
    assert_eq!(f.create("/"), f.create("/"));
}

#[test]
fn test_redundant_slashes() {
    let f = setup();
    assert_eq!(f.create("///").get_path_string(), "/");
    assert_eq!(f.create("///"), f.create("/"));
    assert_eq!(f.create("/foo///bar").get_path_string(), "/foo/bar");
    assert_eq!(f.create("/foo///bar"), f.create("/foo/bar"));
    assert_eq!(f.create("////foo//bar").get_path_string(), "/foo/bar");
    assert_eq!(f.create("////foo//bar"), f.create("/foo/bar"));
}

#[test]
fn test_simple_name_to_simple_name_unix() {
    let f = setup();
    assert_eq!(f.create("/foo").get_path_string(), "/foo");
    assert_eq!(f.create("/foo"), f.create("/foo"));
}

#[test]
fn test_simple_path_to_simple_path_unix() {
    let f = setup();
    assert_eq!(f.create("/foo/bar").get_path_string(), "/foo/bar");
    assert_eq!(f.create("/foo/bar"), f.create("/foo/bar"));
}

#[test]
fn test_get_parent_directory_unix() {
    let f = setup();
    assert_eq!(
        f.create("/foo/bar/wiz").get_parent_directory(),
        Some(f.create("/foo/bar"))
    );
    assert_eq!(
        f.create("/foo/bar").get_parent_directory(),
        Some(f.create("/foo"))
    );
    assert_eq!(f.create("/foo").get_parent_directory(), Some(f.create("/")));
    assert_eq!(f.create("/").get_parent_directory(), None);
}

#[test]
fn test_basename_unix() {
    let f = setup();
    assert_eq!(f.create("/foo/bar").get_base_name(), "bar");
    assert_eq!(f.create("/foo/").get_base_name(), "foo");
    assert_eq!(f.create("/foo").get_base_name(), "foo");
    assert!(f.create("/").get_base_name().is_empty());
}

#[test]
fn test_starts_with_unix() {
    let f = setup();
    let foobar = f.create("/foo/bar");

    // (path, prefix) => true
    assert!(foobar.starts_with(&foobar));
    assert!(foobar.starts_with(&f.create("/")));
    assert!(foobar.starts_with(&f.create("/foo")));
    assert!(foobar.starts_with(&f.create("/foo/")));
    assert!(foobar.starts_with(&f.create("/foo/bar/"))); // Includes trailing slash.

    // (prefix, path) => false
    assert!(!f.create("/foo").starts_with(&foobar));
    assert!(!f.create("/").starts_with(&foobar));

    // (path, sibling) => false
    assert!(!f.create("/foo/wiz").starts_with(&foobar));
    assert!(!foobar.starts_with(&f.create("/foo/wiz")));
}

#[test]
fn test_normalize_unix() {
    let f = setup();
    assert_eq!(f.create("/a/b"), f.create("/a/b"));
    assert_eq!(f.create("/a/b/"), f.create("/a/b"));
    assert_eq!(f.create("/a/./b"), f.create("/a/b"));
    assert_eq!(f.create("/a/../b"), f.create("/b"));
    assert_eq!(f.create("/.."), f.create("/.."));
}

#[test]
fn test_parent_of_root_is_root_unix() {
    let f = setup();
    assert_eq!(f.create("/.."), f.create("/"));
    assert_eq!(f.create("/../../../../../.."), f.create("/"));
    assert_eq!(f.create("/../../../foo"), f.create("/foo"));
}
