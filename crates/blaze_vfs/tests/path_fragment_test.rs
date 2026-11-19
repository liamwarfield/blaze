//! Port of `src/test/java/com/google/devtools/build/lib/vfs/PathFragmentTest.java`.

use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};

use blaze_util::string_encoding::{latin1_decode, unicode_to_internal};
use blaze_vfs::PathFragment;

fn create(s: &str) -> PathFragment {
    PathFragment::create(s)
}

fn empty_fragment() -> PathFragment {
    PathFragment::empty_fragment()
}

fn assert_panics<R>(f: impl FnOnce() -> R) {
    assert!(
        catch_unwind(AssertUnwindSafe(f)).is_err(),
        "expected a panic (Java exception)"
    );
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Guava `EqualsTester`: members of a group are equal (with equal hashes), groups are unequal.
fn equals_tester(groups: &[Vec<PathFragment>]) {
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
fn test_equals_and_hash_code() {
    // The Java test also puts a Path in its own group (a Path never equals a PathFragment); Rust
    // types make that comparison impossible, so that group is omitted.
    equals_tester(&[
        vec![
            create("../relative/path"),
            create("..")
                .get_relative_str("relative")
                .get_relative_str("path"),
            create(
                std::path::Path::new("../relative/path")
                    .to_str()
                    .expect("utf-8"),
            ),
        ],
        vec![create("something/else")],
        vec![create("/something/else")],
        vec![create("/"), create("//////")],
        vec![create(""), empty_fragment()],
    ]);
}

#[test]
fn test_hash_code_cache() {
    let relative_path = create("../relative/path");
    let root_path = create("/");
    let old_result = hash_of(&relative_path);
    let root_result = hash_of(&root_path);
    assert_eq!(hash_of(&relative_path), old_result);
    assert_eq!(hash_of(&root_path), root_result);
}

#[test]
fn test_relative_to() {
    assert_eq!(
        create("foo/bar/baz")
            .relative_to_str("foo")
            .get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz")
            .relative_to_str("/foo")
            .get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("foo/bar/baz")
            .relative_to_str("foo/bar")
            .get_path_string(),
        "baz"
    );
    assert_eq!(
        create("/foo/bar/baz")
            .relative_to_str("/foo/bar")
            .get_path_string(),
        "baz"
    );
    assert_eq!(create("/foo").relative_to_str("/").get_path_string(), "foo");
    assert_eq!(create("foo").relative_to_str("").get_path_string(), "foo");
    assert_eq!(
        create("foo/bar").relative_to_str("").get_path_string(),
        "foo/bar"
    );
}

#[test]
fn test_is_absolute() {
    assert!(create("/absolute/test").is_absolute());
    assert!(!create("relative/test").is_absolute());
    assert!(create(std::path::Path::new("/absolute/test").to_str().unwrap()).is_absolute());
    assert!(!create(std::path::Path::new("relative/test").to_str().unwrap()).is_absolute());
}

#[test]
fn test_is_normalized() {
    assert!(PathFragment::is_normalized("/absolute/path"));
    assert!(PathFragment::is_normalized("some//path"));
    assert!(!PathFragment::is_normalized("some/./path"));
    assert!(!PathFragment::is_normalized("../some/path"));
    assert!(!PathFragment::is_normalized("./some/path"));
    assert!(!PathFragment::is_normalized("some/path/.."));
    assert!(!PathFragment::is_normalized("some/path/."));
    assert!(!PathFragment::is_normalized("some/other/../path"));
    assert!(PathFragment::is_normalized("some/other//tricky..path.."));
    assert!(PathFragment::is_normalized("/some/other//tricky..path.."));
}

#[test]
fn test_contains_up_level_references() {
    assert!(!PathFragment::contains_uplevel_references_str(
        "/absolute/path"
    ));
    assert!(!PathFragment::contains_uplevel_references_str("some//path"));
    assert!(!PathFragment::contains_uplevel_references_str(
        "some/./path"
    ));
    assert!(PathFragment::contains_uplevel_references_str(
        "../some/path"
    ));
    assert!(!PathFragment::contains_uplevel_references_str(
        "./some/path"
    ));
    assert!(PathFragment::contains_uplevel_references_str(
        "some/path/.."
    ));
    assert!(!PathFragment::contains_uplevel_references_str(
        "some/path/."
    ));
    assert!(PathFragment::contains_uplevel_references_str(
        "some/other/../path"
    ));
    assert!(!PathFragment::contains_uplevel_references_str(
        "some/other//tricky..path.."
    ));
    assert!(!PathFragment::contains_uplevel_references_str(
        "/some/other//tricky..path.."
    ));

    // Normalization cannot remove leading uplevel references, so this will be true
    assert!(create("../some/path").contains_uplevel_references());
    // Normalization will remove these, so no uplevel references left
    assert!(!create("some/path/..").contains_uplevel_references());
}

#[test]
fn test_root_node_returns_root_string() {
    let root_fragment = create("/");
    assert_eq!(root_fragment.get_path_string(), "/");
}

#[test]
fn test_get_relative() {
    assert_eq!(create("a").get_relative_str("b").get_path_string(), "a/b");
    assert_eq!(
        create("a/b").get_relative_str("c/d").get_path_string(),
        "a/b/c/d"
    );
    assert_eq!(
        create("c/d").get_relative_str("/a/b").get_path_string(),
        "/a/b"
    );
    assert_eq!(create("a").get_relative_str("").get_path_string(), "a");
    assert_eq!(create("/").get_relative_str("").get_path_string(), "/");
    assert_eq!(
        create("a/b").get_relative_str("../foo").get_path_string(),
        "a/foo"
    );
    assert_eq!(
        create("/a/b").get_relative_str("../foo").get_path_string(),
        "/a/foo"
    );

    // Make sure any fast path of PathFragment#getRelative(PathFragment) works
    assert_eq!(
        create("a/b")
            .get_relative(&create("../foo"))
            .get_path_string(),
        "a/foo"
    );
    assert_eq!(
        create("/a/b")
            .get_relative(&create("../foo"))
            .get_path_string(),
        "/a/foo"
    );

    // Make sure any fast path of PathFragment#getRelative(PathFragment) works
    assert_eq!(
        create("c/d")
            .get_relative(&create("/a/b"))
            .get_path_string(),
        "/a/b"
    );

    // Test normalization
    assert_eq!(create("a").get_relative_str(".").get_path_string(), "a");
}

#[test]
fn get_relative_absolute_path_argument_returns_same_instance() {
    for base_path in ["/c/d", "c/d"] {
        let absolute = PathFragment::create("/a/b");
        assert!(PathFragment::ptr_eq(
            &PathFragment::create(base_path).get_relative(&absolute),
            &absolute
        ));
    }
}

#[test]
fn get_relative_empty_base_path_returns_same_instance() {
    for argument in ["/a/b", "a/b"] {
        let instance = PathFragment::create(argument);
        assert!(PathFragment::ptr_eq(
            &empty_fragment().get_relative(&instance),
            &instance
        ));
    }
}

#[test]
fn test_is_normalized_relative_path() {
    assert!(!PathFragment::is_normalized_relative_path("/a"));
    assert!(!PathFragment::is_normalized_relative_path("a///b"));
    assert!(!PathFragment::is_normalized_relative_path("../a"));
    assert!(!PathFragment::is_normalized_relative_path("a/../b"));
    assert!(PathFragment::is_normalized_relative_path("a/b"));
    assert!(PathFragment::is_normalized_relative_path("ab"));
}

#[test]
fn test_contains_separator() {
    assert!(PathFragment::contains_separator("/a"));
    assert!(PathFragment::contains_separator("a///b"));
    assert!(PathFragment::contains_separator("../a"));
    assert!(PathFragment::contains_separator("a/../b"));
    assert!(PathFragment::contains_separator("a/b"));
    assert!(!PathFragment::contains_separator("ab"));
}

#[test]
fn test_get_child_works() {
    let pf = create("../some/path");
    assert_eq!(pf.get_child("hi"), create("../some/path/hi"));
    assert_eq!(pf.get_child("h\\i"), create("../some/path/h\\i"));
    assert_eq!(
        create("../some/path").get_child(".hi"),
        create("../some/path/.hi")
    );
    assert_eq!(
        create("../some/path").get_child("..hi"),
        create("../some/path/..hi")
    );
}

#[test]
fn test_get_child_rejects_invalid_base_names() {
    let pf = create("../some/path");
    assert_panics(|| pf.get_child("."));
    assert_panics(|| pf.get_child(".."));
    assert_panics(|| pf.get_child("x/y"));
    assert_panics(|| pf.get_child("/y"));
    assert_panics(|| pf.get_child("y/"));
    assert_panics(|| pf.get_child(""));
}

#[test]
fn test_empty_path_to_empty_path() {
    assert_eq!(create("/").get_path_string(), "/");
    assert_eq!(create("").get_path_string(), "");
}

#[test]
fn test_redundant_slashes() {
    assert_eq!(create("///").get_path_string(), "/");
    assert_eq!(create("/foo///bar").get_path_string(), "/foo/bar");
    assert_eq!(create("////foo//bar").get_path_string(), "/foo/bar");
}

#[test]
fn test_simple_name_to_simple_name() {
    assert_eq!(create("/foo").get_path_string(), "/foo");
    assert_eq!(create("foo").get_path_string(), "foo");
}

#[test]
fn test_simple_path_to_simple_path() {
    assert_eq!(create("/foo/bar").get_path_string(), "/foo/bar");
    assert_eq!(create("foo/bar").get_path_string(), "foo/bar");
}

#[test]
fn test_strips_trailing_slash() {
    assert_eq!(create("/foo/bar/").get_path_string(), "/foo/bar");
}

#[test]
fn test_get_parent_directory() {
    let foo_bar_wiz = create("foo/bar/wiz");
    let foo_bar = create("foo/bar");
    let foo = create("foo");
    let empty = create("");
    assert_eq!(foo_bar_wiz.get_parent_directory(), Some(foo_bar.clone()));
    assert_eq!(foo_bar.get_parent_directory(), Some(foo.clone()));
    assert_eq!(foo.get_parent_directory(), Some(empty.clone()));
    assert_eq!(empty.get_parent_directory(), None);

    let foo_bar_wiz_abs = create("/foo/bar/wiz");
    let foo_bar_abs = create("/foo/bar");
    let foo_abs = create("/foo");
    let root_abs = create("/");
    assert_eq!(
        foo_bar_wiz_abs.get_parent_directory(),
        Some(foo_bar_abs.clone())
    );
    assert_eq!(foo_bar_abs.get_parent_directory(), Some(foo_abs.clone()));
    assert_eq!(foo_abs.get_parent_directory(), Some(root_abs.clone()));
    assert_eq!(root_abs.get_parent_directory(), None);
}

#[test]
fn test_segments_count() {
    assert_eq!(create("foo/bar").segment_count(), 2);
    assert_eq!(create("/foo/bar").segment_count(), 2);
    assert_eq!(create("foo//bar").segment_count(), 2);
    assert_eq!(create("/foo//bar").segment_count(), 2);
    assert_eq!(create("foo/").segment_count(), 1);
    assert_eq!(create("/foo/").segment_count(), 1);
    assert_eq!(create("foo").segment_count(), 1);
    assert_eq!(create("/foo").segment_count(), 1);
    assert_eq!(create("/").segment_count(), 0);
    assert_eq!(create("").segment_count(), 0);
}

#[test]
fn is_single_segment_true() {
    for path in ["/foo", "foo"] {
        assert!(create(path).is_single_segment(), "{}", path);
    }
}

#[test]
fn is_single_segment_false() {
    for path in [
        "/",
        "",
        "/foo/bar",
        "foo/bar",
        "/foo/bar/baz",
        "foo/bar/baz",
    ] {
        assert!(!create(path).is_single_segment(), "{}", path);
    }
}

#[test]
fn is_multi_segment_true() {
    for path in ["/foo/bar", "foo/bar", "/foo/bar/baz", "foo/bar/baz"] {
        assert!(create(path).is_multi_segment(), "{}", path);
    }
}

#[test]
fn is_multi_segment_false() {
    for path in ["/", "", "/foo", "foo"] {
        assert!(!create(path).is_multi_segment(), "{}", path);
    }
}

#[test]
fn test_get_segment() {
    assert_eq!(create("foo/bar").get_segment(0), "foo");
    assert_eq!(create("foo/bar").get_segment(1), "bar");
    assert_eq!(create("/foo/bar").get_segment(0), "foo");
    assert_eq!(create("/foo/bar").get_segment(1), "bar");
    assert_eq!(create("foo/").get_segment(0), "foo");
    assert_eq!(create("/foo/").get_segment(0), "foo");
    assert_eq!(create("foo").get_segment(0), "foo");
    assert_eq!(create("/foo").get_segment(0), "foo");
}

#[test]
fn segments() {
    let p = create("/this/is/a/path");
    let segs: Vec<&str> = p.segments().collect();
    assert_eq!(segs, vec!["this", "is", "a", "path"]);
}

#[test]
fn test_basename() {
    assert_eq!(create("foo/bar").get_base_name(), "bar");
    assert_eq!(create("/foo/bar").get_base_name(), "bar");
    assert_eq!(create("foo/").get_base_name(), "foo");
    assert_eq!(create("/foo/").get_base_name(), "foo");
    assert_eq!(create("foo").get_base_name(), "foo");
    assert_eq!(create("/foo").get_base_name(), "foo");
    assert!(create("/").get_base_name().is_empty());
    assert!(create("").get_base_name().is_empty());
}

#[test]
fn test_file_extension() {
    assert_eq!(create("foo.bar").get_file_extension(), "bar");
    assert_eq!(create("foo.barr").get_file_extension(), "barr");
    assert_eq!(create("foo.b").get_file_extension(), "b");
    assert!(create("foo.").get_file_extension().is_empty());
    assert!(create("foo").get_file_extension().is_empty());
    assert!(create(".").get_file_extension().is_empty());
    assert!(create("").get_file_extension().is_empty());
    assert_eq!(create("foo/bar.baz").get_file_extension(), "baz");
    assert_eq!(create("foo.bar.baz").get_file_extension(), "baz");
    assert!(create("foo.bar/baz").get_file_extension().is_empty());
}

fn replace(p: &str, name: &str) -> Option<String> {
    create(p)
        .replace_name(name)
        .map(|f| f.get_path_string().to_owned())
}

#[test]
fn test_replace_name() {
    assert_eq!(replace("foo/bar", "baz").as_deref(), Some("foo/baz"));
    assert_eq!(replace("/foo/bar", "baz").as_deref(), Some("/foo/baz"));
    assert_eq!(replace("foo/bar", "").as_deref(), Some("foo"));
    assert_eq!(replace("foo/", "baz").as_deref(), Some("baz"));
    assert_eq!(replace("/foo/", "baz").as_deref(), Some("/baz"));
    assert_eq!(replace("foo", "baz").as_deref(), Some("baz"));
    assert_eq!(replace("/foo", "baz").as_deref(), Some("/baz"));
    assert_eq!(replace("/", "baz"), None);
    assert_eq!(replace("/", ""), None);
    assert_eq!(replace("", "baz"), None);
    assert_eq!(replace("", ""), None);

    assert_eq!(
        replace("foo/bar", "bar/baz").as_deref(),
        Some("foo/bar/baz")
    );
    assert_eq!(
        replace("foo/bar", "bar/baz/").as_deref(),
        Some("foo/bar/baz")
    );

    // Absolute path arguments will clobber the original path.
    assert_eq!(
        replace("foo/bar", "/absolute").as_deref(),
        Some("/absolute")
    );
    assert_eq!(replace("foo/bar", "/").as_deref(), Some("/"));
}

#[test]
fn test_sub_fragment() {
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(0, 3).get_path_string(),
        "/foo/bar/baz"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment(0, 3).get_path_string(),
        "foo/bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(0, 2).get_path_string(),
        "/foo/bar"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(1, 3).get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(0, 1).get_path_string(),
        "/foo"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(1, 2).get_path_string(),
        "bar"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(2, 3).get_path_string(),
        "baz"
    );
    assert_eq!(
        create("/foo/bar/baz").sub_fragment(0, 0).get_path_string(),
        "/"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment(0, 0).get_path_string(),
        ""
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment(1, 1).get_path_string(),
        ""
    );

    assert_eq!(
        create("/foo/bar/baz")
            .sub_fragment_from(0)
            .get_path_string(),
        "/foo/bar/baz"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment_from(0).get_path_string(),
        "foo/bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz")
            .sub_fragment_from(1)
            .get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment_from(1).get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment_from(2).get_path_string(),
        "baz"
    );
    assert_eq!(
        create("foo/bar/baz").sub_fragment_from(3).get_path_string(),
        ""
    );

    assert_panics(|| create("foo/bar/baz").sub_fragment(3, 2));
    assert_panics(|| create("foo/bar/baz").sub_fragment(4, 4));
    assert_panics(|| create("foo/bar/baz").sub_fragment(3, 2));
    assert_panics(|| create("foo/bar/baz").sub_fragment_from(4));
}

#[test]
fn test_strip_components() {
    let path_fragment_strip_zero = create("/foo/bar/baz");
    assert!(PathFragment::ptr_eq(
        &path_fragment_strip_zero.strip_components(0),
        &path_fragment_strip_zero
    ));

    assert_eq!(
        create("/foo/bar/baz").strip_components(0).get_path_string(),
        "/foo/bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz").strip_components(1).get_path_string(),
        "bar/baz"
    );
    assert_eq!(
        create("/foo/bar/baz").strip_components(2).get_path_string(),
        "baz"
    );
    assert_eq!(
        create("/foo/bar/baz").strip_components(3).get_path_string(),
        ""
    );
    assert_eq!(
        create("/foo/bar/baz").strip_components(4).get_path_string(),
        ""
    );

    let path_fragment_strip_all = create("/foo/bar/baz");
    assert!(PathFragment::ptr_eq(
        &path_fragment_strip_all.strip_components(3),
        &empty_fragment()
    ));
    assert!(PathFragment::ptr_eq(
        &path_fragment_strip_all.strip_components(4),
        &empty_fragment()
    ));

    let path_fragment = create("/foo/bar/baz");
    assert_panics(|| path_fragment.strip_components(-2));
}

#[test]
fn test_starts_with() {
    let foobar = create("/foo/bar");
    let foobar_relative = create("foo/bar");

    // (path, prefix) => true
    assert!(foobar.starts_with(&foobar));
    assert!(foobar.starts_with(&create("/")));
    assert!(foobar.starts_with(&create("/foo")));
    assert!(foobar.starts_with(&create("/foo/")));
    assert!(foobar.starts_with(&create("/foo/bar/"))); // Includes trailing slash.

    // (prefix, path) => false
    assert!(!create("/foo").starts_with(&foobar));
    assert!(!create("/").starts_with(&foobar));

    // (absolute, relative) => false
    assert!(!foobar.starts_with(&foobar_relative));
    assert!(!foobar_relative.starts_with(&foobar));

    // (relative path, relative prefix) => true
    assert!(foobar_relative.starts_with(&foobar_relative));
    assert!(foobar_relative.starts_with(&create("foo")));
    assert!(foobar_relative.starts_with(&create("")));

    // (path, sibling) => false
    assert!(!create("/foo/wiz").starts_with(&foobar));
    assert!(!foobar.starts_with(&create("/foo/wiz")));

    // (path, different case) => false
    assert!(!foobar.starts_with(&create("/Foo/bar")));
    assert!(!foobar.starts_with(&create("/Foo")));
    assert!(
        !create(&unicode_to_internal("/ÄÖÜ/bar"))
            .starts_with(&create(&unicode_to_internal("/äöü")))
    );
    assert!(
        !create(&unicode_to_internal("ÄÖÜ/bar")).starts_with(&create(&unicode_to_internal("äöü")))
    );
}

#[test]
fn test_starts_with_ignoring_case() {
    let foobar = create("/foo/bar");
    let foobar_relative = create("foo/bar");

    // (path, prefix) => true
    assert!(foobar.starts_with_ignoring_case(&foobar));
    assert!(foobar.starts_with_ignoring_case(&create("/")));
    assert!(foobar.starts_with_ignoring_case(&create("/foo")));
    assert!(foobar.starts_with_ignoring_case(&create("/foo/")));
    assert!(foobar.starts_with_ignoring_case(&create("/foo/bar/"))); // Includes trailing slash.

    // (prefix, path) => false
    assert!(!create("/foo").starts_with_ignoring_case(&foobar));
    assert!(!create("/").starts_with_ignoring_case(&foobar));

    // (absolute, relative) => false
    assert!(!foobar.starts_with_ignoring_case(&foobar_relative));
    assert!(!foobar_relative.starts_with_ignoring_case(&foobar));

    // (relative path, relative prefix) => true
    assert!(foobar_relative.starts_with_ignoring_case(&foobar_relative));
    assert!(foobar_relative.starts_with_ignoring_case(&create("foo")));
    assert!(foobar_relative.starts_with_ignoring_case(&create("")));

    // (path, sibling) => false
    assert!(!create("/foo/wiz").starts_with_ignoring_case(&foobar));
    assert!(!foobar.starts_with_ignoring_case(&create("/foo/wiz")));

    // (path, different case) => true
    assert!(foobar.starts_with_ignoring_case(&create("/Foo/bar")));
    assert!(foobar.starts_with_ignoring_case(&create("/Foo")));
    assert!(
        create(&unicode_to_internal("/ÄÖÜ/bar"))
            .starts_with_ignoring_case(&create(&unicode_to_internal("/äöü")))
    );
    assert!(
        create(&unicode_to_internal("ÄÖÜ/bar"))
            .starts_with_ignoring_case(&create(&unicode_to_internal("äöü")))
    );

    // Byte-strings review 2026-11-20 S2-2 (Java PathFragment.java:442-443 converts both paths
    // with internalToUnicode first): `/inc/é` starts with `/inc/É`; the bytes E3 A9 do not
    // start with C3 A9 although U+00E3 and U+00C3 are the two cases of one Latin-1 char.
    assert!(
        create(&latin1_decode("/inc/é".as_bytes()))
            .starts_with_ignoring_case(&create(&latin1_decode("/inc/É".as_bytes())))
    );
    assert!(
        !create(&latin1_decode(b"/inc/\xe3\xa9"))
            .starts_with_ignoring_case(&create(&latin1_decode(b"/inc/\xc3\xa9")))
    );
}

fn to_paths_set(strs: &[&str]) -> Vec<PathFragment> {
    let mut out: Vec<PathFragment> = Vec::new();
    for s in strs {
        let p = create(s);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

#[test]
fn test_check_all_paths_start_with_but_are_not_equal_to() {
    // Check passes:
    PathFragment::check_all_paths_are_under(&to_paths_set(&["a/b", "a/c"]), &create("a"));

    // Check trivially passes:
    PathFragment::check_all_paths_are_under(&Vec::<PathFragment>::new(), &create("a"));

    // Check fails when some path does not start with startingWithPath:
    assert_panics(|| {
        PathFragment::check_all_paths_are_under(&to_paths_set(&["a/b", "b/c"]), &create("a"))
    });

    // Check fails when some path is equal to startingWithPath:
    assert_panics(|| {
        PathFragment::check_all_paths_are_under(&to_paths_set(&["a/b", "a"]), &create("a"))
    });
}

#[test]
fn test_ends_with() {
    let foobar = create("/foo/bar");
    let foobar_relative = create("foo/bar");

    // (path, suffix) => true
    assert!(foobar.ends_with(&foobar));
    assert!(foobar.ends_with(&create("bar")));
    assert!(foobar.ends_with(&create("foo/bar")));
    assert!(foobar.ends_with(&create("/foo/bar")));
    assert!(!foobar.ends_with(&create("/bar")));

    // (prefix, path) => false
    assert!(!create("/foo").ends_with(&foobar));
    assert!(!create("/").ends_with(&foobar));

    // (suffix, path) => false
    assert!(!create("/bar").ends_with(&foobar));
    assert!(!create("bar").ends_with(&foobar));
    assert!(!create("").ends_with(&foobar));

    // (absolute, relative) => true
    assert!(foobar.ends_with(&foobar_relative));

    // (relative, absolute) => false
    assert!(!foobar_relative.ends_with(&foobar));

    // (relative path, relative prefix) => true
    assert!(foobar_relative.ends_with(&foobar_relative));
    assert!(foobar_relative.ends_with(&create("bar")));
    assert!(foobar_relative.ends_with(&create("")));

    // (path, sibling) => false
    assert!(!create("/foo/wiz").ends_with(&foobar));
    assert!(!foobar.ends_with(&create("/foo/wiz")));
}

#[test]
fn test_to_relative() {
    assert_eq!(create("/foo/bar").to_relative(), create("foo/bar"));
    assert_eq!(create("/").to_relative(), create(""));
    assert_panics(|| create("foo").to_relative());
}

#[test]
fn test_get_drive_str() {
    assert_eq!(create("/foo/bar").get_drive_str(), "/");
    assert_eq!(create("/").get_drive_str(), "/");
    assert_panics(|| create("foo").get_drive_str().to_owned());
}

fn to_paths(strs: &[&str]) -> Vec<PathFragment> {
    strs.iter().map(|s| create(s)).collect()
}

const COMPARE_PATHS: [&str; 11] = [
    "",
    "/",
    "foo",
    "/foo",
    "foo/bar",
    "foo.bar",
    "foo/bar.baz",
    "foo/bar/baz",
    "foo/barfile",
    "foo/Bar",
    "Foo/bar",
];

fn signum(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

fn check_comparator_consistency(
    paths: &[PathFragment],
    cmp: impl Fn(&PathFragment, &PathFragment) -> Ordering,
) {
    for x in paths {
        for y in paths {
            for z in paths {
                // Anti-symmetry
                assert_eq!(-signum(cmp(y, x)), signum(cmp(x, y)));
                // Transitivity
                if cmp(x, y) == Ordering::Greater && cmp(y, z) == Ordering::Greater {
                    assert_eq!(cmp(x, z), Ordering::Greater);
                }
                // "Substitutability"
                if cmp(x, y) == Ordering::Equal {
                    assert_eq!(signum(cmp(y, z)), signum(cmp(x, z)));
                }
                // Consistency with equals
                assert_eq!(x == y, cmp(x, y) == Ordering::Equal);
            }
        }
    }
}

// Deterministic stand-in for Collections.shuffle.
fn shuffle(paths: &mut [PathFragment]) {
    paths.reverse();
    let n = paths.len();
    for i in 0..n {
        paths.swap(i, (i * 7 + 3) % n);
    }
}

#[test]
fn test_compare_to() {
    let mut paths = to_paths(&COMPARE_PATHS);
    // First test that compareTo is self-consistent.
    check_comparator_consistency(&paths, |a, b| a.cmp(b));
    // Now test that compareTo does what we expect.  The exact ordering here doesn't matter much.
    shuffle(&mut paths);
    paths.sort();
    let expected_order = to_paths(&[
        "",
        "/",
        "/foo",
        "Foo/bar",
        "foo",
        "foo.bar",
        "foo/Bar",
        "foo/bar",
        "foo/bar.baz",
        "foo/bar/baz",
        "foo/barfile",
    ]);
    assert_eq!(paths, expected_order);
}

#[test]
fn test_hierarchical_comparator() {
    let mut paths = to_paths(&COMPARE_PATHS);
    // First test that compareTo is self-consistent.
    check_comparator_consistency(&paths, PathFragment::hierarchical_compare);
    // Now test that compareTo does what we expect.  The exact ordering here doesn't matter much.
    shuffle(&mut paths);
    paths.sort_by(PathFragment::hierarchical_compare);
    let expected_order = to_paths(&[
        "",
        "/",
        "/foo",
        "Foo/bar",
        "foo",
        "foo/Bar",
        "foo/bar",
        "foo/bar/baz",
        "foo/bar.baz",
        "foo/barfile",
        "foo.bar",
    ]);
    assert_eq!(paths, expected_order);
}

#[test]
fn test_get_safe_path_string() {
    assert_eq!(create("/").get_safe_path_string(), "/");
    assert_eq!(create("/abc").get_safe_path_string(), "/abc");
    assert_eq!(create("").get_safe_path_string(), ".");
    assert_eq!(empty_fragment().get_safe_path_string(), ".");
    assert_eq!(create("abc/def").get_safe_path_string(), "abc/def");
}

#[test]
fn test_normalize() {
    assert_eq!(create("/a/b"), create("/a/b"));
    assert_eq!(create("/a/./b"), create("/a/b"));
    assert_eq!(create("/a/../b"), create("/b"));
    assert_eq!(create("a/b"), create("a/b"));
    assert_eq!(create("a/../../b"), create("../b"));
    assert_eq!(create("a/../.."), create(".."));
    assert_eq!(create("a/../b"), create("b"));
    assert_eq!(create("a/b/../b"), create("a/b"));
    assert_eq!(create("/.."), create("/.."));
    assert_eq!(create(".."), create(".."));
}

#[test]
fn test_segments() {
    assert_eq!(create("").segment_count(), 0);
    assert_eq!(create("a").segment_count(), 1);
    assert_eq!(create("a/b").segment_count(), 2);
    assert_eq!(create("a/b/c").segment_count(), 3);
    assert_eq!(create("/").segment_count(), 0);
    assert_eq!(create("/a").segment_count(), 1);
    assert_eq!(create("/a/b").segment_count(), 2);
    assert_eq!(create("/a/b/c").segment_count(), 3);

    assert!(create("").split_to_list_of_segments().is_empty());
    assert_eq!(create("a").split_to_list_of_segments(), vec!["a"]);
    assert_eq!(create("a/b").split_to_list_of_segments(), vec!["a", "b"]);
    assert_eq!(
        create("a/b/c").split_to_list_of_segments(),
        vec!["a", "b", "c"]
    );

    assert!(create("/").split_to_list_of_segments().is_empty());
    assert_eq!(create("/a").split_to_list_of_segments(), vec!["a"]);
    assert_eq!(create("/a/b").split_to_list_of_segments(), vec!["a", "b"]);
    assert_eq!(
        create("/a/b/c").split_to_list_of_segments(),
        vec!["a", "b", "c"]
    );

    assert_eq!(create("a").get_segment(0), "a");
    assert_eq!(create("a/b").get_segment(0), "a");
    assert_eq!(create("a/b").get_segment(1), "b");
    assert_eq!(create("a/b/c").get_segment(2), "c");

    assert_eq!(create("/a").get_segment(0), "a");
    assert_eq!(create("/a/b").get_segment(0), "a");
    assert_eq!(create("/a/b").get_segment(1), "b");
    assert_eq!(create("/a/b/c").get_segment(2), "c");

    assert_panics(|| create("").get_segment(0).to_owned());
    assert_panics(|| create("a/b").get_segment(2).to_owned());
}

#[test]
#[ignore = "no serialization infrastructure (SerializationTester) yet"]
fn test_codec() {}

#[test]
#[ignore = "no serialization infrastructure (RoundTripping) yet"]
fn test_serialization_simple() {}

#[test]
#[ignore = "no serialization infrastructure (RoundTripping) yet"]
fn test_serialization_absolute() {}

#[test]
#[ignore = "no serialization infrastructure (RoundTripping) yet"]
fn test_serialization_nested() {}

#[test]
fn contains_uplevel_reference_empty_path_returns_false() {
    assert!(!empty_fragment().contains_uplevel_references());
}

#[test]
fn contains_uplevel_reference_uplevel_only_path_returns_true() {
    let path_fragment = create("..");
    assert!(path_fragment.contains_uplevel_references());
}

#[test]
fn contains_uplevel_references_first_segment_starting_with_dot_dot_returns_false() {
    let path_fragment = create("..file");
    assert!(!path_fragment.contains_uplevel_references());
}

#[test]
fn contains_uplevel_references_starts_with_uplevel_reference_returns_true() {
    let path_fragment = create("../file");
    assert!(path_fragment.contains_uplevel_references());
}

#[test]
fn contains_uplevel_references_uplevel_reference_mid_path_normalizes_and_returns_false() {
    let path_fragment = create("a/../b");
    assert!(!path_fragment.contains_uplevel_references());
    assert_eq!(path_fragment.get_path_string(), "b");
}

#[test]
fn contains_uplevel_referenes_uplevel_reference_mid_global_path_normalizes_and_returns_false() {
    let path_fragment = create("/dir1/dir2/../file");
    assert!(!path_fragment.contains_uplevel_references());
    assert_eq!(path_fragment.get_path_string(), "/dir1/file");
}
