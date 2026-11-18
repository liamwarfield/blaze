//! Port of `LabelParserTest` (src/test/java/com/google/devtools/build/lib/cmdline).

use blaze_cmdline::LabelParserParts;

fn parse(raw: &str) -> LabelParserParts {
    LabelParserParts::parse(raw).unwrap()
}

fn validate_and_create(
    repo: Option<&str>,
    repo_is_canonical: bool,
    pkg_is_absolute: bool,
    pkg: &str,
    pkg_ends_with_triple_dots: bool,
    target: &str,
    raw: &str,
) -> LabelParserParts {
    LabelParserParts::validate_and_create(
        repo,
        repo_is_canonical,
        pkg_is_absolute,
        pkg,
        pkg_ends_with_triple_dots,
        target,
        raw,
    )
    .unwrap()
}

/// Checks that the doc table of `LabelParserParts::parse` is accurate.
#[test]
fn parser_table() {
    assert_eq!(
        parse("foo/bar"),
        validate_and_create(None, false, false, "", false, "foo/bar", "foo/bar")
    );
    assert_eq!(
        parse("..."),
        validate_and_create(None, false, false, "", true, "", "...")
    );
    assert_eq!(
        parse("...:all"),
        validate_and_create(None, false, false, "", true, "all", "...:all")
    );
    assert_eq!(
        parse("foo/..."),
        validate_and_create(None, false, false, "foo", true, "", "foo/...")
    );
    assert_eq!(
        parse("//foo/bar"),
        validate_and_create(None, false, true, "foo/bar", false, "bar", "//foo/bar")
    );
    assert_eq!(
        parse("//foo/..."),
        validate_and_create(None, false, true, "foo", true, "", "//foo/...")
    );
    assert_eq!(
        parse("//foo/...:all"),
        validate_and_create(None, false, true, "foo", true, "all", "//foo/...:all")
    );
    assert_eq!(
        parse("//foo/all"),
        validate_and_create(None, false, true, "foo/all", false, "all", "//foo/all")
    );
    assert_eq!(
        parse("@repo"),
        validate_and_create(Some("repo"), false, true, "", false, "repo", "@repo")
    );
    assert_eq!(
        parse("@@repo"),
        validate_and_create(Some("repo"), true, true, "", false, "repo", "@@repo")
    );
    assert_eq!(
        parse("@repo//foo/bar"),
        validate_and_create(
            Some("repo"),
            false,
            true,
            "foo/bar",
            false,
            "bar",
            "@repo//foo/bar"
        )
    );
    assert_eq!(
        parse("@@repo//foo/bar"),
        validate_and_create(
            Some("repo"),
            true,
            true,
            "foo/bar",
            false,
            "bar",
            "@@repo//foo/bar"
        )
    );
    assert_eq!(
        parse(":quux"),
        validate_and_create(None, false, false, "", false, "quux", ":quux")
    );
    assert_eq!(
        parse("foo/bar:quux"),
        validate_and_create(None, false, false, "foo/bar", false, "quux", "foo/bar:quux")
    );
    assert_eq!(
        parse("//foo/bar:quux"),
        validate_and_create(
            None,
            false,
            true,
            "foo/bar",
            false,
            "quux",
            "//foo/bar:quux"
        )
    );
    assert_eq!(
        parse("@repo//foo/bar:quux"),
        validate_and_create(
            Some("repo"),
            false,
            true,
            "foo/bar",
            false,
            "quux",
            "@repo//foo/bar:quux"
        )
    );
}
