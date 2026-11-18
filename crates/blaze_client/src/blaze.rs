//! Port of `src/main/cpp/blaze.cc` (and `main.cc`): the client's `Main` and everything it calls
//! on the Linux path.
//!
//! Blaze runs the server in-process ([`InProcessServer`], see `server.rs`): where the client
//! would exec the server JVM (`--batch`) or start a daemon and talk gRPC to it, it hands the
//! same arguments / `RunRequest` to the in-process server. There is never a running server to
//! connect to, so `BlazeServer::Connect` always fails.

use std::collections::BTreeMap;

use blaze_options::rc::option_processor::{
    BAZEL_SYSTEM_BAZELRC_PATH, OptionProcessor, RcStartupFlag, StartupFlagRegistry,
};
use blaze_protos::command_server::{RunRequest, StartupOption};

use crate::archive_utils::{
    EmbeddedArchive, LoggingInfo, RestartReason, determine_archive_contents, extract_build_label,
    extract_data, get_server_jar_path,
};
use crate::option_processor::{
    client_environment, parse_options_with_logging, print_startup_options_provenance_message,
};
use crate::platform::{
    DurationMillis, EnvVarAction, EnvVarValue, K_LIST_SEPARATOR, K_SERVER_PID_FILE, LockHandle,
    LockMode, acquire_lock, create_secure_directory, exclude_path_from_backup, exists_env,
    get_milliseconds_monotonic, get_oom_file_path, get_process_id_as_string, get_processed_env,
    get_self_path, get_system_javabase, is_running_within_test, is_shared_library, release_lock,
    set_debug_log, set_scheduling, setup_std_streams, symlink_directories, unlimit_coredumps,
    unlimit_resources, verify_server_process, warn_filesystem_type, warn_if_started_from_desktop,
};
use crate::server::{InProcessServer, ServerStartup};
use crate::startup_options::StartupOptions;
use crate::util::errors::get_last_error_string;
use crate::util::exit_code;
use crate::util::file::{
    can_access_directory_path, change_directory, get_cwd, is_directory_path, make_canonical,
    make_directories_path, path_exists_path, read_directory_symlink, read_file_path,
    set_mtime_to_now_if_possible, unlink_path_path, write_file_path,
};
use crate::util::logging::{
    self, BazelLogHandler, LogLevel, LoggingDetail, bazel_die, bazel_log, set_log_handler,
};
use crate::util::numbers::safe_strto32;
use crate::util::path::{
    Path, are_paths_equivalent, convert_path, dirname, is_absolute, join_path,
};
use crate::workspace_layout::WorkspaceLayout;

const BLAZE_CC: &str = "src/main/cpp/blaze.cc";
const BLAZE_UTIL_POSIX_CC: &str = "src/main/cpp/blaze_util_posix.cc";
/// The file name logged for fatal errors that only Blaze has (no line of blaze.cc to name).
const BLAZE_RS: &str = file!();

// Java: src/main/cpp/blaze.cc:175 ReasonString
/// String representation of RestartReason.
fn reason_string(reason: RestartReason) -> &'static str {
    match reason {
        RestartReason::NoRestart => "no_restart",
        RestartReason::NoDaemon => "no_daemon",
        RestartReason::NewVersion => "new_version",
        RestartReason::NewOptions => "new_options",
        RestartReason::PidFileButNoServer => "pid_file_but_no_server",
        RestartReason::ServerVanished => "server_vanished",
        RestartReason::ServerUnresponsive => "server_unresponsive",
    }
}

// Java: src/main/cpp/server_process_info.h:27 ServerProcessInfo
/// Information around the blaze server process and its configuration.
#[derive(Clone, Debug)]
pub struct ServerProcessInfo {
    /// When running as a daemon, where the deamonized server's stdout and stderr should be
    /// written.
    pub jvm_log_file_: Path,
    /// Whether or not the jvm_log_file should be opened with O_APPEND.
    pub jvm_log_file_append_: bool,
    pub server_pid_: i32,
}

impl ServerProcessInfo {
    // Java: src/main/cpp/server_process_info.cc:30 ServerProcessInfo::ServerProcessInfo
    pub fn new(output_base: &Path, server_jvm_out: &Path) -> Self {
        ServerProcessInfo {
            jvm_log_file_: get_jvm_out_file(output_base, server_jvm_out),
            jvm_log_file_append_: !server_jvm_out.is_empty(),
            // TODO(b/69972303): Formalize the "no server" magic value or rm it.
            server_pid_: -1,
        }
    }
}

// Java: src/main/cpp/server_process_info.cc:21 GetJvmOutFile
fn get_jvm_out_file(output_base: &Path, server_jvm_out: &Path) -> Path {
    if !server_jvm_out.is_empty() {
        server_jvm_out.clone()
    } else {
        output_base.get_relative("server/jvm.out")
    }
}

// Java: src/main/cpp/blaze.cc:199 BlazeServer
pub struct BlazeServer {
    install_base_lock_: Option<LockHandle>,
    output_base_lock_: Option<LockHandle>,
    /// `client_ != nullptr`: in-process, set once the in-process server "started".
    connected: bool,
    process_info_: ServerProcessInfo,
    #[allow(dead_code)]
    connect_timeout_secs_: i32,
    batch_: bool,
    block_for_lock_: bool,
    quiet_: bool,
    preemptible_: bool,
    lock_install_base_: bool,
    install_base_: Path,
    output_base_: Path,
}

impl BlazeServer {
    // Java: src/main/cpp/blaze.cc:1715 BlazeServer::BlazeServer
    pub fn new(startup_options: &StartupOptions) -> Self {
        // PORT: the cancel-thread pipe (CreatePipe) is not created: there is no gRPC
        // connection to cancel in-process.
        BlazeServer {
            install_base_lock_: None,
            output_base_lock_: None,
            connected: false,
            process_info_: ServerProcessInfo::new(
                &startup_options.output_base,
                &startup_options.server_jvm_out,
            ),
            connect_timeout_secs_: startup_options.connect_timeout_secs,
            batch_: startup_options.batch,
            block_for_lock_: startup_options.block_for_lock,
            quiet_: startup_options.quiet,
            preemptible_: startup_options.preemptible,
            lock_install_base_: startup_options.lock_install_base,
            install_base_: startup_options.install_base.clone(),
            output_base_: startup_options.output_base.clone(),
        }
    }

    // Java: src/main/cpp/blaze.cc:209 BlazeServer::Connected
    /// Whether there is an active connection to a server.
    pub fn connected(&self) -> bool {
        self.connected
    }

    // Java: src/main/cpp/blaze.cc:240 BlazeServer::ProcessInfo
    pub fn process_info(&self) -> &ServerProcessInfo {
        &self.process_info_
    }

    // Java: src/main/cpp/blaze.cc:296 BlazeServer::AcquireLocks
    /// Acquires locks for the install and output bases this server is running in. Returns the
    /// time spent waiting for locks.
    pub fn acquire_locks(&mut self) -> DurationMillis {
        let mut wait_time = DurationMillis::default();

        if self.lock_install_base_ {
            // Take a shared lock on the install base, because two simultaneous commands may run
            // against the same install base. The lock file is a sibling of the install base
            // directory to make it possible to synchronize concurrent creation through an
            // atomic rename. An exclusive lock is only required to run automatic garbage
            // collection of stale install bases. Always block waiting for the lock: in the event
            // that the install base is currently being garbage collected, we want to recreate it.
            if self.install_base_lock_.is_some() {
                bazel_die(
                    exit_code::INTERNAL_ERROR,
                    BLAZE_CC,
                    308,
                    "AcquireLocks() called but the install base lock is already held.",
                );
            }
            let install_base_parent = self.install_base_.get_parent();
            make_directories_path(&install_base_parent, 0o777);
            let (lock, wait) = acquire_lock(
                "install base",
                &install_base_parent
                    .get_relative(&format!("{}.lock", self.install_base_.get_base_name())),
                LockMode::Shared,
                self.batch_,
                /* block= */ true,
            );
            self.install_base_lock_ = Some(lock);
            wait_time.add_assign(&wait);
        }

        // Take an exclusive lock on the output base, because two simultaneous commands may not
        // run against the same output base.
        if self.output_base_lock_.is_some() {
            bazel_die(
                exit_code::INTERNAL_ERROR,
                BLAZE_CC,
                324,
                "AcquireLocks() called but the output base lock is already held.",
            );
        }
        let (lock, wait) = acquire_lock(
            "output base",
            &self.output_base_.get_relative("lock"),
            LockMode::Exclusive,
            self.batch_,
            self.block_for_lock_,
        );
        self.output_base_lock_ = Some(lock);
        wait_time.add_assign(&wait);

        wait_time
    }

    // Java: src/main/cpp/blaze.cc:336 BlazeServer::ReleaseLocks
    // PORT: never called while the server runs in-process: `communicate` keeps both locks for
    // the whole command (see the PORT comment there). Kept for the gRPC client (W8), which
    // releases them once a long-lived server has accepted the request.
    #[allow(dead_code)]
    fn release_locks(&mut self) {
        let Some(output_base_lock) = self.output_base_lock_.take() else {
            bazel_die(
                exit_code::INTERNAL_ERROR,
                BLAZE_CC,
                338,
                "ReleaseLocks() called but the output base lock has already been released.",
            );
        };
        release_lock(output_base_lock);

        if self.lock_install_base_ {
            let Some(install_base_lock) = self.install_base_lock_.take() else {
                bazel_die(
                    exit_code::INTERNAL_ERROR,
                    BLAZE_CC,
                    347,
                    "ReleaseLocks() called but the install base lock has already been released.",
                );
            };
            release_lock(install_base_lock);
        }
    }

    // Java: src/main/cpp/blaze.cc:1757 BlazeServer::Connect
    /// Connects to the server. In-process there is never a running server to connect to.
    pub fn connect(&mut self) -> bool {
        assert!(!self.connected());
        // PORT: no gRPC server exists until W8; the in-process server is started per command
        // (StartServerAndConnect), so connecting to an existing one always fails.
        false
    }

    // Java: src/main/cpp/blaze.cc:936 StartServerAndConnect
    /// Puts this object in connected state for the in-process server.
    // PORT: ExecuteDaemon + ConnectOrDie: the server runs in this process (it is started by
    // InProcessServer::run_command), so "connecting" always succeeds and the server pid is ours.
    pub fn connect_in_process(&mut self) {
        assert!(!self.connected());
        self.process_info_.server_pid_ = std::process::id() as i32;
        self.connected = true;
    }

    // Java: src/main/cpp/blaze.cc:1947 BlazeServer::KillRunningServer
    pub fn kill_running_server(&mut self) {
        assert!(self.connected());
        ::blaze_util::unported!(
            "BlazeServer::KillRunningServer",
            "there is no running server in-process"
        )
    }

    // Java: src/main/cpp/blaze.cc:2018 BlazeServer::Communicate
    /// Sends the command line to the (in-process) server. Returns the desired exit code.
    pub fn communicate(
        &mut self,
        command: &str,
        command_args: &[String],
        invocation_policy: &str,
        original_startup_options: &[RcStartupFlag],
        logging_info: &LoggingInfo,
        client_startup_duration: DurationMillis,
        extract_data_duration: Option<DurationMillis>,
        command_wait_duration: Option<DurationMillis>,
        server: &dyn InProcessServer,
        startup: &ServerStartup,
    ) -> i32 {
        assert!(self.connected());
        assert!(self.process_info_.server_pid_ > 0);

        let mut arg_vector: Vec<String> = Vec::new();
        if !command.is_empty() {
            arg_vector.push(command.to_string());
            add_logging_args(
                logging_info,
                client_startup_duration,
                extract_data_duration,
                command_wait_duration,
                &mut arg_vector,
            );
        }
        arg_vector.extend(command_args.iter().cloned());

        let request = build_run_request(
            &arg_vector,
            invocation_policy,
            original_startup_options,
            self.block_for_lock_,
            self.quiet_,
            self.preemptible_,
        );

        // Java: src/main/cpp/blaze.cc:2067 BlazeServer::Communicate
        // The C++ releases the client-side locks here ("as the server may outlive the client and
        // must implement its own locking of the install and output bases"), logs "Released the
        // client-side locks on the install and output bases" and calls ReleaseLocks().
        //
        // PORT: the locks are NOT released. Blaze has no long-lived server: the command runs in
        // this process (`server.run_command` below), so the client's file locks are the only
        // locks there are between two Blaze processes. Both stay held for the whole command, as
        // in RunBatchMode, and are released when the process exits (the descriptors are never
        // closed; BlazeServer has no destructor in C++ either). What the Java server does
        // instead does not apply in-process:
        // - BlazeCommandDispatcher.java:239-266 (the server's command lock: "Another command (%s)
        //   is running. Waiting for it to complete on the server%s..." and, with
        //   --noblock_for_lock, "Another command (%s) is running. Exiting immediately." with
        //   LOCK_HELD_NOBLOCK_FOR_LOCK) can never be observed between two Blaze clients: the
        //   second client waits in AcquireLock ("Another command holds the output base lock:
        //   ...", "Waiting for it to complete...") or, with --noblock_for_lock, exits there with
        //   LOCK_HELD_NOBLOCK_FOR_LOCK ("Exiting because the output base lock is held and
        //   --noblock_for_lock was given."). That is what Bazel prints in --batch mode.
        // - BlazeRuntime.java:1396-1408 (the server's own shared lock on the install base) is
        //   covered by the client's shared install base lock, held as long as the server runs.
        // Outside --batch the lock descriptors are O_CLOEXEC (AcquireLock), so processes the
        // server spawns do not inherit the locks, and an exec of this process (the `run`
        // command's ExecRequest, once ported) drops them, as Bazel's client has by then.

        // PORT: in-process, the server runs the request here and writes to this process's
        // stdout/stderr itself; broken-pipe handling, ExecRequest (`run`) and the abrupt-exit
        // paths of the response loop belong to the gRPC client (W8). The cancel thread runs while
        // the command does (Ctrl-C: `signal_handler`).
        crate::signal_handler::with_cancel_thread(server.canceller(), || {
            server.run_command(startup, request)
        })
    }
}

/// The `RunRequest` that `BlazeServer::Communicate` sends.
pub fn build_run_request(
    arg_vector: &[String],
    invocation_policy: &str,
    original_startup_options: &[RcStartupFlag],
    block_for_lock: bool,
    quiet: bool,
    preemptible: bool,
) -> RunRequest {
    // Java: src/main/cpp/blaze.cc:2038 BlazeServer::Communicate
    let mut request = RunRequest::default();
    // PORT: request_cookie_ comes from server_info.rawproto; the in-process server has none.
    request.cookie = String::new();
    request.block_for_lock = block_for_lock;
    request.quiet = quiet;
    request.preemptible = preemptible;
    request.client_description = format!("pid={}", get_process_id_as_string());
    for arg in arg_vector {
        request.arg.push(arg.as_bytes().to_vec());
    }
    if !invocation_policy.is_empty() {
        request.invocation_policy = invocation_policy.to_string();
    }
    // Bazel's command_extension_adder is null.

    for startup_option in original_startup_options {
        let proto_option_field = StartupOption {
            source: startup_option.source.as_bytes().to_vec(),
            option: startup_option.value.as_bytes().to_vec(),
        };
        request.startup_options.push(proto_option_field);
        // The C++ calls request.add_startup_options() a second time for each option
        // (blaze.cc:2057), so every option is followed by an empty StartupOption.
        request.startup_options.push(StartupOption::default());
    }
    request
}

// Java: src/main/cpp/blaze.cc:363 EscapeForOptionSource
/// Escapes colons by replacing them with '_C' and underscores by replacing them with '_U'. E.g.
/// "name:foo_bar" becomes "name_Cfoo_Ubar".
pub fn escape_for_option_source(input: &str) -> String {
    input.replace('_', "_U").replace(':', "_C")
}

// Java: src/main/cpp/blaze.cc:371 GetServerExeArgs
/// Returns the JVM command argument array and the index of the first startup option in it.
pub fn get_server_exe_args(
    jvm_path: &Path,
    server_jar_path: &str,
    archive_contents: &[String],
    install_md5: &str,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    startup_options: &StartupOptions,
) -> (Vec<String>, usize) {
    get_server_exe_args_with(
        jvm_path,
        server_jar_path,
        archive_contents,
        install_md5,
        workspace_layout,
        workspace,
        startup_options,
        &get_system_javabase,
    )
}

/// [`get_server_exe_args`] with `GetSystemJavabase()` injected (for tests).
pub fn get_server_exe_args_with(
    jvm_path: &Path,
    server_jar_path: &str,
    archive_contents: &[String],
    install_md5: &str,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    startup_options: &StartupOptions,
    get_system_javabase: &dyn Fn() -> String,
) -> (Vec<String>, usize) {
    let mut result: Vec<String> = Vec::new();

    // e.g. A Blaze server process running in ~/src/build_root (where there's a
    // ~/src/build_root/WORKSPACE file) will appear in ps(1) as "blaze(src)".
    result.push(format!(
        "{}({})",
        startup_options.get_lowercase_product_name(),
        workspace_layout.get_pretty_workspace_name(workspace)
    ));
    startup_options.add_jvm_argument_prefix(&jvm_path.get_parent().get_parent(), &mut result);

    // com.google.devtools.build.lib.unsafe.StringUnsafe uses reflection to access private
    // fields in java.lang.String. The Bazel server requires Java 11, so this option is known to
    // be supported.
    result.push("--add-opens=java.base/java.lang=ALL-UNNAMED".to_string());

    result.push("-Xverify:none".to_string());

    let user_options = startup_options.host_jvm_args.clone();

    // Add JVM arguments particular to building blaze64 and particular JVM versions.
    if let Err((jvm_args_exit_code, error)) = startup_options.add_jvm_arguments(
        &startup_options.get_server_javabase(),
        &mut result,
        &user_options,
    ) {
        bazel_die(jvm_args_exit_code, BLAZE_CC, 403, error);
    }

    // We put all directories on java.library.path that contain .so/.dll files.
    let mut java_library_paths: std::collections::BTreeSet<String> = Default::default();
    let mut java_library_path = String::from("-Djava.library.path=");
    let real_install_dir = startup_options.install_base.clone();

    for it in archive_contents {
        if is_shared_library(it) {
            let libpath = real_install_dir
                .get_relative(&dirname(it))
                .as_jvm_argument();
            // Only add the library path if it's not added yet.
            if java_library_paths.insert(libpath.clone()) {
                if java_library_paths.len() > 1 {
                    java_library_path.push(K_LIST_SEPARATOR);
                }
                java_library_path.push_str(&libpath);
            }
        }
    }
    result.push(java_library_path);

    // TODO: Investigate whether this still has any effect. File name encoding is governed by
    // sun.jnu.encoding in JDKs with JEP 400, which can't be influenced by setting a property.
    result.push("-Dfile.encoding=ISO-8859-1".to_string());
    // Force into the root locale to ensure consistent behavior of string operations across
    // machines (e.g. in the tr_TR locale, capital ASCII 'I' turns into a special Unicode 'i'
    // when converted to lower case).
    result.push("-Duser.country=".to_string());
    result.push("-Duser.language=".to_string());
    result.push("-Duser.variant=".to_string());

    // Allow more files to be watched per directory than the default limit of 500.
    result.push("-Djdk.nio.file.WatchService.maxEventsPerPoll=10000".to_string());

    // Disable warnings about unsafe memory access, which still occurs in protobuf.
    result.push("-Dsun.misc.unsafe.memory.access=allow".to_string());

    // PORT: skipped, Windows only: -XX:+IgnoreUnrecognizedVMOptions,
    // -XX:+UseAllWindowsProcessorGroups.

    if startup_options.host_jvm_debug {
        bazel_log(
            LogLevel::User,
            BLAZE_CC,
            459,
            "Running host JVM under debugger (listening on TCP port 5005).",
        );
        // Start JVM so that it listens for a connection from a JDWP-compliant debugger:
        result.push("-agentlib:jdwp=transport=dt_socket,server=y,address=5005".to_string());
    }
    result.extend(user_options.iter().cloned());

    startup_options.add_jvm_argument_suffix(&real_install_dir, server_jar_path, &mut result);

    // JVM arguments are complete. Now pass in Blaze startup options.
    // Note that we always use the --flag=ARG form (instead of the --flag ARG one) so that
    // BlazeRuntime#splitStartupOptions has an easy job.
    let first_startup_arg = result.len();

    if !startup_options.batch {
        result.push(format!("--max_idle_secs={}", startup_options.max_idle_secs));
        if startup_options.shutdown_on_low_sys_mem {
            result.push("--shutdown_on_low_sys_mem".to_string());
        } else {
            result.push("--noshutdown_on_low_sys_mem".to_string());
        }
    } else {
        // --batch must come first in the arguments to Java main() because the code expects it to
        // be at args[0] if it's been set.
        result.push("--batch".to_string());
    }

    if startup_options.command_port != 0 {
        result.push(format!("--command_port={}", startup_options.command_port));
    }

    result.push(format!(
        "--connect_timeout_secs={}",
        startup_options.connect_timeout_secs
    ));

    result.push(format!(
        "--output_user_root={}",
        startup_options.output_user_root.as_command_line_argument()
    ));
    result.push(format!(
        "--install_base={}",
        startup_options.install_base.as_command_line_argument()
    ));
    result.push(format!("--install_md5={}", install_md5));
    if startup_options.lock_install_base {
        // This flag is not user-settable. Its sole purpose is to alter the behavior for Blaze
        // and Bazel. Do *not* explicitly set it to disabled.
        result.push("--lock_install_base".to_string());
    }
    result.push(format!(
        "--output_base={}",
        startup_options.output_base.as_command_line_argument()
    ));
    result.push(format!("--workspace_directory={}", convert_path(workspace)));
    if startup_options.autodetect_server_javabase {
        result.push(format!(
            "--default_system_javabase={}",
            get_system_javabase()
        ));
    }

    if !startup_options.server_jvm_out.is_empty() {
        result.push(format!(
            "--server_jvm_out={}",
            startup_options.server_jvm_out.as_command_line_argument()
        ));
    }

    if !startup_options.failure_detail_out.is_empty() {
        result.push(format!(
            "--failure_detail_out={}",
            startup_options
                .failure_detail_out
                .as_command_line_argument()
        ));
    }

    if !startup_options.digest_function.is_empty() {
        // Only include this if a value is requested - we rely on the empty case being "null" to
        // set the programmatic default in the server.
        result.push(format!(
            "--digest_function={}",
            startup_options.digest_function
        ));
    }
    if !startup_options.unix_digest_hash_attribute_name.is_empty() {
        result.push(format!(
            "--unix_digest_hash_attribute_name={}",
            startup_options.unix_digest_hash_attribute_name
        ));
    }
    if startup_options.idle_server_tasks {
        result.push("--idle_server_tasks".to_string());
    } else {
        result.push("--noidle_server_tasks".to_string());
    }

    if startup_options.write_command_log {
        result.push("--write_command_log".to_string());
    } else {
        result.push("--nowrite_command_log".to_string());
    }

    if startup_options.fatal_event_bus_exceptions {
        result.push("--fatal_event_bus_exceptions".to_string());
    } else {
        result.push("--nofatal_event_bus_exceptions".to_string());
    }
    if startup_options.windows_enable_symlinks {
        result.push("--windows_enable_symlinks".to_string());
    } else {
        result.push("--nowindows_enable_symlinks".to_string());
    }
    if startup_options.remote_repo_contents_cache {
        result.push("--experimental_remote_repo_contents_cache".to_string());
        // Don't set the flag to false if it's not set - non-OSS Blaze does not know about this
        // flag.
    }
    // We use this syntax so that the logic in AreStartupOptionsDifferent() that decides whether
    // the server needs killing is simpler.
    if startup_options.client_debug {
        result.push("--client_debug=true".to_string());
    } else {
        result.push("--client_debug=false".to_string());
    }

    // These flags are passed to the java process only for Blaze reporting purposes; the real
    // interpretation of the jvm flags occurs when we set up the java command line.
    if !startup_options.get_explicit_server_javabase().is_empty() {
        result.push(format!(
            "--server_javabase={}",
            startup_options
                .get_explicit_server_javabase()
                .as_command_line_argument()
        ));
    }
    if startup_options.host_jvm_debug {
        result.push("--host_jvm_debug".to_string());
    }
    if !startup_options.host_jvm_args.is_empty() {
        for arg in &startup_options.host_jvm_args {
            result.push(format!("--host_jvm_args={}", arg));
        }
    }
    if !startup_options.extra_classpath.is_empty() {
        result.push(format!(
            "--extra_classpath={}",
            startup_options.extra_classpath
        ));
    }

    // Pass in invocation policy as a startup argument for batch mode only.
    if startup_options.batch && !startup_options.invocation_policy.is_empty() {
        result.push(format!(
            "--invocation_policy={}",
            startup_options.invocation_policy
        ));
    }

    result.push(format!("--product_name={}", startup_options.product_name));

    if !startup_options.cgroup_parent.is_empty() {
        result.push(format!(
            "--experimental_cgroup_parent={}",
            startup_options.cgroup_parent
        ));
    }

    if startup_options.run_in_user_cgroup {
        result.push("--experimental_run_in_user_cgroup".to_string());
    }

    startup_options.add_extra_options(&mut result);

    // The option sources are transmitted in the following format:
    // --option_sources=option1:source1:option2:source2:...
    let mut option_sources = String::from("--option_sources=");
    let mut first = true;
    for (key, value) in &startup_options.option_sources {
        if !first {
            option_sources.push(':');
        }
        first = false;
        option_sources.push_str(&escape_for_option_source(key));
        option_sources.push(':');
        option_sources.push_str(&escape_for_option_source(value));
    }

    result.push(option_sources);
    (result, first_startup_arg)
}

// Java: src/main/cpp/blaze.cc:638 AddLoggingArgs
/// Adds common command options for logging to the given argument array.
pub fn add_logging_args(
    logging_info: &LoggingInfo,
    client_startup_duration: DurationMillis,
    extract_data_duration: Option<DurationMillis>,
    command_wait_duration: Option<DurationMillis>,
    args: &mut Vec<String>,
) {
    // The time in ms the launcher spends before sending the request to the blaze server.
    args.push(format!("--startup_time={}", client_startup_duration.millis));

    // The time in ms a command had to wait on a busy Blaze server process. This is part of
    // startup_time.
    if let Some(d) = command_wait_duration {
        args.push(format!("--command_wait_time={}", d.millis));
    }

    // The time in ms spent on extracting the new blaze version. This is part of startup_time.
    if let Some(d) = extract_data_duration {
        args.push(format!("--extract_data_time={}", d.millis));
    }

    if logging_info.restart_reason != RestartReason::NoRestart {
        args.push(format!(
            "--restart_reason={}",
            reason_string(logging_info.restart_reason)
        ));
    }
    args.push(format!("--binary_path={}", logging_info.binary_path));
}

// Java: src/main/cpp/blaze.cc:672 GetArgumentString
/// Joins the elements of the specified array with NUL's (\0's), akin to the format of
/// /proc/$PID/cmdline.
pub fn get_argument_string(argument_array: &[String]) -> Vec<u8> {
    argument_array.join("\0").into_bytes()
}

// Java: src/main/cpp/blaze.cc:678 EnsureServerDir
fn ensure_server_dir(server_dir: &Path) {
    // The server dir has the connection info - don't allow access by other users.
    if !make_directories_path(server_dir, 0o700) {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            681,
            format!(
                "server directory '{}' could not be created: {}",
                server_dir.as_printable_path(),
                get_last_error_string()
            ),
        );
    }
}

// Java: src/main/cpp/blaze.cc:688 GoToWorkspace
/// Does a chdir into the workspace, and dies if it fails.
fn go_to_workspace(workspace_layout: &WorkspaceLayout, workspace: &str) {
    if workspace_layout.in_workspace(workspace) && !change_directory(workspace) {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            BLAZE_CC,
            692,
            format!(
                "changing directory into {} failed: {}",
                workspace,
                get_last_error_string()
            ),
        );
    }
}

// Java: src/main/cpp/blaze.cc:698 IsServerMode
fn is_server_mode(command: &str) -> bool {
    command == "exec-server"
}

// Java: src/main/cpp/blaze.cc:704 CanRunOutsideWorkspace
/// Keep this in sync with built-in commands annotated with @Command(mustRunInWorkspace =
/// false).
fn can_run_outside_workspace(command: &str) -> bool {
    command == "canonicalize-flags"
        || command == "dump"
        || command == "help"
        || command == "license"
        || command == "shutdown"
        || command == "version"
}

// Java: src/main/cpp/blaze.cc:711 RunServerMode
fn run_server_mode(startup_options: &StartupOptions, server: &BlazeServer) -> i32 {
    if startup_options.batch {
        bazel_die(
            exit_code::BAD_ARGV,
            BLAZE_CC,
            717,
            "exec-server command is not compatible with --batch",
        );
    }

    bazel_log(LogLevel::Info, BLAZE_CC, 721, "Running in server mode.");

    if server.connected() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            724,
            format!(
                "exec-server failed, please shut down existing server pid={} and retry.",
                server.process_info().server_pid_
            ),
        );
    }

    // PORT: exec-server replaces this process with a long-lived server JVM (writes
    // server.pid.txt and cmdline, then execs); Blaze has no standalone server until W8.
    ::blaze_util::unported!("RunServerMode", "exec-server")
}

// Java: src/main/cpp/blaze.cc:754 RunBatchMode
/// Runs blaze in standalone/batch mode: the server handles the command and exits.
fn run_batch_mode(
    server_exe: &Path,
    server_exe_args: &[String],
    first_startup_arg: usize,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    option_processor: &OptionProcessor,
    startup_options: &StartupOptions,
    logging_info: &LoggingInfo,
    extract_data_duration: Option<DurationMillis>,
    command_wait_duration: Option<DurationMillis>,
    blaze_server: &mut BlazeServer,
    server: &dyn InProcessServer,
) -> i32 {
    if blaze_server.connected() {
        blaze_server.kill_running_server();
    }

    let client_startup_duration =
        DurationMillis::new(logging_info.start_time_ms, get_milliseconds_monotonic());

    bazel_log(
        LogLevel::Info,
        BLAZE_CC,
        769,
        format!("Starting {} in batch mode.", startup_options.product_name),
    );

    let command = option_processor.get_command();
    let command_arguments = option_processor.get_command_arguments();

    if !command_arguments.is_empty() && command == "shutdown" {
        let product = startup_options.get_lowercase_product_name();
        bazel_log(
            LogLevel::Warning,
            BLAZE_CC,
            778,
            format!(
                "Running command \"shutdown\" in batch mode.  Batch mode is triggered\nwhen not \
                 running {} within a workspace. If you intend to shutdown an\nexisting {} \
                 server, run \"{} shutdown\" from the directory where\nit was started.",
                startup_options.product_name, startup_options.product_name, product
            ),
        );
    }

    let mut jvm_args_vector: Vec<String> = server_exe_args.to_vec();

    if !command.is_empty() {
        jvm_args_vector.push(command.clone());
        add_logging_args(
            logging_info,
            client_startup_duration,
            extract_data_duration,
            command_wait_duration,
            &mut jvm_args_vector,
        );
    }

    jvm_args_vector.extend(command_arguments.iter().cloned());

    go_to_workspace(workspace_layout, workspace);

    set_scheduling(
        startup_options.batch_cpu_scheduling,
        startup_options.io_nice_level,
    );

    // PORT: in-process, the environment PrepareEnvironmentForJvm computes is not applied
    // (WithEnvVars); the call is kept for its warnings.
    let _env = prepare_environment_for_jvm();

    // Java: src/main/cpp/blaze_util_posix.cc:322 ExecuteProgram
    bazel_log(
        LogLevel::Info,
        BLAZE_UTIL_POSIX_CC,
        325,
        format!(
            "Invoking binary {} in {}",
            server_exe.as_printable_path(),
            get_cwd()
        ),
    );
    // PORT: instead of execv'ing the JVM, run BlazeRuntime.main in-process with the same
    // arguments (without the JVM part). The client locks stay held, as they would by the
    // exec'd process.
    let startup = ServerStartup {
        server_exe_args: server_exe_args.to_vec(),
        first_startup_arg,
    };
    let args = jvm_args_vector[first_startup_arg..].to_vec();
    let _ = std::io::Write::flush(&mut std::io::stdout());
    server.batch_main(&startup, args)
}

// Java: src/main/cpp/blaze.cc:842 GetServerPid
/// After connecting to the Blaze server, returns its PID, or -1 if there was an error.
fn get_server_pid(server_dir: &Path) -> i32 {
    // Note: there is no race here on startup since the server creates the pid file strictly
    // before it binds the socket.
    let pid_file = server_dir.get_relative(K_SERVER_PID_FILE);
    let Some(bufstr) = read_file_path(&pid_file, 32) else {
        return -1;
    };
    // PORT: byte-strings (C++ parses the pid file's raw bytes; a non-digit fails either way).
    let (ok, result) = safe_strto32(&String::from_utf8_lossy(&bufstr));
    if !ok {
        return -1;
    }
    result
}

// PORT: Blaze-only (review 2026-11-18, S2-1); there is no such function in blaze.cc.
//
// Blaze computes the same default output user root and output base as Bazel (GetCacheDir,
// GetHashedBaseDir), so a real Bazel server may be running in the output base. What Bazel's own
// client does then: it connects to that server (BlazeServer::Connect); if the install base
// differs it kills the server ("Killing running server because it is using another version of
// Bazel", EnsureCorrectRunningVersion) and repoints <output_base>/install; if the startup options
// differ it kills it ("Running Bazel server needs to be killed, because the following startup
// options are different:", KillRunningServerIfDifferentStartupOptions); a server it cannot
// connect to is killed with SIGKILL (EnsurePreviousServerProcessTerminated -> KillServerProcess,
// "Killed non-responsive server process (pid=%d)"); then it starts a new server, deleting
// server/command_port and rewriting server/cmdline.
//
// Blaze refuses instead: it has no server protocol (it cannot connect to, reuse or shut down a
// server), and it must not kill another tool's server or change the files that server owns.
// Continuing would repoint `install`, delete `command_port` and run a second writer in the
// output base next to the live server, whose commands are not excluded by the client locks
// (Bazel's client releases them once the server has the request).
//
// The server is identified the way EnsurePreviousServerProcessTerminated identifies it: the pid
// in server/server.pid.txt (GetServerPid), alive and with the start time recorded in
// server/server.starttime (VerifyServerProcess). A pid file whose process is gone, or whose pid
// now belongs to another process (different start time), is stale and ignored as before.
/// The pid of a live server process, other than this process, that owns `output_base`.
pub fn find_live_foreign_server(output_base: &Path) -> Option<i32> {
    let server_pid = get_server_pid(&output_base.get_relative("server"));
    if is_live_foreign_server(server_pid, output_base) {
        Some(server_pid)
    } else {
        None
    }
}

fn is_live_foreign_server(server_pid: i32, output_base: &Path) -> bool {
    // PORT: a pid file naming this very process cannot be a server (Blaze writes no pid file):
    // the pid of a dead server was reused for us.
    server_pid > 0
        && server_pid != std::process::id() as i32
        && verify_server_process(server_pid, output_base)
}

/// The fatal error for a live server (`server_pid`) that is not this process.
fn die_foreign_server_is_running(server_pid: i32, output_base: &Path) -> ! {
    // LOCAL_ENVIRONMENTAL_ERROR, as for RunServerMode's "exec-server failed, please shut down
    // existing server pid=%d and retry." (blaze.cc:724).
    bazel_die(
        exit_code::LOCAL_ENVIRONMENTAL_ERROR,
        BLAZE_RS,
        line!(),
        format!(
            "blaze: a running Bazel server (pid={}) owns the output base '{}'. Blaze has no \
             server protocol and will not kill or modify another tool's server. Run 'bazel \
             shutdown' in this workspace, or run Blaze with a different --output_user_root or \
             --output_base.",
            server_pid,
            output_base.as_printable_path()
        ),
    )
}

/// Dies if a live server process that is not this process owns `output_base`. Reads only;
/// nothing under the output base is created, changed or deleted.
pub fn die_if_foreign_server_is_running(output_base: &Path) {
    if let Some(server_pid) = find_live_foreign_server(output_base) {
        die_foreign_server_is_running(server_pid, output_base);
    }
}

// Java: src/main/cpp/blaze.cc:916 EnsurePreviousServerProcessTerminated
/// Ensures that any server previously associated with `server_dir` is no longer running.
fn ensure_previous_server_process_terminated(
    server_dir: &Path,
    startup_options: &StartupOptions,
    logging_info: &mut LoggingInfo,
) {
    let server_pid = get_server_pid(server_dir);
    if server_pid > 0 {
        if is_live_foreign_server(server_pid, &startup_options.output_base) {
            // PORT: the C++ kills the server here (KillServerProcess: SIGKILL to its process
            // group, "Killed non-responsive server process (pid=%d)", restart reason
            // SERVER_UNRESPONSIVE, or SERVER_VANISHED if it died meanwhile). Blaze never kills
            // another tool's server (see find_live_foreign_server). RunLauncher already checked
            // twice, so this is a server that appeared while this client held the output base
            // lock.
            die_foreign_server_is_running(server_pid, &startup_options.output_base);
        } else {
            logging_info.set_restart_reason_if_not_set(RestartReason::PidFileButNoServer);
        }
    }
}

// Java: src/main/cpp/blaze.cc:936 StartServerAndConnect
/// Starts up a new server and connects to it. Exits if it didn't work out.
fn start_server_and_connect(
    server_exe_args: &[String],
    server_dir: &Path,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    startup_options: &StartupOptions,
    logging_info: &mut LoggingInfo,
    blaze_server: &mut BlazeServer,
    build_label: &str,
) {
    // Bazel's StartupInterceptor is null.

    // Delete the old command_port file if it already exists. Otherwise we might run into the
    // race condition that we read the old command_port file before the new server has written
    // the new file and we try to connect to the old port, run into a timeout and try again.
    let _ = unlink_path_path(&server_dir.get_relative("command_port"));

    // Delete the OOM file if it exists (e.g. leftover from a previous server that OOmed).
    // Otherwise in the future we might incorrectly think this server OOMed.
    if unlink_path_path(&get_oom_file_path(&startup_options.output_base)) {
        bazel_log(LogLevel::Info, BLAZE_CC, 957, "Deleted old OOM file.");
    }

    ensure_server_dir(server_dir);

    // Really make sure there's no other server running in this output base (even an
    // unresponsive one), as that could cause major problems.
    ensure_previous_server_process_terminated(server_dir, startup_options, logging_info);

    // cmdline file is used to validate the server running in this server_dir. There's no server
    // running now so we're safe to unconditionally write this.
    write_file_path(
        &get_argument_string(server_exe_args),
        &server_dir.get_relative("cmdline"),
        0o644,
    );

    // Do this here instead of in the daemon so the user can see if it fails.
    go_to_workspace(workspace_layout, workspace);

    logging_info.set_restart_reason_if_not_set(RestartReason::NoDaemon);

    set_scheduling(
        startup_options.batch_cpu_scheduling,
        startup_options.io_nice_level,
    );

    bazel_log(
        LogLevel::User,
        BLAZE_CC,
        980,
        format!(
            "Starting local {} server ({}) and connecting to it...",
            startup_options.product_name, build_label
        ),
    );
    // PORT: in-process, the environment PrepareEnvironmentForJvm computes is not applied (it
    // would be the daemon's environment); the call is kept for its warnings.
    let _env = prepare_environment_for_jvm();

    // PORT: ExecuteDaemon + ConnectOrDie: see BlazeServer::connect_in_process.
    blaze_server.connect_in_process();
}

// Java: src/main/cpp/blaze.cc:1090 KillRunningServerIfDifferentStartupOptions
/// Kills the running Blaze server, if any, if the startup options do not match. Returns true if
/// the server has been killed.
fn kill_running_server_if_different_startup_options(blaze_server: &BlazeServer) -> bool {
    if !blaze_server.connected() {
        return false;
    }
    ::blaze_util::unported!(
        "KillRunningServerIfDifferentStartupOptions",
        "there is no running server in-process"
    )
}

// Java: src/main/cpp/blaze.cc:1148 EnsureCorrectRunningVersion
/// Kills the old running server if it is not the same version as us, dealing with various
/// combinations of installation scheme (installation symlink and older MD5_MANIFEST contents).
/// This function requires that the installation be complete, and the server lock acquired.
fn ensure_correct_running_version(
    startup_options: &StartupOptions,
    logging_info: &mut LoggingInfo,
    blaze_server: &mut BlazeServer,
) {
    // Read the previous installation's semaphore symlink in output_base. If the target dirs
    // don't match, or if the symlink was not present, then kill any running servers. Lastly,
    // symlink to our installation so others know which installation is running.
    let install_base_symlink = startup_options.output_base.get_relative("install");
    let prev_install_base = read_directory_symlink(&install_base_symlink);
    let same = match &prev_install_base {
        Some(prev) => are_paths_equivalent(prev, &startup_options.install_base),
        None => false,
    };
    if !same {
        if blaze_server.connected() {
            bazel_log(
                LogLevel::Info,
                BLAZE_CC,
                1163,
                format!(
                    "Killing running server because it is using another version of {}",
                    startup_options.product_name
                ),
            );
            blaze_server.kill_running_server();
            logging_info.restart_reason = RestartReason::NewVersion;
        }

        unlink_path_path(&install_base_symlink);
        if !symlink_directories(&startup_options.install_base, &install_base_symlink) {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_CC,
                1173,
                format!(
                    "failed to create installation symlink '{}': {}",
                    install_base_symlink.as_printable_path(),
                    get_last_error_string()
                ),
            );
        }

        // Update the mtime of the install base so that cleanup tools can find install bases
        // that haven't been used for a long time. Ignore permissions errors (i.e. if the install
        // base is not writable).
        if !set_mtime_to_now_if_possible(&startup_options.install_base) {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_CC,
                1184,
                format!(
                    "failed to set timestamp on '{}': {}",
                    startup_options.install_base.as_printable_path(),
                    get_last_error_string()
                ),
            );
        }
    }
}

// Java: src/main/cpp/blaze.cc:1200 RunClientServerMode
/// Runs the launcher in client/server mode: ensures that there's a running server, then
/// forwards the user's command to it.
fn run_client_server_mode(
    server_exe_args: &[String],
    first_startup_arg: usize,
    server_dir: &Path,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    option_processor: &OptionProcessor,
    startup_options: &StartupOptions,
    logging_info: &mut LoggingInfo,
    extract_data_duration: Option<DurationMillis>,
    command_wait_duration: Option<DurationMillis>,
    blaze_server: &mut BlazeServer,
    build_label: &str,
    server: &dyn InProcessServer,
) -> i32 {
    if !blaze_server.connected() {
        start_server_and_connect(
            server_exe_args,
            server_dir,
            workspace_layout,
            workspace,
            startup_options,
            logging_info,
            blaze_server,
            build_label,
        );
    }
    // PORT: the check that the server's cwd (/proc/<pid>/cwd) is still the workspace is
    // skipped: the in-process server shares this process's cwd.

    bazel_log(
        LogLevel::Info,
        BLAZE_CC,
        1247,
        format!(
            "Connected (server pid={}).",
            blaze_server.process_info().server_pid_
        ),
    );

    // Wall clock time since process startup.
    let client_startup_duration =
        DurationMillis::new(logging_info.start_time_ms, get_milliseconds_monotonic());

    crate::signal_handler::install(
        &startup_options.product_name,
        &startup_options.output_base,
        blaze_server.process_info().server_pid_,
        &blaze_server.process_info().jvm_log_file_,
    );
    let startup = ServerStartup {
        server_exe_args: server_exe_args.to_vec(),
        first_startup_arg,
    };
    let _ = std::io::Write::flush(&mut std::io::stdout());
    // SignalHandler::PropagateSignalOrExit(exit_code): the caller exits with the code.
    let exit_code = blaze_server.communicate(
        &option_processor.get_command(),
        &option_processor.get_command_arguments(),
        &startup_options.invocation_policy,
        &startup_options.original_startup_options_,
        logging_info,
        client_startup_duration,
        extract_data_duration,
        command_wait_duration,
        server,
        &startup,
    );
    crate::signal_handler::propagate_signal_or_exit(exit_code)
}

// Java: src/main/cpp/blaze.cc:1265 ParseOptionsOrDie
fn parse_options_or_die(
    cwd: &str,
    workspace: &str,
    option_processor: &mut OptionProcessor,
    env: &blaze_options::rc::option_processor::ClientEnvironment,
    startup_options: &mut StartupOptions,
    args: Vec<String>,
) {
    if let Err((parse_exit_code, error)) = parse_options_with_logging(
        option_processor,
        env,
        BAZEL_SYSTEM_BAZELRC_PATH,
        args,
        workspace,
        cwd,
        startup_options,
    ) {
        print_startup_options_provenance_message(startup_options);
        bazel_die(parse_exit_code, BLAZE_CC, 1275, error);
    }
}

// Java: src/main/cpp/blaze.cc:1279 GetCanonicalCwd
fn get_canonical_cwd() -> String {
    let result = make_canonical(&get_cwd());
    if result.is_empty() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            1282,
            format!(
                "blaze_util::MakeCanonical('{}') failed: {}",
                get_cwd(),
                get_last_error_string()
            ),
        );
    }
    result
}

// Java: src/main/cpp/blaze.cc:1289 PrepareDirectories
fn prepare_directories(startup_options: &mut StartupOptions) {
    create_secure_directory(&startup_options.output_user_root);

    if !path_exists_path(&startup_options.output_base) {
        if !make_directories_path(&startup_options.output_base, 0o777) {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                BLAZE_CC,
                1295,
                format!(
                    "Output base directory '{}' could not be created: {}",
                    startup_options.output_base.as_printable_path(),
                    get_last_error_string()
                ),
            );
        }
    } else if !is_directory_path(&startup_options.output_base) {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            1302,
            format!(
                "Output base directory '{}' could not be created. It exists but is not a \
                 directory.",
                startup_options.output_base.as_printable_path()
            ),
        );
    }
    if !can_access_directory_path(&startup_options.output_base) {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            1309,
            format!(
                "Output base directory '{}' must be readable and writable.",
                startup_options.output_base.as_printable_path()
            ),
        );
    }

    exclude_path_from_backup(&startup_options.output_base);

    startup_options.output_base = startup_options.output_base.canonicalize();
    if startup_options.output_base.is_empty() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            BLAZE_CC,
            1319,
            format!(
                "Output base directory could not be canonicalized: {}",
                get_last_error_string()
            ),
        );
    }
}

// Java: src/main/cpp/blaze.cc:1328 PrepareEnvironmentForJvm
/// Prepares the environment to be suitable to start a JVM. Changes made to the environment in
/// this function *will not* be part of '--client_env'.
pub fn prepare_environment_for_jvm() -> BTreeMap<String, EnvVarValue> {
    let mut result: BTreeMap<String, EnvVarValue> = BTreeMap::new();

    // Make sure all existing environment variables appear as part of the resulting map unless
    // they are overridden below by UNSET values.
    for var_value in get_processed_env() {
        let Some(equals) = var_value.find('=') else {
            // Ignore possibly-bad environment.
            continue;
        };
        let var = var_value[..equals].to_string();
        let value = var_value[equals + 1..].to_string();
        result.insert(var, EnvVarValue::new(EnvVarAction::Set, value));
    }

    if exists_env("LD_ASSUME_KERNEL") {
        // Fix for bug: if ulimit -s and LD_ASSUME_KERNEL are both specified, the JVM fails to
        // create threads.  See thread_stack_regtest. This is also provoked by
        // LD_LIBRARY_PATH=/usr/lib/debug, or anything else that causes the JVM to use
        // LinuxThreads.
        bazel_log(
            LogLevel::Warning,
            BLAZE_CC,
            1355,
            "ignoring LD_ASSUME_KERNEL in environment.",
        );
        result.insert(
            "LD_ASSUME_KERNEL".to_string(),
            EnvVarValue::new(EnvVarAction::Unset, ""),
        );
    }

    if exists_env("LD_PRELOAD") {
        bazel_log(
            LogLevel::Warning,
            BLAZE_CC,
            1360,
            "ignoring LD_PRELOAD in environment.",
        );
        result.insert(
            "LD_PRELOAD".to_string(),
            EnvVarValue::new(EnvVarAction::Unset, ""),
        );
    }

    // These would override --host_jvm_args
    for var in ["_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS"] {
        if exists_env(var) {
            bazel_log(
                LogLevel::Warning,
                BLAZE_CC,
                1368,
                format!("ignoring {} in environment.", var),
            );
            result.insert(var.to_string(), EnvVarValue::new(EnvVarAction::Unset, ""));
        }
    }

    // Ensure that the JVM runs with a locale that supports Unicode characters in filenames,
    // environment variables, etc. On Linux, the JVM goes through the regular locale mechanism.
    // In particular, we can only force UTF-8 if we can find a locale that supports it.
    // Furthermore, for backwards compatibility with setups using other encodings, we pick a
    // Latin-1 locale if possible to support arbitrary byte sequences, not just valid UTF-8.
    let mut want_locale: Option<&str> = None;
    for candidate_locale in ["en_US.ISO-8859-1", "C.UTF-8", "en_US.UTF-8"] {
        let c = std::ffi::CString::new(candidate_locale).unwrap_or_default();
        // SAFETY: newlocale with a NUL-terminated name and no base locale; freed below.
        let locale =
            unsafe { libc::newlocale(libc::LC_CTYPE_MASK, c.as_ptr(), std::ptr::null_mut()) };
        if !locale.is_null() {
            // SAFETY: locale was returned by newlocale.
            unsafe { libc::freelocale(locale) };
            want_locale = Some(candidate_locale);
            break;
        }
    }

    if let Some(locale) = want_locale {
        result.insert(
            "LC_ALL".to_string(),
            EnvVarValue::new(EnvVarAction::Set, locale),
        );
    }

    result
}

// Java: src/main/cpp/blaze.cc:1405 CheckAndGetBinaryPath
fn check_and_get_binary_path(cwd: &str, argv0: &str) -> String {
    if is_absolute(argv0) {
        argv0.to_string()
    } else {
        let abs_path = join_path(cwd, argv0);
        let resolved_path = make_canonical(&abs_path);
        if !resolved_path.is_empty() {
            resolved_path
        } else {
            // This happens during our integration tests, but thats okay, as we won't log the
            // invocation anyway.
            abs_path
        }
    }
}

// Java: src/main/cpp/blaze.cc:1451 PrintBazelLeaf
/// Bazel's basil leaf and B-shaped logo at initial release, back in 2015.
pub fn print_bazel_leaf() {
    let leaf = concat!(
        ".:                                                                    \n",
        "+: `:`                                                                \n",
        "o:  -o:`                                                              \n",
        "o:   `:+/-.`                                                          \n",
        "o:     `./oo+:-.``                                                    \n",
        "o:        ``-/+soo//-`                                                \n",
        "oo.            ..:+osso:-`                                            \n",
        "oo.  `.``          `-:+sss+.`                                         \n",
        "oo.  `://:.`           `-+sys:`                                       \n",
        "oo.      .://:.`          `:oys:`                                     \n",
        "oo+         .:+o/:`         `:syo-                                    \n",
        "ooo            -/oo+:         `:ss-                                   \n",
        "ooo              `:oso-.        -ys-                                  \n",
        "ooo:               `:sss+.       :ss                                  \n",
        ".oo/                 .osss-       :s/                                 \n",
        ".oo+.                 `:sss+-      oo.                                \n",
        " :oo/`                  -osss-     -s-                                \n",
        " `+oo/                   .osss+.   `s-       `...:/://::::-           \n",
        "  `+oo/`                  .ossso.   :-   `-/+oosos+sssssss-           \n",
        "   `:+o+.`                 -ossso`  `` `:ossssssssssssssso.           \n",
        "     `:+oo:.`               :ssss+`   .ossssssssssssssooo/            \n",
        "       `:/+++/-.``           +ssss:   +sssssssssssoooooo+`            \n",
        "          `-:++o+/:::````    -oooos` .osssoooooooooooooo-             \n",
        "               ...-::::::---``ooooo- /oooo//ooooooooooo-              \n",
        "                             `ooooo+`/oo//ooooooooooo+.               \n",
        "                              /ooooo+oo/:oooooooooo/:.                \n",
        "                              .ooooooo-.//+oooo//-.                   \n",
        "                              .oooooo-     ````                       \n",
        "                              .ooooo/`                                \n",
        "                             `+/+oo+                                  \n",
        "                             `++++++   `.--//////////:..``            \n",
        "                             `+++++/`-:+++++++++++++++++//:.`         \n",
        "                             `++++++/++++++++++++++++++++++//-.       \n",
        "                             .++++++++++++/::.......-://+++++//:`     \n",
        "                             /+++++++++/--`           `.-////////.`   \n",
        "                             /+++++++/-`                 `-///////-   \n",
        "                            `///////:`                     `-//////-  \n",
        "                            -//////-                         .//////. \n",
        "                            -/////-                           ://///: \n",
        "                           `//////.                           `////::.\n",
        "                           ./////:`                           `/:::::.\n",
        "                           .//////.                           `::::::.\n",
        "                           `::////.                           `:::::- \n",
        "                            -::::::`                         `::::::- \n",
        "                            `:::::::`                       `-::::::` \n",
        "                             `:::::::.                     `-::::::.  \n",
        "                              `:::::::-.                ``-:-::::-.   \n",
        "                               `.:::::::--``         ``.--::-::--`    \n",
        "                                 .-::::::::---.....---::::----.`      \n",
        "                                  ``.-::::::::::::-::--------`        \n",
        "                                      `.----------------..`           \n",
        "                                         ```.........```             \n",
    );
    println!("{}", leaf);
}

// Java: src/main/cpp/blaze.cc:1512 PrintVersionInfo
/// Prints client version information to standard output, e.g. when invoking the client with
/// "--version".
pub fn print_version_info(build_label: &str, product_name: &str) {
    println!("{} {}", product_name, build_label);
}

// Java: src/main/cpp/blaze.cc:1516 RunLauncher
fn run_launcher(
    self_path: &str,
    archive: &EmbeddedArchive,
    archive_contents: &[String],
    install_md5: &str,
    startup_options: &StartupOptions,
    option_processor: &OptionProcessor,
    workspace_layout: &WorkspaceLayout,
    workspace: &str,
    build_label: &str,
    logging_info: &mut LoggingInfo,
    server: &dyn InProcessServer,
) -> i32 {
    // PORT: Blaze-only (see find_live_foreign_server): refuse to touch an output base that a
    // live Bazel server owns, before anything in it is changed. Even the lock file is left
    // alone (AcquireLock rewrites it with this client's pid).
    die_if_foreign_server_is_running(&startup_options.output_base);

    // PORT: `blaze_server` is dropped when this function returns, which does not release the
    // locks: a LockHandle is a plain descriptor that only ReleaseLock closes. They are held until
    // the process exits (the C++ never deletes its global `blaze_server` either).
    let mut blaze_server = BlazeServer::new(startup_options);

    let command_wait_duration = Some(blaze_server.acquire_locks());
    let wait_ms = command_wait_duration.map(|d| d.millis).unwrap_or(0);
    bazel_log(
        LogLevel::Info,
        BLAZE_CC,
        1532,
        format!("Acquired the client lock, waited {} milliseconds", wait_ms),
    );

    // PORT: Blaze-only: check again now that the output base lock is held. A Bazel client
    // starts its server while it holds that lock, so a server that came up while this client
    // waited for the lock is seen here, and none can come up from now on.
    die_if_foreign_server_is_running(&startup_options.output_base);

    warn_filesystem_type(&startup_options.output_base);

    let extract_data_duration = extract_data(
        self_path,
        archive,
        archive_contents,
        install_md5,
        startup_options,
        logging_info,
    );

    blaze_server.connect();

    let command = option_processor.get_command();
    if !startup_options.batch && command == "shutdown" && !blaze_server.connected() {
        // TODO(b/134525510): Connected() can return false when the server process is alive but
        // unresponsive, so bailing early here might not always be the right thing to do.
        return exit_code::SUCCESS;
    }

    ensure_correct_running_version(startup_options, logging_info, &mut blaze_server);

    let jvm_path = startup_options.get_jvm();
    let server_jar_path = get_server_jar_path(archive);

    let server_exe = startup_options.get_exe(&jvm_path, &server_jar_path);

    let (server_exe_args, first_startup_arg) = get_server_exe_args(
        &jvm_path,
        &server_jar_path,
        archive_contents,
        install_md5,
        workspace_layout,
        workspace,
        startup_options,
    );
    // PORT: skipped, OpenBSD only: server_exe_args[0] = server_exe.

    if kill_running_server_if_different_startup_options(&blaze_server) && command == "shutdown" {
        return exit_code::SUCCESS;
    }

    let server_dir = startup_options.output_base.get_relative("server");
    if is_server_mode(&command) {
        run_server_mode(startup_options, &blaze_server)
    } else if startup_options.batch {
        run_batch_mode(
            &server_exe,
            &server_exe_args,
            first_startup_arg,
            workspace_layout,
            workspace,
            option_processor,
            startup_options,
            logging_info,
            extract_data_duration,
            command_wait_duration,
            &mut blaze_server,
            server,
        )
    } else {
        run_client_server_mode(
            &server_exe_args,
            first_startup_arg,
            &server_dir,
            workspace_layout,
            workspace,
            option_processor,
            startup_options,
            logging_info,
            extract_data_duration,
            command_wait_duration,
            &mut blaze_server,
            build_label,
            server,
        )
    }
}

// Java: src/main/cpp/blaze.cc:1603 Main
fn blaze_main(
    argv: Vec<String>,
    workspace_layout: &WorkspaceLayout,
    startup_options: &mut StartupOptions,
    server: &dyn InProcessServer,
    archive: &EmbeddedArchive,
    start_time: u64,
) -> i32 {
    // InitializeStdOutErrForUtf8() is a no-op on POSIX.

    // Logging must be set first to assure no log statements are missed.
    set_log_handler(Some(Box::new(BazelLogHandler::new())));

    assert!(!argv.is_empty(), "argv must contain the program name");
    let self_path = get_self_path(&argv[0]);

    if argv.len() == 2 && argv[1] == "leaf" {
        print_bazel_leaf();
        return exit_code::SUCCESS;
    }

    // Extract build_label to be used in two places:
    // 1) PrintVersionInfo()
    // 2) Resolving conditional bazel imports in .bazelrc files (eg. import rc file if bazel
    //    version >=8).
    let build_label = extract_build_label(archive);

    if argv.len() == 2 && argv[1] == "--version" {
        print_version_info(&build_label, &startup_options.get_lowercase_product_name());
        return exit_code::SUCCESS;
    }

    let cwd = get_canonical_cwd();
    let mut logging_info = LoggingInfo::new(check_and_get_binary_path(&cwd, &argv[0]), start_time);

    setup_std_streams();
    if argv.len() == 1 && warn_if_started_from_desktop() {
        // Only check and warn for from-desktop start if there were no args.
        return exit_code::LOCAL_ENVIRONMENTAL_ERROR;
    }

    // Best-effort operation to raise the resource limits from soft to hard.  We do this early
    // during the main program instead of just before execing the Blaze server binary, because
    // it's easier (for testing purposes) and because the Blaze client also benefits from this
    // (e.g. during installation).
    unlimit_resources();

    // PORT: skipped, Windows only: DetectBashAndExportBazelSh.

    if is_running_within_test() {
        bazel_log(
            LogLevel::User,
            BLAZE_CC,
            1660,
            "$TEST_TMPDIR defined, some defaults will be overridden",
        );
    }

    let workspace = workspace_layout.get_workspace(&cwd);
    // Java: src/main/cpp/main.cc:33 (new OptionProcessor) and blaze.cc:1627 SetBuildLabel
    let env = client_environment();
    let mut option_processor = OptionProcessor::new(
        StartupFlagRegistry::bazel(),
        BAZEL_SYSTEM_BAZELRC_PATH,
        &build_label,
        env.clone(),
    );
    parse_options_or_die(
        &cwd,
        &workspace,
        &mut option_processor,
        &env,
        startup_options,
        argv,
    );
    startup_options.maybe_log_startup_option_warnings();

    if startup_options.client_debug {
        set_debug_log(LoggingDetail::Debug);
    } else if startup_options.quiet {
        set_debug_log(LoggingDetail::Quiet);
    } else {
        set_debug_log(LoggingDetail::User);
    }

    // If client_debug was false, this is ignored, so it's accurate.
    bazel_log(
        LogLevel::Info,
        BLAZE_CC,
        1677,
        "Debug logging requested, sending all client log statements to stderr",
    );

    bazel_log(
        LogLevel::Info,
        BLAZE_CC,
        1680,
        format!("Running (pid={})", get_process_id_as_string()),
    );

    // PORT: Blaze-only: --host_jvm_args that configure the JVM's TLS trust store or key store
    // cannot be honoured without a JVM and must not be ignored silently (see
    // StartupOptions::unported_host_jvm_arg_error). BAD_ARGV: the user must change the command
    // line or the rc file the provenance message names.
    if let Some(error) = startup_options.unported_host_jvm_arg_error() {
        print_startup_options_provenance_message(startup_options);
        bazel_die(exit_code::BAD_ARGV, BLAZE_RS, line!(), error);
    }

    if startup_options.unlimit_coredumps {
        unlimit_coredumps();
    }

    if !workspace_layout.in_workspace(&workspace) {
        let command = option_processor.get_command();
        if !can_run_outside_workspace(&command) {
            bazel_log(
                LogLevel::Error,
                BLAZE_CC,
                1689,
                format!(
                    "{}{} must be invoked from within a workspace (below a directory having a \
                     MODULE.bazel file).\nSee documentation at \
                     https://bazel.build/concepts/build-ref#workspace",
                    startup_options.product_name,
                    if command.is_empty() {
                        String::new()
                    } else {
                        format!(" '{}'", command)
                    }
                ),
            );
            return exit_code::BAD_ARGV;
        }
        startup_options.batch = true;
    }

    let (archive_contents, install_md5) = determine_archive_contents(archive);

    startup_options.update_configuration(
        &install_md5,
        &workspace,
        is_server_mode(&option_processor.get_command()),
    );

    prepare_directories(startup_options);

    run_launcher(
        &self_path,
        archive,
        &archive_contents,
        &install_md5,
        startup_options,
        &option_processor,
        workspace_layout,
        &workspace,
        &build_label,
        &mut logging_info,
        server,
    )
}

// Java: src/main/cpp/main.cc:24 main_impl
/// The client's `main`: runs Bazel's client logic for `argv` (`argv[0]` is the program name)
/// with the in-process `server` and the embedded install `archive`. Returns the exit code.
/// Fatal errors (`BAZEL_DIE`) exit the process directly, as the C++ does.
pub fn main(argv: Vec<String>, server: &dyn InProcessServer, archive: &EmbeddedArchive) -> i32 {
    // C++ static initializers run before main.
    crate::util::file::init_static_times();
    let start_time = get_milliseconds_monotonic();
    let workspace_layout = WorkspaceLayout::new();
    let mut startup_options = StartupOptions::bazel();
    let code = blaze_main(
        argv,
        &workspace_layout,
        &mut startup_options,
        server,
        archive,
        start_time,
    );
    // Returning from main (or exit()) runs static destructors: the log handler's.
    let _ = std::io::Write::flush(&mut std::io::stdout());
    logging::close_at_exit();
    code
}
