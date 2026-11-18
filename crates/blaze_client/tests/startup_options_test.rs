//! Ports of src/test/cpp/startup_options_test.cc, src/test/cpp/bazel_startup_options_test.cc
//! and the helpers of src/test/cpp/test_util.cc.
//!
//! Every test takes `ENV` because several tests change the process environment (C++'s
//! SetEnv/UnsetEnv), and constructing StartupOptions reads it.

use std::sync::{Mutex, MutexGuard};

use blaze_client::platform::{get_home_dir, get_path_env, set_env, unset_env};
use blaze_client::startup_options::StartupOptions;
use blaze_client::util::file::get_cwd;
use blaze_client::util::path::Path;
use blaze_options::rc::option_processor::RcStartupFlag;

static ENV: Mutex<()> = Mutex::new(());

/// Holds the environment lock and restores the variables the fixture saves.
struct Fixture {
    saved: Vec<(&'static str, Option<String>)>,
    _guard: MutexGuard<'static, ()>,
}

impl Fixture {
    // Java: src/test/cpp/startup_options_test.cc:55 StartupOptionsTest::SetUp
    fn new() -> Self {
        let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let saved = ["HOME", "TEST_TMPDIR", "USER", "XDG_CACHE_HOME"]
            .into_iter()
            .map(|k| (k, std::env::var(k).ok()))
            .collect();
        Fixture {
            saved,
            _guard: guard,
        }
    }
}

impl Drop for Fixture {
    // Java: src/test/cpp/startup_options_test.cc:65 StartupOptionsTest::TearDown
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY: ENV is held for the whole test.
            unsafe {
                match v {
                    Some(v) => set_env(k, v),
                    None => unset_env(k),
                }
            }
        }
    }
}

fn setenv(k: &str, v: &str) {
    // SAFETY: callers hold ENV through a Fixture.
    unsafe { set_env(k, v) }
}

fn unsetenv(k: &str) {
    // SAFETY: callers hold ENV through a Fixture.
    unsafe { unset_env(k) }
}

fn flag(source: &str, value: &str) -> RcStartupFlag {
    RcStartupFlag {
        source: source.to_string(),
        value: value.to_string(),
    }
}

// Java: src/test/cpp/test_util.cc:26 ExpectValidNullaryOption
fn expect_valid_nullary_option(options: &StartupOptions, flag_name: &str) {
    assert_eq!(
        Ok(true),
        options.maybe_check_valid_nullary(&format!("--{}", flag_name))
    );
    assert_eq!(
        Ok(true),
        options.maybe_check_valid_nullary(&format!("--no{}", flag_name))
    );
    assert_eq!(
        Ok(false),
        options.maybe_check_valid_nullary(&format!("--{}__invalid", flag_name))
    );
    assert_eq!(
        Err(format!(
            "In argument '--{0}=foo': option '--{0}' does not take a value.",
            flag_name
        )),
        options.maybe_check_valid_nullary(&format!("--{}=foo", flag_name))
    );
    assert_eq!(
        Err(format!(
            "In argument '--no{0}=foo': option '--no{0}' does not take a value.",
            flag_name
        )),
        options.maybe_check_valid_nullary(&format!("--no{}=foo", flag_name))
    );
    assert!(!options.is_unary(&format!("--{}", flag_name)));
    assert!(!options.is_unary(&format!("--no{}", flag_name)));
}

// Java: src/test/cpp/test_util.cc:76 ExpectIsUnaryOption
fn expect_is_unary_option(options: &StartupOptions, flag_name: &str) {
    assert!(options.is_unary(&format!("--{}", flag_name)));
    assert!(options.is_unary(&format!("--{}=", flag_name)));
    assert!(options.is_unary(&format!("--{}=foo", flag_name)));
    assert!(!options.is_unary(&format!("--{}__invalid", flag_name)));
    assert_eq!(
        Ok(false),
        options.maybe_check_valid_nullary(&format!("--{}", flag_name))
    );
    assert_eq!(
        Ok(false),
        options.maybe_check_valid_nullary(&format!("--no{}", flag_name))
    );
}

// Java: src/test/cpp/test_util.cc:98 ParseStartupOptionsAndExpectWarning
fn parse_startup_options_and_expect_warning(
    startup_options: &mut StartupOptions,
    options_to_parse: &[&str],
    expected_warning: &str,
) {
    let flags: Vec<RcStartupFlag> = options_to_parse.iter().map(|o| flag("", o)).collect();
    assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
    let output: String = startup_options
        .startup_option_warnings()
        .into_iter()
        .map(|(_, m)| format!("WARNING: {}\n", m))
        .collect();
    assert_eq!(expected_warning, output);
}

// ---------------------------------------------------------------------------------------------
// startup_options_test.cc (FakeStartupOptions)

// Java: src/test/cpp/startup_options_test.cc:85 StartupOptionsTest.ProductName
#[test]
fn product_name() {
    let _f = Fixture::new();
    let startup_options = StartupOptions::fake_for_testing();
    assert_eq!("Bazel", startup_options.product_name);
}

// Java: src/test/cpp/startup_options_test.cc:89 StartupOptionsTest.JavaLoggingOptions
#[test]
fn java_logging_options() {
    let _f = Fixture::new();
    let startup_options = StartupOptions::fake_for_testing();
    assert_eq!(
        "com.google.devtools.build.lib.util.SingleLineFormatter",
        startup_options.java_logging_formatter
    );
}

// Java: src/test/cpp/startup_options_test.cc:94 StartupOptionsTest.OutputUserRootTildeExpansion
#[test]
fn output_user_root_tilde_expansion() {
    let _f = Fixture::new();
    let home_str = "/nonexistent/home/";
    setenv("HOME", home_str);
    let mut startup_options = StartupOptions::fake_for_testing();
    let home = Path::new(home_str);
    assert_eq!(home_str, get_home_dir());

    {
        let flags = vec![flag("somewhere", "--output_user_root=~/test")];
        assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
        assert_eq!(home.get_relative("test"), startup_options.output_user_root);
    }
    {
        let flags = vec![flag("somewhere", "--output_user_root=~")];
        assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
        assert_eq!(home, startup_options.output_user_root);
    }
}

// Java: src/test/cpp/startup_options_test.cc:136 StartupOptionsTest.EmptyFlagsAreInvalidTest
#[test]
fn empty_flags_are_invalid_test() {
    let _f = Fixture::new();
    let startup_options = StartupOptions::fake_for_testing();
    assert_eq!(Ok(false), startup_options.maybe_check_valid_nullary(""));
    assert_eq!(Ok(false), startup_options.maybe_check_valid_nullary("--"));
    assert!(!startup_options.is_unary(""));
    assert!(!startup_options.is_unary("--"));
}

// Java: src/test/cpp/startup_options_test.cc:156 StartupOptionsTest.ProcessSpaceSeparatedArgsTest
#[test]
fn process_space_separated_args_test() {
    let _f = Fixture::new();
    let mut startup_options = StartupOptions::fake_for_testing();
    let flags = vec![
        flag("somewhere", "--max_idle_secs"),
        flag("somewhere", "42"),
    ];
    assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
    assert_eq!(42, startup_options.max_idle_secs);
    assert_eq!(
        "somewhere",
        startup_options.original_startup_options_[0].source
    );
    assert_eq!(
        "--max_idle_secs=42",
        startup_options.original_startup_options_[0].value
    );
}

// Java: src/test/cpp/startup_options_test.cc:173 StartupOptionsTest.ProcessEqualsSeparatedArgsTest
#[test]
fn process_equals_separated_args_test() {
    let _f = Fixture::new();
    let mut startup_options = StartupOptions::fake_for_testing();
    let flags = vec![flag("somewhere", "--max_idle_secs=36")];
    assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
    assert_eq!(36, startup_options.max_idle_secs);
    assert_eq!(
        "somewhere",
        startup_options.original_startup_options_[0].source
    );
    assert_eq!(
        "--max_idle_secs=36",
        startup_options.original_startup_options_[0].value
    );
}

// Java: src/test/cpp/startup_options_test.cc:189 StartupOptionsTest.ProcessIncorrectArgValueTest
#[test]
fn process_incorrect_arg_value_test() {
    let _f = Fixture::new();
    let mut startup_options = StartupOptions::fake_for_testing();
    let flags = vec![flag("somewhere", "--max_idle_secs=notANumber")];
    assert_eq!(
        Err("Invalid argument to --max_idle_secs: 'notANumber'.".to_string()),
        startup_options.process_args_impl(&flags)
    );
    // Even for a failing args processing step, expect the original value to be stored.
    assert_eq!(
        "somewhere",
        startup_options.original_startup_options_[0].source
    );
    assert_eq!(
        "--max_idle_secs=notANumber",
        startup_options.original_startup_options_[0].value
    );
}

// Java: src/test/cpp/startup_options_test.cc:206 StartupOptionsTest.ProcessArgsWithMultipleArgstest
#[test]
fn process_args_with_multiple_argstest() {
    let _f = Fixture::new();
    let mut startup_options = StartupOptions::fake_for_testing();
    let flags = vec![
        flag("somewhere", "--max_idle_secs=36"),
        flag("somewhereElse", "--nowrite_command_log"),
    ];
    assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
    assert_eq!(36, startup_options.max_idle_secs);
    assert!(!startup_options.write_command_log);
    assert_eq!(
        "somewhere",
        startup_options.original_startup_options_[0].source
    );
    assert_eq!(
        "--max_idle_secs=36",
        startup_options.original_startup_options_[0].value
    );
    assert_eq!(
        "somewhereElse",
        startup_options.original_startup_options_[1].source
    );
    assert_eq!(
        "--nowrite_command_log",
        startup_options.original_startup_options_[1].value
    );
}

// ---------------------------------------------------------------------------------------------
// bazel_startup_options_test.cc

// Java: src/test/cpp/bazel_startup_options_test.cc:48 BazelStartupOptionsTest::UpdateConfiguration
fn update_configuration(startup_options: &mut StartupOptions) {
    startup_options.update_configuration("deadbeef", "workspace", false);
}

// Java: src/test/cpp/bazel_startup_options_test.cc:58 BazelStartupOptionsTest.ProductName
#[test]
fn bazel_product_name() {
    let _f = Fixture::new();
    assert_eq!("Bazel", StartupOptions::bazel().product_name);
}

// Java: src/test/cpp/bazel_startup_options_test.cc:62 BazelStartupOptionsTest.JavaLoggingOptions
#[test]
fn bazel_java_logging_options() {
    let _f = Fixture::new();
    assert_eq!(
        "com.google.devtools.build.lib.util.SingleLineFormatter",
        StartupOptions::bazel().java_logging_formatter
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:67 BazelStartupOptionsTest.EmptyFlagsAreInvalid
#[test]
fn empty_flags_are_invalid() {
    let _f = Fixture::new();
    let startup_options = StartupOptions::bazel();
    assert_eq!(Ok(false), startup_options.maybe_check_valid_nullary(""));
    assert_eq!(Ok(false), startup_options.maybe_check_valid_nullary("--"));
    assert!(!startup_options.is_unary(""));
    assert!(!startup_options.is_unary("--"));
}

// Java: src/test/cpp/bazel_startup_options_test.cc:88 BazelStartupOptionsTest.UpdateConfigurationOnLinuxOrDarwinWithHome
#[test]
fn update_configuration_on_linux_or_darwin_with_home() {
    let _f = Fixture::new();
    setenv("USER", "gandalf");
    setenv("HOME", "/nonexistent/home");
    unsetenv("TEST_TMPDIR");
    unsetenv("XDG_CACHE_HOME");
    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);

    assert_eq!(
        Path::new("/nonexistent/home/.cache/bazel/_bazel_gandalf"),
        startup_options.output_user_root
    );
    assert_eq!(
        Path::new("/nonexistent/home/.cache/bazel/_bazel_gandalf/install/deadbeef"),
        startup_options.install_base
    );
    assert_eq!(
        Path::new("/nonexistent/home/.cache/bazel/_bazel_gandalf/1629dee48cc4e53161f9b2be8614e062"),
        startup_options.output_base
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:120 BazelStartupOptionsTest.UpdateConfigurationOnLinuxOrDarwinWithTestTmpdir
#[test]
fn update_configuration_on_linux_or_darwin_with_test_tmpdir() {
    let _f = Fixture::new();
    setenv("USER", "gandalf");
    setenv("HOME", "/nonexistent/home");
    setenv("XDG_CACHE_HOME", "/nonexistent/cache");
    setenv("TEST_TMPDIR", "/nonexistent/tmpdir");
    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);

    assert_eq!(
        Path::new("/nonexistent/tmpdir/_bazel_gandalf"),
        startup_options.output_user_root
    );
    assert_eq!(
        Path::new("/nonexistent/tmpdir/_bazel_gandalf/install/deadbeef"),
        startup_options.install_base
    );
    assert_eq!(
        Path::new("/nonexistent/tmpdir/_bazel_gandalf/1629dee48cc4e53161f9b2be8614e062"),
        startup_options.output_base
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:139 BazelStartupOptionsTest.UpdateConfigurationOnLinuxOrDarwinWithXdgCacheHome
#[test]
fn update_configuration_on_linux_or_darwin_with_xdg_cache_home() {
    let _f = Fixture::new();
    setenv("USER", "gandalf");
    setenv("HOME", "/nonexistent/home");
    setenv("XDG_CACHE_HOME", "/nonexistent/cache");
    unsetenv("TEST_TMPDIR");
    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);

    assert_eq!(
        Path::new("/nonexistent/cache/bazel/_bazel_gandalf"),
        startup_options.output_user_root
    );
    assert_eq!(
        Path::new("/nonexistent/cache/bazel/_bazel_gandalf/install/deadbeef"),
        startup_options.install_base
    );
    assert_eq!(
        Path::new("/nonexistent/cache/bazel/_bazel_gandalf/1629dee48cc4e53161f9b2be8614e062"),
        startup_options.output_base
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:158 BazelStartupOptionsTest.UpdateConfigurationOnLinuxOrDarwinNoShellExpansion
#[test]
fn update_configuration_on_linux_or_darwin_no_shell_expansion() {
    let _f = Fixture::new();
    setenv("USER", "gandalf");
    setenv("TEST_TMPDIR", "~/\"$foo/test\"");
    setenv("XDG_CACHE_HOME", "~/cache${bar}");
    setenv("HOME", "~/home$(echo baz)");

    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);
    assert_eq!(
        Path::new(&(get_cwd() + "/~/\"$foo/test\"" + "/_bazel_gandalf")),
        startup_options.output_user_root
    );

    unsetenv("TEST_TMPDIR");
    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);
    assert_eq!(
        Path::new(&(get_cwd() + "/~/cache${bar}/bazel/_bazel_gandalf")),
        startup_options.output_user_root
    );

    unsetenv("XDG_CACHE_HOME");
    let mut startup_options = StartupOptions::bazel();
    update_configuration(&mut startup_options);
    assert_eq!(
        Path::new(&(get_cwd() + "/~/home$(echo baz)/.cache/bazel/_bazel_gandalf")),
        startup_options.output_user_root
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:198 BazelStartupOptionsTest.UpdateConfigurationOnWindowsWithHome
#[test]
#[ignore = "Windows only"]
fn update_configuration_on_windows_with_home() {}

// Java: src/test/cpp/bazel_startup_options_test.cc:214 BazelStartupOptionsTest.UpdateConfigurationOnWindowsWithUserProfile
#[test]
#[ignore = "Windows only"]
fn update_configuration_on_windows_with_user_profile() {}

// Java: src/test/cpp/bazel_startup_options_test.cc:234 BazelStartupOptionsTest.ValidStartupFlags
#[test]
fn valid_startup_flags() {
    let _f = Fixture::new();
    let options = StartupOptions::bazel();
    expect_valid_nullary_option(&options, "batch");
    expect_valid_nullary_option(&options, "batch_cpu_scheduling");
    expect_valid_nullary_option(&options, "block_for_lock");
    expect_valid_nullary_option(&options, "client_debug");
    expect_valid_nullary_option(&options, "fatal_event_bus_exceptions");
    expect_valid_nullary_option(&options, "home_rc");
    expect_valid_nullary_option(&options, "host_jvm_debug");
    expect_valid_nullary_option(&options, "autodetect_server_javabase");
    expect_valid_nullary_option(&options, "ignore_all_rc_files");
    expect_valid_nullary_option(&options, "shutdown_on_low_sys_mem");
    expect_valid_nullary_option(&options, "system_rc");
    expect_valid_nullary_option(&options, "workspace_rc");
    expect_valid_nullary_option(&options, "write_command_log");
    expect_is_unary_option(&options, "bazelrc");
    expect_is_unary_option(&options, "command_port");
    expect_is_unary_option(&options, "connect_timeout_secs");
    expect_is_unary_option(&options, "digest_function");
    expect_is_unary_option(&options, "host_jvm_args");
    expect_is_unary_option(&options, "install_base");
    expect_is_unary_option(&options, "invocation_policy");
    expect_is_unary_option(&options, "io_nice_level");
    expect_is_unary_option(&options, "local_startup_timeout_secs");
    expect_is_unary_option(&options, "macos_qos_class");
    expect_is_unary_option(&options, "max_idle_secs");
    expect_is_unary_option(&options, "output_base");
    expect_is_unary_option(&options, "output_user_root");
    expect_is_unary_option(&options, "server_javabase");
}

// Java: src/test/cpp/bazel_startup_options_test.cc:268 BazelStartupOptionsTest.BlazercFlagsAreNotAccepted
#[test]
fn blazerc_flags_are_not_accepted() {
    let _f = Fixture::new();
    let startup_options = StartupOptions::bazel();
    assert_eq!(
        Ok(false),
        startup_options.maybe_check_valid_nullary("--master_blazerc")
    );
    assert!(!startup_options.is_unary("--master_blazerc"));
    assert_eq!(
        Ok(false),
        startup_options.maybe_check_valid_nullary("--blazerc")
    );
    assert!(!startup_options.is_unary("--blazerc"));
}

// Java: src/test/cpp/bazel_startup_options_test.cc:290 BazelStartupOptionsTest.IgnoredBazelrcFlagWarns
#[test]
fn ignored_bazelrc_flag_warns() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--bazelrc=somefile", "--ignore_all_rc_files"],
        "WARNING: Value of --bazelrc is ignored, since --ignore_all_rc_files is on.\n",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:297 BazelStartupOptionsTest.IgnoredBazelrcFlagWarnsWhenAfterIgnore
#[test]
fn ignored_bazelrc_flag_warns_when_after_ignore() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--ignore_all_rc_files", "--bazelrc=somefile"],
        "WARNING: Value of --bazelrc is ignored, since --ignore_all_rc_files is on.\n",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:304 BazelStartupOptionsTest.IgnoredWorkspaceRcFlagWarns
#[test]
fn ignored_workspace_rc_flag_warns() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--workspace_rc", "--ignore_all_rc_files"],
        "WARNING: Explicit value of --workspace_rc is ignored, since --ignore_all_rc_files is \
         on.\n",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:311 BazelStartupOptionsTest.IgnoredWorkspaceRcFlagWarnsAfterIgnore
#[test]
fn ignored_workspace_rc_flag_warns_after_ignore() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--ignore_all_rc_files", "--workspace_rc"],
        "WARNING: Explicit value of --workspace_rc is ignored, since --ignore_all_rc_files is \
         on.\n",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:318 BazelStartupOptionsTest.MultipleIgnoredRcFlagsWarnOnceEach
#[test]
fn multiple_ignored_rc_flags_warn_once_each() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &[
            "--workspace_rc",
            "--bazelrc=somefile",
            "--ignore_all_rc_files",
            "--bazelrc=thefinalfile",
            "--workspace_rc",
        ],
        "WARNING: Value of --bazelrc is ignored, since --ignore_all_rc_files is on.\n\
         WARNING: Explicit value of --workspace_rc is ignored, since --ignore_all_rc_files is \
         on.\n",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:329 BazelStartupOptionsTest.IgnoredNoMasterBazelrcDoesNotWarn
#[test]
fn ignored_no_master_bazelrc_does_not_warn() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--ignore_all_rc_files"],
        "",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:337 BazelStartupOptionsTest.IgnoreOptionDoesNotWarnOnItsOwn
#[test]
fn ignore_option_does_not_warn_on_its_own() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--ignore_all_rc_files"],
        "",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:342 BazelStartupOptionsTest.NonIgnoredOptionDoesNotWarn
#[test]
fn non_ignored_option_does_not_warn() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--bazelrc=somefile"],
        "",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:347 BazelStartupOptionsTest.FinalValueOfIgnoreIsUsedForWarning
#[test]
fn final_value_of_ignore_is_used_for_warning() {
    let _f = Fixture::new();
    parse_startup_options_and_expect_warning(
        &mut StartupOptions::bazel(),
        &["--ignore_all_rc_files", "--noignore_all_rc_files"],
        "",
    );
}

// Java: src/test/cpp/bazel_startup_options_test.cc:354 BazelStartupOptionsTest.LockInstallBase
#[test]
fn lock_install_base() {
    let _f = Fixture::new();
    assert!(StartupOptions::bazel().lock_install_base);
}

/// The no-rc flags are rejected in rc files with the C++ message.
#[test]
fn no_rc_flag_in_rc_file_is_rejected() {
    let _f = Fixture::new();
    let mut startup_options = StartupOptions::bazel();
    assert_eq!(
        Err("Can't specify --nohome_rc in the .bazelrc file.".to_string()),
        startup_options.process_args_impl(&[flag("/x/.bazelrc", "--nohome_rc")])
    );
    assert_eq!(
        Err("Can't specify --bazelrc in the RC file.".to_string()),
        StartupOptions::bazel().process_args_impl(&[flag("/x/.bazelrc", "--bazelrc=foo")])
    );
    assert_eq!(
        Err(
            "Unknown startup option: '--foo'.\n  For more info, run 'bazel help \
             startup_options'."
                .to_string()
        ),
        StartupOptions::bazel().process_args_impl(&[flag("", "--foo")])
    );
}

#[test]
fn test_tmpdir_is_read_from_the_environment() {
    let _f = Fixture::new();
    setenv("TEST_TMPDIR", "/x");
    assert_eq!("/x", get_path_env("TEST_TMPDIR"));
    assert_eq!(15, StartupOptions::bazel().max_idle_secs);
    unsetenv("TEST_TMPDIR");
    assert_eq!(10800, StartupOptions::bazel().max_idle_secs);
}
