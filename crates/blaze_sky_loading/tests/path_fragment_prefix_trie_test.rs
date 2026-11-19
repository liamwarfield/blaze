//! Port of `src/test/java/com/google/devtools/build/lib/collect/PathFragmentPrefixTrieTest.java`.

use blaze_sky_loading::path_fragment_prefix_trie::PathFragmentPrefixTrie;
use blaze_vfs::PathFragment;

fn p(s: &str) -> PathFragment {
    PathFragment::create(s)
}

#[test]
fn test_empty() {
    let trie = PathFragmentPrefixTrie::new();
    assert!(!trie.includes(&p("a")));
    assert!(!trie.includes(&p("a/b")));
}

#[test]
#[should_panic(expected = "path fragment cannot be the empty fragment.")]
fn test_empty_path_fragment_is_disallowed_for_put() {
    let mut trie = PathFragmentPrefixTrie::new();
    let _ = trie.put(&PathFragment::empty_fragment(), true);
}

#[test]
fn test_simple_inclusions() {
    let mut trie = PathFragmentPrefixTrie::new();
    trie.put(&p("a"), true).unwrap();
    trie.put(&p("b"), true).unwrap();
    assert!(!trie.includes(&PathFragment::empty_fragment()));
    assert!(trie.includes(&p("a")));
    assert!(trie.includes(&p("a/b")));
    assert!(trie.includes(&p("a/b/c")));
    assert!(trie.includes(&p("b")));
    assert!(!trie.includes(&p("c")));
}

#[test]
fn test_simple_exclusions() {
    let mut trie = PathFragmentPrefixTrie::new();
    trie.put(&p("a"), true).unwrap();
    trie.put(&p("a/b"), false).unwrap();
    trie.put(&p("a/b/c"), true).unwrap();
    assert!(trie.includes(&p("a")));
    assert!(!trie.includes(&p("a/b")));
    assert!(trie.includes(&p("a/b/c")));
    assert!(!trie.includes(&p("a/b/d")));
}

#[test]
fn test_ancestors() {
    let mut trie = PathFragmentPrefixTrie::new();
    trie.put(&p("a/b/c"), true).unwrap();
    assert!(!trie.includes(&p("a")));
    assert!(!trie.includes(&p("a/b")));
    assert!(trie.includes(&p("a/b/c"))); // toggled explicitly
    assert!(trie.includes(&p("a/b/c/d"))); // toggled automatically

    trie.put(&p("a/b/c/d"), false).unwrap();
    assert!(!trie.includes(&p("a")));
    assert!(!trie.includes(&p("a/b")));
    assert!(trie.includes(&p("a/b/c")));
    assert!(!trie.includes(&p("a/b/c/d"))); // toggled explicitly

    trie.put(&p("a"), true).unwrap();
    assert!(trie.includes(&p("a"))); // toggled explicitly
    assert!(trie.includes(&p("a/b"))); // toggled automatically
    assert!(trie.includes(&p("a/b/c")));
    assert!(!trie.includes(&p("a/b/c/d")));

    trie.put(&p("a/b"), false).unwrap();
    assert!(trie.includes(&p("a")));
    assert!(!trie.includes(&p("a/b"))); // toggled explicitly
    assert!(trie.includes(&p("a/b/c")));
    assert!(!trie.includes(&p("a/b/c/d")));
}

#[test]
fn test_same_path_fragment_included_and_excluded_is_disallowed() {
    let mut trie = PathFragmentPrefixTrie::new();
    trie.put(&p("a/b/c"), false).unwrap();
    let e = trie.put(&p("a/b/c"), true).unwrap_err();
    assert!(e.0.contains(
        "a/b/c has already been explicitly marked as excluded. Current state: [included: [], excluded: [a/b/c]]"
    ));

    trie.put(&p("a/b"), true).unwrap();
    let e = trie.put(&p("a/b"), false).unwrap_err();
    assert!(e.0.contains(
        "a/b has already been explicitly marked as included. Current state: [included: [a/b], excluded: [a/b/c]]"
    ));
}

#[test]
fn test_string_repr() {
    let mut trie = PathFragmentPrefixTrie::new();
    trie.put(&p("a"), true).unwrap();
    trie.put(&p("a/b"), false).unwrap();
    trie.put(&p("a/b/c"), true).unwrap();
    trie.put(&p("a/b/d"), false).unwrap();
    trie.put(&p("e"), true).unwrap();
    assert_eq!(
        trie.to_string(),
        "[included: [a, a/b/c, e], excluded: [a/b, a/b/d]]"
    );
}

#[test]
fn test_has_included_paths() {
    let mut trie = PathFragmentPrefixTrie::new();
    assert!(!trie.has_included_paths());
    trie.put(&p("a"), true).unwrap();
    assert!(trie.has_included_paths());
}
