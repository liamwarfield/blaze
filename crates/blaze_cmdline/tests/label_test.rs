//! Port of `LabelTest`.

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{
    Label, LabelPackageContext, LabelRepoContextImpl, LabelSimpleRepoMappingRecorder,
    LabelSyntaxException, PackageIdentifier, RepositoryMapping, RepositoryName,
};
use blaze_vfs::PathFragment;

const INVALID_TARGET_NAME: &str = "invalid target name";
const INVALID_PACKAGE_NAME: &str = "invalid package name";

fn pc(raw: &str) -> Label {
    Label::parse_canonical(raw).unwrap()
}

fn rn(name: &str) -> RepositoryName {
    RepositoryName::create(name).unwrap()
}

#[test]
fn test_absolute() {
    {
        let l = pc("//foo/bar:baz");
        assert_eq!(l.get_package_name(), "foo/bar");
        assert_eq!(l.get_name(), "baz");
    }
    {
        let l = pc("//foo/bar");
        assert_eq!(l.get_package_name(), "foo/bar");
        assert_eq!(l.get_name(), "bar");
    }
    {
        let l = pc("//:bar");
        assert!(l.get_package_name().is_empty());
        assert_eq!(l.get_name(), "bar");
    }
    {
        let l = pc("@foo");
        assert_eq!(l.get_repository().get_name(), "foo");
        assert!(l.get_package_name().is_empty());
        assert_eq!(l.get_name(), "foo");
    }
    {
        let l = pc("@foo//bar");
        assert_eq!(l.get_repository().get_name(), "foo");
        assert_eq!(l.get_package_name(), "bar");
        assert_eq!(l.get_name(), "bar");
    }
    {
        let l = pc("@@foo//bar");
        assert_eq!(l.get_repository().get_name(), "foo");
        assert_eq!(l.get_package_name(), "bar");
        assert_eq!(l.get_name(), "bar");
    }
    {
        let l = pc("//@foo");
        assert_eq!(*l.get_repository(), *MAIN);
        assert_eq!(l.get_package_name(), "@foo");
        assert_eq!(l.get_name(), "@foo");
    }
    {
        let l = pc("//xyz/@foo:abc");
        assert_eq!(*l.get_repository(), *MAIN);
        assert_eq!(l.get_package_name(), "xyz/@foo");
        assert_eq!(l.get_name(), "abc");
    }
}

#[test]
fn parse_with_repo_context() {
    let foo = RepositoryName::create_unvalidated("foo");
    let bar = RepositoryName::create_unvalidated("bar");
    let quux = RepositoryName::create_unvalidated("quux");
    let repo_context = LabelRepoContextImpl::of(
        foo.clone(),
        RepositoryMapping::create([("bar", quux.clone())], foo.clone()),
    );
    {
        let l = Label::parse_with_repo_context("//lol:kek", &repo_context).unwrap();
        assert_eq!(*l.get_repository(), foo);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_repo_context("@bar//lol:kek", &repo_context).unwrap();
        assert_eq!(*l.get_repository(), quux);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_repo_context("@@bar//lol:kek", &repo_context).unwrap();
        assert_eq!(*l.get_repository(), bar);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_repo_context("@quux//lol:kek", &repo_context).unwrap();
        assert_eq!(*l.get_repository(), quux.to_non_visible(&foo));
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
}

#[test]
fn parse_with_package_context() {
    let foo = RepositoryName::create_unvalidated("foo");
    let bar = RepositoryName::create_unvalidated("bar");
    let quux = RepositoryName::create_unvalidated("quux");
    let package_context = LabelPackageContext::of(
        &PackageIdentifier::create(foo.clone(), PathFragment::create("hah")),
        RepositoryMapping::create([("bar", quux.clone())], foo.clone()),
    );
    {
        let l = Label::parse_with_package_context(":kek", &package_context).unwrap();
        assert_eq!(*l.get_repository(), foo);
        assert_eq!(l.get_package_name(), "hah");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_package_context("//lol:kek", &package_context).unwrap();
        assert_eq!(*l.get_repository(), foo);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_package_context("@bar//lol:kek", &package_context).unwrap();
        assert_eq!(*l.get_repository(), quux);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_package_context("@@bar//lol:kek", &package_context).unwrap();
        assert_eq!(*l.get_repository(), bar);
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
    {
        let l = Label::parse_with_package_context("@quux//lol:kek", &package_context).unwrap();
        assert_eq!(*l.get_repository(), quux.to_non_visible(&foo));
        assert_eq!(l.get_package_name(), "lol");
        assert_eq!(l.get_name(), "kek");
    }
}

#[test]
fn test_factory() {
    let l = Label::create("foo/bar", "quux").unwrap();
    assert_eq!(l.get_package_name(), "foo/bar");
    assert_eq!(l.get_name(), "quux");
}

#[test]
fn test_identities() {
    use std::hash::{BuildHasher, RandomState};
    let l1 = pc("//foo/bar:baz");
    let l2 = pc("//foo/bar:baz");
    let l3 = pc("//foo/bar:quux");
    // EqualsTester: groups {l1, l2} and {l3}.
    let s = RandomState::new();
    assert_eq!(l1, l2);
    assert_eq!(s.hash_one(&l1), s.hash_one(&l2));
    assert_ne!(l1, l3);
    assert_ne!(l2, l3);
}

#[test]
fn test_to_string() {
    {
        let s = "@//foo/bar:baz";
        let l = pc(s);
        assert_eq!(l.to_string(), "//foo/bar:baz");
    }
    {
        let l = pc("//foo/bar");
        assert_eq!(l.to_string(), "//foo/bar:bar");
    }
    {
        let l = pc("@foo");
        assert_eq!(l.to_string(), "@@foo//:foo");
    }
}

#[test]
fn test_dot_dot() {
    pc("//foo/bar:baz..gif");
}

/// Asserts that creating a label gives a `LabelSyntaxException` containing `expected_error`.
fn assert_syntax_error(expected_error: &str, label: &str) {
    let e: LabelSyntaxException = Label::parse_canonical(label).expect_err(&format!(
        "Label '{}' did not contain a syntax error, but was expected to",
        label
    ));
    assert!(
        e.get_message().contains(expected_error),
        "{:?} does not contain {:?}",
        e.get_message(),
        expected_error
    );
}

#[test]
fn test_bad_characters() {
    assert_syntax_error("target names may not contain ':'", "//foo:bar:baz");
    assert_syntax_error("target names may not contain ':'", "//foo:bar:");
    assert_syntax_error("target names may not contain ':'", "//foo/bar::");
}

#[test]
fn test_uplevel_references() {
    assert_syntax_error(INVALID_PACKAGE_NAME, "//foo/bar/..:baz");
    assert_syntax_error(INVALID_PACKAGE_NAME, "//foo/../baz:baz");
    assert_syntax_error(INVALID_PACKAGE_NAME, "//../bar/baz:baz");
    assert_syntax_error(INVALID_PACKAGE_NAME, "//..:foo");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:bar/../baz");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:../bar/baz");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:bar/baz/..");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:..");
}

#[test]
fn test_dot_as_a_path_segment() {
    assert_syntax_error(INVALID_PACKAGE_NAME, "//foo/bar/.:baz");
    assert_syntax_error(INVALID_PACKAGE_NAME, "//foo/./baz:baz");
    assert_syntax_error(INVALID_PACKAGE_NAME, "//./bar/baz:baz");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:bar/./baz");
    assert_syntax_error(INVALID_TARGET_NAME, "//foo:./bar/baz");
    // TODO(bazel-team): enable when we have removed the "Workaround" in Label
    // that rewrites broken Labels by removing the trailing '.'
    // assert_syntax_error(INVALID_PACKAGE_NAME, "//foo:bar/baz/.");
    // assert_syntax_error(INVALID_PACKAGE_NAME, "//foo:.");
}

#[test]
fn test_trailing_dot_segment() {
    assert_eq!(pc("//foo:dir"), pc("//foo:dir/."));
}

#[test]
fn test_some_other_bad_labels() {
    assert_syntax_error("package names may not end with '/'", "//foo/:bar");
    assert_syntax_error("package names may not start with '/'", "///p:foo");
    assert_syntax_error(
        "package names may not contain '//' path separators",
        "//a//b:foo",
    );
}

#[test]
fn test_some_good_labels() {
    pc("//foo:..bar");
    pc("//Foo:..bar");
    pc("//-Foo:..bar");
    pc("//00:..bar");
    pc("//package:foo+bar");
    pc("//package:foo_bar");
    pc("//package:foo=bar");
    pc("//package:foo-bar");
    pc("//package:foo.bar");
    pc("//package:foo@bar");
    pc("//package:foo~bar");
    pc("//$( ):$( )");
}

#[test]
fn test_double_slash_path_separator() {
    assert_syntax_error(
        "package names may not contain '//' path separators",
        "//foo//bar:baz",
    );
    assert_syntax_error(
        "target names may not contain '//' path separator",
        "//foo:bar//baz",
    );
}

#[test]
fn test_non_printable_characters() {
    assert_syntax_error(
        "target names may not contain non-printable characters: '\\x02'",
        "//foo:..\u{2}bar",
    );
}

/// Make sure that control characters - such as CR - are escaped on output.
#[test]
fn test_invalid_line_endings() {
    assert_syntax_error(
        "invalid target name '..bar\\r': target names may not end with carriage returns",
        "//foo:..bar\r",
    );
}

#[test]
fn test_empty_name() {
    assert_syntax_error("invalid target name '': empty target name", "//foo/bar:");
}

#[test]
fn test_repo_label() {
    let label = pc("@foo//bar/baz:bat/boo");
    assert_eq!(label.to_string(), "@@foo//bar/baz:bat/boo");
}

#[test]
fn test_no_repo() {
    let label = pc("//bar/baz:bat/boo");
    assert_eq!(label.to_string(), "//bar/baz:bat/boo");
}

#[test]
fn test_invalid_repo() {
    let e = Label::parse_canonical("foo//bar/baz:bat/boo").unwrap_err();
    assert_eq!(
        e.get_message(),
        "invalid package name 'foo//bar/baz': package names may not contain '//' path separators"
    );
}

#[test]
fn test_invalid_repo_with_colon() {
    let e = Label::parse_canonical("@foo:xyz").unwrap_err();
    assert!(
        e.get_message()
            .contains("invalid repository name 'foo:xyz': repo names may contain only"),
        "{}",
        e.get_message()
    );
}

#[test]
fn test_get_workspace_root() {
    // StarlarkSemantics.DEFAULT: --experimental_sibling_repository_layout=false
    let label = pc("//bar/baz");
    assert!(
        label
            .get_workspace_root_for_starlark_only(false)
            .unwrap()
            .is_empty()
    );
    let label = pc("@repo//bar/baz");
    assert_eq!(
        label.get_workspace_root_for_starlark_only(false).unwrap(),
        "external/repo"
    );
}

#[test]
fn test_get_containing_directory() {
    assert_eq!(
        Label::get_containing_directory(&Label::parse_canonical_unchecked("//a:b")),
        PathFragment::create("a")
    );
    assert_eq!(
        Label::get_containing_directory(&Label::parse_canonical_unchecked("//a/b:c")),
        PathFragment::create("a/b")
    );
    assert_eq!(
        Label::get_containing_directory(&Label::parse_canonical_unchecked("//a:b/c")),
        PathFragment::create("a/b")
    );
    assert_eq!(
        Label::get_containing_directory(&Label::parse_canonical_unchecked("//a/b/c")),
        PathFragment::create("a/b/c")
    );
}

#[test]
fn test_workspace_name() {
    assert_eq!(pc("@foo//bar:baz").get_workspace_name().unwrap(), "foo");
    assert!(pc("//bar:baz").get_workspace_name().unwrap().is_empty());
    assert!(pc("@//bar:baz").get_workspace_name().unwrap().is_empty());
}

#[test]
fn test_unambiguous_canonical_form() {
    assert_eq!(
        pc("//foo/bar:baz").get_unambiguous_canonical_form(),
        "@@//foo/bar:baz"
    );
    assert_eq!(
        pc("@foo//bar:baz").get_unambiguous_canonical_form(),
        "@@foo//bar:baz"
    );
    assert_eq!(
        Label::create_in_package(
            &PackageIdentifier::create(
                rn("foo").to_non_visible(&rn("bar")),
                PathFragment::create("baz")
            ),
            "quux"
        )
        .unwrap()
        .get_unambiguous_canonical_form(),
        "@@[unknown repo 'foo' requested from @@bar]//baz:quux"
    );
}

fn display_form_for(raw_label: &str, repository_mapping: Option<&RepositoryMapping>) -> String {
    pc(raw_label).get_display_form(repository_mapping)
}

#[test]
fn test_display_form() {
    let canonical_name = rn("canonical");
    let repository_mapping = RepositoryMapping::create(
        [("", MAIN.clone()), ("local", canonical_name)],
        MAIN.clone(),
    );
    let m = Some(&repository_mapping);

    assert_eq!(display_form_for("//foo/bar:bar", m), "//foo/bar:bar");
    assert_eq!(display_form_for("//foo/bar:baz", m), "//foo/bar:baz");

    assert_eq!(
        display_form_for("@canonical//bar:bar", m),
        "@local//bar:bar"
    );
    assert_eq!(
        display_form_for("@canonical//bar:baz", m),
        "@local//bar:baz"
    );
    assert_eq!(
        display_form_for("@canonical//:canonical", m),
        "@local//:canonical"
    );
    assert_eq!(display_form_for("@canonical//:local", m), "@local//:local");

    assert_eq!(display_form_for("@other//bar:bar", m), "@@other//bar:bar");
    assert_eq!(display_form_for("@other//bar:baz", m), "@@other//bar:baz");
    assert_eq!(display_form_for("@other//:other", m), "@@other//:other");

    assert_eq!(display_form_for("@@other", m), "@@other//:other");

    assert_eq!(
        Label::parse_with_repo_context(
            "@bad//abc",
            &LabelRepoContextImpl::of(MAIN.clone(), repository_mapping.clone())
        )
        .unwrap()
        .get_display_form(m),
        "@@[unknown repo 'bad' requested from @@]//abc:abc"
    );
}

#[test]
fn test_display_form_null_mapping() {
    assert_eq!(display_form_for("//foo/bar:bar", None), "//foo/bar:bar");
    assert_eq!(display_form_for("@@foo//bar:bar", None), "@@foo//bar:bar");
}

fn shorthand_display_form_for(raw_label: &str, repository_mapping: &RepositoryMapping) -> String {
    pc(raw_label).get_shorthand_display_form(Some(repository_mapping))
}

#[test]
fn test_shorthand_display_form() {
    let canonical_name = rn("canonical");
    let m = RepositoryMapping::create(
        [("", MAIN.clone()), ("local", canonical_name)],
        MAIN.clone(),
    );

    assert_eq!(shorthand_display_form_for("//foo/bar:bar", &m), "//foo/bar");
    assert_eq!(
        shorthand_display_form_for("//foo/bar:baz", &m),
        "//foo/bar:baz"
    );

    assert_eq!(
        shorthand_display_form_for("@canonical//bar:bar", &m),
        "@local//bar"
    );
    assert_eq!(
        shorthand_display_form_for("@canonical//bar:baz", &m),
        "@local//bar:baz"
    );
    assert_eq!(
        shorthand_display_form_for("@canonical//:canonical", &m),
        "@local//:canonical"
    );
    assert_eq!(
        shorthand_display_form_for("@canonical//:local", &m),
        "@local"
    );

    assert_eq!(
        shorthand_display_form_for("@other//bar:bar", &m),
        "@@other//bar"
    );
    assert_eq!(
        shorthand_display_form_for("@other//bar:baz", &m),
        "@@other//bar:baz"
    );
    assert_eq!(shorthand_display_form_for("@other//:other", &m), "@@other");

    assert_eq!(shorthand_display_form_for("@@other", &m), "@@other");

    assert_eq!(
        Label::parse_with_repo_context(
            "@bad//abc",
            &LabelRepoContextImpl::of(MAIN.clone(), m.clone())
        )
        .unwrap()
        .get_shorthand_display_form(Some(&m)),
        "@@[unknown repo 'bad' requested from @@]//abc"
    );
}

#[test]
#[ignore = "needs the Starlark layer (Starlark.repr of a Label); str() is covered by starlark_str_* tests"]
fn starlark_str_and_repr() {}

#[test]
fn starlark_str_canonical_label_literal() {
    // StarlarkSemantics.DEFAULT: --incompatible_unambiguous_label_stringification=true
    assert_eq!(pc("//x").str(true), "@@//x:x");
    assert_eq!(pc("@x//y").str(true), "@@x//y:y");
}

#[test]
fn starlark_str_ambiguous_and_canonical_label_literal() {
    assert_eq!(pc("//x").str(false), "//x:x");
    assert_eq!(pc("@x//y").str(false), "@@x//y:y");
}

#[test]
#[ignore = "needs serialization infrastructure (SerializationTester)"]
fn test_codec() {}

#[test]
fn parse_with_package_context_recording_repo_mapping() {
    let foo = RepositoryName::create_unvalidated("foo");
    let bar = RepositoryName::create_unvalidated("bar");
    let quux = RepositoryName::create_unvalidated("quux");
    let foo_package_context = LabelPackageContext::of(
        &PackageIdentifier::create(foo.clone(), PathFragment::create("hah")),
        RepositoryMapping::create([("bar", quux.clone()), ("quux", bar.clone())], foo.clone()),
    );
    let bar_package_context = LabelPackageContext::of(
        &PackageIdentifier::create(bar.clone(), PathFragment::empty_fragment()),
        RepositoryMapping::create([("foo", quux.clone()), ("quux", foo.clone())], bar.clone()),
    );
    let mut recorder = LabelSimpleRepoMappingRecorder::new();

    // Not recorded: no repo part
    Label::parse_with_package_context_recording(
        "//foo/bar",
        &foo_package_context,
        Some(&mut recorder),
    )
    .unwrap();
    // Recorded: <foo, bar, quux>
    Label::parse_with_package_context_recording(
        "@bar//foo/bar",
        &foo_package_context,
        Some(&mut recorder),
    )
    .unwrap();
    // Recorded: <bar, quux, foo>
    Label::parse_with_package_context_recording(
        "@quux//foo/bar",
        &bar_package_context,
        Some(&mut recorder),
    )
    .unwrap();
    // Recorded: <bar, foo, quux>
    Label::parse_with_package_context_recording(
        "@foo//foo/bar",
        &bar_package_context,
        Some(&mut recorder),
    )
    .unwrap();
    // Not recorded: canonical repo name
    Label::parse_with_package_context_recording(
        "@@quux//foo/bar",
        &foo_package_context,
        Some(&mut recorder),
    )
    .unwrap();

    // Recorded entries are sorted by row and then column
    assert_eq!(
        recorder.recorded_entries(),
        vec![
            (bar.clone(), "foo".to_owned(), quux.clone()),
            (bar.clone(), "quux".to_owned(), foo.clone()),
            (foo.clone(), "bar".to_owned(), quux.clone()),
        ]
    );
}
