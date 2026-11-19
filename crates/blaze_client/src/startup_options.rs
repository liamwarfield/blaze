//! Port of `src/main/cpp/startup_options.{h,cc}` and `src/main/cpp/bazel_startup_options.{h,cc}`.
//!
//! C++ has an abstract `StartupOptions` with the site-specific subclass `BazelStartupOptions`
//! (and `FakeStartupOptions` in startup_options_test.cc). Here one struct holds every field and
//! [`StartupOptionsSubclass`] selects the overridden virtual methods.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use blaze_options::rc::option_processor::{ExitCode, RcStartupFlag, StartupArgsProcessor};

use crate::platform::{
    K_LIST_SEPARATOR, get_cache_dir, get_java_binary_under_javabase, get_oom_file_path,
    get_system_javabase, get_user_name, is_running_within_test,
};
use crate::util::errors::{errno, get_last_error_string, strerror};
use crate::util::exit_code;
use crate::util::file::{
    can_execute_file_path, can_read_file_path, path_exists_path, write_file_path,
};
use crate::util::logging::{LogLevel, bazel_die, bazel_log};
use crate::util::numbers::safe_strto32;
use crate::util::path::Path;

const STARTUP_OPTIONS_CC: &str = "src/main/cpp/startup_options.cc";
const BAZEL_STARTUP_OPTIONS_CC: &str = "src/main/cpp/bazel_startup_options.cc";

// Java: src/main/cpp/startup_options.h:212 StartupOptions::JavabaseType
/// The source of truth for the server javabase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavabaseType {
    Unknown,
    /// An explicit --server_javabase startup option.
    Explicit,
    /// The embedded JDK.
    Embedded,
    /// The default system JVM.
    System,
}

/// Which C++ subclass of `StartupOptions` this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupOptionsSubclass {
    // Java: src/main/cpp/bazel_startup_options.h:26 BazelStartupOptions
    Bazel {
        /// `user_bazelrc_`.
        user_bazelrc: String,
    },
    /// `FakeStartupOptions` of src/test/cpp/startup_options_test.cc: no extra options, no
    /// warnings, default output root `/output_root`.
    Fake,
}

/// The `bool*` a nullary flag writes (C++ keeps raw pointers to the fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NullaryFlag {
    Batch,
    BatchCpuScheduling,
    BlockForLock,
    Quiet,
    ClientDebug,
    Preemptible,
    FatalEventBusExceptions,
    HostJvmDebug,
    AutodetectServerJavabase,
    IdleServerTasks,
    ShutdownOnLowSysMem,
    IgnoreAllRcFiles,
    UnlimitCoredumps,
    WriteCommandLog,
    WindowsEnableSymlinks,
    RemoteRepoContentsCache,
    RunInUserCgroup,
    UseHomeRc,
    UseSystemRc,
    UseWorkspaceRc,
}

// Java: src/main/cpp/startup_options.h:59 StartupOptions
/// The startup options accepted by all versions of Bazel, and the parsed values.
pub struct StartupOptions {
    /// The capitalized name of this binary.
    pub product_name: String,
    /// If supplied, alternate location to write the blaze server's jvm's stdout.
    pub server_jvm_out: Path,
    /// If supplied, alternate location to write a serialized failure_detail proto.
    pub failure_detail_out: Path,
    /// A directory suitable for storing cached files (see UpdateConfiguration()).
    pub output_user_root: Path,
    /// The directory where build outputs are placed.
    pub output_base: Path,
    /// The directory where the decompressed Bazel server is installed.
    pub install_base: Path,
    /// Whether the install base should be locked before use.
    pub lock_install_base: bool,
    /// Override more finegrained rc file flags and ignore them all.
    pub ignore_all_rc_files: bool,
    /// Block for the Blaze server lock.
    pub block_for_lock: bool,
    pub host_jvm_debug: bool,
    pub autodetect_server_javabase: bool,
    pub host_jvm_args: Vec<String>,
    pub extra_classpath: String,
    pub batch: bool,
    pub batch_cpu_scheduling: bool,
    /// If negative, don't mess with ionice. Otherwise, a level from 0-7.
    pub io_nice_level: i32,
    pub max_idle_secs: i32,
    pub shutdown_on_low_sys_mem: bool,
    pub oom_more_eagerly: bool,
    pub oom_more_eagerly_threshold: i32,
    pub write_command_log: bool,
    /// Temporary flag for enabling EventBus exceptions to be fatal.
    pub fatal_event_bus_exceptions: bool,
    /// Where each option comes from: "" for the command line, an rc file path otherwise; absent
    /// for the default.
    pub option_sources: BTreeMap<String, String>,
    /// Port to start up the gRPC command server on. If 0, let the kernel choose.
    pub command_port: i32,
    /// Connection timeout for each gRPC connection attempt.
    pub connect_timeout_secs: i32,
    /// Local server startup timeout duration.
    pub local_startup_timeout_secs: i32,
    /// Invocation policy proto, or an empty string.
    pub invocation_policy: String,
    /// Invocation policy can only be specified once.
    pub have_invocation_policy_: bool,
    /// Whether to emit as little output as possible.
    pub quiet: bool,
    /// Whether to output addition debugging information in the client.
    pub client_debug: bool,
    /// Whether the resulting command will be preempted if a subsequent command is run.
    pub preemptible: bool,
    /// Value of the java.util.logging.FileHandler.formatter Java property.
    pub java_logging_formatter: String,
    /// The hash function to use when computing file digests.
    pub digest_function: String,
    pub unix_digest_hash_attribute_name: String,
    pub idle_server_tasks: bool,
    /// The startup options as received from the user and rc files, tagged with their origin.
    pub original_startup_options_: Vec<RcStartupFlag>,
    // PORT: skipped, macOS only: macos_qos_class.
    /// Whether to raise the soft coredump limit to the hard one or not.
    pub unlimit_coredumps: bool,
    pub cgroup_parent: String,
    /// If enabled, the Bazel server will be run with systemd-run.
    pub run_in_user_cgroup: bool,
    /// Whether to create symbolic links on Windows for files.
    pub windows_enable_symlinks: bool,
    /// Whether to use a remote cache to store the contents of reproducible external repos.
    pub remote_repo_contents_cache: bool,

    // BazelStartupOptions fields.
    use_system_rc: bool,
    use_workspace_rc: bool,
    use_home_rc: bool,
    subclass: StartupOptionsSubclass,

    // Private fields of StartupOptions.
    explicit_server_javabase_: Path,
    default_server_javabase_: OnceCell<(Path, JavabaseType)>,
    all_nullary_startup_flags_: HashMap<String, NullaryFlag>,
    no_rc_nullary_startup_flags_: HashSet<String>,
    // PORT: special_nullary_startup_flags_ omitted: RegisterSpecialNullaryStartupFlag has no
    // caller in Bazel.
    valid_unary_startup_flags_: HashSet<String>,
    option_sources_key_override_: HashMap<String, String>,
}

/// `GetUnaryOption(arg, next_arg, key)` with C's nullable `next_arg`: `Some((value,
/// value_is_next_arg))`.
fn get_unary_option<'a>(
    arg: &'a str,
    next_arg: Option<&'a str>,
    key: &str,
) -> Option<(&'a str, bool)> {
    let value = arg.strip_prefix(key)?;
    if let Some(v) = value.strip_prefix('=') {
        Some((v, false))
    } else if !value.is_empty() {
        None // trailing garbage in key name
    } else {
        next_arg.map(|n| (n, true))
    }
}

impl StartupOptions {
    // Java: src/main/cpp/startup_options.cc:39 StartupOptions::RegisterNullaryStartupFlag
    fn register_nullary_startup_flag(&mut self, flag_name: &str, value: NullaryFlag) {
        self.all_nullary_startup_flags_
            .insert(format!("--{}", flag_name), value);
        self.all_nullary_startup_flags_
            .insert(format!("--no{}", flag_name), value);
    }

    // Java: src/main/cpp/startup_options.cc:45 StartupOptions::RegisterNullaryStartupFlagNoRc
    fn register_nullary_startup_flag_no_rc(&mut self, flag_name: &str, value: NullaryFlag) {
        self.register_nullary_startup_flag(flag_name, value);
        self.no_rc_nullary_startup_flags_
            .insert(format!("--{}", flag_name));
        self.no_rc_nullary_startup_flags_
            .insert(format!("--no{}", flag_name));
    }

    // Java: src/main/cpp/startup_options.cc:59 StartupOptions::RegisterUnaryStartupFlag
    fn register_unary_startup_flag(&mut self, flag_name: &str) {
        self.valid_unary_startup_flags_
            .insert(format!("--{}", flag_name));
    }

    // Java: src/main/cpp/startup_options.cc:63 StartupOptions::OverrideOptionSourcesKey
    #[allow(dead_code)]
    fn override_option_sources_key(&mut self, flag_name: &str, new_name: &str) {
        self.option_sources_key_override_
            .insert(flag_name.to_string(), new_name.to_string());
    }

    fn nullary_flag_mut(&mut self, flag: NullaryFlag) -> &mut bool {
        match flag {
            NullaryFlag::Batch => &mut self.batch,
            NullaryFlag::BatchCpuScheduling => &mut self.batch_cpu_scheduling,
            NullaryFlag::BlockForLock => &mut self.block_for_lock,
            NullaryFlag::Quiet => &mut self.quiet,
            NullaryFlag::ClientDebug => &mut self.client_debug,
            NullaryFlag::Preemptible => &mut self.preemptible,
            NullaryFlag::FatalEventBusExceptions => &mut self.fatal_event_bus_exceptions,
            NullaryFlag::HostJvmDebug => &mut self.host_jvm_debug,
            NullaryFlag::AutodetectServerJavabase => &mut self.autodetect_server_javabase,
            NullaryFlag::IdleServerTasks => &mut self.idle_server_tasks,
            NullaryFlag::ShutdownOnLowSysMem => &mut self.shutdown_on_low_sys_mem,
            NullaryFlag::IgnoreAllRcFiles => &mut self.ignore_all_rc_files,
            NullaryFlag::UnlimitCoredumps => &mut self.unlimit_coredumps,
            NullaryFlag::WriteCommandLog => &mut self.write_command_log,
            NullaryFlag::WindowsEnableSymlinks => &mut self.windows_enable_symlinks,
            NullaryFlag::RemoteRepoContentsCache => &mut self.remote_repo_contents_cache,
            NullaryFlag::RunInUserCgroup => &mut self.run_in_user_cgroup,
            NullaryFlag::UseHomeRc => &mut self.use_home_rc,
            NullaryFlag::UseSystemRc => &mut self.use_system_rc,
            NullaryFlag::UseWorkspaceRc => &mut self.use_workspace_rc,
        }
    }

    // Java: src/main/cpp/startup_options.cc:68 StartupOptions::StartupOptions
    /// The base-class constructor (`subclass` decides the virtual methods).
    pub fn new(
        product_name: &str,
        lock_install_base: bool,
        subclass: StartupOptionsSubclass,
    ) -> Self {
        let mut o = StartupOptions {
            product_name: product_name.to_string(),
            server_jvm_out: Path::default(),
            failure_detail_out: Path::default(),
            output_user_root: Path::default(),
            output_base: Path::default(),
            install_base: Path::default(),
            lock_install_base,
            ignore_all_rc_files: false,
            block_for_lock: true,
            host_jvm_debug: false,
            autodetect_server_javabase: true,
            host_jvm_args: Vec::new(),
            extra_classpath: String::new(),
            batch: false,
            batch_cpu_scheduling: false,
            io_nice_level: -1,
            // Use a smaller default idle timer when in a test.
            max_idle_secs: if is_running_within_test() {
                15
            } else {
                3 * 3600
            },
            shutdown_on_low_sys_mem: false,
            oom_more_eagerly: false,
            oom_more_eagerly_threshold: 100,
            write_command_log: true,
            fatal_event_bus_exceptions: false,
            option_sources: BTreeMap::new(),
            command_port: 0,
            connect_timeout_secs: 30,
            local_startup_timeout_secs: 120,
            invocation_policy: String::new(),
            have_invocation_policy_: false,
            quiet: false,
            client_debug: false,
            preemptible: false,
            java_logging_formatter: "com.google.devtools.build.lib.util.SingleLineFormatter"
                .to_string(),
            digest_function: String::new(),
            unix_digest_hash_attribute_name: String::new(),
            idle_server_tasks: true,
            original_startup_options_: Vec::new(),
            unlimit_coredumps: false,
            cgroup_parent: String::new(),
            run_in_user_cgroup: false,
            windows_enable_symlinks: false,
            remote_repo_contents_cache: false,
            use_system_rc: true,
            use_workspace_rc: true,
            use_home_rc: true,
            subclass,
            explicit_server_javabase_: Path::default(),
            default_server_javabase_: OnceCell::new(),
            all_nullary_startup_flags_: HashMap::new(),
            no_rc_nullary_startup_flags_: HashSet::new(),
            valid_unary_startup_flags_: HashSet::new(),
            option_sources_key_override_: HashMap::new(),
        };
        // PORT: skipped, Windows only: DetectBashAndExportBazelSh.

        // IMPORTANT: Before modifying the statements below please contact a Bazel core team
        // member that knows the internal procedure for adding/deprecating startup flags.
        o.register_nullary_startup_flag("batch", NullaryFlag::Batch);
        o.register_nullary_startup_flag("batch_cpu_scheduling", NullaryFlag::BatchCpuScheduling);
        o.register_nullary_startup_flag("block_for_lock", NullaryFlag::BlockForLock);
        o.register_nullary_startup_flag("quiet", NullaryFlag::Quiet);
        o.register_nullary_startup_flag("client_debug", NullaryFlag::ClientDebug);
        o.register_nullary_startup_flag("preemptible", NullaryFlag::Preemptible);
        o.register_nullary_startup_flag(
            "fatal_event_bus_exceptions",
            NullaryFlag::FatalEventBusExceptions,
        );
        o.register_nullary_startup_flag("host_jvm_debug", NullaryFlag::HostJvmDebug);
        o.register_nullary_startup_flag(
            "autodetect_server_javabase",
            NullaryFlag::AutodetectServerJavabase,
        );
        o.register_nullary_startup_flag("idle_server_tasks", NullaryFlag::IdleServerTasks);
        o.register_nullary_startup_flag(
            "shutdown_on_low_sys_mem",
            NullaryFlag::ShutdownOnLowSysMem,
        );
        o.register_nullary_startup_flag_no_rc("ignore_all_rc_files", NullaryFlag::IgnoreAllRcFiles);
        o.register_nullary_startup_flag("unlimit_coredumps", NullaryFlag::UnlimitCoredumps);
        o.register_nullary_startup_flag("write_command_log", NullaryFlag::WriteCommandLog);
        o.register_nullary_startup_flag(
            "windows_enable_symlinks",
            NullaryFlag::WindowsEnableSymlinks,
        );
        o.register_nullary_startup_flag(
            "experimental_remote_repo_contents_cache",
            NullaryFlag::RemoteRepoContentsCache,
        );
        o.register_nullary_startup_flag(
            "experimental_run_in_user_cgroup",
            NullaryFlag::RunInUserCgroup,
        );
        o.register_unary_startup_flag("command_port");
        o.register_unary_startup_flag("connect_timeout_secs");
        o.register_unary_startup_flag("local_startup_timeout_secs");
        o.register_unary_startup_flag("digest_function");
        o.register_unary_startup_flag("unix_digest_hash_attribute_name");
        o.register_unary_startup_flag("server_javabase");
        o.register_unary_startup_flag("host_jvm_args");
        o.register_unary_startup_flag("host_jvm_profile");
        o.register_unary_startup_flag("invocation_policy");
        o.register_unary_startup_flag("io_nice_level");
        o.register_unary_startup_flag("install_base");
        o.register_unary_startup_flag("macos_qos_class");
        o.register_unary_startup_flag("max_idle_secs");
        o.register_unary_startup_flag("output_base");
        o.register_unary_startup_flag("output_user_root");
        o.register_unary_startup_flag("server_jvm_out");
        o.register_unary_startup_flag("failure_detail_out");
        o.register_unary_startup_flag("experimental_cgroup_parent");
        o.register_unary_startup_flag("extra_classpath");
        o
    }

    // Java: src/main/cpp/bazel_startup_options.cc:30 BazelStartupOptions::BazelStartupOptions
    pub fn bazel() -> Self {
        let mut o = StartupOptions::new(
            "Bazel",
            /* lock_install_base= */ true,
            StartupOptionsSubclass::Bazel {
                user_bazelrc: String::new(),
            },
        );
        o.register_nullary_startup_flag_no_rc("home_rc", NullaryFlag::UseHomeRc);
        o.register_nullary_startup_flag_no_rc("system_rc", NullaryFlag::UseSystemRc);
        o.register_nullary_startup_flag_no_rc("workspace_rc", NullaryFlag::UseWorkspaceRc);
        o.register_unary_startup_flag("bazelrc");
        o
    }

    /// `FakeStartupOptions` of src/test/cpp/startup_options_test.cc.
    pub fn fake_for_testing() -> Self {
        StartupOptions::new(
            "Bazel",
            /* lock_install_base= */ true,
            StartupOptionsSubclass::Fake,
        )
    }

    pub fn subclass(&self) -> &StartupOptionsSubclass {
        &self.subclass
    }

    // Java: src/main/cpp/startup_options.cc:168 StartupOptions::GetLowercaseProductName
    pub fn get_lowercase_product_name(&self) -> String {
        self.product_name.to_ascii_lowercase()
    }

    // Java: src/main/cpp/startup_options.cc:172 StartupOptions::IsUnary
    pub fn is_unary(&self, arg: &str) -> bool {
        match arg.find('=') {
            None => self.valid_unary_startup_flags_.contains(arg),
            Some(i) => self.valid_unary_startup_flags_.contains(&arg[..i]),
        }
    }

    // Java: src/main/cpp/startup_options.cc:183 StartupOptions::MaybeCheckValidNullary
    /// `Ok(result)` (true iff `arg` is definitely a valid nullary option), or `Err(error)` when
    /// `arg` passes a value to a nullary option.
    pub fn maybe_check_valid_nullary(&self, arg: &str) -> Result<bool, String> {
        let Some(i) = arg.find('=') else {
            return Ok(self.all_nullary_startup_flags_.contains_key(arg));
        };
        let f = &arg[..i];
        if !self.all_nullary_startup_flags_.contains_key(f) {
            return Ok(false);
        }
        Err(format!(
            "In argument '{}': option '{}' does not take a value.",
            arg, f
        ))
    }

    // Java: src/main/cpp/startup_options.cc:203 StartupOptions::AddExtraOptions
    // Java: src/main/cpp/bazel_startup_options.cc:106 BazelStartupOptions::AddExtraOptions
    pub fn add_extra_options(&self, _result: &mut Vec<String>) {}

    // Java: src/main/cpp/startup_options.cc:205 StartupOptions::ProcessArg
    /// Parses a single argument. `rcfile` is "" for the command line. Returns
    /// `(is_space_separated, result)`.
    fn process_arg(
        &mut self,
        argstr: &str,
        next_argstr: &str,
        rcfile: &str,
    ) -> (bool, Result<(), String>) {
        // We have to parse a specific option syntax, so GNU getopts won't do. All options begin
        // with "--" or "-". Values are given together with the option delimited by '=' or in the
        // next option.
        let arg = argstr;
        let next_arg: Option<&str> = if next_argstr.is_empty() {
            None
        } else {
            Some(next_argstr)
        };

        let is_nullary = match self.maybe_check_valid_nullary(argstr) {
            Ok(r) => r,
            Err(error) => return (false, Err(error)),
        };

        if is_nullary {
            // 'enabled' is true if 'argstr' is "--foo", and false if it's "--nofoo".
            let enabled = !argstr.starts_with("--no");
            if self.no_rc_nullary_startup_flags_.contains(argstr) {
                // no_rc_nullary_startup_flags_ are forbidden in .bazelrc files.
                if !rcfile.is_empty() {
                    return (
                        false,
                        Err(format!(
                            "Can't specify {} in the {} file.",
                            argstr,
                            self.get_rc_file_base_name()
                        )),
                    );
                }
            }
            // 'argstr' is either "--foo" or "--nofoo", and the map entry is the field storing the
            // flag's value.
            let flag = self.all_nullary_startup_flags_[argstr];
            *self.nullary_flag_mut(flag) = enabled;
            // Use the key "foo" for 'argstr' of "--foo" / "--nofoo", unless there's an overridden
            // name we must use.
            let mut key = argstr[if enabled { 2 } else { 4 }..].to_string();
            if let Some(k) = self.option_sources_key_override_.get(&key) {
                key = k.clone();
            }
            self.option_sources.insert(key, rcfile.to_string());
            return (false, Ok(()));
        }

        let value: Option<(&str, bool)>;
        if let Some(v) = get_unary_option(arg, next_arg, "--output_base") {
            value = Some(v);
            self.output_base = Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("output_base".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--install_base") {
            value = Some(v);
            self.install_base = Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("install_base".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--output_user_root") {
            value = Some(v);
            self.output_user_root = Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("output_user_root".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--server_jvm_out") {
            value = Some(v);
            self.server_jvm_out = Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("server_jvm_out".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--failure_detail_out") {
            value = Some(v);
            self.failure_detail_out = Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("failure_detail_out".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--server_javabase") {
            value = Some(v);
            // TODO(bazel-team): Consider examining the javabase and re-execing in case of
            // architecture mismatch.
            self.explicit_server_javabase_ =
                Path::new(&crate::platform::absolute_path_from_flag(v.0));
            self.option_sources
                .insert("server_javabase".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--host_jvm_args") {
            value = Some(v);
            self.host_jvm_args.push(v.0.to_string());
            self.option_sources
                .insert("host_jvm_args".into(), rcfile.into()); // NB: This is incorrect
        } else if let Some(v) = get_unary_option(arg, next_arg, "--extra_classpath") {
            value = Some(v);
            self.extra_classpath = v.0.to_string();
            self.option_sources
                .insert("extra_classpath".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--io_nice_level") {
            value = Some(v);
            let (ok, parsed) = safe_strto32(v.0);
            self.io_nice_level = parsed;
            if !ok || self.io_nice_level > 7 {
                return (
                    false,
                    Err(format!(
                        "Invalid argument to --io_nice_level: '{}'. Must not exceed 7.",
                        v.0
                    )),
                );
            }
            self.option_sources
                .insert("io_nice_level".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--max_idle_secs") {
            value = Some(v);
            let (ok, parsed) = safe_strto32(v.0);
            self.max_idle_secs = parsed;
            if !ok || self.max_idle_secs < 0 {
                return (
                    false,
                    Err(format!("Invalid argument to --max_idle_secs: '{}'.", v.0)),
                );
            }
            self.option_sources
                .insert("max_idle_secs".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--macos_qos_class") {
            value = Some(v);
            // We parse the value of this flag on all platforms even if it is macOS-specific to
            // ensure that rc files mentioning it are valid.
            if v.0 == "utility" {
                // PORT: skipped, macOS only: macos_qos_class = QOS_CLASS_UTILITY.
            } else if v.0 == "background" {
                // PORT: skipped, macOS only: macos_qos_class = QOS_CLASS_BACKGROUND.
            } else {
                return (
                    false,
                    Err(format!("Invalid argument to --macos_qos_class: '{}'.", v.0)),
                );
            }
            self.option_sources
                .insert("macos_qos_class".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--connect_timeout_secs") {
            value = Some(v);
            let (ok, parsed) = safe_strto32(v.0);
            self.connect_timeout_secs = parsed;
            if !ok || self.connect_timeout_secs < 1 || self.connect_timeout_secs > 3600 {
                return (
                    false,
                    Err(format!(
                        "Invalid argument to --connect_timeout_secs: '{}'.\n\
                         Must be an integer between 1 and 3600.\n",
                        v.0
                    )),
                );
            }
            self.option_sources
                .insert("connect_timeout_secs".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--local_startup_timeout_secs") {
            value = Some(v);
            let (ok, parsed) = safe_strto32(v.0);
            self.local_startup_timeout_secs = parsed;
            if !ok || self.local_startup_timeout_secs < 1 {
                return (
                    false,
                    Err(format!(
                        "Invalid argument to --local_startup_timeout_secs: '{}'.\n\
                         Must be a positive integer.\n",
                        v.0
                    )),
                );
            }
            self.option_sources
                .insert("local_startup_timeout_secs".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--digest_function") {
            value = Some(v);
            self.digest_function = v.0.to_string();
            self.option_sources
                .insert("digest_function".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--unix_digest_hash_attribute_name")
        {
            value = Some(v);
            self.unix_digest_hash_attribute_name = v.0.to_string();
            self.option_sources
                .insert("unix_digest_hash_attribute_name".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--command_port") {
            value = Some(v);
            let (ok, parsed) = safe_strto32(v.0);
            self.command_port = parsed;
            if !ok || self.command_port < 0 || self.command_port > 65535 {
                return (
                    false,
                    Err(format!(
                        "Invalid argument to --command_port: '{}'.\n\
                         Must be a valid port number or 0.\n",
                        v.0
                    )),
                );
            }
            self.option_sources
                .insert("command_port".into(), rcfile.into());
        } else if let Some(v) = get_unary_option(arg, next_arg, "--invocation_policy") {
            value = Some(v);
            if !self.have_invocation_policy_ {
                self.have_invocation_policy_ = true;
                self.invocation_policy = v.0.to_string();
                self.option_sources
                    .insert("invocation_policy".into(), rcfile.into());
            } else {
                return (
                    false,
                    Err(
                        "The startup flag --invocation_policy cannot be specified multiple times."
                            .to_string(),
                    ),
                );
            }
        } else if let Some(v) = get_unary_option(arg, next_arg, "--experimental_cgroup_parent") {
            value = Some(v);
            self.cgroup_parent = v.0.to_string();
            self.option_sources
                .insert("cgroup_parent".into(), rcfile.into());
        } else {
            let (extra_value, extra_argument_processed, result) =
                self.process_arg_extra(arg, next_arg, rcfile);
            value = extra_value;
            if let Err(error) = result {
                return (false, Err(error));
            }
            if !extra_argument_processed {
                return (
                    false,
                    Err(format!(
                        "Unknown startup option: '{}'.\n  For more info, run '{} help \
                         startup_options'.",
                        arg,
                        self.get_lowercase_product_name()
                    )),
                );
            }
        }

        // is_space_separated = (value == next_arg) && (value != nullptr)
        let is_space_separated = matches!(value, Some((_, true)));
        (is_space_separated, Ok(()))
    }

    // Java: src/main/cpp/bazel_startup_options.cc:46 BazelStartupOptions::ProcessArgExtra
    /// Returns `(value, is_processed, result)`.
    fn process_arg_extra<'a>(
        &mut self,
        arg: &'a str,
        next_arg: Option<&'a str>,
        rcfile: &str,
    ) -> (Option<(&'a str, bool)>, bool, Result<(), String>) {
        match &mut self.subclass {
            StartupOptionsSubclass::Bazel { user_bazelrc } => {
                let value = get_unary_option(arg, next_arg, "--bazelrc");
                if let Some(v) = value {
                    if !rcfile.is_empty() {
                        return (
                            value,
                            false,
                            Err("Can't specify --bazelrc in the RC file.".to_string()),
                        );
                    }
                    *user_bazelrc = v.0.to_string();
                } else {
                    return (value, false, Ok(()));
                }
                (value, true, Ok(()))
            }
            // FakeStartupOptions::ProcessArgExtra: *is_processed = false.
            StartupOptionsSubclass::Fake => (None, false, Ok(())),
        }
    }

    // Java: src/main/cpp/startup_options.cc:411 StartupOptions::ProcessArgs
    /// Processes an ordered list of RcStartupFlags using ProcessArg.
    pub fn process_args_impl(&mut self, rcstartup_flags: &[RcStartupFlag]) -> Result<(), String> {
        let mut i = 0;
        while i < rcstartup_flags.len() {
            let next_value = if i == rcstartup_flags.len() - 1 {
                String::new()
            } else {
                rcstartup_flags[i + 1].value.clone()
            };
            let (is_space_separated, result) = self.process_arg(
                &rcstartup_flags[i].value,
                &next_value,
                &rcstartup_flags[i].source,
            );
            // Store the provided option in --flag(=value)? form. Store these before propagating
            // any error code, since we want to have the correct information for the output. The
            // fact that the options aren't parseable doesn't matter for this step.
            if is_space_separated {
                let combined_value = format!("{}={}", rcstartup_flags[i].value, next_value);
                self.original_startup_options_.push(RcStartupFlag {
                    source: rcstartup_flags[i].source.clone(),
                    value: combined_value,
                });
                i += 2;
            } else {
                self.original_startup_options_.push(RcStartupFlag {
                    source: rcstartup_flags[i].source.clone(),
                    value: rcstartup_flags[i].value.clone(),
                });
                i += 1;
            }
            result?;
        }
        Ok(())
    }

    // Java: src/main/cpp/startup_options.cc:444 StartupOptions::GetSystemJavabase
    fn get_system_javabase(&self) -> Path {
        Path::new(&get_system_javabase())
    }

    // Java: src/main/cpp/startup_options.cc:448 StartupOptions::GetEmbeddedJavabase
    /// Returns the embedded JDK, or an empty path.
    pub fn get_embedded_javabase(&self) -> Path {
        let bundled_jre_path = self
            .install_base
            .get_relative("embedded_tools")
            .get_relative("jdk");
        if can_execute_file_path(&bundled_jre_path.get_relative(&get_java_binary_under_javabase()))
        {
            return bundled_jre_path;
        }
        Path::default()
    }

    // Java: src/main/cpp/startup_options.cc:459 StartupOptions::GetServerJavabaseAndType
    pub fn get_server_javabase_and_type(&self) -> (Path, JavabaseType) {
        // 1) Allow overriding the server_javabase via --server_javabase.
        if !self.explicit_server_javabase_.is_empty() {
            return (
                self.explicit_server_javabase_.clone(),
                JavabaseType::Explicit,
            );
        }
        if let Some(v) = self.default_server_javabase_.get() {
            return v.clone();
        }
        let bundled_jre_path = self.get_embedded_javabase();
        let v = if !bundled_jre_path.is_empty() {
            // 2) Use a bundled JVM if we have one.
            (bundled_jre_path, JavabaseType::Embedded)
        } else if !self.autodetect_server_javabase {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                STARTUP_OPTIONS_CC,
                472,
                "Could not find embedded or explicit server javabase, and \
                 --noautodetect_server_javabase is set.",
            );
        } else {
            // 3) Otherwise fall back to using the default system JVM.
            let system_javabase = self.get_system_javabase();
            if system_javabase.is_empty() {
                bazel_die(
                    exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                    STARTUP_OPTIONS_CC,
                    479,
                    "Could not find system javabase. Ensure JAVA_HOME is set, or javac is on \
                     your PATH.",
                );
            }
            (system_javabase, JavabaseType::System)
        };
        let _ = self.default_server_javabase_.set(v.clone());
        v
    }

    // Java: src/main/cpp/startup_options.cc:490 StartupOptions::GetServerJavabase
    pub fn get_server_javabase(&self) -> Path {
        self.get_server_javabase_and_type().0
    }

    // Java: src/main/cpp/startup_options.cc:494 StartupOptions::GetExplicitServerJavabase
    pub fn get_explicit_server_javabase(&self) -> Path {
        self.explicit_server_javabase_.clone()
    }

    // Java: src/main/cpp/startup_options.cc:498 StartupOptions::GetJvm
    /// Returns the path to the JVM. In-process the JVM is never started; the path only goes into
    /// the server command line.
    pub fn get_jvm(&self) -> Path {
        let (javabase, javabase_type) = self.get_server_javabase_and_type();
        let sanity_check_code = self.sanity_check_javabase(&javabase, javabase_type);
        if sanity_check_code != exit_code::SUCCESS {
            crate::util::logging::exit_process(sanity_check_code);
        }
        javabase.get_relative(&get_java_binary_under_javabase())
    }

    // Java: src/main/cpp/startup_options.cc:510 BadServerJavabaseError
    /// Prints an appropriate error message and returns an appropriate error exit code for a
    /// server javabase which failed sanity checks.
    fn bad_server_javabase_error(&self, javabase_type: JavabaseType) -> i32 {
        match javabase_type {
            JavabaseType::Explicit => {
                let rc_file = match self.option_sources.get("server_javabase") {
                    Some(s) if !s.is_empty() => s.clone(),
                    _ => String::new(),
                };
                bazel_log(
                    LogLevel::Error,
                    STARTUP_OPTIONS_CC,
                    520,
                    format!(
                        "  The java path was specified by a '--server_javabase' option {}",
                        if rc_file.is_empty() {
                            "on the command line".to_string()
                        } else {
                            format!("in {}", rc_file)
                        }
                    ),
                );
                exit_code::BAD_ARGV
            }
            JavabaseType::Embedded => {
                bazel_log(
                    LogLevel::Error,
                    STARTUP_OPTIONS_CC,
                    526,
                    "  Internal error: embedded JDK fails sanity check.",
                );
                exit_code::INTERNAL_ERROR
            }
            JavabaseType::System => exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            JavabaseType::Unknown => {
                bazel_log(
                    LogLevel::Error,
                    STARTUP_OPTIONS_CC,
                    531,
                    "  Internal error: server javabase type was not initialized.",
                );
                exit_code::INTERNAL_ERROR
            }
        }
    }

    // Java: src/main/cpp/startup_options.cc:538 StartupOptions::SanityCheckJavabase
    fn sanity_check_javabase(&self, javabase: &Path, javabase_type: JavabaseType) -> i32 {
        let java_program = javabase.get_relative(&get_java_binary_under_javabase());
        if !can_execute_file_path(&java_program) {
            if !path_exists_path(&java_program) {
                bazel_log(
                    LogLevel::Error,
                    STARTUP_OPTIONS_CC,
                    545,
                    format!(
                        "Couldn't find java at '{}'.",
                        java_program.as_printable_path()
                    ),
                );
            } else {
                bazel_log(
                    LogLevel::Error,
                    STARTUP_OPTIONS_CC,
                    548,
                    format!(
                        "Java at '{}' exists but is not executable: {}",
                        java_program.as_printable_path(),
                        get_last_error_string()
                    ),
                );
            }
            return self.bad_server_javabase_error(javabase_type);
        }
        if can_read_file_path(&javabase.get_relative("jre/lib/rt.jar"))
            || can_read_file_path(&javabase.get_relative("lib/rt.jar"))
            || can_read_file_path(&javabase.get_relative("bin/java"))
            || can_read_file_path(&javabase.get_relative("bin/java.exe"))
        {
            return exit_code::SUCCESS;
        }
        bazel_log(
            LogLevel::Error,
            STARTUP_OPTIONS_CC,
            563,
            format!(
                "Problem with java installation: couldn't find/access rt.jar or java in {}",
                javabase.as_printable_path()
            ),
        );
        self.bad_server_javabase_error(javabase_type)
    }

    // Java: src/main/cpp/startup_options.cc:569 StartupOptions::GetExe
    pub fn get_exe(&self, jvm: &Path, _jar_path: &str) -> Path {
        jvm.clone()
    }

    // Java: src/main/cpp/startup_options.cc:574 StartupOptions::AddJVMArgumentPrefix
    pub fn add_jvm_argument_prefix(&self, _javabase: &Path, _result: &mut Vec<String>) {}

    // Java: src/main/cpp/startup_options.cc:577 StartupOptions::AddJVMArgumentSuffix
    pub fn add_jvm_argument_suffix(
        &self,
        real_install_dir: &Path,
        jar_path: &str,
        result: &mut Vec<String>,
    ) {
        if self.extra_classpath.is_empty() {
            result.push("-jar".to_string());
            result.push(real_install_dir.get_relative(jar_path).as_jvm_argument());
        } else {
            // Since we're not executing the server jar directly, we must manually set module
            // opening directives on the command line (getJvmModuleOptions(), generated from the
            // server jar's manifest).
            ::blaze_util::unported!(
                "StartupOptions::AddJVMArgumentSuffix",
                "--extra_classpath needs getJvmModuleOptions() (jvm_module_options.h); classpath \
                 would be {}{}{}",
                real_install_dir.get_relative(jar_path).as_jvm_argument(),
                K_LIST_SEPARATOR,
                self.extra_classpath
            );
        }
    }

    // Java: src/main/cpp/startup_options.cc:597 StartupOptions::AddJVMArguments
    /// Adds JVM tuning flags for Blaze. `Err((exit_code, error))` on failure.
    pub fn add_jvm_arguments(
        &self,
        server_javabase: &Path,
        result: &mut Vec<String>,
        user_options: &[String],
    ) -> Result<(), (i32, String)> {
        self.add_jvm_logging_arguments(result);

        // Disable the JVM's own unlimiting of file descriptors.  We do this ourselves in
        // blaze.cc so we want our setting to propagate to the JVM.
        result.push("-XX:-MaxFDLimit".to_string());

        result.push("-Djava.lang.Thread.allowVirtualThreads=true".to_string());

        result.push(format!(
            "-XX:OnOutOfMemoryError=touch {}",
            get_oom_file_path(&self.output_base).as_jvm_argument()
        ));

        self.add_jvm_memory_arguments(server_javabase, result, user_options)
    }

    // Java: src/main/cpp/startup_options.cc:637 StartupOptions::AddJVMLoggingArguments
    fn add_jvm_logging_arguments(&self, result: &mut Vec<String>) {
        // Configure logging
        let prop_file = self.output_base.get_relative("javalog.properties");
        let java_log = self.output_base.get_relative("java.log");
        let logging_props = get_simple_log_handler_props(&java_log, &self.java_logging_formatter);

        if !write_file_path(
            &blaze_util::string_encoding::latin1_encode(&logging_props),
            &prop_file,
            0o644,
        ) {
            // perror(...)
            let err = strerror(errno());
            let mut line = crate::util::strings::console_bytes(&format!(
                "Couldn't write logging file {}: {}",
                prop_file.as_printable_path(),
                err
            ))
            .into_owned();
            line.push(b'\n');
            let _ = std::io::Write::write_all(&mut std::io::stderr(), &line);
        } else {
            result.push(format!(
                "-Djava.util.logging.config.file={}",
                prop_file.as_jvm_argument()
            ));
            result.push(
                "-Dcom.google.devtools.build.lib.util.LogHandlerQuerier.class=\
                 com.google.devtools.build.lib.util.SimpleLogHandler$HandlerQuerier"
                    .to_string(),
            );
        }
    }

    // Java: src/main/cpp/startup_options.cc:657 StartupOptions::AddJVMMemoryArguments
    fn add_jvm_memory_arguments(
        &self,
        _server_javabase: &Path,
        _result: &mut Vec<String>,
        _user_options: &[String],
    ) -> Result<(), (i32, String)> {
        Ok(())
    }

    // Java: src/main/cpp/bazel_startup_options.h:42 BazelStartupOptions::GetRcFileBaseName
    fn get_rc_file_base_name(&self) -> &'static str {
        ".bazelrc"
    }

    // Java: src/main/cpp/bazel_startup_options.cc:42 BazelStartupOptions::GetDefaultOutputRoot
    fn get_default_output_root(&self) -> Path {
        match self.subclass {
            StartupOptionsSubclass::Bazel { .. } => Path::new(&get_cache_dir()),
            StartupOptionsSubclass::Fake => Path::new("/output_root"),
        }
    }

    // Java: src/main/cpp/startup_options.cc:663 StartupOptions::UpdateConfiguration
    /// Updates the parsed startup options to fill in defaults.
    pub fn update_configuration(&mut self, install_md5: &str, workspace: &str, server_mode: bool) {
        if self.output_user_root.is_empty() {
            // The default production output_user_root is
            // <default_output_root>/_<product_name>_<username>.
            // In a test, use a subdirectory of TEST_TMPDIR to be hermetic.
            let output_root = if is_running_within_test() {
                Path::new(&crate::platform::get_path_env("TEST_TMPDIR"))
            } else {
                self.get_default_output_root()
            };
            self.output_user_root = output_root.get_relative(&format!(
                "_{}_{}",
                self.get_lowercase_product_name(),
                get_user_name()
            ));
        }

        if self.install_base.is_empty() {
            // The default install_base is <output_user_root>/install/<md5(blaze)>.
            // However, exec-server requires it to be explicitly set.
            if server_mode {
                bazel_die(
                    exit_code::BAD_ARGV,
                    STARTUP_OPTIONS_CC,
                    683,
                    "exec-server requires --install_base",
                );
            }
            self.install_base = self
                .output_user_root
                .get_relative("install")
                .get_relative(install_md5);
        }

        if self.output_base.is_empty() {
            // The default output_base is <output_user_root>/<md5(workspace)>.
            // However, exec-server requires it to be explicitly set.
            if server_mode {
                bazel_die(
                    exit_code::BAD_ARGV,
                    STARTUP_OPTIONS_CC,
                    694,
                    "exec-server requires --output_base",
                );
            }
            self.output_base =
                crate::platform::get_hashed_base_dir(&self.output_user_root, workspace);
        }

        if self.failure_detail_out.is_empty() {
            self.failure_detail_out = self.output_base.get_relative("failure_detail.rawproto");
        }
    }

    // Java: src/main/cpp/bazel_startup_options.cc:66 BazelStartupOptions::MaybeLogStartupOptionWarnings
    /// The warnings `MaybeLogStartupOptionWarnings` logs, as `(line, message)` pairs.
    pub fn startup_option_warnings(&self) -> Vec<(u32, String)> {
        let mut warnings = Vec::new();
        let StartupOptionsSubclass::Bazel { user_bazelrc } = &self.subclass else {
            // FakeStartupOptions::MaybeLogStartupOptionWarnings does nothing.
            return warnings;
        };
        if self.ignore_all_rc_files {
            if !user_bazelrc.is_empty() {
                warnings.push((
                    69,
                    "Value of --bazelrc is ignored, since --ignore_all_rc_files is on.".to_string(),
                ));
            }
            if self.use_home_rc && self.option_sources.contains_key("home_rc") {
                warnings.push((
                    74,
                    "Explicit value of --home_rc is ignored, since --ignore_all_rc_files is on."
                        .to_string(),
                ));
            }
            if self.use_system_rc && self.option_sources.contains_key("system_rc") {
                warnings.push((
                    79,
                    "Explicit value of --system_rc is ignored, since --ignore_all_rc_files is on."
                        .to_string(),
                ));
            }
            if self.use_workspace_rc && self.option_sources.contains_key("workspace_rc") {
                warnings.push((
                    84,
                    "Explicit value of --workspace_rc is ignored, since --ignore_all_rc_files \
                     is on."
                        .to_string(),
                ));
            }
        }
        if self.output_user_root.contains_char(' ') {
            warnings.push((
                89,
                format!(
                    "Output user root \"{}\" contains a space. This will probably break the \
                     build. You should set a different --output_user_root.",
                    self.output_user_root.as_printable_path()
                ),
            ));
        } else if self.output_base.contains_char(' ') {
            // output_base is computed from output_user_root by default. If output_user_root was
            // bad, don't check output_base: while output_base may also be bad, we already warned
            // about output_user_root so there's no point in another warning.
            warnings.push((
                98,
                format!(
                    "Output base \"{}\" contains a space. This will probably break the build. \
                     You should not set --output_base and let Bazel use the default, or set \
                     --output_base to a path without space.",
                    self.output_base.as_printable_path()
                ),
            ));
        }
        warnings
    }

    // Java: src/main/cpp/bazel_startup_options.cc:66 BazelStartupOptions::MaybeLogStartupOptionWarnings
    /// Once startup options have been parsed, warn the user if certain options might combine in
    /// surprising ways.
    pub fn maybe_log_startup_option_warnings(&self) {
        for (line, message) in self.startup_option_warnings() {
            bazel_log(LogLevel::Warning, BAZEL_STARTUP_OPTIONS_CC, line, message);
        }
    }

    // PORT: Blaze-only check (review 2026-11-18, S3 "TLS trust"); not in startup_options.cc.
    //
    // Bazel's HTTP downloads use the server JVM's trust store, by default the embedded JDK's
    // lib/security/cacerts. The documented way to add certificate authorities (or a client
    // certificate) is --host_jvm_args=-Djavax.net.ssl.trustStore=<file>, together with
    // trustStorePassword / trustStoreType and the keyStore counterparts. Blaze has no JVM:
    // --host_jvm_args only reach the server command line (GetServerExeArgs, written to
    // server/cmdline) and the server's --host_jvm_args startup option, and are otherwise
    // ignored. Ignoring these ones silently would make Blaze trust a different set of
    // authorities than the user configured, so they fail loudly until they are ported. Every
    // other --host_jvm_args value is still accepted and ignored, as before.
    //
    // -DBAZEL_TRACK_SOURCE_DIRECTORIES sets the JVM system property that
    // TrackSourceDirectoriesFlag reads (com/google/devtools/build/lib/skyframe/
    // TrackSourceDirectoriesFlag.java:29); Blaze fixes the flag at Bazel's default
    // (blaze_sky_exec TrackSourceDirectoriesFlag), so the option fails loudly too.
    /// The error for the first `--host_jvm_args` value that sets a `javax.net.ssl` trust store
    /// or key store property or the `BAZEL_TRACK_SOURCE_DIRECTORIES` property, or `None`. The
    /// message names the property, never its value (it may be a password).
    pub fn unported_host_jvm_arg_error(&self) -> Option<String> {
        const TLS_STORES: &str = "the JVM trust store and key store options \
             (-Djavax.net.ssl.trustStore*, -Djavax.net.ssl.keyStore*) are not ported";
        const TRACK_SOURCE_DIRECTORIES: &str = "the JVM system property \
             BAZEL_TRACK_SOURCE_DIRECTORIES (TrackSourceDirectoriesFlag) is not ported; Blaze \
             always uses Bazel's default";
        const UNPORTED_PREFIXES: [(&str, &str); 3] = [
            ("-Djavax.net.ssl.trustStore", TLS_STORES),
            ("-Djavax.net.ssl.keyStore", TLS_STORES),
            ("-DBAZEL_TRACK_SOURCE_DIRECTORIES", TRACK_SOURCE_DIRECTORIES),
        ];
        let (arg, what) = self.host_jvm_args.iter().find_map(|arg| {
            UNPORTED_PREFIXES
                .iter()
                .find(|(prefix, _)| arg.starts_with(*prefix))
                .map(|(_, what)| (arg, *what))
        })?;
        let property = arg.split('=').next().unwrap_or(arg.as_str());
        Some(format!(
            "blaze: not ported yet: --host_jvm_args={}: {}. Blaze has no JVM and would silently \
             ignore the option.",
            property, what
        ))
    }
}

impl StartupArgsProcessor for StartupOptions {
    // Java: src/main/cpp/startup_options.cc:411 StartupOptions::ProcessArgs
    fn process_args(
        &mut self,
        rcstartup_flags: &[RcStartupFlag],
    ) -> Result<(), (ExitCode, String)> {
        // Every error ProcessArg returns is BAD_ARGV.
        self.process_args_impl(rcstartup_flags)
            .map_err(|e| (ExitCode::BadArgv, e))
    }
}

// Java: src/main/cpp/startup_options.cc:621 GetSimpleLogHandlerProps
fn get_simple_log_handler_props(java_log: &Path, java_logging_formatter: &str) -> String {
    format!(
        "handlers=com.google.devtools.build.lib.util.SimpleLogHandler\n\
         .level=INFO\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.level=INFO\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.prefix={}\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.limit=1024000\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.total_limit=20971520\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.formatter={}\n",
        java_log.as_jvm_argument(),
        java_logging_formatter
    )
}
