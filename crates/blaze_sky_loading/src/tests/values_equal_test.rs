//! Change pruning of this crate's keys (package I2): `Key::values_equal` against the Java
//! value classes' `equals`, through Guava `EqualsTester` groups where the Java test suite has
//! them, and through the evaluator (a recomputed equal value does not rebuild its parent).

use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_skyframe::testing::{parent_rebuilt, values_equal_tester};
use blaze_vfs::{PathFragment, Root, RootedPath};

use crate::actions::FileStateValue;
use crate::build_file_name::BuildFileName;
use crate::containing_package_lookup::{ContainingPackageLookupKey, ContainingPackageLookupValue};
use crate::file_state::FileStateKey;
use crate::local_repository_lookup::{LocalRepositoryLookupKey, LocalRepositoryLookupValue};
use crate::package_lookup::{PackageLookupKey, PackageLookupValue};
use crate::recursive_pkg::{RecursivePkgKey, RecursivePkgValue};

fn no_repository(name: &str) -> PackageLookupValue {
    PackageLookupValue::NoRepository {
        repository_name: RepositoryName::create(name).unwrap(),
        reason: "no such repository".to_string(),
    }
}

/// Java's `testPackageLookupValueHashCodeAndEqualsContract` groups, over `values_equal`, plus
/// `NoRepositoryPackageLookupValue` (no `equals`: identity, so each is its own group).
// Java: PackageLookupFunctionTest.testPackageLookupValueHashCodeAndEqualsContract
#[test]
fn package_lookup_values_equal_contract() {
    let scratch = crate::test_util::Scratch::new();
    let root1 = Root::from_path(scratch.path("root1"));
    let root2 = Root::from_path(scratch.path("root2"));
    values_equal_tester::<PackageLookupKey>(&[
        vec![
            PackageLookupValue::success(root1.clone(), BuildFileName::Build),
            PackageLookupValue::success(root1.clone(), BuildFileName::Build),
        ],
        vec![
            PackageLookupValue::success(root2.clone(), BuildFileName::Build),
            PackageLookupValue::success(root2.clone(), BuildFileName::Build),
        ],
        vec![
            PackageLookupValue::NO_BUILD_FILE_VALUE,
            PackageLookupValue::NO_BUILD_FILE_VALUE,
        ],
        vec![
            PackageLookupValue::DELETED_PACKAGE_VALUE,
            PackageLookupValue::DELETED_PACKAGE_VALUE,
        ],
        vec![
            PackageLookupValue::invalid_package_name("nope1"),
            PackageLookupValue::invalid_package_name("nope1"),
        ],
        vec![
            PackageLookupValue::invalid_package_name("nope2"),
            PackageLookupValue::invalid_package_name("nope2"),
        ],
    ]);
    assert!(!<PackageLookupKey as blaze_skyframe::Key>::values_equal(
        &no_repository("foo"),
        &no_repository("foo")
    ));
}

/// Java's `testEqualsAndHashCodeContract` over `values_equal`, plus a `noContainingPackage`
/// with a reason (a new object per computation: never equal).
// Java: ContainingPackageLookupFunctionTest.testEqualsAndHashCodeContract
#[test]
fn containing_package_lookup_values_equal_contract() {
    let scratch = crate::test_util::Scratch::new();
    let root = Root::from_path(scratch.path("root"));
    let other_root = Root::from_path(scratch.path("other_root"));
    let b = PackageIdentifier::create_in_main_repo_str("b");
    let c = PackageIdentifier::create_in_main_repo_str("c");
    values_equal_tester::<ContainingPackageLookupKey>(&[
        vec![
            ContainingPackageLookupValue::NONE,
            ContainingPackageLookupValue::NONE,
        ],
        vec![
            ContainingPackageLookupValue::with_containing_package(b.clone(), root.clone()),
            ContainingPackageLookupValue::with_containing_package(b, root.clone()),
        ],
        vec![
            ContainingPackageLookupValue::with_containing_package(c.clone(), root.clone()),
            ContainingPackageLookupValue::with_containing_package(c.clone(), root),
        ],
        vec![ContainingPackageLookupValue::with_containing_package(
            c, other_root,
        )],
    ]);
    let reason = ContainingPackageLookupValue::no_containing_package("not a package");
    assert!(!<ContainingPackageLookupKey as blaze_skyframe::Key>::values_equal(&reason, &reason));
}

/// Java: a successful lookup compares the repository name only; all main and all not-found
/// values are equal.
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:174 SuccessfulLocalRepositoryLookupValue.equals
#[test]
fn local_repository_lookup_values_equal_contract() {
    let repo = RepositoryName::create("repo").unwrap();
    values_equal_tester::<LocalRepositoryLookupKey>(&[
        vec![
            LocalRepositoryLookupValue::main_repository(),
            LocalRepositoryLookupValue::main_repository(),
        ],
        vec![
            LocalRepositoryLookupValue::success(repo.clone(), PathFragment::create("a")),
            LocalRepositoryLookupValue::success(repo, PathFragment::create("b")),
        ],
        vec![LocalRepositoryLookupValue::success(
            RepositoryName::create("other").unwrap(),
            PathFragment::create("a"),
        )],
        vec![
            LocalRepositoryLookupValue::NotFound,
            LocalRepositoryLookupValue::NotFound,
        ],
    ]);
}

/// Java: only the `EMPTY` singleton (no packages, no errors) is equal to itself.
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgValue.java:51 RecursivePkgValue.create
#[test]
fn recursive_pkg_values_equal_only_when_empty() {
    let pkg = || RecursivePkgValue::for_testing(vec!["a".to_string()], false);
    values_equal_tester::<RecursivePkgKey>(&[
        vec![
            RecursivePkgValue::for_testing(vec![], false),
            RecursivePkgValue::for_testing(vec![], false),
        ],
        vec![pkg()],
        vec![pkg()],
        vec![RecursivePkgValue::for_testing(vec![], true)],
    ]);
}

fn rooted(scratch: &crate::test_util::Scratch, rel: &str) -> RootedPath {
    RootedPath::to_rooted_path(
        Root::from_path(scratch.path("")),
        &PathFragment::create(rel),
    )
}

/// Change pruning through the evaluator: a package lookup recomputed to an equal success
/// does not rebuild its parent; a `NoRepository` lookup always does (Java identity).
#[test]
fn recomputed_equal_package_lookup_prunes_parent() {
    let scratch = crate::test_util::Scratch::new();
    let root = Root::from_path(scratch.path("root"));
    let key = PackageLookupKey::key(PackageIdentifier::create_in_main_repo_str("pkg"));
    assert!(!parent_rebuilt(
        key.clone(),
        PackageLookupValue::success(root.clone(), BuildFileName::Build),
        PackageLookupValue::success(root.clone(), BuildFileName::Build),
    ));
    assert!(parent_rebuilt(
        key.clone(),
        PackageLookupValue::success(root.clone(), BuildFileName::Build),
        PackageLookupValue::success(root, BuildFileName::BuildDotBazel),
    ));
    assert!(parent_rebuilt(
        key,
        no_repository("foo"),
        no_repository("foo")
    ));
}

/// A file state recomputed equal (same digest and size) prunes; a changed size does not.
#[test]
fn recomputed_equal_file_state_prunes_parent() {
    let scratch = crate::test_util::Scratch::new();
    let key = FileStateKey::key(rooted(&scratch, "f"));
    let file = |size| FileStateValue::RegularFileWithDigest {
        size,
        digest: vec![1, 2, 3],
    };
    assert!(!parent_rebuilt(key.clone(), file(3), file(3)));
    assert!(parent_rebuilt(key, file(3), file(4)));
}

/// A local repository lookup that only moved its path prunes (Java ignores the path).
#[test]
fn recomputed_local_repository_lookup_with_other_path_prunes_parent() {
    let scratch = crate::test_util::Scratch::new();
    let repo = RepositoryName::create("repo").unwrap();
    assert!(!parent_rebuilt(
        LocalRepositoryLookupKey::key(rooted(&scratch, "dir")),
        LocalRepositoryLookupValue::success(repo.clone(), PathFragment::create("a")),
        LocalRepositoryLookupValue::success(repo, PathFragment::create("b")),
    ));
}
