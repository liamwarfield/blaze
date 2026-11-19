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

// Java: src/test/cpp/blaze_util_test.cc:41 BlazeUtilTest.ForkAndWrite
fn fork_and_write(fds: [libc::c_int; 2], input1: &str, input2: &str) {
    // SAFETY: the child only calls close/write/usleep/_exit (async-signal-safe) on buffers
    // prepared before the fork.
    unsafe {
        let r = libc::fork();
        if r == 0 {
            libc::close(fds[0]);
            libc::write(fds[1], input1.as_ptr().cast(), input1.len());
            libc::usleep(500); // sleep for 50ms
            libc::write(fds[1], input2.as_ptr().cast(), input2.len());
            libc::close(fds[1]);
            libc::_exit(0);
        } else if r < 0 {
            panic!("fork(): {}", std::io::Error::last_os_error());
        } else {
            libc::close(fds[1]);
        }
    }
}

// Java: src/test/cpp/blaze_util_test.cc:58 BlazeUtilTest.WriteFileDescriptor2
fn write_file_descriptor2(input1: &str, input2: &str) -> libc::c_int {
    // create a fd for the input string
    let mut fds: [libc::c_int; 2] = [-1, -1];
    // SAFETY: plain pipe/fcntl/write/close calls on descriptors created here.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) == -1 {
            return -1;
        }
        if libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK) == -1
            || libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK) == -1
        {
            return -1;
        }
        if !input2.is_empty() {
            fork_and_write(fds, input1, input2);
        } else {
            libc::write(fds[1], input1.as_ptr().cast(), input1.len());
            libc::close(fds[1]);
        }
    }
    fds[0]
}

// Java: src/test/cpp/blaze_util_test.cc:77 BlazeUtilTest.AssertReadFrom2
fn assert_read_from2(input1: &str, input2: &str) {
    let fd = write_file_descriptor2(input1, input2);
    assert!(fd >= 0, "Unable to create a pipe!");
    let result = blaze_client::util::file::read_from(fd, 0);
    // SAFETY: fd was returned by write_file_descriptor2.
    unsafe { libc::close(fd) };
    let result = result.expect("Unable to read file descriptor!");
    assert_eq!(format!("{}{}", input1, input2).into_bytes(), result);
}

// Java: src/test/cpp/blaze_util_test.cc:95 BlazeUtilTest.AssertReadFrom
fn assert_read_from(input: &str) {
    assert_read_from2(input, "");
}

// Java: src/test/cpp/blaze_util_test.cc:109 BlazeUtilTest.ReadFrom
#[test]
fn read_from() {
    assert_read_from("DummyJDK Blabla\nMore DummyJDK Blabla\n");
    assert_read_from("dummyjdk version \"1.42.qual\"\nDummyJDK Blabla\nMore DummyJDK Blabla\n");
    assert_read_from2("first_line\n", "second line version \"1.4.2_0\"\n");
}

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

// Java: src/test/cpp/blaze_util_posix_test.cc:41 UnlimitResourcesTest
/// Runs `body` in a forked subprocess, as the C++ fixture does: resource limits are process-wide,
/// so the testing must not interfere with the other tests of this program. The body must not use
/// the test framework's assertions; it reports a failure by returning `Err` (`Die`).
fn in_forked_child(body: fn() -> Result<(), String>) {
    // SAFETY: the child runs `body` (system calls and the client's UnlimitResources) and leaves
    // with _exit, without returning into the test harness.
    let pid = unsafe { libc::fork() };
    assert_ne!(-1, pid);
    if pid == 0 {
        let code = match body() {
            Ok(()) => libc::EXIT_SUCCESS,
            Err(message) => {
                // Die(): print and _exit(EXIT_FAILURE).
                // SAFETY: write(2) of a valid buffer.
                unsafe { libc::write(2, message.as_ptr().cast(), message.len()) };
                libc::EXIT_FAILURE
            }
        };
        // SAFETY: ends the child without running the parent's destructors.
        unsafe { libc::_exit(code) };
    }
    let mut status: libc::c_int = 0;
    // SAFETY: waits for the child forked above.
    assert_ne!(-1, unsafe { libc::waitpid(pid, &mut status, 0) });
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::EXIT_SUCCESS, libc::WEXITSTATUS(status));
}

/// Description of the resource limits to test for.
const LIMITS: [(&str, libc::__rlimit_resource_t); 2] = [
    ("RLIMIT_NOFILE", libc::RLIMIT_NOFILE),
    ("RLIMIT_NPROC", libc::RLIMIT_NPROC),
];

// Java: src/test/cpp/blaze_util_posix_test.cc:90 UnlimitResourcesTest.GetrlimitOrDie
fn getrlimit_or_die(resource: libc::__rlimit_resource_t) -> Result<libc::rlimit, String> {
    let mut rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: rl is a valid out-parameter.
    if unsafe { libc::getrlimit(resource, &mut rl) } == -1 {
        return Err(format!(
            "getrlimit({}) failed: {}\n",
            resource,
            std::io::Error::last_os_error()
        ));
    }
    Ok(rl)
}

// Java: src/test/cpp/blaze_util_posix_test.cc:100 UnlimitResourcesTest.SetrlimitOrDie
fn setrlimit_or_die(resource: libc::__rlimit_resource_t, rl: libc::rlimit) -> Result<(), String> {
    // SAFETY: rl is a valid limit.
    if unsafe { libc::setrlimit(resource, &rl) } == -1 {
        return Err(format!(
            "setrlimit({}) failed with cur={}, max={}: {}\n",
            resource,
            rl.rlim_cur,
            rl.rlim_max,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

// Java: src/test/cpp/blaze_util_posix_test.cc:117 UnlimitResourcesTest.SuccessWithExplicitLimits
#[test]
fn success_with_explicit_limits() {
    in_forked_child(|| {
        // Lower the limits to very low values that should always work.
        for (_, resource) in LIMITS {
            let mut rl = getrlimit_or_die(resource)?;
            rl.rlim_cur = 1;
            rl.rlim_max = 8;
            setrlimit_or_die(resource, rl)?;
        }

        if !blaze_client::platform::unlimit_resources() {
            return Err("UnlimitResources returned error; see output for diagnostics\n".into());
        }

        // Check that the soft limits were raised to the explicit hard limits we set.
        for (name, resource) in LIMITS {
            let rl = getrlimit_or_die(resource)?;
            if rl.rlim_cur != rl.rlim_max {
                return Err(format!(
                    "UnlimitResources did not increase the soft {} to its hard limit\n",
                    name
                ));
            }
        }
        Ok(())
    });
}

// Java: src/test/cpp/blaze_util_posix_test.cc:144 UnlimitResourcesTest.SuccessWithPossiblyInfiniteLimits
#[test]
fn success_with_possibly_infinite_limits() {
    in_forked_child(|| {
        if blaze_client::platform::get_explicit_system_limit(-1) == -1 {
            eprintln!(
                "GetExplicitSystemLimit not implemented for this platform; cannot verify the \
                 behavior of UnlimitResources"
            );
            return Ok(());
        }

        // Lower only the soft limits to very low values and assume that the hard limits are set
        // to infinity; otherwise, there is nothing we can do because we may not have permissions
        // to increase them.
        for (name, resource) in LIMITS {
            let mut rl = getrlimit_or_die(resource)?;
            if rl.rlim_max != libc::RLIM_INFINITY {
                eprintln!(
                    "Hard resource limit for {} is not infinity; will not be able to \
                     meaningfully test anything",
                    name
                );
            }
            rl.rlim_cur = 1;
            setrlimit_or_die(resource, rl)?;
        }

        if !blaze_client::platform::unlimit_resources() {
            return Err("UnlimitResources returned error; see output for diagnostics\n".into());
        }

        // Check that the soft limits were increased to a higher explicit number.
        for (name, resource) in LIMITS {
            let rl = getrlimit_or_die(resource)?;
            if rl.rlim_cur == 1 || rl.rlim_cur == libc::RLIM_INFINITY {
                return Err(format!(
                    "UnlimitResources did not increase the soft {} to the system limit\n",
                    name
                ));
            }
        }
        Ok(())
    });
}

// Java: src/test/cpp/blaze_util_posix_test.cc:182 UnlimitResourcesTest.Coredumps
#[test]
fn coredumps() {
    in_forked_child(|| {
        // Lower only the soft limit to a very low value and assume that the hard limit is
        // non-zero.
        let mut rl = getrlimit_or_die(libc::RLIMIT_CORE)?;
        if rl.rlim_max <= 1 {
            eprintln!(
                "Hard resource limit for RLIMIT_CORE is {}; cannot test anything meaningful",
                rl.rlim_max
            );
            return Ok(());
        }
        rl.rlim_cur = 1;
        setrlimit_or_die(libc::RLIMIT_CORE, rl)?;

        if !blaze_client::platform::unlimit_coredumps() {
            return Err("UnlimitCoredumps returned error; see output for diagnostics\n".into());
        }

        // Check that the soft limits were increased to a higher explicit number.
        let rl = getrlimit_or_die(libc::RLIMIT_CORE)?;
        if rl.rlim_cur == 1 {
            return Err(
                "UnlimitCoredumps did not increase the soft RLIMIT_CORE to the system limit\n"
                    .into(),
            );
        }
        Ok(())
    });
}
