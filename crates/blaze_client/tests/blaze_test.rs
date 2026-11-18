//! Tests of blaze.cc functions against values the oracle (Bazel 9.3.2 on this machine) printed.

use std::os::unix::fs::PermissionsExt;

use blaze_client::archive_utils::{LoggingInfo, RestartReason};
use blaze_client::blaze::{
    add_logging_args, build_run_request, escape_for_option_source, get_argument_string,
    get_server_exe_args_with,
};
use blaze_client::platform::{DurationMillis, get_hashed_base_dir};
use blaze_client::startup_options::StartupOptions;
use blaze_client::util::file::{make_canonical, make_directories, read_file};
use blaze_client::util::path::Path;
use blaze_client::workspace_layout::WorkspaceLayout;
use blaze_options::rc::option_processor::RcStartupFlag;

const ORACLE_ROOT: &str = "/home/liamwarfield/.cache/blaze/oracle-out/ur-w1";
const BROTLI: &str = "/home/liamwarfield/.cache/blaze/work/w1/brotli";
const RE2: &str = "/home/liamwarfield/.cache/blaze/work/w1/re2";
const ORACLE_INSTALL_MD5: &str = "58cf1ecbff441bac762b0050d6da21e9";

/// `<ORACLE_ROOT>/73a93e10e881efba110a09db7263fe1d/server/cmdline` as the oracle wrote it (one
/// argument per line; `<ROOT>` is the output user root).
const ORACLE_BROTLI_CMDLINE: &[&str] = &[
    "bazel(brotli)",
    "--add-opens=java.base/java.lang=ALL-UNNAMED",
    "-Xverify:none",
    "-Djava.util.logging.config.file=<ROOT>/73a93e10e881efba110a09db7263fe1d/javalog.properties",
    "-Dcom.google.devtools.build.lib.util.LogHandlerQuerier.class=com.google.devtools.build.lib.util.SimpleLogHandler$HandlerQuerier",
    "-XX:-MaxFDLimit",
    "-Djava.lang.Thread.allowVirtualThreads=true",
    "-XX:OnOutOfMemoryError=touch <ROOT>/73a93e10e881efba110a09db7263fe1d/blaze_oomed",
    "-Djava.library.path=<ROOT>/install/58cf1ecbff441bac762b0050d6da21e9/embedded_tools/jdk/lib:<ROOT>/install/58cf1ecbff441bac762b0050d6da21e9/embedded_tools/jdk/lib/server:<ROOT>/install/58cf1ecbff441bac762b0050d6da21e9/",
    "-Dfile.encoding=ISO-8859-1",
    "-Duser.country=",
    "-Duser.language=",
    "-Duser.variant=",
    "-Djdk.nio.file.WatchService.maxEventsPerPoll=10000",
    "-Dsun.misc.unsafe.memory.access=allow",
    "-jar",
    "<ROOT>/install/58cf1ecbff441bac762b0050d6da21e9/A-server.jar",
    "--max_idle_secs=10800",
    "--noshutdown_on_low_sys_mem",
    "--connect_timeout_secs=30",
    "--output_user_root=<ROOT>",
    "--install_base=<ROOT>/install/58cf1ecbff441bac762b0050d6da21e9",
    "--install_md5=58cf1ecbff441bac762b0050d6da21e9",
    "--lock_install_base",
    "--output_base=<ROOT>/73a93e10e881efba110a09db7263fe1d",
    "--workspace_directory=/home/liamwarfield/.cache/blaze/work/w1/brotli",
    "--default_system_javabase=/usr/lib/jvm/java-11-temurin-jdk",
    "--failure_detail_out=<ROOT>/73a93e10e881efba110a09db7263fe1d/failure_detail.rawproto",
    "--idle_server_tasks",
    "--write_command_log",
    "--nofatal_event_bus_exceptions",
    "--nowindows_enable_symlinks",
    "--client_debug=false",
    "--product_name=Bazel",
    "--option_sources=home_Urc::output_Uuser_Uroot::system_Urc:",
];

/// The oracle archive's members that matter for GetServerExeArgs (the server jar first, the
/// shared libraries, `install_base_key` last), in zip order.
const ORACLE_ARCHIVE_CONTENTS: &[&str] = &[
    "A-server.jar",
    "build-label.txt",
    "daemonize",
    "embedded_tools/MODULE.bazel",
    "embedded_tools/jdk/bin/java",
    "embedded_tools/jdk/lib/libdt_socket.so",
    "embedded_tools/jdk/lib/libextnet.so",
    "embedded_tools/jdk/lib/libinstrument.so",
    "embedded_tools/jdk/lib/libjava.so",
    "embedded_tools/jdk/lib/libjdwp.so",
    "embedded_tools/jdk/lib/libjimage.so",
    "embedded_tools/jdk/lib/libjli.so",
    "embedded_tools/jdk/lib/libjsig.so",
    "embedded_tools/jdk/lib/libmanagement.so",
    "embedded_tools/jdk/lib/libmanagement_ext.so",
    "embedded_tools/jdk/lib/libnet.so",
    "embedded_tools/jdk/lib/libnio.so",
    "embedded_tools/jdk/lib/libsimdsort.so",
    "embedded_tools/jdk/lib/libsyslookup.so",
    "embedded_tools/jdk/lib/libverify.so",
    "embedded_tools/jdk/lib/libzip.so",
    "embedded_tools/jdk/lib/server/libjsig.so",
    "embedded_tools/jdk/lib/server/libjvm.so",
    "libcpu_profiler.so",
    "linux-sandbox",
    "platforms/os/BUILD",
    "process-wrapper",
    "xcode-locator",
    "install_base_key",
];

fn cmdline_flag(value: &str) -> RcStartupFlag {
    RcStartupFlag {
        source: String::new(),
        value: value.to_string(),
    }
}

/// Output base directory names the oracle used (`<output_user_root>/<md5(workspace)>`).
#[test]
fn output_base_is_md5_of_workspace() {
    let root = Path::new(ORACLE_ROOT);
    assert_eq!(
        "8f50d653dfc169b8c3e11480ca4388c0",
        get_hashed_base_dir(&root, RE2).get_base_name()
    );
    assert_eq!(
        "73a93e10e881efba110a09db7263fe1d",
        get_hashed_base_dir(&root, BROTLI).get_base_name()
    );
    assert_eq!(
        Path::new(&format!("{}/73a93e10e881efba110a09db7263fe1d", ORACLE_ROOT)),
        get_hashed_base_dir(&root, BROTLI)
    );
}

/// GetServerExeArgs reproduces the oracle's server/cmdline for brotli, given the oracle's
/// install md5, archive contents and system javabase (output user root moved to a temp dir).
#[test]
fn get_server_exe_args_matches_oracle_cmdline() {
    let tmp = tempfile::tempdir().unwrap();
    let root = format!("{}/ur-w1", make_canonical(tmp.path().to_str().unwrap()));

    let mut startup_options = StartupOptions::bazel();
    startup_options
        .process_args_impl(&[
            cmdline_flag(&format!("--output_user_root={}", root)),
            cmdline_flag("--nohome_rc"),
            cmdline_flag("--nosystem_rc"),
        ])
        .unwrap();
    startup_options.update_configuration(ORACLE_INSTALL_MD5, BROTLI, false);
    assert_eq!(
        format!("{}/73a93e10e881efba110a09db7263fe1d", root),
        startup_options.output_base.as_printable_path()
    );
    assert_eq!(
        format!("{}/install/{}", root, ORACLE_INSTALL_MD5),
        startup_options.install_base.as_printable_path()
    );

    // The embedded JDK GetJvm() sanity-checks, and the output base AddJVMLoggingArguments writes
    // javalog.properties into.
    let java = startup_options
        .install_base
        .get_relative("embedded_tools/jdk/bin/java");
    assert!(make_directories(
        &java.get_parent().as_printable_path(),
        0o755
    ));
    std::fs::write(java.as_str(), "").unwrap();
    std::fs::set_permissions(java.as_str(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(make_directories(
        &startup_options.output_base.as_printable_path(),
        0o755
    ));

    let jvm_path = startup_options.get_jvm();
    assert_eq!(java, jvm_path);
    let archive_contents: Vec<String> = ORACLE_ARCHIVE_CONTENTS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (args, first_startup_arg) = get_server_exe_args_with(
        &jvm_path,
        "A-server.jar",
        &archive_contents,
        ORACLE_INSTALL_MD5,
        &WorkspaceLayout::new(),
        BROTLI,
        &startup_options,
        &|| "/usr/lib/jvm/java-11-temurin-jdk".to_string(),
    );

    let expected: Vec<String> = ORACLE_BROTLI_CMDLINE
        .iter()
        .map(|a| a.replace("<ROOT>", &root))
        .collect();
    assert_eq!(expected, args);
    assert_eq!(17, first_startup_arg);
    assert_eq!("--max_idle_secs=10800", args[first_startup_arg]);

    // AddJVMLoggingArguments wrote the logging properties.
    let props = read_file(
        &startup_options
            .output_base
            .get_relative("javalog.properties")
            .as_printable_path(),
        0,
    )
    .unwrap();
    assert_eq!(
        format!(
            "handlers=com.google.devtools.build.lib.util.SimpleLogHandler\n\
             .level=INFO\n\
             com.google.devtools.build.lib.util.SimpleLogHandler.level=INFO\n\
             com.google.devtools.build.lib.util.SimpleLogHandler.prefix={}/73a93e10e881efba110a09db7263fe1d/java.log\n\
             com.google.devtools.build.lib.util.SimpleLogHandler.limit=1024000\n\
             com.google.devtools.build.lib.util.SimpleLogHandler.total_limit=20971520\n\
             com.google.devtools.build.lib.util.SimpleLogHandler.formatter=com.google.devtools.build.lib.util.SingleLineFormatter\n",
            root
        ),
        String::from_utf8(props).unwrap()
    );

    // The cmdline file is the NUL-joined argument vector.
    let joined = get_argument_string(&args);
    assert_eq!(expected.join("\0").into_bytes(), joined);
}

/// The oracle's own server/cmdline file, when present on this machine, matches the expected
/// list above.
#[test]
fn oracle_cmdline_file_matches_recorded_list() {
    let path = format!(
        "{}/73a93e10e881efba110a09db7263fe1d/server/cmdline",
        ORACLE_ROOT
    );
    let Some(content) = read_file(&path, 0) else {
        eprintln!("skipping: {} not present", path);
        return;
    };
    let expected: Vec<String> = ORACLE_BROTLI_CMDLINE
        .iter()
        .map(|a| a.replace("<ROOT>", ORACLE_ROOT))
        .collect();
    assert_eq!(get_argument_string(&expected), content);
}

// Java: src/main/cpp/blaze.cc:363 EscapeForOptionSource
#[test]
fn escape_for_option_source_test() {
    assert_eq!("name_Cfoo_Ubar", escape_for_option_source("name:foo_bar"));
    assert_eq!("", escape_for_option_source(""));
    assert_eq!("home_Urc", escape_for_option_source("home_rc"));
    assert_eq!(
        "/home/x/.bazelrc",
        escape_for_option_source("/home/x/.bazelrc")
    );
}

#[test]
fn add_logging_args_order() {
    let mut logging_info = LoggingInfo::new("/usr/bin/bazel", 0);
    let mut args = vec!["build".to_string()];
    add_logging_args(
        &logging_info,
        DurationMillis { millis: 12 },
        None,
        Some(DurationMillis::default()),
        &mut args,
    );
    assert_eq!(
        vec![
            "build",
            "--startup_time=12",
            "--command_wait_time=0",
            "--binary_path=/usr/bin/bazel"
        ],
        args
    );

    logging_info.set_restart_reason_if_not_set(RestartReason::NoDaemon);
    logging_info.set_restart_reason_if_not_set(RestartReason::NewOptions);
    let mut args = Vec::new();
    add_logging_args(
        &logging_info,
        DurationMillis { millis: 5 },
        Some(DurationMillis { millis: 3 }),
        Some(DurationMillis { millis: 1 }),
        &mut args,
    );
    assert_eq!(
        vec![
            "--startup_time=5",
            "--command_wait_time=1",
            "--extract_data_time=3",
            "--restart_reason=no_daemon",
            "--binary_path=/usr/bin/bazel"
        ],
        args
    );
}

/// BlazeServer::Communicate adds an extra, empty StartupOption after each option (it calls
/// add_startup_options() twice per option).
#[test]
fn run_request_startup_options_are_followed_by_empty_entries() {
    let request = build_run_request(
        &["version".to_string(), "--startup_time=1".to_string()],
        "",
        &[
            RcStartupFlag {
                source: "/etc/bazel.bazelrc".into(),
                value: "--max_idle_secs=5".into(),
            },
            cmdline_flag("--nohome_rc"),
        ],
        true,
        false,
        false,
    );
    assert_eq!(
        vec![b"version".to_vec(), b"--startup_time=1".to_vec()],
        request.arg
    );
    assert_eq!(4, request.startup_options.len());
    assert_eq!(
        b"/etc/bazel.bazelrc".to_vec(),
        request.startup_options[0].source
    );
    assert_eq!(
        b"--max_idle_secs=5".to_vec(),
        request.startup_options[0].option
    );
    assert!(request.startup_options[1].source.is_empty());
    assert!(request.startup_options[1].option.is_empty());
    assert!(request.startup_options[2].source.is_empty());
    assert_eq!(b"--nohome_rc".to_vec(), request.startup_options[2].option);
    assert!(request.startup_options[3].option.is_empty());
    assert!(request.block_for_lock);
    assert!(request.cookie.is_empty());
    assert!(request.invocation_policy.is_empty());
    assert_eq!(
        format!("pid={}", std::process::id()),
        request.client_description
    );
}
