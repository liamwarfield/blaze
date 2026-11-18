//! Port of `LabelValidatorTest`.

use blaze_cmdline::LabelValidatorPackageAndTarget as PackageAndTarget;
use blaze_cmdline::label_validator::{
    self, PACKAGE_NAME_DOT_ERROR, PACKAGE_NAME_ERROR, validate_package_name, validate_target_name,
};

fn new_foo_target() -> PackageAndTarget {
    PackageAndTarget::new("foo", "foo")
}

fn new_bar_target() -> PackageAndTarget {
    PackageAndTarget::new("bar", "bar")
}

#[test]
fn test_validate_package_name() {
    // OK:
    for ok in [
        "foo",
        "Foo",
        "FOO",
        "foO",
        "foo-bar",
        "Foo-Bar",
        "FOO-BAR",
        "bar.baz",
        "a/..b",
        "a/.b",
        "a/b.",
        "a/b..",
        "a$( )/b..",
        // These are in ascii code order.
        "foo!bar",
        "foo\"bar",
        "foo#bar",
        "foo$bar",
        "foo%bar",
        "foo&bar",
        "foo'bar",
        "foo(bar",
        "foo)bar",
        "foo*bar",
        "foo+bar",
        "foo,bar",
        "foo-bar",
        "foo.bar",
        "foo+bar",
        "foo;bar",
        "foo<bar",
        "foo=bar",
        "foo>bar",
        "foo?bar",
        "foo@bar",
        "foo[bar",
        "foo]bar",
        "foo^bar",
        "foo_bar",
        "foo`bar",
        "foo{bar",
        "foo|bar",
        "foo}bar",
        "foo~bar",
    ] {
        assert_eq!(validate_package_name(ok), None, "{}", ok);
    }

    // Bad:
    assert_eq!(
        validate_package_name("/foo").as_deref(),
        Some("package names may not start with '/'")
    );
    assert_eq!(
        validate_package_name("foo/").as_deref(),
        Some("package names may not end with '/'")
    );
    assert_eq!(
        validate_package_name("foo:bar").as_deref(),
        Some(PACKAGE_NAME_ERROR)
    );
    for bad in [
        "bar/../baz",
        "bar/..",
        "../bar",
        "bar/...",
        "bar/./baz",
        "bar/.",
        "./bar",
    ] {
        assert_eq!(
            validate_package_name(bad).as_deref(),
            Some(PACKAGE_NAME_DOT_ERROR),
            "{}",
            bad
        );
    }
}

#[test]
fn test_validate_target_name() {
    for ok in [
        "foo", "foo!bar", "foo\"bar", "foo#bar", "foo$bar", "foo%bar", "foo&bar", "foo'bar",
        "foo(bar", "foo)bar", "foo*bar", "foo+bar", "foo,bar", "foo-bar", "foo.bar", "foo+bar",
        "foo;bar", "foo<bar", "foo=bar", "foo>bar", "foo?bar", "foo[bar", "foo]bar", "foo^bar",
        "foo_bar", "foo`bar", "foo{bar", "foo|bar", "foo}bar", "foo~bar", "foo/bar", "foo@bar",
    ] {
        assert_eq!(validate_target_name(ok), None, "{}", ok);
    }

    assert_eq!(
        validate_target_name("foo/").as_deref(),
        Some("target names may not end with '/'")
    );
    assert_eq!(
        validate_target_name("bar:baz").as_deref(),
        Some("target names may not contain ':'")
    );
    assert_eq!(
        validate_target_name("bar:").as_deref(),
        Some("target names may not contain ':'")
    );
}

#[test]
fn test_validate_absolute_label() {
    let v = |s: &str| label_validator::validate_absolute_label(s).unwrap();
    let mut empty_package = PackageAndTarget::new("", "bar");
    assert_eq!(v("//:bar"), empty_package);
    assert_eq!(v("@repo//:bar"), empty_package);
    assert_eq!(v("@repo//foo:bar"), PackageAndTarget::new("foo", "bar"));
    assert_eq!(v("@//foo:bar"), PackageAndTarget::new("foo", "bar"));
    empty_package = PackageAndTarget::new("", "b$() ar");
    assert_eq!(v("//:b$() ar"), empty_package);
    assert_eq!(v("@repo//:b$() ar"), empty_package);
    assert_eq!(
        v("@repo//f$( )oo:b$() ar"),
        PackageAndTarget::new("f$( )oo", "b$() ar")
    );
    assert_eq!(
        v("@//f$( )oo:b$() ar"),
        PackageAndTarget::new("f$( )oo", "b$() ar")
    );
    assert_eq!(v("//f@oo"), PackageAndTarget::new("f@oo", "f@oo"));
    assert_eq!(v("//@foo"), PackageAndTarget::new("@foo", "@foo"));
    assert_eq!(v("//@foo:@bar"), PackageAndTarget::new("@foo", "@bar"));
}

#[test]
fn test_package_and_target_hash_code_distinct_but_equal_objects() {
    use std::hash::{BuildHasher, RandomState};
    let foo_target1 = new_foo_target();
    let foo_target2 = new_foo_target();
    let s = RandomState::new();
    assert_eq!(
        s.hash_one(&foo_target1),
        s.hash_one(&foo_target2),
        "Should have same hash code"
    );
}

#[test]
fn test_package_and_target_equals_distinct_but_equal_objects() {
    let foo_target1 = new_foo_target();
    let foo_target2 = new_foo_target();
    assert_eq!(foo_target1, foo_target2, "Should be equal");
}

#[test]
fn test_package_and_target_equals_unequal_objects() {
    assert!(new_foo_target() != new_bar_target(), "should be unequal");
}

#[test]
fn test_package_and_target_to_string() {
    assert_eq!(new_foo_target().to_string(), "//foo:foo");
    assert_eq!(new_bar_target().to_string(), "//bar:bar");
}

#[test]
fn test_slashless_label_infers_target_name_from_repo_name() {
    assert_eq!(
        label_validator::parse_absolute_label("@foo")
            .unwrap()
            .to_string(),
        "//:foo"
    );
}
