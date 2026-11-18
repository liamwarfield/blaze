//! Port of src/test/cpp/blaze_util_test.cc. SearchNullaryOption and GetAllUnaryOptionValues are
//! ported in blaze_options (rc processing), which the client uses.

use blaze_options::rc::option_processor::{get_all_unary_option_values, search_nullary_option};

fn v(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn search(args: &[&str], flag: &str, default: bool) -> bool {
    search_nullary_option(&v(args), flag, default).unwrap()
}

fn all_unary(args: &[&str], key: &str) -> Vec<String> {
    get_all_unary_option_values(&v(args), key, None)
}

// Java: src/test/cpp/blaze_util_test.cc:109 BlazeUtilTest.ReadFrom
#[test]
#[ignore = "blaze_util::ReadFrom(fd) is not ported: the client reads files with std::fs"]
fn read_from() {}

// Java: src/test/cpp/blaze_util_test.cc:111 BlazeUtilTest.TestSearchNullaryEmptyCase
#[test]
fn test_search_nullary_empty_case() {
    assert!(!search(&[], "flag", false));
    assert!(search(&[], "flag", true));
}

// Java: src/test/cpp/blaze_util_test.cc:116 BlazeUtilTest.TestSearchNullaryForEmpty
#[test]
fn test_search_nullary_for_empty() {
    assert!(search(&["bazel", "build", ":target"], "", true));
    assert!(!search(&["bazel", "build", ":target"], "", false));
}

// Java: src/test/cpp/blaze_util_test.cc:121 BlazeUtilTest.TestSearchNullaryForFlagNotPresent
#[test]
fn test_search_nullary_for_flag_not_present() {
    assert!(!search(&["bazel", "build", ":target"], "flag", false));
    assert!(search(&["bazel", "build", ":target"], "flag", true));
}

// Java: src/test/cpp/blaze_util_test.cc:128 BlazeUtilTest.TestSearchNullaryStartupOption
#[test]
fn test_search_nullary_startup_option() {
    assert!(search(
        &["bazel", "--flag", "build", ":target"],
        "flag",
        false
    ));
    assert!(search(
        &["bazel", "--flag", "build", ":target"],
        "flag",
        true
    ));
}

// Java: src/test/cpp/blaze_util_test.cc:135 BlazeUtilTest.TestSearchNullaryStartupOptionWithEquals
#[test]
fn test_search_nullary_startup_option_with_equals() {
    // ASSERT_DEATH in C++ (GetNullaryOption's BAZEL_DIE); an error here.
    let err = search_nullary_option(
        &v(&["bazel", "--flag=value", "build", ":target"]),
        "flag",
        false,
    )
    .unwrap_err();
    assert!(err.contains("In argument '--flag=value': option '--flag' does not take a value"));
}

// Java: src/test/cpp/blaze_util_test.cc:142 BlazeUtilTest.TestSearchNullaryCommandOption
#[test]
fn test_search_nullary_command_option() {
    assert!(search(
        &["bazel", "build", ":target", "--flag"],
        "flag",
        false
    ));
}

// Java: src/test/cpp/blaze_util_test.cc:147 BlazeUtilTest.TestSearchNullarySkipsAfterDashDash
#[test]
fn test_search_nullary_skips_after_dash_dash() {
    assert!(!search(
        &["bazel", "build", ":target", "--", "--flag"],
        "flag",
        false
    ));
}

// Java: src/test/cpp/blaze_util_test.cc:152 BlazeUtilTest.TestSearchNullarySucceedsWithEqualsAndDashDash
#[test]
fn test_search_nullary_succeeds_with_equals_and_dash_dash() {
    assert!(!search(
        &["bazel", "build", ":target", "--", "--flag=value"],
        "flag",
        false
    ));
}

// Java: src/test/cpp/blaze_util_test.cc:157 BlazeUtilTest.TestSearchNullaryLastFlagWins
#[test]
fn test_search_nullary_last_flag_wins() {
    assert!(!search(
        &["bazel", "--flag", "--noflag", "build"],
        "flag",
        false
    ));
    assert!(!search(
        &["bazel", "--flag", "--noflag", "build"],
        "flag",
        true
    ));
}

// Java: src/test/cpp/blaze_util_test.cc:169 BlazeUtilTest.TestGetAllUnaryForEmpty
#[test]
fn test_get_all_unary_for_empty() {
    assert_eq!(v(&[]), all_unary(&["bazel", "build", ":target"], ""));
}

// Java: src/test/cpp/blaze_util_test.cc:174 BlazeUtilTest.TestGetAllUnaryFlagNotPresent
#[test]
fn test_get_all_unary_flag_not_present() {
    assert_eq!(v(&[]), all_unary(&["bazel", "build", ":target"], "--flag"));
}

// Java: src/test/cpp/blaze_util_test.cc:179 BlazeUtilTest.TestGetAllUnaryOptionWithEquals
#[test]
fn test_get_all_unary_option_with_equals() {
    assert_eq!(
        v(&["value"]),
        all_unary(&["bazel", "--flag=value", "build", ":target"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:185 BlazeUtilTest.TestGetAllUnaryOptionWithEquals2
#[test]
fn test_get_all_unary_option_with_equals2() {
    assert_eq!(
        v(&["value1", "value2"]),
        all_unary(
            &[
                "bazel",
                "--flag=value1",
                "--flag=value2",
                "build",
                ":target"
            ],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:193 BlazeUtilTest.TestGetAllUnaryOptionWithRepeatingFlag
#[test]
fn test_get_all_unary_option_with_repeating_flag() {
    assert_eq!(
        v(&["--flag"]),
        all_unary(
            &["bazel", "--flag", "--flag", "value1", "build", ":target"],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:200 BlazeUtilTest.TestGetAllUnaryOptionWithRepeatingFlagOptions
#[test]
fn test_get_all_unary_option_with_repeating_flag_options() {
    assert_eq!(
        v(&["--flag"]),
        all_unary(&["bazel", "--flag", "--flag", "value1"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:206 BlazeUtilTest.TestGetAllUnaryOptionValuesWithEquals
#[test]
fn test_get_all_unary_option_values_with_equals() {
    assert_eq!(
        v(&["--flag", "value1"]),
        all_unary(&["bazel", "--flag=--flag", "--flag", "value1"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:213 BlazeUtilTest.TestGetAllUnaryOptionWithEquals3
#[test]
fn test_get_all_unary_option_with_equals3() {
    assert_eq!(
        v(&["value1", "value2", "value3"]),
        all_unary(
            &[
                "bazel",
                "--flag=value1",
                "--flag=value2",
                "--flag=value3",
                "build",
                ":target"
            ],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:221 BlazeUtilTest.TestGetAllUnaryOptionWithoutEquals
#[test]
fn test_get_all_unary_option_without_equals() {
    assert_eq!(
        v(&["value"]),
        all_unary(&["bazel", "--flag", "value", "build", ":target"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:228 BlazeUtilTest.TestGetAllUnaryOptionWithoutEquals2
#[test]
fn test_get_all_unary_option_without_equals2() {
    assert_eq!(
        v(&["value1", "value2"]),
        all_unary(
            &[
                "bazel", "--flag", "value1", "--flag", "value2", "build", ":target"
            ],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:236 BlazeUtilTest.TestGetAllUnaryCommandOptionWithEquals
#[test]
fn test_get_all_unary_command_option_with_equals() {
    assert_eq!(
        v(&["value"]),
        all_unary(&["bazel", "build", ":target", "--flag", "value"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:243 BlazeUtilTest.TestGetAllUnaryCommandOptionWithEquals2
#[test]
fn test_get_all_unary_command_option_with_equals2() {
    assert_eq!(
        v(&["value1", "value2"]),
        all_unary(
            &[
                "bazel", "build", ":target", "--flag", "value1", "--flag", "value2"
            ],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:251 BlazeUtilTest.TestGetAllUnaryCommandOptionWithoutEquals
#[test]
fn test_get_all_unary_command_option_without_equals() {
    assert_eq!(
        v(&["value"]),
        all_unary(&["bazel", "build", ":target", "--flag=value"], "--flag")
    );
}

// Java: src/test/cpp/blaze_util_test.cc:257 BlazeUtilTest.TestGetAllUnaryCommandOptionWithoutEquals2
#[test]
fn test_get_all_unary_command_option_without_equals2() {
    assert_eq!(
        v(&["value1", "value2"]),
        all_unary(
            &[
                "bazel",
                "build",
                ":target",
                "--flag=value1",
                "--flag=value2"
            ],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:265 BlazeUtilTest.TestGetAllUnarySkipsAfterDashDashWithEquals
#[test]
fn test_get_all_unary_skips_after_dash_dash_with_equals() {
    assert_eq!(
        v(&[]),
        all_unary(
            &["bazel", "build", ":target", "--", "--flag", "value"],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:272 BlazeUtilTest.TestGetAllUnarySkipsAfterDashDashWithoutEquals
#[test]
fn test_get_all_unary_skips_after_dash_dash_without_equals() {
    assert_eq!(
        v(&[]),
        all_unary(
            &["bazel", "build", ":target", "--", "--flag=value"],
            "--flag"
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:278 BlazeUtilTest.TestGetAllUnaryCommandOptionWithIgnoreAfter
#[test]
fn test_get_all_unary_command_option_with_ignore_after() {
    assert_eq!(
        v(&["value1", "/dev/null"]),
        get_all_unary_option_values(
            &v(&[
                "bazel",
                "build",
                ":target",
                "--flag",
                "value1",
                "--flag",
                "/dev/null",
                "--flag",
                "value3"
            ]),
            "--flag",
            Some("/dev/null")
        )
    );
}

// Java: src/test/cpp/blaze_util_test.cc:286 BlazeUtilTest.TestGetAllUnaryCommandOptionWithIgnoreAfterDevNull
#[test]
fn test_get_all_unary_command_option_with_ignore_after_dev_null() {
    assert_eq!(
        v(&["/dev/null"]),
        get_all_unary_option_values(
            &v(&[
                "bazel",
                "build",
                ":target",
                "--flag",
                "/dev/null",
                "--flag",
                "value2",
                "--flag",
                "value3"
            ]),
            "--flag",
            Some("/dev/null")
        )
    );
}

// Java: src/test/cpp/blaze_util_posix_test.cc:117 UnlimitResourcesTest.SuccessWithExplicitLimits
#[test]
#[ignore = "needs a fork per test to run UnlimitResources with GetExplicitSystemLimit stubbed; \
            Linux's GetExplicitSystemLimit is -1"]
fn success_with_explicit_limits() {}

// Java: src/test/cpp/blaze_util_posix_test.cc:144 UnlimitResourcesTest.SuccessWithPossiblyInfiniteLimits
#[test]
#[ignore = "needs a fork per test (raises this process's rlimits)"]
fn success_with_possibly_infinite_limits() {}

// Java: src/test/cpp/blaze_util_posix_test.cc:182 UnlimitResourcesTest.Coredumps
#[test]
#[ignore = "needs a fork per test (raises this process's rlimits)"]
fn coredumps() {}
