//! Port of `src/test/java/com/google/devtools/build/lib/collect/SimpleTargetPatternMatcherTest.java`.
//!
//! The Java test is parameterized (`TargetPatternProvider`); each test runs every case.

use blaze_cmdline::Label;
use blaze_sky_loading::simple_target_pattern_matcher::SimpleTargetPatternMatcher;

/// `TargetPatternProvider.provideValues`: (included, patterns, label).
fn cases() -> Vec<(bool, Vec<&'static str>, &'static str)> {
    vec![
        // Single pattern
        (true, vec!["//foo:foo"], "//foo:foo"),
        (true, vec!["//foo:foo"], "//foo"),
        (true, vec!["//foo"], "//foo:foo"),
        (true, vec!["//foo"], "//foo"),
        (false, vec!["//foo:foo"], "//foo:bar"),
        (false, vec!["//foo"], "//foo:bar"),
        (true, vec!["//foo/..."], "//foo:foo"),
        (true, vec!["//foo/..."], "//foo/bar:bar"),
        (false, vec!["//foo/..."], "//bar:bar"),
        (false, vec!["//foo/bar/..."], "//foo:foo"),
        (false, vec!["//foo"], "//fooooooo"),
        (false, vec!["//foo/..."], "//fooooooo"),
        // Multiple patterns
        (true, vec!["//foo:foo", "//bar:bar"], "//foo:foo"),
        (true, vec!["//foo:foo", "//bar:bar"], "//bar:bar"),
        (false, vec!["//foo:foo", "//bar:bar"], "//quux:quux"),
        // Negative patterns
        (false, vec!["-//foo:foo"], "//foo:foo"),
        (false, vec!["-//foo/..."], "//foo:foo"),
        (false, vec!["//foo/...", "-//foo/bar/..."], "//foo/bar:bar"),
        (true, vec!["//foo/...", "-//foo/bar/..."], "//foo:foo"),
        (
            true,
            vec!["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo/bar/baz",
        ),
        (
            true,
            vec!["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo:foo",
        ),
        (
            false,
            vec!["//foo/...", "-//foo/bar/...", "//foo/bar/baz/..."],
            "//foo/bar/quux",
        ),
    ]
}

fn patterns(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

#[test]
fn contains() {
    for (included, p, label) in cases() {
        let matcher = SimpleTargetPatternMatcher::create(&patterns(&p)).unwrap();
        let label = Label::parse_canonical_unchecked(label);
        assert_eq!(
            matcher.contains(&label),
            included,
            "matcher {} contains {}",
            matcher,
            label
        );
    }
}

#[test]
fn to_string() {
    for (_, p, _) in cases() {
        let matcher = SimpleTargetPatternMatcher::create(&patterns(&p)).unwrap();
        assert_eq!(matcher.to_string(), format!("[{}]", p.join(",")));
    }
}
