//! Port of `PackageIdentifierTest`.

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{PackageIdentifier, RepositoryMapping, RepositoryName};
use blaze_vfs::PathFragment;

fn rn(name: &str) -> RepositoryName {
    RepositoryName::create(name).unwrap()
}

fn create(repo: &str, pkg: &str) -> PackageIdentifier {
    PackageIdentifier::create_from_repo_string(repo, PathFragment::create(pkg)).unwrap()
}

#[test]
fn test_parsing() {
    let foo_a = PackageIdentifier::parse("@foo//a").unwrap();
    assert_eq!(foo_a.get_repository().get_name(), "foo");
    assert_eq!(foo_a.get_package_fragment().get_path_string(), "a");
    assert_eq!(
        foo_a.get_package_path(false),
        PathFragment::create("external/foo/a")
    );
    assert_eq!(foo_a.get_package_path(true), PathFragment::create("a"));

    let absolute_a = PackageIdentifier::parse("//a").unwrap();
    assert_eq!(absolute_a.get_repository().get_name(), "");
    assert_eq!(absolute_a.get_package_fragment().get_path_string(), "a");
    assert_eq!(
        absolute_a.get_package_path(false),
        PathFragment::create("a")
    );
    assert_eq!(absolute_a.get_package_path(true), PathFragment::create("a"));

    let plain_a = PackageIdentifier::parse("a").unwrap();
    assert_eq!(plain_a.get_repository().get_name(), "");
    assert_eq!(plain_a.get_package_fragment().get_path_string(), "a");
    assert_eq!(plain_a.get_package_path(false), PathFragment::create("a"));
    assert_eq!(plain_a.get_package_path(true), PathFragment::create("a"));

    let main_a = PackageIdentifier::parse("@//a").unwrap();
    assert_eq!(*main_a.get_repository(), *MAIN);
    assert_eq!(main_a.get_package_fragment().get_path_string(), "a");
    assert_eq!(main_a.get_package_path(false), PathFragment::create("a"));
    assert_eq!(main_a.get_package_path(true), PathFragment::create("a"));
}

#[test]
fn test_to_string() {
    let local = create("", "bar/baz");
    assert_eq!(local.to_string(), "bar/baz");
    let external = create("foo", "bar/baz");
    assert_eq!(external.to_string(), "@@foo//bar/baz");
}

#[test]
fn test_compare_to() {
    use std::cmp::Ordering;
    let foo1 = create("foo", "bar/baz");
    let foo2 = create("foo", "bar/baz");
    let foo3 = create("foo", "bar/bz");
    let bar = create("bar", "bar/baz");
    assert_eq!(foo1.cmp(&foo2), Ordering::Equal);
    assert_eq!(foo1.cmp(&foo3), Ordering::Less);
    assert_eq!(foo1.cmp(&bar), Ordering::Greater);
}

#[test]
fn test_invalid_package_name() {
    // This shouldn't throw an exception, package names aren't validated.
    create("foo", "bar.baz");
}

#[test]
fn test_package_fragment_equality() {
    // Make sure package fragments are canonicalized.
    let p1 = create("whatever", "foo/bar");
    let p2 = create("whatever", "foo/bar");
    assert!(PathFragment::ptr_eq(
        p1.get_package_fragment(),
        p2.get_package_fragment()
    ));
}

#[test]
fn test_runfiles_dir() {
    assert_eq!(
        create("foo", "bar/baz").get_runfiles_path(),
        PathFragment::create("../foo/bar/baz")
    );
    assert_eq!(
        create("", "bar/baz").get_runfiles_path(),
        PathFragment::create("bar/baz")
    );
}

#[test]
fn test_unambiguous_canonical_form() {
    assert_eq!(
        PackageIdentifier::create_in_main_repo_str("foo/bar").get_unambiguous_canonical_form(),
        "@@//foo/bar"
    );
    assert_eq!(
        create("foo", "bar").get_unambiguous_canonical_form(),
        "@@foo//bar"
    );
    assert_eq!(
        PackageIdentifier::create(
            rn("foo").to_non_visible(&rn("bar")),
            PathFragment::create("baz")
        )
        .get_unambiguous_canonical_form(),
        "@@[unknown repo 'foo' requested from @@bar]//baz"
    );
}

#[test]
fn test_display_form_in_main_repository() {
    let pkg = PackageIdentifier::create(MAIN.clone(), PathFragment::create("some/pkg"));

    assert_eq!(
        pkg.get_display_form(Some(&RepositoryMapping::create(
            [("foo", rn("bar"))],
            MAIN.clone()
        ))),
        "//some/pkg"
    );
    assert_eq!(pkg.get_display_form(None), "//some/pkg");
}

#[test]
fn test_display_form_in_external_repository() {
    let repo = rn("canonical");
    let pkg = PackageIdentifier::create(repo.clone(), PathFragment::create("some/pkg"));

    assert_eq!(
        pkg.get_display_form(Some(&RepositoryMapping::create(
            [("local", repo.clone())],
            MAIN.clone()
        ))),
        "@local//some/pkg"
    );
    assert_eq!(
        pkg.get_display_form(Some(&RepositoryMapping::create(
            [("local", rn("other_repo"))],
            MAIN.clone()
        ))),
        "@@canonical//some/pkg"
    );
    assert_eq!(pkg.get_display_form(None), "@@canonical//some/pkg");
}

#[test]
#[ignore = "needs serialization infrastructure (SerializationTester)"]
fn test_serialization() {}
