//! Port of `exec/TestPolicy.java`: a policy for running tests. It currently only encompasses the
//! environment computation for the test.

use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::analysis::test::test_runner_action::TestRunnerAction;

/// The user name of the user running Bazel; this may differ from ${USER} for tests that are run
/// remotely.
// Java: com/google/devtools/build/lib/exec/TestPolicy.java:32 TestPolicy.SYSTEM_USER_NAME
pub const SYSTEM_USER_NAME: &str = "${SYSTEM_USER_NAME}";

/// An absolute path to a writable directory that is reserved for the current test.
// Java: com/google/devtools/build/lib/exec/TestPolicy.java:35 TestPolicy.TEST_TMP_DIR
pub const TEST_TMP_DIR: &str = "${TEST_TMP_DIR}";

/// The path of the runfiles directory.
// Java: com/google/devtools/build/lib/exec/TestPolicy.java:38 TestPolicy.RUNFILES_DIR
pub const RUNFILES_DIR: &str = "${RUNFILES_DIR}";

// Java: com/google/devtools/build/lib/exec/TestPolicy.java:40 TestPolicy.INHERITED
pub const INHERITED: &str = "${inherited}";

/// `UserUtils.getUserName()`: the JVM's `user.name` system property, which is the name of the
/// effective user (`getpwuid(geteuid())`).
// Java: com/google/devtools/build/lib/util/UserUtils.java:37 UserUtils.getUserName
pub fn get_user_name() -> String {
    // SAFETY: getpwuid_r writes into the buffers we own; the result pointer is checked.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16384];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            libc::geteuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        );
        if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
            return std::ffi::CStr::from_ptr(pwd.pw_name)
                .to_string_lossy()
                .into_owned();
        }
    }
    // The JVM falls back to "?" when the user cannot be looked up.
    "?".to_string()
}

/// A policy for running tests. It currently only encompasses the environment computation for the
/// test.
// Java: com/google/devtools/build/lib/exec/TestPolicy.java:27 TestPolicy
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestPolicy {
    env_variables: IndexMap<String, String>,
}

impl TestPolicy {
    /// `TestPolicy.EMPTY_POLICY`.
    // Java: com/google/devtools/build/lib/exec/TestPolicy.java:42 TestPolicy.EMPTY_POLICY
    pub fn empty_policy() -> TestPolicy {
        TestPolicy::new(IndexMap::new())
    }

    /// Creates a new instance. The map's keys are the names of the environment variables, while
    /// the values can be either fixed values, or one of the constants in this module,
    /// specifically [`SYSTEM_USER_NAME`], [`TEST_TMP_DIR`], [`RUNFILES_DIR`], or [`INHERITED`].
    // Java: com/google/devtools/build/lib/exec/TestPolicy.java:51 TestPolicy.<init>
    pub fn new(env_variables: IndexMap<String, String>) -> TestPolicy {
        TestPolicy { env_variables }
    }

    /// Returns a mutable map of the environment variables for a specific test. This is intended
    /// to be the final, complete environment - callers should avoid relying on the mutability of
    /// the return value, and instead change the policy itself.
    // Java: com/google/devtools/build/lib/exec/TestPolicy.java:60 TestPolicy.computeTestEnvironment
    pub fn compute_test_environment(
        &self,
        test_action: &TestRunnerAction,
        client_env: &IndexMap<String, String>,
        relative_runfiles_dir: &PathFragment,
        tmp_dir: &PathFragment,
    ) -> IndexMap<String, String> {
        let mut env: IndexMap<String, String> = IndexMap::new();

        // Add all env variables, allow some string replacements and inheritance.
        let user_prop = get_user_name();
        let tmp_dir_path = tmp_dir.get_path_string();
        let runfiles_dir_path = relative_runfiles_dir.get_path_string();
        for (key, value) in &self.env_variables {
            let mut val = value.clone();
            if val.contains("${") {
                if val == INHERITED {
                    match client_env.get(key) {
                        None => continue,
                        Some(v) => val = v.clone(),
                    }
                } else {
                    val = val.replace(SYSTEM_USER_NAME, &user_prop);
                    val = val.replace(TEST_TMP_DIR, tmp_dir_path);
                    val = val.replace(RUNFILES_DIR, runfiles_dir_path);
                }
            }
            env.insert(key.clone(), val);
        }

        // Overwrite with the environment common to all actions, see --action_env.
        test_action
            .get_configuration()
            .action_environment
            .resolve(&mut env, client_env);

        // Overwrite with the environment common to all tests, see --test_env.
        test_action
            .get_configuration()
            .test_action_environment
            .resolve(&mut env, client_env);

        // Rule-specified test env.
        test_action
            .get_extra_test_env()
            .resolve(&mut env, client_env);

        // Setup bazel test-specific env variables; note that this does not overwrite
        // some values if they're already set.
        test_action.setup_env_variables(&mut env);

        env
    }
}
