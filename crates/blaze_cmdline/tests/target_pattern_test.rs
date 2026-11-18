//! Port of `TargetPatternTest`.

use blaze_cmdline::repository_mapping::EMPTY;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryMapping, RepositoryName, TargetPattern,
    TargetPatternInterpretPathAsTarget as InterpretPathAsTarget, TargetPatternParser,
    TargetPatternSingleTarget as SingleTarget,
    TargetPatternTargetsBelowDirectory as TargetsBelowDirectory,
    TargetPatternTargetsBelowDirectoryContainsResult as ContainsResult,
    TargetPatternTargetsInPackage as TargetsInPackage, TargetPatternType,
};
use blaze_vfs::PathFragment;

fn label(raw: &str) -> Label {
    Label::parse_canonical_unchecked(raw)
}

fn pkg(raw: &str) -> PackageIdentifier {
    PackageIdentifier::parse(raw).unwrap()
}

fn single(p: &str, l: &str) -> TargetPattern {
    SingleTarget::new(p, label(l)).into()
}

fn in_pkg(p: &str, d: &str, suffix: &str, abs: bool, rules_only: bool) -> TargetPattern {
    TargetsInPackage::new(p, pkg(d), suffix, abs, rules_only).into()
}

fn below(p: &str, d: &str, rules_only: bool) -> TargetPattern {
    TargetsBelowDirectory::new(p, pkg(d), rules_only).into()
}

fn path(p: &str, path: &str) -> TargetPattern {
    InterpretPathAsTarget::new(p, path).into()
}

fn canonical_repo_mapping(context: RepositoryName) -> RepositoryMapping {
    RepositoryMapping::create(
        [("repo", RepositoryName::create_unvalidated("canonical_repo"))],
        context,
    )
}

#[test]
fn valid_patterns_main_repo_at_repo_root() {
    let parser = TargetPatternParser::new(
        PathFragment::empty_fragment(),
        MAIN.clone(),
        canonical_repo_mapping(MAIN.clone()),
    );
    let p = |s: &str| parser.parse(s).unwrap();

    assert_eq!(p(":foo"), single(":foo", "@@//:foo"));
    assert_eq!(p("foo:bar"), single("foo:bar", "@@//foo:bar"));
    assert_eq!(
        p("foo:all"),
        in_pkg("foo:all", "@@//foo", "all", false, true)
    );
    assert_eq!(p("foo/...:all"), below("foo/...:all", "@@//foo", true));
    assert_eq!(p("foo:*"), in_pkg("foo:*", "@@//foo", "*", false, false));
    assert_eq!(p("foo"), path("foo", "foo"));
    assert_eq!(p("..."), below("...", "@@//", true));
    assert_eq!(p("foo/bar"), path("foo/bar", "foo/bar"));

    assert_eq!(p("//foo"), single("//foo", "@@//foo:foo"));
    assert_eq!(p("//foo:bar"), single("//foo:bar", "@@//foo:bar"));
    assert_eq!(
        p("//foo:all"),
        in_pkg("//foo:all", "@@//foo", "all", true, true)
    );
    assert_eq!(p("//foo/all"), single("//foo/all", "@@//foo/all:all"));
    assert_eq!(p("//foo/...:all"), below("//foo/...:all", "@@//foo", true));
    assert_eq!(p("//..."), below("//...", "@@//", true));

    assert_eq!(p("@repo"), single("@repo", "@@canonical_repo//:repo"));
    assert_eq!(
        p("@repo//foo:bar"),
        single("@repo//foo:bar", "@@canonical_repo//foo:bar")
    );
    assert_eq!(
        p("@repo//foo:all"),
        in_pkg("@repo//foo:all", "@@canonical_repo//foo", "all", true, true)
    );
    assert_eq!(
        p("@repo//:bar"),
        single("@repo//:bar", "@@canonical_repo//:bar")
    );
    assert_eq!(
        p("@repo//..."),
        below("@repo//...", "@@canonical_repo//", true)
    );

    assert_eq!(p("@@repo"), single("@@repo", "@@repo//:repo"));
    assert_eq!(
        p("@@repo//foo:all"),
        in_pkg("@@repo//foo:all", "@@repo//foo", "all", true, true)
    );
    assert_eq!(p("@@repo//:bar"), single("@@repo//:bar", "@@repo//:bar"));
}

#[test]
fn valid_patterns_main_repo_in_some_relative_directory() {
    let parser = TargetPatternParser::new(
        PathFragment::create("base"),
        MAIN.clone(),
        canonical_repo_mapping(MAIN.clone()),
    );
    let p = |s: &str| parser.parse(s).unwrap();

    assert_eq!(p(":foo"), single(":foo", "@@//base:foo"));
    assert_eq!(p("foo:bar"), single("foo:bar", "@@//base/foo:bar"));
    assert_eq!(
        p("foo:all"),
        in_pkg("foo:all", "@@//base/foo", "all", false, true)
    );
    assert_eq!(p("foo/...:all"), below("foo/...:all", "@@//base/foo", true));
    assert_eq!(
        p("foo:*"),
        in_pkg("foo:*", "@@//base/foo", "*", false, false)
    );
    assert_eq!(p("foo"), path("foo", "base/foo"));
    assert_eq!(p("..."), below("...", "@@//base", true));
    assert_eq!(p("foo/bar"), path("foo/bar", "base/foo/bar"));

    assert_eq!(p("//foo"), single("//foo", "@@//foo:foo"));
    assert_eq!(p("//foo:bar"), single("//foo:bar", "@@//foo:bar"));
    assert_eq!(
        p("//foo:all"),
        in_pkg("//foo:all", "@@//foo", "all", true, true)
    );
    assert_eq!(p("//foo/all"), single("//foo/all", "@@//foo/all:all"));
    assert_eq!(p("//foo/...:all"), below("//foo/...:all", "@@//foo", true));
    assert_eq!(p("//..."), below("//...", "@@//", true));

    assert_eq!(p("@repo"), single("@repo", "@@canonical_repo//:repo"));
    assert_eq!(
        p("@repo//foo:bar"),
        single("@repo//foo:bar", "@@canonical_repo//foo:bar")
    );
    assert_eq!(
        p("@repo//foo:all"),
        in_pkg("@repo//foo:all", "@@canonical_repo//foo", "all", true, true)
    );
    assert_eq!(
        p("@repo//:bar"),
        single("@repo//:bar", "@@canonical_repo//:bar")
    );
    assert_eq!(
        p("@repo//..."),
        below("@repo//...", "@@canonical_repo//", true)
    );

    assert_eq!(p("@@repo"), single("@@repo", "@@repo//:repo"));
    assert_eq!(
        p("@@repo//foo:all"),
        in_pkg("@@repo//foo:all", "@@repo//foo", "all", true, true)
    );
    assert_eq!(p("@@repo//:bar"), single("@@repo//:bar", "@@repo//:bar"));
}

#[test]
fn valid_patterns_non_main_repo() {
    let my_repo = RepositoryName::create_unvalidated("my_repo");
    let parser = TargetPatternParser::new(
        PathFragment::empty_fragment(),
        my_repo.clone(),
        canonical_repo_mapping(my_repo.clone()),
    );
    let p = |s: &str| parser.parse(s).unwrap();

    assert_eq!(p(":foo"), single(":foo", "@@my_repo//:foo"));
    assert_eq!(p("foo:bar"), single("foo:bar", "@@my_repo//foo:bar"));
    assert_eq!(
        p("foo:all"),
        in_pkg("foo:all", "@@my_repo//foo", "all", false, true)
    );
    assert_eq!(
        p("foo/...:all"),
        below("foo/...:all", "@@my_repo//foo", true)
    );
    assert_eq!(
        p("foo:*"),
        in_pkg("foo:*", "@@my_repo//foo", "*", false, false)
    );
    assert_eq!(p("foo"), single("foo", "@@my_repo//:foo"));
    assert_eq!(p("..."), below("...", "@@my_repo//", true));
    assert_eq!(p("foo/bar"), single("foo/bar", "@@my_repo//:foo/bar"));

    assert_eq!(p("//foo"), single("//foo", "@@my_repo//foo:foo"));
    assert_eq!(p("//foo:bar"), single("//foo:bar", "@@my_repo//foo:bar"));
    assert_eq!(
        p("//foo:all"),
        in_pkg("//foo:all", "@@my_repo//foo", "all", true, true)
    );
    assert_eq!(
        p("//foo/all"),
        single("//foo/all", "@@my_repo//foo/all:all")
    );
    assert_eq!(
        p("//foo/...:all"),
        below("//foo/...:all", "@@my_repo//foo", true)
    );
    assert_eq!(p("//..."), below("//...", "@@my_repo//", true));

    assert_eq!(p("@repo"), single("@repo", "@@canonical_repo//:repo"));
    assert_eq!(
        p("@repo//foo:bar"),
        single("@repo//foo:bar", "@@canonical_repo//foo:bar")
    );
    assert_eq!(
        p("@repo//foo:all"),
        in_pkg("@repo//foo:all", "@@canonical_repo//foo", "all", true, true)
    );
    assert_eq!(
        p("@repo//:bar"),
        single("@repo//:bar", "@@canonical_repo//:bar")
    );
    assert_eq!(
        p("@repo//..."),
        below("@repo//...", "@@canonical_repo//", true)
    );

    assert_eq!(p("@@repo"), single("@@repo", "@@repo//:repo"));
    assert_eq!(
        p("@@repo//foo:all"),
        in_pkg("@@repo//foo:all", "@@repo//foo", "all", true, true)
    );
    assert_eq!(p("@@repo//:bar"), single("@@repo//:bar", "@@repo//:bar"));
}

#[test]
fn invalid_patterns() {
    let bad_patterns = ["//Bar\\java", "", "/foo", "///foo", "@", "@foo//", "@@"];
    let my_repo = RepositoryName::create("my_repo").unwrap();
    for parser in [
        TargetPatternParser::new(
            PathFragment::empty_fragment(),
            MAIN.clone(),
            canonical_repo_mapping(MAIN.clone()),
        ),
        TargetPatternParser::new(
            PathFragment::create("base"),
            MAIN.clone(),
            canonical_repo_mapping(MAIN.clone()),
        ),
        TargetPatternParser::new(
            PathFragment::empty_fragment(),
            my_repo.clone(),
            canonical_repo_mapping(my_repo.clone()),
        ),
    ] {
        for pattern in bad_patterns {
            if let Ok(parsed) = parser.parse(pattern) {
                panic!(
                    "parsing should have failed for pattern \"{}\" with parser in repo {} at \
                     relative directory [{}], but succeeded with the result:\n{}",
                    pattern,
                    parser.get_current_repo(),
                    parser.get_relative_directory(),
                    parsed
                );
            }
        }
    }
}

#[test]
fn invalid_parser_non_main_repo_non_empty_relative_directory() {
    let result = std::panic::catch_unwind(|| {
        TargetPatternParser::new(
            PathFragment::create("base"),
            RepositoryName::create("my_repo").unwrap(),
            EMPTY.clone(),
        )
    });
    assert!(result.is_err());
}

#[test]
fn test_normalize() {
    // Good cases.
    assert_eq!(TargetPattern::normalize("empty"), "empty");
    assert_eq!(TargetPattern::normalize("a/b"), "a/b");
    assert_eq!(TargetPattern::normalize("a/b/c"), "a/b/c");
    assert_eq!(TargetPattern::normalize("a/b/c.d"), "a/b/c.d");
    assert_eq!(TargetPattern::normalize("a/b/c.."), "a/b/c..");
    assert_eq!(TargetPattern::normalize("a/b/c..."), "a/b/c...");

    assert_eq!(TargetPattern::normalize("a/b/"), "a/b"); // Remove trailing empty segments
    assert_eq!(TargetPattern::normalize("a//c"), "a/c"); // Remove empty inner segments
    assert_eq!(TargetPattern::normalize("a/./d"), "a/d"); // Remove inner dot segments
    assert_eq!(TargetPattern::normalize("a/."), "a"); // Remove trailing dot segments

    // Remove .. segment and its predecessor
    assert_eq!(TargetPattern::normalize("a/b/../e"), "a/e");

    // Remove trailing .. segment and its predecessor
    assert_eq!(TargetPattern::normalize("a/g/b/.."), "a/g");

    // Remove double .. segments and two predecessors
    assert_eq!(TargetPattern::normalize("a/b/c/../../h"), "a/h");

    // Don't remove leading .. segments
    assert_eq!(TargetPattern::normalize("../a"), "../a");
    assert_eq!(TargetPattern::normalize("../../a"), "../../a");
    assert_eq!(TargetPattern::normalize("../../../a"), "../../../a");
    assert_eq!(TargetPattern::normalize("a/../../../b"), "../../b");
}

fn parse_as_tbd(pattern: &str) -> TargetsBelowDirectory {
    let parsed_pattern = TargetPattern::default_parser().parse(pattern).unwrap();
    assert_eq!(
        parsed_pattern.get_type(),
        TargetPatternType::TargetsBelowDirectory
    );
    assert_eq!(parsed_pattern.get_original_pattern(), pattern);
    parsed_pattern.as_targets_below_directory().unwrap().clone()
}

#[test]
fn test_targets_below_directory_contains_colon_star() {
    // Given an outer pattern '//foo/...', that matches rules only,
    let outer_pattern = parse_as_tbd("//foo/...");
    // And a nested inner pattern '//foo/bar/...:*', that matches all targets,
    let inner_pattern = parse_as_tbd("//foo/bar/...:*");
    // Then a directory exclusion would exactly describe the subtraction of the inner pattern from
    // the outer pattern,
    assert_eq!(
        outer_pattern.contains(&inner_pattern),
        ContainsResult::DirectoryExclusionWouldBeExact
    );
    // And the inner pattern does not contain the outer pattern.
    assert_eq!(
        inner_pattern.contains(&outer_pattern),
        ContainsResult::NotContained
    );
}

#[test]
fn test_targets_below_directory_colon_star_contains() {
    // Given an outer pattern '//foo/...:*', that matches all targets,
    let outer_pattern = parse_as_tbd("//foo/...:*");
    // And a nested inner pattern '//foo/bar/...', that matches rules only,
    let inner_pattern = parse_as_tbd("//foo/bar/...");
    // Then a directory exclusion would be too broad,
    assert_eq!(
        outer_pattern.contains(&inner_pattern),
        ContainsResult::DirectoryExclusionWouldBeTooBroad
    );
    // And the inner pattern does not contain the outer pattern.
    assert_eq!(
        inner_pattern.contains(&outer_pattern),
        ContainsResult::NotContained
    );
}

#[test]
fn test_targets_below_directory_contains_nested_patterns() {
    // Given an outer pattern '//foo/...',
    let outer_pattern = parse_as_tbd("//foo/...");
    // And a nested inner pattern '//foo/bar/...',
    let inner_pattern = parse_as_tbd("//foo/bar/...");
    // Then the outer pattern contains the inner pattern,
    assert_eq!(
        outer_pattern.contains(&inner_pattern),
        ContainsResult::DirectoryExclusionWouldBeExact
    );
    // And the inner pattern does not contain the outer pattern.
    assert_eq!(
        inner_pattern.contains(&outer_pattern),
        ContainsResult::NotContained
    );
}

#[test]
fn test_targets_below_directory_is_excludable_from_for_independent_patterns() {
    // Given a pattern '//foo/...',
    let pattern_foo = parse_as_tbd("//foo/...");
    // And a pattern '//bar/...',
    let pattern_bar = parse_as_tbd("//bar/...");
    // Then neither pattern contains the other.
    assert_eq!(
        pattern_foo.contains(&pattern_bar),
        ContainsResult::NotContained
    );
    assert_eq!(
        pattern_bar.contains(&pattern_foo),
        ContainsResult::NotContained
    );
}

#[test]
fn test_targets_below_directory_does_not_contain_coincident_prefix_patterns() {
    // Given a TargetsBelowDirectory pattern, tbdFoo of '//foo/...',
    let tbd_foo = parse_as_tbd("//foo/...");
    // And a target pattern with prefix equal to the directory of the TBD pattern, but not below
    // it,
    let targets_below_directory_pattern = parse_as_tbd("//food/...");
    // Then it is not contained in the first pattern.
    assert_eq!(
        tbd_foo.contains(&targets_below_directory_pattern),
        ContainsResult::NotContained
    );
}

#[test]
fn test_depot_root_targets_below_directory_contains_patterns() {
    // Given a TargetsBelowDirectory pattern, tbdDepot of '//...',
    let tbd_depot = parse_as_tbd("//...");
    // And a target pattern for a directory,
    let tbd_foo = parse_as_tbd("//foo/...");
    // Then the pattern is contained by tbdDepot, and does not contain tbdDepot.
    assert_eq!(
        tbd_depot.contains(&tbd_foo),
        ContainsResult::DirectoryExclusionWouldBeExact
    );
    assert_eq!(tbd_foo.contains(&tbd_depot), ContainsResult::NotContained);
}
