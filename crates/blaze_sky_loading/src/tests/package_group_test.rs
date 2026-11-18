//! Port of the BUILD-loading tests of PackageGroupTest and of
//! PackageGroupStaticInitializationTest, on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/PackageGroupTest.java and
//! PackageGroupStaticInitializationTest.java. `testStringification` loads no BUILD file and is
//! ported in `crates/blaze_packages/tests/package_group_test.rs`.
//!
//! Java `getPackageGroup(pkg, name)` is `(PackageGroup) getTarget("//pkg:name")`, which loads the
//! package with the test's reporter: [`get_package_group`] for the tests that keep
//! `failFastHandler` (an ERROR event fails the test), [`get_package_group_and_events`] for the
//! tests that remove it and assert on the events.

use std::sync::Arc;

use blaze_cmdline::repository_mapping::EMPTY;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_packages::target::{PackageGroup, PackageSpecification};

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase};

/// Convenience method for obtaining a PackageIdentifier.
// Java: PackageGroupTest.pkgId(String)
fn pkg_id(package_name: &str) -> PackageIdentifier {
    PackageIdentifier::create_unchecked(/* repository= */ "", package_name)
}

/// Convenience method for obtaining a PackageIdentifier outside the main repo.
// Java: PackageGroupTest.pkgId(String, String)
fn pkg_id_in(repo_name: &str, package_name: &str) -> PackageIdentifier {
    PackageIdentifier::create_unchecked(repo_name, package_name)
}

/// Evaluates and returns the requested package_group target, and the loaded package with the
/// events of loading it (for the Java tests that call `reporter.removeHandler(failFastHandler)`).
// Java: PackageGroupTest.getPackageGroup
fn get_package_group_and_events(
    t: &PackageLoadingTestCase,
    pkg: &str,
    name: &str,
) -> (Arc<PackageGroup>, LoadedPackage) {
    let loaded = t.get_package(pkg);
    let group = loaded
        .target(name)
        .as_package_group()
        .unwrap_or_else(|| panic!("//{pkg}:{name} is not a package_group"))
        .clone();
    (group, loaded)
}

/// Evaluates and returns the requested package_group target; an ERROR event fails the test (Java
/// `failFastHandler`).
// Java: PackageGroupTest.getPackageGroup
fn get_package_group(t: &PackageLoadingTestCase, pkg: &str, name: &str) -> Arc<PackageGroup> {
    let (group, loaded) = get_package_group_and_events(t, pkg, name);
    loaded.assert_no_error_events();
    group
}

/// Truth `containsExactly` (order-insensitive).
fn assert_contains_exactly(actual: Vec<String>, expected: &[&str]) {
    let mut actual = actual;
    let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

// Java: PackageGroupTest.testDoesNotFailHorribly
#[test]
fn test_does_not_fail_horribly() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"//random\"],",
            ")",
        ],
    );
    // Note that, for our purposes, the packages listed in the package_group need not exist.

    get_package_group(&t, "fruits", "apple");
}

// Regression test for: "Package group with empty name causes Blaze exception"
// Java: PackageGroupTest.testEmptyPackageGroupNameDoesNotThrow
#[test]
fn test_empty_package_group_name_does_not_throw() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "strawberry/BUILD",
        &[
            "package_group(",
            "    name = \"\",",
            "    packages = [],",
            ")",
        ],
    );

    // Call getTarget() directly since getPackageGroup() requires a name.
    let loaded = t.get_package("strawberry");
    loaded.target("BUILD");
    loaded.assert_contains_event("package group has invalid name");
}

// Java: PackageGroupTest.testAbsolutePackagesWork
#[test]
fn test_absolute_packages_work() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"//vegetables\"],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "fruits", "apple");
    assert!(grp.contains(&pkg_id("vegetables")));
    assert!(!grp.contains(&pkg_id("fruits/vegetables")));
}

// Java: PackageGroupTest.testPackagesWithoutDoubleSlashDoNotWork
#[test]
fn test_packages_without_double_slash_do_not_work() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"vegetables\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "apple");
    loaded.assert_contains_event("invalid package name 'vegetables'");
}

// Java: PackageGroupTest.testPackagesWithRepositoryWorks
#[test]
fn test_packages_with_repository_works() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"banana\",",
            "    packages = [\"@@veggies//cucumber\"],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "fruits", "banana");
    assert!(grp.contains(&pkg_id_in("veggies", "cucumber")));
}

// Java: PackageGroupTest.testAllPackagesInMainRepositoryWorks
#[test]
fn test_all_packages_in_main_repository_works() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"@//...\"],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "fruits", "apple");
    assert!(grp.contains(&pkg_id("anything")));
}

// TODO(brandjon): It'd be nice to include a test here that you can cross repositories via
// `includes`: if package_group //:A includes package_group @repo//:B that has "//foo" in its
// `packages`, then //:A admits package @repo//foo. Unfortunately PackageLoadingTestCase doesn't
// support resolving repos, but similar functionality is tested in
// BzlLoadFunctionTest#testBzlVisibility_enumeratedPackagesMultipleRepos.

// Java: PackageGroupTest.testTargetNameAsPackageDoesNotWork1
#[test]
fn test_target_name_as_package_does_not_work1() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"//vegetables:carrot\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "apple");
    loaded.assert_contains_event("invalid package name '//vegetables:carrot'");
}

// Java: PackageGroupTest.testTargetNameAsPackageDoesNotWork2
#[test]
fn test_target_name_as_package_does_not_work2() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\":carrot\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "apple");
    loaded.assert_contains_event("invalid package name ':carrot'");
}

// Java: PackageGroupTest.testAllBeneathSpecificationWorks
#[test]
fn test_all_beneath_specification_works() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"maracuja\",",
            "    packages = [\"//tropics/...\"],",
            ")",
        ],
    );

    get_package_group(&t, "fruits", "maracuja");
}

// Java: PackageGroupTest.testNegative
#[test]
fn test_negative() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/BUILD",
        &[
            "package_group(",
            "    name = \"packages\",",
            "    packages = [",
            "        \"-//four\",",
            "        \"-//three\",",
            "        \"//one\",",
            "        \"//two\",",
            "    ],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "test", "packages");
    assert!(grp.contains(&pkg_id("one")));
    assert!(grp.contains(&pkg_id("two")));
    assert!(!grp.contains(&pkg_id("three")));
    assert!(!grp.contains(&pkg_id("four")));
}

// Java: PackageGroupTest.testNegative_noSubpackages
#[test]
fn test_negative_no_subpackages() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/BUILD",
        &[
            "package_group(",
            "    name = \"packages\",",
            "    packages = [",
            "        \"-//pkg/one\",",
            "        \"//pkg/...\",",
            "    ],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "test", "packages");
    assert!(grp.contains(&pkg_id("pkg")));
    assert!(!grp.contains(&pkg_id("pkg/one")));
    assert!(grp.contains(&pkg_id("pkg/one/two")));
}

// Java: PackageGroupTest.testNegative_subpackages
#[test]
fn test_negative_subpackages() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/BUILD",
        &[
            "package_group(",
            "    name = \"packages\",",
            "    packages = [",
            "        \"-//pkg/one/...\",",
            "        \"//pkg/...\",",
            "    ],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "test", "packages");
    assert!(grp.contains(&pkg_id("pkg")));
    assert!(!grp.contains(&pkg_id("pkg/one")));
    assert!(!grp.contains(&pkg_id("pkg/one/two")));
}

// Java: PackageGroupTest.testEverythingSpecificationWorks
#[test]
fn test_everything_specification_works() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_package_group_has_public_syntax=true"]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"mango\",",
            "    packages = [\"public\"],",
            ")",
        ],
    );
    let grp = get_package_group(&t, "fruits", "mango");

    // Assert that we're using the right package spec.
    assert_contains_exactly(
        grp.get_contained_packages(/* include_double_slash= */ true),
        &["public"],
    );
    // Assert that this package spec contains packages from both inside and outside the main repo.
    assert!(grp.contains(&pkg_id("pkg")));
    assert!(grp.contains(&pkg_id_in("somerepo", "pkg")));
}

// Java: PackageGroupTest.testNothingSpecificationWorks
#[test]
fn test_nothing_specification_works() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_package_group_has_public_syntax=true"]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"mango\",",
            "    packages = [\"private\"],",
            ")",
        ],
    );
    let grp = get_package_group(&t, "fruits", "mango");

    // Assert that we're using the right package spec.
    assert_contains_exactly(
        grp.get_contained_packages(/* include_double_slash= */ true),
        &["private"],
    );
    assert!(!grp.contains(&pkg_id("anything")));
}

// Java: PackageGroupTest.testPublicPrivateAreNotAccessibleWithoutFlag
#[test]
fn test_public_private_are_not_accessible_without_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        // Flag being tested
        "--incompatible_package_group_has_public_syntax=false",
        // Must also be disabled in order to disable the above
        "--incompatible_fix_package_group_reporoot_syntax=false",
    ]);

    t.scratch_file(
        "foo/BUILD",
        &[
            "package_group(",
            "    name = \"grp1\",",
            "    packages = [\"public\"],",
            ")",
        ],
    );
    t.scratch_file(
        "bar/BUILD",
        &[
            "package_group(",
            "    name = \"grp2\",",
            "    packages = [\"private\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "foo", "grp1");
    loaded.assert_contains_event(
        "Use of \"public\" package specification requires enabling \
         --incompatible_package_group_has_public_syntax",
    );
    let (_, loaded) = get_package_group_and_events(&t, "bar", "grp2");
    loaded.assert_contains_event(
        "Use of \"private\" package specification requires enabling \
         --incompatible_package_group_has_public_syntax",
    );
}

// Java: PackageGroupTest.testRepoRootSubpackagesIsPublic_withoutFlag
#[test]
fn test_repo_root_subpackages_is_public_without_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_fix_package_group_reporoot_syntax=false"]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"mango\",",
            "    packages = [\"//...\"],",
            ")",
        ],
    );
    let grp = get_package_group(&t, "fruits", "mango");

    // Use includeDoubleSlash=true to make package spec stringification distinguish AllPackages
    // from AllPackagesBeneath with empty package path.
    assert_contains_exactly(
        grp.get_contained_packages(/* include_double_slash= */ true),
        // Assert that "//..." gave us AllPackages.
        &["public"],
    );
    assert!(grp.contains(&pkg_id("pkg")));
    assert!(grp.contains(&pkg_id_in("somerepo", "pkg")));
}

// Java: PackageGroupTest.testRepoRootSubpackagesIsNotPublic_withFlag
#[test]
fn test_repo_root_subpackages_is_not_public_with_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--incompatible_package_group_has_public_syntax=true",
        "--incompatible_fix_package_group_reporoot_syntax=true",
    ]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"mango\",",
            "    packages = [\"//...\"],",
            ")",
        ],
    );
    let grp = get_package_group(&t, "fruits", "mango");

    // Use includeDoubleSlash=true to make package spec stringification distinguish AllPackages
    // from AllPackagesBeneath with empty package path.
    assert_contains_exactly(
        grp.get_contained_packages(/* include_double_slash= */ true),
        // Assert that "//..." gave us AllPackagesBeneath.
        &["//..."],
    );
    assert!(grp.contains(&pkg_id("pkg")));
    assert!(!grp.contains(&pkg_id_in("somerepo", "pkg")));
}

// Java: PackageGroupTest.testCannotUseNewRepoRootSyntaxWithoutPublicSyntax
#[test]
fn test_cannot_use_new_repo_root_syntax_without_public_syntax() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--incompatible_package_group_has_public_syntax=false",
        "--incompatible_fix_package_group_reporoot_syntax=true",
    ]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"mango\",",
            "    packages = [\"//something\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "mango");
    loaded.assert_contains_event(
        "Cannot use new \"//...\" meaning without allowing new \"public\" syntax.",
    );
}

// Java: PackageGroupTest.testNegative_repoRootSubpackages
#[test]
fn test_negative_repo_root_subpackages() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/BUILD",
        &[
            "package_group(",
            "    name = \"packages\",",
            "    packages = [",
            "        \"-//...\",",
            "        \"//pkg/one\",",
            "    ],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "test", "packages");
    assert!(!grp.contains(&pkg_id("pkg")));
    assert!(!grp.contains(&pkg_id("pkg/one")));
    assert!(!grp.contains(&pkg_id("pkg/one/two")));
}

// Java: PackageGroupTest.testNegative_public
#[test]
fn test_negative_public() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_package_group_has_public_syntax=true"]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"-public\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "apple");
    loaded.assert_contains_event("Cannot negate \"public\" package specification");
}

// Java: PackageGroupTest.testNegative_private
#[test]
fn test_negative_private() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_package_group_has_public_syntax=true"]);

    t.scratch_file(
        "fruits/BUILD",
        &[
            "package_group(",
            "    name = \"apple\",",
            "    packages = [\"-private\"],",
            ")",
        ],
    );

    let (_, loaded) = get_package_group_and_events(&t, "fruits", "apple");
    loaded.assert_contains_event("Cannot negate \"private\" package specification");
}

// Java: PackageGroupTest.testDuplicatePackage
#[test]
fn test_duplicate_package() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/BUILD",
        &[
            "package_group(",
            "    name = \"packages\",",
            "    packages = [\"//one/two\"],",
            ")",
        ],
    );

    let grp = get_package_group(&t, "test", "packages");
    assert!(grp.contains(&pkg_id("one/two")));
}

// Java: PackageGroupTest.testReduceForSerialization
//   setBuildLanguageOptions("--incompatible_package_group_has_public_syntax=true");
//   fruits/BUILD: package_group(name = "apple", packages = ["//vegetables"]) and
//   package_group(name = "mango", packages = ["public"]);
//   for each: assertThat(grp).hasSamePropertiesAs(grp.reduceForSerialization());
#[test]
#[ignore = "needs PackageGroup.reduceForSerialization (PackageGroup.java:175, the PackageGroupData TargetData; not ported) and the TargetData properties TargetDataSubject.hasSamePropertiesAs compares"]
fn test_reduce_for_serialization() {}

// Java: PackageGroupStaticInitializationTest.testNoDeadlockOnPackageGroupCreation
// Java checks against a class-initialization deadlock between PackageGroup and
// PackageSpecification: one thread parses a package specification and hands it over a
// SynchronousQueue while another loads a package_group and then takes it. Here the producer is
// a thread, the consumer is the test thread (the harness is not moved across threads), and the
// rendezvous is a zero-capacity channel. Java joins without a limit
// (`TestUtils.WAIT_TIMEOUT_MILLISECONDS` is `Long.MAX_VALUE`, a deadlock is the test runner's
// timeout); here a deadlock fails after 60 seconds.
#[test]
fn test_no_deadlock_on_package_group_creation() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "fruits/BUILD",
        &["package_group(name = 'mango', packages = ['//...'])"],
    );

    let (group_queue, group_queue_receiver) =
        std::sync::mpsc::sync_channel::<PackageSpecification>(0);
    let producing_thread = std::thread::spawn(move || {
        let default_repo_name = Label::parse_canonical_unchecked("//context")
            .get_repository()
            .clone();
        let specification = PackageSpecification::from_string(
            &EMPTY,
            &default_repo_name,
            "//fruits/...",
            /* allow_public_private= */ true,
            /* repo_root_means_current_repo= */ true,
        )
        .expect("a valid package specification");
        // The consumer hanging up is its failure, reported there.
        let _ = group_queue.send(specification);
    });

    // The consuming thread.
    get_package_group(&t, "fruits", "mango");
    group_queue_receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the producing thread did not hand over its PackageSpecification");

    producing_thread.join().expect("the producing thread");
}
