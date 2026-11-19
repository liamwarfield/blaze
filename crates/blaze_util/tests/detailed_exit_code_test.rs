//! Port of `src/test/java/com/google/devtools/build/lib/util/DetailedExitCodeTest.java`, plus
//! tests of the category-name constructors Blaze uses to rebuild a reduced `DetailedExitCode`
//! (expected exit codes from the `(metadata).exit_code` options in failure_details.proto).

use blaze_protos::failure_details::{
    FailureDetail, Interrupted, failure_detail::Category, interrupted,
};
use blaze_util::{DetailedExitCode, ExitCode};

// Java: src/test/java/com/google/devtools/build/lib/util/DetailedExitCodeTest.java:31 DetailedExitCodeTest.testGetInterruptedExitCode
#[test]
fn test_get_interrupted_exit_code() {
    let code = DetailedExitCode::of(FailureDetail {
        message: "interrupted".to_owned(),
        category: Some(Category::Interrupted(Interrupted {
            code: interrupted::Code::InterruptedUnknown as i32,
        })),
    });
    assert_eq!(code.get_exit_code(), ExitCode::INTERRUPTED);
}

/// failure_details.proto: `ExternalDeps.MODULE_NOT_FOUND = 1 [(metadata) = { exit_code: 48 }]`.
#[test]
fn of_category_name_external_deps() {
    let code = DetailedExitCode::of_category_name("msg", "ExternalDeps", Some("MODULE_NOT_FOUND"))
        .unwrap();
    assert_eq!(code.get_exit_code().get_numeric_exit_code(), 48);
    let detail = code.get_failure_detail().unwrap();
    assert_eq!(detail.message, "msg");
    match &detail.category {
        Some(Category::ExternalDeps(e)) => assert_eq!(e.code, 1),
        other => panic!("unexpected category {other:?}"),
    }
}

/// `ExternalRepository.getDefaultInstance()`: code 0, `EXTERNAL_REPOSITORY_UNKNOWN`
/// `[(metadata) = { exit_code: 37 }]`.
#[test]
fn of_category_name_default_instance() {
    let code = DetailedExitCode::of_category_name("m", "ExternalRepository", None).unwrap();
    assert_eq!(code.get_exit_code().get_numeric_exit_code(), 37);
}

/// `BuildConfiguration.INVALID_OUTPUT_DIRECTORY_MNEMONIC = 11 [(metadata) = { exit_code: 2 }]`.
#[test]
fn of_category_name_build_configuration() {
    let code = DetailedExitCode::of_category_name(
        "m",
        "BuildConfiguration",
        Some("INVALID_OUTPUT_DIRECTORY_MNEMONIC"),
    )
    .unwrap();
    assert_eq!(code.get_exit_code().get_numeric_exit_code(), 2);
}

/// `PackageLoading.BUILD_FILE_MISSING = 3 [(metadata) = { exit_code: 1 }]`.
#[test]
fn of_category_value_package_loading() {
    let code = DetailedExitCode::of_category_value("m", "PackageLoading", 3).unwrap();
    assert_eq!(code.get_exit_code().get_numeric_exit_code(), 1);
}

#[test]
fn of_category_name_unknown_names() {
    assert!(DetailedExitCode::of_category_name("m", "NoSuchCategory", None).is_none());
    assert!(
        DetailedExitCode::of_category_name("m", "ExternalDeps", Some("NO_SUCH_CODE")).is_none()
    );
}
