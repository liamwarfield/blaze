//! Port of `RepositoryNameTest`.

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use blaze_vfs::PathFragment;

fn rn(name: &str) -> RepositoryName {
    RepositoryName::create(name).unwrap()
}

fn assert_not_valid(name: &str, expected_message: &str) {
    let expected = RepositoryName::create(name).unwrap_err();
    assert!(
        expected.get_message().contains(expected_message),
        "{:?} does not contain {:?}",
        expected.get_message(),
        expected_message
    );
}

#[test]
fn test_validate_repository_name() {
    assert_eq!(rn("foo").get_name(), "foo");
    assert_eq!(rn("").get_name(), "");
    assert!(RepositoryName::ptr_eq(&rn(""), &MAIN));
    assert_eq!(rn("foo_bar").get_name(), "foo_bar");
    assert_eq!(rn("foo-bar").get_name(), "foo-bar");
    assert_eq!(rn("foo.bar").get_name(), "foo.bar");
    assert_eq!(rn("..foo").get_name(), "..foo");
    assert_eq!(rn("foo..").get_name(), "foo..");
    assert_eq!(rn(".foo").get_name(), ".foo");
    assert_eq!(rn("foo+bar").get_name(), "foo+bar");

    assert_not_valid(".", "repo names are not allowed to be '.'");
    assert_not_valid("..", "repo names are not allowed to be '..'");
    assert_not_valid(
        "foo/bar",
        "repo names may contain only A-Z, a-z, 0-9, '-', '_', '.' and '+'",
    );
    assert_not_valid(
        "foo@",
        "repo names may contain only A-Z, a-z, 0-9, '-', '_', '.' and '+'",
    );
    assert_not_valid(
        "foo\0",
        "repo names may contain only A-Z, a-z, 0-9, '-', '_', '.' and '+'",
    );
}

#[test]
fn validate_user_provided_repo_name() {
    for ok in ["foo", "foo_bar", "foo-bar", "foo.bar", "foo..", "foo.33"] {
        RepositoryName::validate_user_provided_repo_name(ok).unwrap();
    }
    for bad in [".foo", "_foo", "foo/bar", "@foo"] {
        assert!(RepositoryName::validate_user_provided_repo_name(bad).is_err());
    }
}

#[test]
fn test_runfiles_dir() {
    assert_eq!(
        rn("foo").get_runfiles_path(),
        PathFragment::create("../foo")
    );
    assert_eq!(rn("").get_runfiles_path(), PathFragment::empty_fragment());
}

#[test]
fn test_get_default_canonical_form() {
    assert_eq!(rn("").get_canonical_form(), "");
    assert_eq!(rn("foo").get_canonical_form(), "@@foo");
}

#[test]
fn test_get_display_form() {
    let repository_mapping = RepositoryMapping::create([("local", rn("canonical"))], MAIN.clone());

    assert_eq!(rn("").get_display_form(Some(&repository_mapping)), "");
    assert_eq!(
        rn("canonical").get_display_form(Some(&repository_mapping)),
        "@local"
    );
    assert_eq!(
        rn("other").get_display_form(Some(&repository_mapping)),
        "@@other"
    );

    assert_eq!(
        rn("")
            .to_non_visible(&rn("owner"))
            .get_display_form(Some(&repository_mapping)),
        "@@[unknown repo '' requested from @@owner]"
    );
    assert_eq!(
        rn("local")
            .to_non_visible(&rn("owner"))
            .get_display_form(Some(&repository_mapping)),
        "@@[unknown repo 'local' requested from @@owner]"
    );
}

#[test]
fn test_get_display_form_with_null_mapping() {
    assert_eq!(rn("").get_display_form(None), "");
    assert_eq!(rn("canonical").get_display_form(None), "@@canonical");

    assert_eq!(
        rn("").to_non_visible(&rn("owner")).get_display_form(None),
        "@@[unknown repo '' requested from @@owner]"
    );
    assert_eq!(
        rn("canonical")
            .to_non_visible(&rn("owner"))
            .get_display_form(None),
        "@@[unknown repo 'canonical' requested from @@owner]"
    );
}

#[test]
#[ignore = "needs serialization infrastructure (SerializationTester)"]
fn test_serialization() {}
