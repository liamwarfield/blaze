//! Port of `lib/util/TestType.java`: whether this process runs inside a test, from the
//! environment the test runner sets (`TEST_TMPDIR`, and `BAZEL_SHELL_TEST` for shell integration
//! tests). A server started by a test's client inherits that environment.

use std::sync::OnceLock;

/// The kind of test this process runs in.
// Java: com/google/devtools/build/lib/util/TestType.java:24 TestType
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestType {
    Production,
    UnknownTest,
    JavaIntegration,
    ShellIntegration,
}

static TEST_TYPE: OnceLock<TestType> = OnceLock::new();

impl TestType {
    // Java: com/google/devtools/build/lib/util/TestType.java:52 TestType.inTest
    pub fn in_test(self) -> bool {
        !matches!(self, TestType::Production)
    }

    // Java: com/google/devtools/build/lib/util/TestType.java:38 TestType.getTestTypeFromEnvVars
    fn get_test_type_from_env_vars() -> TestType {
        let in_test = std::env::var_os("TEST_TMPDIR").is_some();
        let in_shell_integration_test = std::env::var_os("BAZEL_SHELL_TEST").is_some();
        if in_shell_integration_test {
            TestType::ShellIntegration
        } else if in_test {
            TestType::UnknownTest
        } else {
            TestType::Production
        }
    }

    // Java: com/google/devtools/build/lib/util/TestType.java:44 TestType.getTestType
    pub fn get_test_type() -> TestType {
        *TEST_TYPE.get_or_init(TestType::get_test_type_from_env_vars)
    }

    // Java: com/google/devtools/build/lib/util/TestType.java:48 TestType.isInTest
    pub fn is_in_test() -> bool {
        TestType::get_test_type().in_test()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_production_is_not_in_test() {
        assert!(!TestType::Production.in_test());
        assert!(TestType::UnknownTest.in_test());
        assert!(TestType::JavaIntegration.in_test());
        assert!(TestType::ShellIntegration.in_test());
    }
}
