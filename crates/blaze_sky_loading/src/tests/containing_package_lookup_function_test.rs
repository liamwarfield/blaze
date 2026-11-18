//! Port of ContainingPackageLookupFunctionTest.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/ContainingPackageLookupFunctionTest.java.

use blaze_cmdline::PackageIdentifier;
use blaze_vfs::{PathFragment, Root};

use crate::containing_package_lookup::{ContainingPackageLookupKey, ContainingPackageLookupValue};
use crate::package_lookup::{ErrorReason, PackageLookupKey, PackageLookupValue};
use crate::test_util::{FakeRepos, TestSetup, value_of};

struct Fixture {
    setup: TestSetup,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            setup: TestSetup::new(FakeRepos::default()),
        }
    }

    fn file(&self, rel: &str) {
        self.setup.scratch.file(&self.setup.ws(rel), "");
    }

    fn root_directory(&self) -> Root {
        Root::from_path(self.setup.workspace().clone())
    }

    fn lookup_containing_package(&self, package_name: &str) -> ContainingPackageLookupValue {
        self.lookup_containing_package_id(PackageIdentifier::create_in_main_repo_str(package_name))
    }

    fn lookup_containing_package_id(
        &self,
        package_identifier: PackageIdentifier,
    ) -> ContainingPackageLookupValue {
        let evaluator = self.setup.evaluator();
        (*value_of(
            &evaluator,
            ContainingPackageLookupKey::key(package_identifier),
        ))
        .clone()
    }

    fn lookup_package(&self, package_identifier: PackageIdentifier) -> PackageLookupValue {
        let evaluator = self.setup.evaluator();
        (*value_of(&evaluator, PackageLookupKey::key(package_identifier))).clone()
    }
}

#[test]
fn test_no_containing_package() {
    let f = Fixture::new();
    let value = f.lookup_containing_package("a/b");
    assert!(!value.has_containing_package());
}

#[test]
fn test_containing_package_is_parent() {
    let f = Fixture::new();
    f.file("a/BUILD");
    let value = f.lookup_containing_package("a/b");
    assert!(value.has_containing_package());
    assert_eq!(
        *value.get_containing_package_name(),
        PackageIdentifier::create_in_main_repo_str("a")
    );
    assert_eq!(*value.get_containing_package_root(), f.root_directory());
}

#[test]
fn test_containing_package_is_self() {
    let f = Fixture::new();
    f.file("a/b/BUILD");
    let value = f.lookup_containing_package("a/b");
    assert!(value.has_containing_package());
    assert_eq!(
        *value.get_containing_package_name(),
        PackageIdentifier::create_in_main_repo_str("a/b")
    );
    assert_eq!(*value.get_containing_package_root(), f.root_directory());
}

#[test]
fn test_equals_and_hash_code_contract() {
    let f = Fixture::new();
    let root_directory = f.setup.workspace().clone();
    let value_a1 = ContainingPackageLookupValue::NONE;
    let value_a2 = ContainingPackageLookupValue::NONE;
    let value_b1 = ContainingPackageLookupValue::with_containing_package(
        PackageIdentifier::create_in_main_repo_str("b"),
        Root::from_path(root_directory.clone()),
    );
    let value_b2 = ContainingPackageLookupValue::with_containing_package(
        PackageIdentifier::create_in_main_repo_str("b"),
        Root::from_path(root_directory.clone()),
    );
    let c_frag = PackageIdentifier::create_in_main_repo_str("c");
    let value_c1 = ContainingPackageLookupValue::with_containing_package(
        c_frag.clone(),
        Root::from_path(root_directory.clone()),
    );
    let value_c2 = ContainingPackageLookupValue::with_containing_package(
        c_frag.clone(),
        Root::from_path(root_directory.clone()),
    );
    let value_c_other = ContainingPackageLookupValue::with_containing_package(
        c_frag,
        Root::from_path(root_directory.get_relative_str("other_root")),
    );
    super::equals_tester(&[
        vec![value_a1, value_a2],
        vec![value_b1, value_b2],
        vec![value_c1, value_c2],
        vec![value_c_other],
    ]);
}

#[test]
fn test_non_existent_external_repository_error_reason() {
    let f = Fixture::new();
    let identifier =
        PackageIdentifier::create_from_repo_string("some_repo", PathFragment::create(":atarget"))
            .unwrap();
    let value = f.lookup_containing_package_id(identifier);
    assert!(!value.has_containing_package());
    assert!(matches!(
        value,
        ContainingPackageLookupValue::NoContainingPackage { .. }
    ));
    assert_eq!(
        value.get_reason_for_no_containing_package(),
        Some(
            "The repository '@@some_repo' could not be resolved: Repository '@@some_repo' is not \
             defined"
        )
    );
}

#[test]
fn test_invalid_package_label_error_reason() {
    let f = Fixture::new();
    let value = f.lookup_containing_package("invalidpackagename:42/BUILD");
    assert!(!value.has_containing_package());
    assert!(matches!(
        value,
        ContainingPackageLookupValue::NoContainingPackage { .. }
    ));
    // As for invalid package name we continue to climb up the parent packages,
    // we will find the top-level package with the path "" - empty string.
    assert_eq!(value.get_reason_for_no_containing_package(), None);
}

#[test]
fn test_deleted_package_error_reason() {
    let f = Fixture::new();
    let identifier = PackageIdentifier::create_in_main_repo_str("deletedpackage");
    f.setup
        .config
        .deleted_packages
        .write()
        .insert(identifier.clone());
    f.file("BUILD");

    let package_lookup_value = f.lookup_package(identifier.clone());
    assert!(!package_lookup_value.package_exists());
    assert_eq!(
        package_lookup_value.get_error_reason(),
        ErrorReason::DeletedPackage
    );
    assert_eq!(
        package_lookup_value.get_error_msg(),
        "Package is considered deleted due to --deleted_packages"
    );

    let value = f.lookup_containing_package_id(identifier);
    assert!(value.has_containing_package());
    assert!(value.get_containing_package_name().to_string().is_empty());
    assert!(matches!(
        value,
        ContainingPackageLookupValue::ContainingPackage { .. }
    ));
}

#[test]
fn test_no_build_file_error_reason() {
    let f = Fixture::new();
    let value = f.lookup_containing_package("abc");
    assert!(!value.has_containing_package());
    assert!(matches!(
        value,
        ContainingPackageLookupValue::NoContainingPackage { .. }
    ));
    assert_eq!(value.get_reason_for_no_containing_package(), None);
}
