//! Port of PackageLookupFunctionTest (and its IgnoreLabelViolationsTest / ErrorLabelViolationsTest
//! subclasses, as the `ignore` and `error` modules).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/PackageLookupFunctionTest.java.

use blaze_cmdline::repository_name::{BAZEL_TOOLS, MAIN};
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_vfs::{PathFragment, Root};

use crate::build_file_name::BuildFileName;
use crate::package_lookup::{
    CrossRepositoryLabelViolationStrategy, ErrorReason, PackageLookupKey, PackageLookupValue,
};
use crate::test_util::{FakeRepos, TestSetup, value_of};

struct Fixture {
    setup: TestSetup,
    empty_package_path: blaze_vfs::Path,
}

impl Fixture {
    // Java: PackageLookupFunctionTest.setUp
    fn new(strategy: CrossRepositoryLabelViolationStrategy) -> Fixture {
        let mut setup = TestSetup::new(FakeRepos::default());
        setup.config.cross_repository_label_violation_strategy = strategy;
        let root_directory = setup.workspace().clone();
        let empty_package_path = root_directory.get_relative_str("somewhere/else");
        setup.scratch.file(&setup.ws("parentpackage/BUILD"), "");
        setup.set_package_path(vec![
            Root::from_path(empty_package_path.clone()),
            Root::from_path(root_directory),
        ]);
        Fixture {
            setup,
            empty_package_path,
        }
    }

    fn file(&self, rel: &str) {
        self.setup.scratch.file(&self.setup.ws(rel), "");
    }

    fn lookup_package(&self, package_name: &str) -> PackageLookupValue {
        self.lookup_package_id(PackageIdentifier::create_in_main_repo_str(package_name))
    }

    fn lookup_package_id(&self, package_id: PackageIdentifier) -> PackageLookupValue {
        let evaluator = self.setup.evaluator();
        (*value_of(&evaluator, PackageLookupKey::key(package_id))).clone()
    }

    fn root_directory(&self) -> Root {
        Root::from_path(self.setup.workspace().clone())
    }
}

fn test_no_build_file(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/nobuildfile/foo.txt");
    let package_lookup_value = f.lookup_package("parentpackage/nobuildfile");
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::NoBuildFile
    );
    assert!(!package_lookup_value.get_error_msg().is_empty());
}

fn test_no_build_file_and_no_parent_package(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("noparentpackage/foo.txt");
    let package_lookup_value = f.lookup_package("noparentpackage");
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::NoBuildFile
    );
    assert!(!package_lookup_value.get_error_msg().is_empty());
}

fn test_deleted_package(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/deletedpackage/BUILD");
    f.setup
        .config
        .deleted_packages
        .write()
        .insert(PackageIdentifier::create_in_main_repo_str(
            "parentpackage/deletedpackage",
        ));
    let package_lookup_value = f.lookup_package("parentpackage/deletedpackage");
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::DeletedPackage
    );
    assert!(!package_lookup_value.get_error_msg().is_empty());
}

fn test_ignored_package(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("ignored/subdir/BUILD");
    f.file("ignored/BUILD");
    f.setup.scratch.file(&f.setup.ws(".bazelignore"), "ignored");

    let pkgs = ["ignored/subdir", "ignored"];
    for pkg in pkgs {
        let package_lookup_value = f.lookup_package(pkg);
        assert!(!package_lookup_value.package_exists());
        assert_eq!(
            package_lookup_value.get_error_reason(),
            ErrorReason::DeletedPackage
        );
        assert!(!package_lookup_value.get_error_msg().is_empty());
    }

    // PORT: Java invalidates the .bazelignore FileStateValue; Blaze evaluates in a fresh
    // evaluator (each lookup_package builds one), which is the non-incremental equivalent.
    f.setup
        .scratch
        .file(&f.setup.ws(".bazelignore"), "not_ignored");
    for pkg in pkgs {
        let package_lookup_value = f.lookup_package(pkg);
        assert!(package_lookup_value.package_exists());
    }
}

fn test_invalid_package_name(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/invalidpackagename:42/BUILD");
    let package_lookup_value = f.lookup_package("parentpackage/invalidpackagename:42");
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::InvalidPackageName
    );
    assert!(!package_lookup_value.get_error_msg().is_empty());
}

fn test_directory_named_build(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.setup
        .scratch
        .dir(&f.setup.ws("parentpackage/isdirectory/BUILD"));
    let package_lookup_value = f.lookup_package("parentpackage/isdirectory");
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::NoBuildFile
    );
    assert!(!package_lookup_value.get_error_msg().is_empty());
}

fn test_everything_is_good_build(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/everythinggood/BUILD");
    let package_lookup_value = f.lookup_package("parentpackage/everythinggood");
    assert!(package_lookup_value.package_exists());
    assert_eq!(*package_lookup_value.get_root(), f.root_directory());
    assert_eq!(
        package_lookup_value.get_build_file_name(),
        BuildFileName::Build
    );
}

fn test_everything_is_good_build_bazel(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/everythinggood/BUILD.bazel");
    let package_lookup_value = f.lookup_package("parentpackage/everythinggood");
    assert!(package_lookup_value.package_exists());
    assert_eq!(*package_lookup_value.get_root(), f.root_directory());
    assert_eq!(
        package_lookup_value.get_build_file_name(),
        BuildFileName::BuildDotBazel
    );
}

fn test_everything_is_good_both(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("parentpackage/everythinggood/BUILD");
    f.file("parentpackage/everythinggood/BUILD.bazel");
    let package_lookup_value = f.lookup_package("parentpackage/everythinggood");
    assert!(package_lookup_value.package_exists());
    assert_eq!(*package_lookup_value.get_root(), f.root_directory());
    assert_eq!(
        package_lookup_value.get_build_file_name(),
        BuildFileName::BuildDotBazel
    );
}

fn test_build_files_in_multiple_package_paths(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.setup.scratch.file(
        &format!("{}/foo/BUILD", f.empty_package_path.get_path_string()),
        "",
    );
    f.file("foo/BUILD.bazel");

    // BUILD file in the first package path should be preferred to BUILD.bazel in the second.
    let package_lookup_value = f.lookup_package("foo");
    assert!(package_lookup_value.package_exists());
    assert_eq!(
        *package_lookup_value.get_root(),
        Root::from_path(f.empty_package_path.clone())
    );
    assert_eq!(
        package_lookup_value.get_build_file_name(),
        BuildFileName::Build
    );
}

fn test_empty_package_name(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("BUILD");
    let package_lookup_value = f.lookup_package("");
    assert!(package_lookup_value.package_exists());
    assert_eq!(*package_lookup_value.get_root(), f.root_directory());
    assert_eq!(
        package_lookup_value.get_build_file_name(),
        BuildFileName::Build
    );
}

fn invisible_repo_main(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    f.file("BUILD");
    let package_lookup_value = f.lookup_package_id(PackageIdentifier::create(
        MAIN.to_non_visible(&BAZEL_TOOLS),
        PathFragment::empty_fragment(),
    ));
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::RepositoryNotFound
    );
    assert!(
        package_lookup_value
            .get_error_msg()
            .contains("No repository visible as")
    );
}

fn invisible_repo_non_main(s: CrossRepositoryLabelViolationStrategy) {
    let f = Fixture::new(s);
    let package_lookup_value = f.lookup_package_id(PackageIdentifier::create(
        RepositoryName::create_unvalidated("local").to_non_visible(&BAZEL_TOOLS),
        PathFragment::empty_fragment(),
    ));
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::RepositoryNotFound
    );
    assert!(
        package_lookup_value
            .get_error_msg()
            .contains("No repository visible as")
    );
}

fn test_package_lookup_value_hash_code_and_equals_contract(
    _s: CrossRepositoryLabelViolationStrategy,
) {
    let scratch = crate::test_util::Scratch::new();
    let root1 = Root::from_path(scratch.path("root1"));
    let root2 = Root::from_path(scratch.path("root2"));
    // Our (seeming) duplication of parameters here is intentional. Some of the subclasses of
    // PackageLookupValue are supposed to have reference equality semantics, and some are
    // supposed to have logical equality semantics.
    let groups = vec![
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
    ];
    super::equals_tester(&groups);
}

macro_rules! strategy_tests {
    ($module:ident, $strategy:expr) => {
        mod $module {
            use super::*;

            #[test]
            fn test_no_build_file() {
                super::test_no_build_file($strategy);
            }
            #[test]
            fn test_no_build_file_and_no_parent_package() {
                super::test_no_build_file_and_no_parent_package($strategy);
            }
            #[test]
            fn test_deleted_package() {
                super::test_deleted_package($strategy);
            }
            #[test]
            fn test_ignored_package() {
                super::test_ignored_package($strategy);
            }
            #[test]
            fn test_invalid_package_name() {
                super::test_invalid_package_name($strategy);
            }
            #[test]
            fn test_directory_named_build() {
                super::test_directory_named_build($strategy);
            }
            #[test]
            fn test_everything_is_good_build() {
                super::test_everything_is_good_build($strategy);
            }
            #[test]
            fn test_everything_is_good_build_bazel() {
                super::test_everything_is_good_build_bazel($strategy);
            }
            #[test]
            fn test_everything_is_good_both() {
                super::test_everything_is_good_both($strategy);
            }
            #[test]
            fn test_build_files_in_multiple_package_paths() {
                super::test_build_files_in_multiple_package_paths($strategy);
            }
            #[test]
            fn test_empty_package_name() {
                super::test_empty_package_name($strategy);
            }
            #[test]
            fn invisible_repo_main() {
                super::invisible_repo_main($strategy);
            }
            #[test]
            fn invisible_repo_non_main() {
                super::invisible_repo_non_main($strategy);
            }
            #[test]
            fn test_package_lookup_value_hash_code_and_equals_contract() {
                super::test_package_lookup_value_hash_code_and_equals_contract($strategy);
            }
        }
    };
}

// Java: PackageLookupFunctionTest.IgnoreLabelViolationsTest
strategy_tests!(
    ignore_label_violations_test,
    CrossRepositoryLabelViolationStrategy::Ignore
);
// Java: PackageLookupFunctionTest.ErrorLabelViolationsTest
strategy_tests!(
    error_label_violations_test,
    CrossRepositoryLabelViolationStrategy::Error
);
