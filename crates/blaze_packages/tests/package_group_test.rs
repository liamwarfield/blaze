//! Port of `src/test/java/com/google/devtools/build/lib/packages/PackageGroupTest.java`: the
//! test that loads no BUILD file (`testStringification`), and checks of the
//! `PackageSpecification` / `PackageGroupContents` part of the others.
//!
//! The tests that evaluate a BUILD file (Java `PackageLoadingTestCase`: `scratch.file` +
//! `getTarget`), and `PackageGroupStaticInitializationTest`, are ported in
//! `crates/blaze_sky_loading/src/tests/package_group_test.rs` (this crate cannot load packages).

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_mapping::EMPTY;
use blaze_cmdline::repository_name::MAIN;
use blaze_packages::target::PackageGroupContents;
use blaze_packages::target::PackageSpecification;

/// Convenience method for obtaining a PackageSpecification.
fn pkg_spec(repository: &RepositoryName, spec: &str) -> PackageSpecification {
    PackageSpecification::from_string(
        &EMPTY, repository, spec, /* allow_public_private= */ true,
        /* repo_root_means_current_repo= */ true,
    )
    .unwrap()
}

/// Convenience method for obtaining a PackageIdentifier.
fn pkg_id(package_name: &str) -> PackageIdentifier {
    PackageIdentifier::create_unchecked(/* repository= */ "", package_name)
}

/// Convenience method for obtaining a PackageIdentifier outside the main repo.
fn pkg_id_in(repo_name: &str, package_name: &str) -> PackageIdentifier {
    PackageIdentifier::create_unchecked(repo_name, package_name)
}

/// The `PackageGroupContents` of a `package_group` in the main repo with `packages`, as
/// `PackageGroup`'s constructor computes it (without loading a BUILD file).
fn contents(
    packages: &[&str],
    allow_public_private: bool,
    repo_root_means_current_repo: bool,
) -> Result<PackageGroupContents, String> {
    let mut specs = Vec::new();
    for p in packages {
        specs.push(
            PackageSpecification::from_string(
                &EMPTY,
                &MAIN,
                p,
                allow_public_private,
                repo_root_means_current_repo,
            )
            .map_err(|e| e.0)?,
        );
    }
    Ok(PackageGroupContents::create(&specs))
}

/// Truth `containsExactly` (order-insensitive multiset equality).
fn assert_contains_exactly(actual: Vec<String>, expected: &[&str]) {
    let mut actual = actual;
    let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

// PackageGroupTest's BUILD-loading tests: crates/blaze_sky_loading/src/tests/package_group_test.rs

#[test]
fn test_stringification() {
    let main = MAIN.clone();
    let other = RepositoryName::create("other").unwrap();
    let contents = PackageGroupContents::create(&[
        pkg_spec(&main, "//a"),
        pkg_spec(&main, "//a/b/..."),
        pkg_spec(&main, "-//c"),
        pkg_spec(&main, "-//c/d/..."),
        pkg_spec(&main, "//..."),
        pkg_spec(&main, "-//..."),
        pkg_spec(&main, "//"),
        pkg_spec(&main, "-//"),
        pkg_spec(&other, "//z"),
        pkg_spec(&other, "//..."),
        pkg_spec(&main, "public"),
        pkg_spec(&main, "private"),
    ]);
    assert_contains_exactly(
        contents.package_strings(/* include_double_slash= */ false),
        &[
            "a",
            "",
            "@@other//z",
            "a/b/...",
            "//...",
            "@@other//...",
            "-c",
            "-",
            "-c/d/...",
            "-//...",
            "//...", // legacy syntax for public
            "private",
        ],
    );
    assert_contains_exactly(
        contents.package_strings(/* include_double_slash= */ true),
        &[
            "//a",
            "//a/b/...",
            "-//c",
            "-//c/d/...",
            "//...",
            "-//...",
            "//",
            "-//",
            "@@other//z",
            "@@other//...",
            "public",
            "private",
        ],
    );
    assert_contains_exactly(
        contents.package_strings_with_double_slash_and_without_repository(),
        &[
            "//a",
            "//a/b/...",
            "-//c",
            "-//c/d/...",
            "//...",
            "-//...",
            "//",
            "-//",
            "//z",
            "//...",
            "public",
            "private",
        ],
    );
}

// The following check the PackageSpecification / PackageGroupContents part of the BUILD-loading
// tests (ported in blaze_sky_loading, see above), with the Java tests' packages and expectations
// (the defaults of --incompatible_package_group_has_public_syntax and
// --incompatible_fix_package_group_reporoot_syntax are true).

#[test]
fn test_absolute_packages_work_without_build_loading() {
    let grp = contents(&["//vegetables"], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("vegetables")));
    assert!(!grp.contains_package(&pkg_id("fruits/vegetables")));
}

#[test]
fn test_packages_without_double_slash_do_not_work_without_build_loading() {
    let e = contents(&["vegetables"], true, true).unwrap_err();
    assert!(e.contains("invalid package name 'vegetables'"), "{e}");
}

#[test]
fn test_packages_with_repository_works_without_build_loading() {
    let grp = contents(&["@@veggies//cucumber"], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id_in("veggies", "cucumber")));
}

#[test]
fn test_all_packages_in_main_repository_works_without_build_loading() {
    let grp = contents(&["@//..."], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("anything")));
}

#[test]
fn test_target_name_as_package_does_not_work1_without_build_loading() {
    let e = contents(&["//vegetables:carrot"], true, true).unwrap_err();
    assert!(
        e.contains("invalid package name '//vegetables:carrot'"),
        "{e}"
    );
}

#[test]
fn test_target_name_as_package_does_not_work2_without_build_loading() {
    let e = contents(&[":carrot"], true, true).unwrap_err();
    assert!(e.contains("invalid package name ':carrot'"), "{e}");
}

#[test]
fn test_negative_without_build_loading() {
    let grp = contents(&["-//four", "-//three", "//one", "//two"], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("one")));
    assert!(grp.contains_package(&pkg_id("two")));
    assert!(!grp.contains_package(&pkg_id("three")));
    assert!(!grp.contains_package(&pkg_id("four")));
}

#[test]
fn test_negative_no_subpackages_without_build_loading() {
    let grp = contents(&["-//pkg/one", "//pkg/..."], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("pkg")));
    assert!(!grp.contains_package(&pkg_id("pkg/one")));
    assert!(grp.contains_package(&pkg_id("pkg/one/two")));
}

#[test]
fn test_negative_subpackages_without_build_loading() {
    let grp = contents(&["-//pkg/one/...", "//pkg/..."], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("pkg")));
    assert!(!grp.contains_package(&pkg_id("pkg/one")));
    assert!(!grp.contains_package(&pkg_id("pkg/one/two")));
}

#[test]
fn test_everything_specification_works_without_build_loading() {
    let grp = contents(&["public"], true, true).unwrap();
    assert_eq!(grp.package_strings(true), vec!["public".to_owned()]);
    assert!(grp.contains_package(&pkg_id("pkg")));
    assert!(grp.contains_package(&pkg_id_in("somerepo", "pkg")));
}

#[test]
fn test_nothing_specification_works_without_build_loading() {
    let grp = contents(&["private"], true, true).unwrap();
    assert_eq!(grp.package_strings(true), vec!["private".to_owned()]);
    assert!(!grp.contains_package(&pkg_id("anything")));
}

#[test]
fn test_public_private_are_not_accessible_without_flag_without_build_loading() {
    let e = contents(&["public"], false, false).unwrap_err();
    assert!(e.contains(
        "Use of \"public\" package specification requires enabling \
         --incompatible_package_group_has_public_syntax"
    ));
    let e = contents(&["private"], false, false).unwrap_err();
    assert!(e.contains(
        "Use of \"private\" package specification requires enabling \
         --incompatible_package_group_has_public_syntax"
    ));
}

#[test]
fn test_repo_root_subpackages_is_public_without_flag_without_build_loading() {
    let grp = contents(&["//..."], true, false).unwrap();
    assert_eq!(grp.package_strings(true), vec!["public".to_owned()]);
    assert!(grp.contains_package(&pkg_id("pkg")));
    assert!(grp.contains_package(&pkg_id_in("somerepo", "pkg")));
}

#[test]
fn test_repo_root_subpackages_is_not_public_with_flag_without_build_loading() {
    let grp = contents(&["//..."], true, true).unwrap();
    assert_eq!(grp.package_strings(true), vec!["//...".to_owned()]);
    assert!(grp.contains_package(&pkg_id("pkg")));
    assert!(!grp.contains_package(&pkg_id_in("somerepo", "pkg")));
}

#[test]
fn test_cannot_use_new_repo_root_syntax_without_public_syntax_without_build_loading() {
    let e = contents(&["//something"], false, true).unwrap_err();
    assert!(e.contains("Cannot use new \"//...\" meaning without allowing new \"public\" syntax."));
}

#[test]
fn test_negative_repo_root_subpackages_without_build_loading() {
    let grp = contents(&["-//...", "//pkg/one"], true, true).unwrap();
    assert!(!grp.contains_package(&pkg_id("pkg")));
    assert!(!grp.contains_package(&pkg_id("pkg/one")));
    assert!(!grp.contains_package(&pkg_id("pkg/one/two")));
}

#[test]
fn test_negative_public_without_build_loading() {
    let e = contents(&["-public"], true, true).unwrap_err();
    assert!(e.contains("Cannot negate \"public\" package specification"));
}

#[test]
fn test_negative_private_without_build_loading() {
    let e = contents(&["-private"], true, true).unwrap_err();
    assert!(e.contains("Cannot negate \"private\" package specification"));
}

#[test]
fn test_duplicate_package_without_build_loading() {
    let grp = contents(&["//one/two"], true, true).unwrap();
    assert!(grp.contains_package(&pkg_id("one/two")));
}
