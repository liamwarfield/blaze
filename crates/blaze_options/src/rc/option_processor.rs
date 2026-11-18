//! Port of the rc-file parts of the client's `OptionProcessor` (`src/main/cpp/option_processor.cc`):
//! splitting startup options from the command, finding and reading the rc files in Bazel's
//! order, and turning rc contents into the `--rc_source`/`--default_override` arguments the
//! server receives.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::rc_file::{ParseError, RcFile, make_canonical, read_file_default};
use super::sem_ver::SemVer;
use super::tokenize::{join_path, split};

// Java: src/main/cpp/option_processor.cc:57 kRcBasename
pub const RC_BASENAME: &str = ".bazelrc";

// Java: src/main/cpp/BUILD:177 BAZEL_SYSTEM_BAZELRC_PATH
pub const BAZEL_SYSTEM_BAZELRC_PATH: &str = "/etc/bazel.bazelrc";

// Java: src/main/cpp/util/exit_code.h:22 blaze_exit_code::ExitCode
/// Client exit codes used by rc processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    BadArgv = 2,
    LocalEnvironmentalError = 36,
    InternalError = 37,
}

// Java: src/main/cpp/startup_options.h:39 RcStartupFlag
/// A startup flag and the rc file it came from ("" for the command line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcStartupFlag {
    pub source: String,
    pub value: String,
}

// Java: src/main/cpp/option_processor.h:52 BlazercOption
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlazercOption {
    pub source_path: String,
    pub command: String,
    pub option: String,
}

// Java: src/main/cpp/option_processor.h:33 CommandLine
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    pub path_to_binary: String,
    pub startup_args: Vec<String>,
    pub command: String,
    pub command_args: Vec<String>,
}

/// The startup-flag registry of `StartupOptions`/`BazelStartupOptions` (names only; parsing
/// the values belongs to the client's `StartupOptions` port, see [`StartupArgsProcessor`]).
#[derive(Clone, Debug)]
pub struct StartupFlagRegistry {
    product_name: String,
    all_nullary_startup_flags: HashSet<String>,
    no_rc_nullary_startup_flags: HashSet<String>,
    valid_unary_startup_flags: HashSet<String>,
}

impl StartupFlagRegistry {
    // Java: src/main/cpp/startup_options.cc:39 StartupOptions::RegisterNullaryStartupFlag
    fn register_nullary_startup_flag(&mut self, flag_name: &str) {
        self.all_nullary_startup_flags
            .insert(format!("--{}", flag_name));
        self.all_nullary_startup_flags
            .insert(format!("--no{}", flag_name));
    }

    // Java: src/main/cpp/startup_options.cc:45 StartupOptions::RegisterNullaryStartupFlagNoRc
    fn register_nullary_startup_flag_no_rc(&mut self, flag_name: &str) {
        self.register_nullary_startup_flag(flag_name);
        self.no_rc_nullary_startup_flags
            .insert(format!("--{}", flag_name));
        self.no_rc_nullary_startup_flags
            .insert(format!("--no{}", flag_name));
    }

    // Java: src/main/cpp/startup_options.cc:59 StartupOptions::RegisterUnaryStartupFlag
    fn register_unary_startup_flag(&mut self, flag_name: &str) {
        self.valid_unary_startup_flags
            .insert(format!("--{}", flag_name));
    }

    // Java: src/main/cpp/startup_options.cc:68 StartupOptions::StartupOptions
    // Java: src/main/cpp/bazel_startup_options.cc:29 BazelStartupOptions::BazelStartupOptions
    /// The flags registered by `BazelStartupOptions` on Linux.
    pub fn bazel() -> Self {
        let mut r = StartupFlagRegistry {
            product_name: "Bazel".to_string(),
            all_nullary_startup_flags: HashSet::new(),
            no_rc_nullary_startup_flags: HashSet::new(),
            valid_unary_startup_flags: HashSet::new(),
        };
        for f in [
            "batch",
            "batch_cpu_scheduling",
            "block_for_lock",
            "quiet",
            "client_debug",
            "preemptible",
            "fatal_event_bus_exceptions",
            "host_jvm_debug",
            "autodetect_server_javabase",
            "idle_server_tasks",
            "shutdown_on_low_sys_mem",
        ] {
            r.register_nullary_startup_flag(f);
        }
        r.register_nullary_startup_flag_no_rc("ignore_all_rc_files");
        for f in [
            "unlimit_coredumps",
            "write_command_log",
            "windows_enable_symlinks",
            "experimental_remote_repo_contents_cache",
            "experimental_run_in_user_cgroup",
        ] {
            r.register_nullary_startup_flag(f);
        }
        for f in [
            "command_port",
            "connect_timeout_secs",
            "local_startup_timeout_secs",
            "digest_function",
            "unix_digest_hash_attribute_name",
            "server_javabase",
            "host_jvm_args",
            "host_jvm_profile",
            "invocation_policy",
            "io_nice_level",
            "install_base",
            "macos_qos_class",
            "max_idle_secs",
            "output_base",
            "output_user_root",
            "server_jvm_out",
            "failure_detail_out",
            "experimental_cgroup_parent",
            "extra_classpath",
        ] {
            r.register_unary_startup_flag(f);
        }
        r.register_nullary_startup_flag_no_rc("home_rc");
        r.register_nullary_startup_flag_no_rc("system_rc");
        r.register_nullary_startup_flag_no_rc("workspace_rc");
        r.register_unary_startup_flag("bazelrc");
        r
    }

    // Java: src/main/cpp/startup_options.cc:168 StartupOptions::GetLowercaseProductName
    pub fn get_lowercase_product_name(&self) -> String {
        self.product_name.to_lowercase()
    }

    /// Whether `flag` (`--foo`/`--nofoo`) is a nullary flag forbidden in rc files.
    pub fn is_no_rc_nullary(&self, flag: &str) -> bool {
        self.no_rc_nullary_startup_flags.contains(flag)
    }

    // Java: src/main/cpp/startup_options.cc:172 StartupOptions::IsUnary
    pub fn is_unary(&self, arg: &str) -> bool {
        match arg.find('=') {
            None => self.valid_unary_startup_flags.contains(arg),
            Some(i) => self.valid_unary_startup_flags.contains(&arg[..i]),
        }
    }

    // Java: src/main/cpp/startup_options.cc:183 StartupOptions::MaybeCheckValidNullary
    /// `Ok(is_nullary)`, or `Err(error)` if a nullary flag was given a value.
    pub fn maybe_check_valid_nullary(&self, arg: &str) -> Result<bool, String> {
        let Some(i) = arg.find('=') else {
            return Ok(self.all_nullary_startup_flags.contains(arg));
        };
        let f = &arg[..i];
        if !self.all_nullary_startup_flags.contains(f) {
            return Ok(false);
        }
        Err(format!(
            "In argument '{}': option '{}' does not take a value.",
            arg, f
        ))
    }
}

/// Processes the ordered startup flags (Java: `StartupOptions::ProcessArgs`). Implemented by the
/// client's `StartupOptions` port.
pub trait StartupArgsProcessor {
    fn process_args(&mut self, rcstartup_flags: &[RcStartupFlag])
    -> Result<(), (ExitCode, String)>;
}

/// A processor that only records the flags (no validation of values).
#[derive(Default, Debug)]
pub struct RecordingStartupArgsProcessor {
    pub flags: Vec<RcStartupFlag>,
}

impl StartupArgsProcessor for RecordingStartupArgsProcessor {
    fn process_args(
        &mut self,
        rcstartup_flags: &[RcStartupFlag],
    ) -> Result<(), (ExitCode, String)> {
        self.flags = rcstartup_flags.to_vec();
        Ok(())
    }
}

// Java: src/main/cpp/blaze_util.cc:53 GetUnaryOption
/// The value of `key` in `arg` (`--key=value`) or `next_arg` (`--key value`), else `None`.
/// The second element is true when the value came from `next_arg`.
pub fn get_unary_option<'a>(arg: &'a str, next_arg: &'a str, key: &str) -> Option<(&'a str, bool)> {
    let value = arg.strip_prefix(key)?;
    if let Some(v) = value.strip_prefix('=') {
        Some((v, false))
    } else if !value.is_empty() {
        None // trailing garbage in key name
    } else {
        Some((next_arg, true))
    }
}

// Java: src/main/cpp/blaze_util.cc:67 GetNullaryOption
/// Whether `arg` is exactly `key`. A value (`key=...`) is a fatal BAD_ARGV error in the client.
pub fn get_nullary_option(arg: &str, key: &str) -> Result<bool, String> {
    let Some(value) = arg.strip_prefix(key) else {
        return Ok(false);
    };
    if value.starts_with('=') {
        return Err(format!(
            "In argument '{}': option '{}' does not take a value.",
            arg, key
        ));
    }
    Ok(value.is_empty())
}

// Java: src/main/cpp/blaze_util.cc:82 GetAllUnaryOptionValues
pub fn get_all_unary_option_values(
    args: &[String],
    key: &str,
    ignore_after_value: Option<&str>,
) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            return values;
        }
        let next_arg = &args[(i + 1).min(args.len() - 1)];
        let result = get_unary_option(&args[i], next_arg, key);
        if let Some((value, _)) = result {
            values.push(value.to_string());
            if ignore_after_value == Some(value) {
                break;
            }
        }
        if let Some((_, true)) = result {
            i += 1;
        }
        i += 1;
    }
    values
}

// Java: src/main/cpp/blaze_util.cc:116 SearchNullaryOption
pub fn search_nullary_option(
    args: &[String],
    flag_name: &str,
    default_value: bool,
) -> Result<bool, String> {
    let positive_flag = format!("--{}", flag_name);
    let negative_flag = format!("--no{}", flag_name);
    let mut result = default_value;
    for arg in args {
        if arg == "--" {
            break;
        }
        if get_nullary_option(arg, &positive_flag)? {
            result = true;
        } else if get_nullary_option(arg, &negative_flag)? {
            result = false;
        }
    }
    Ok(result)
}

// Java: src/main/cpp/blaze_util.cc:134 IsArg
pub fn is_arg(arg: &str) -> bool {
    arg.starts_with('-') && arg != "--help" && arg != "-help" && arg != "-h"
}

/// The parts of the client environment that rc processing reads.
#[derive(Clone, Debug, Default)]
pub struct ClientEnvironment {
    /// `blaze_util::GetCwd()`.
    pub cwd: String,
    /// `$HOME` (`GetHomeDir`); empty if unset.
    pub home: String,
    /// `$BAZELRC`.
    pub bazelrc_env: String,
    /// Environment used by `${VAR}` expansion in the system rc path.
    pub env: HashMap<String, String>,
    /// `IsStandardTerminal()`.
    pub is_standard_terminal: bool,
    /// `GetTerminalColumns()`.
    pub terminal_columns: i32,
    /// `IsEmacsTerminal()`.
    pub is_emacs_terminal: bool,
    /// `blaze::GetProcessedEnv()`: `NAME=value` strings passed to the server.
    pub processed_env: Vec<String>,
}

impl ClientEnvironment {
    // Java: src/main/cpp/blaze_util.cc:139 AbsolutePathFromFlag
    pub fn absolute_path_from_flag(&self, value: &str) -> String {
        if value.is_empty() {
            self.cwd.clone()
        } else if let Some(rest) = value.strip_prefix('~') {
            join_path(&self.home, rest)
        } else {
            self.make_absolute(value)
        }
    }

    // Java: src/main/cpp/util/path_posix.cc:67 blaze_util::MakeAbsolute
    pub fn make_absolute(&self, path: &str) -> String {
        if path.starts_with('/') || path.is_empty() {
            return path.to_string();
        }
        join_path(&self.cwd, path)
    }

    // Java: src/main/cpp/util/path_posix.cc:75 blaze_util::ResolveEnvvars
    pub fn resolve_envvars(&self, path: &str) -> Result<String, String> {
        let mut result = path.to_string();
        let mut start = 0;
        while let Some(s) = result[start..].find("${").map(|i| i + start) {
            let Some(end) = result[s + 1..].find('}').map(|i| i + s + 1) else {
                return Err(format!(
                    "ResolveEnvvars({}): incomplete variable at position {}",
                    path, s
                ));
            };
            let name = result[s + 2..end].to_string();
            let value = self.env.get(&name).cloned().unwrap_or_default();
            result.replace_range(s..=end, &value);
            start = s + value.len();
        }
        Ok(result)
    }

    // Java: src/main/cpp/util/path_posix.cc:97 blaze_util::MakeAbsoluteAndResolveEnvvars
    pub fn make_absolute_and_resolve_envvars(&self, path: &str) -> Result<String, String> {
        Ok(self.make_absolute(&self.resolve_envvars(path)?))
    }
}

// Java: <unistd.h> R_OK
const R_OK: std::ffi::c_int = 4;

// Java: src/main/cpp/util/file_posix.cc:389 blaze_util::CanAccess
/// `access(path, mode) == 0`. The file is never opened (opening a FIFO would block).
// PORT: blaze_options does not depend on the `libc` crate, so access(2) is declared here (std
// links the C library). CanAccess(path, read, write, exec) builds the mode from three flags;
// this takes the mode itself, and its only caller needs R_OK.
fn can_access(path: &str, mode: std::ffi::c_int) -> bool {
    unsafe extern "C" {
        fn access(pathname: *const std::ffi::c_char, mode: std::ffi::c_int) -> std::ffi::c_int;
    }
    // PORT: C++ passes path.c_str(), which stops at an embedded NUL; such a path is reported as
    // not accessible here.
    let Ok(c_path) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: c_path is NUL-terminated and outlives the call.
    unsafe { access(c_path.as_ptr(), mode) == 0 }
}

// Java: src/main/cpp/util/file_posix.cc:403 blaze_util::CanReadFile
pub fn can_read_file(path: &str) -> bool {
    // Java: src/main/cpp/util/file_posix.cc:427 blaze_util::IsDirectory
    // `Path::is_dir` is stat(2) and S_ISDIR, false when stat fails.
    !std::path::Path::new(path).is_dir() && can_access(path, R_OK)
}

// Java: src/main/cpp/option_processor.cc:245 internal::DedupeBlazercPaths
/// Dedupes paths by canonical form, dropping paths that cannot be canonicalized.
pub fn dedupe_blazerc_paths(paths: &[String]) -> Vec<String> {
    let mut canonical_paths = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let canonical_path = make_canonical(path);
        if canonical_path.is_empty() {
            // Blazerc paths may point to invalid locations.
        } else if canonical_paths.insert(canonical_path) {
            result.push(path.clone());
        }
    }
    result
}

// Java: src/main/cpp/option_processor.cc:176 internal::FindLegacyUserBazelrc
pub fn find_legacy_user_bazelrc(
    env: &ClientEnvironment,
    cmd_line_rc_file: Option<&str>,
    workspace: &str,
) -> String {
    if let Some(f) = cmd_line_rc_file {
        let rc_file = env.absolute_path_from_flag(f);
        if !can_read_file(&rc_file) {
            return String::new();
        }
        return rc_file;
    }
    let workspace_rc_file = join_path(workspace, RC_BASENAME);
    if can_read_file(&workspace_rc_file) {
        return workspace_rc_file;
    }
    if !env.home.is_empty() {
        let user_rc_file = join_path(&env.home, RC_BASENAME);
        if can_read_file(&user_rc_file) {
            return user_rc_file;
        }
    }
    String::new()
}

// Java: src/main/cpp/option_processor.cc:274 internal::FindRcAlongsideBinary
pub fn find_rc_alongside_binary(cwd: &str, path_to_binary: &str) -> String {
    let path = if path_to_binary.starts_with('/') {
        path_to_binary.to_string()
    } else {
        join_path(cwd, path_to_binary)
    };
    let base = path_to_binary.rsplit('/').next().unwrap_or("");
    let binary_blazerc_path = format!("{}.{}rc", path, base);
    if can_read_file(&binary_blazerc_path) {
        binary_blazerc_path
    } else {
        String::new()
    }
}

// Java: src/main/cpp/option_processor.cc:265 internal::FindSystemWideRc
pub fn find_system_wide_rc(env: &ClientEnvironment, system_bazelrc_path: &str) -> String {
    let path = env
        .make_absolute_and_resolve_envvars(system_bazelrc_path)
        .unwrap_or_default();
    if can_read_file(&path) {
        path
    } else {
        String::new()
    }
}

// Java: src/main/cpp/workspace_layout.cc:57 WorkspaceLayout::GetWorkspaceRcPath
pub fn get_workspace_rc_path(workspace: &str) -> String {
    join_path(workspace, "tools/bazel.rc")
}

// Java: src/main/cpp/option_processor.cc:203 internal::GetOldRcPaths
pub fn get_old_rc_paths(
    env: &ClientEnvironment,
    workspace: &str,
    cwd: &str,
    path_to_binary: &str,
    startup_args: &[String],
    system_bazelrc_path: &str,
) -> Result<BTreeSet<String>, String> {
    let mut candidate_bazelrc_paths = Vec::new();
    if search_nullary_option(startup_args, "master_bazelrc", true)? {
        let workspace_rc = get_workspace_rc_path(workspace);
        let binary_rc = find_rc_alongside_binary(cwd, path_to_binary);
        candidate_bazelrc_paths = vec![workspace_rc, binary_rc, system_bazelrc_path.to_string()];
    }
    let cmd_line_rc_files =
        get_all_unary_option_values(startup_args, "--bazelrc", Some("/dev/null"));
    if cmd_line_rc_files.is_empty() {
        let user_bazelrc_path = find_legacy_user_bazelrc(env, None, workspace);
        if !user_bazelrc_path.is_empty() {
            candidate_bazelrc_paths.push(user_bazelrc_path);
        }
    } else {
        for rc_file in &cmd_line_rc_files {
            let user_bazelrc_path = find_legacy_user_bazelrc(env, Some(rc_file), workspace);
            if !user_bazelrc_path.is_empty() {
                candidate_bazelrc_paths.push(user_bazelrc_path);
            }
        }
    }
    Ok(dedupe_blazerc_paths(&candidate_bazelrc_paths)
        .into_iter()
        .collect())
}

// Java: src/main/cpp/option_processor.cc:287 internal::ParseErrorToExitCode
pub fn parse_error_to_exit_code(parse_error: ParseError) -> ExitCode {
    match parse_error {
        ParseError::None => ExitCode::Success,
        ParseError::UnreadableFile => ExitCode::InternalError,
        ParseError::InvalidFormat | ParseError::ImportLoop => ExitCode::BadArgv,
    }
}

// Java: src/main/cpp/option_processor.cc:307 internal::WarnAboutDuplicateRcFiles
/// Returns the warnings the client logs (BAZEL_LOG(WARNING)).
pub fn warn_about_duplicate_rc_files(
    read_files: &BTreeSet<String>,
    loaded_rcs: &[String],
) -> Vec<String> {
    let mut warnings = Vec::new();
    let top_level_rc = &loaded_rcs[0];
    let unique_loaded_rcs: BTreeSet<&String> = loaded_rcs.iter().collect();
    for loaded_rc in unique_loaded_rcs {
        if read_files.contains(loaded_rc) {
            if loaded_rc == top_level_rc {
                warnings.push(format!(
                    "Duplicate rc file: {} is read multiple times, it is a standard rc file \
                     location but must have been unnecessarily imported earlier.",
                    loaded_rc
                ));
            } else {
                warnings.push(format!(
                    "Duplicate rc file: {} is read multiple times, most recently imported from {}",
                    loaded_rc, top_level_rc
                ));
            }
        }
        if loaded_rcs.iter().filter(|r| *r == loaded_rc).count() > 1 {
            warnings.push(format!(
                "Duplicate rc file: {} is imported multiple times from {}",
                loaded_rc, top_level_rc
            ));
        }
    }
    warnings
}

// Java: src/main/cpp/option_processor.cc:342 internal::GetLostFiles
pub fn get_lost_files(
    old_files: &BTreeSet<String>,
    read_files_canon: &BTreeSet<String>,
) -> Vec<String> {
    old_files
        .iter()
        .filter(|old| {
            let old_canon = make_canonical(old);
            !old_canon.is_empty() && !read_files_canon.contains(&old_canon)
        })
        .cloned()
        .collect()
}

// Java: src/main/cpp/option_processor.cc:356 internal::GetAndValidateExistingRcFiles
/// Appends the absolute forms of `input_rc_files`; `Err(log message)` if one is unreadable.
pub fn get_and_validate_existing_rc_files(
    env: &ClientEnvironment,
    input_rc_files: &[String],
    result_rc_files: &mut Vec<String>,
) -> Result<(), String> {
    for rc_file in input_rc_files {
        let absolute_rc = env.absolute_path_from_flag(rc_file);
        if !can_read_file(&absolute_rc) {
            return Err(format!(
                "Error: Unable to read .bazelrc file '{}'.",
                absolute_rc
            ));
        }
        result_rc_files.push(absolute_rc);
    }
    Ok(())
}

// Java: src/main/cpp/option_processor.cc:492 ParseRcFile
pub fn parse_rc_file(
    workspace: &str,
    rc_file_path: &str,
    build_label: &str,
    sem_ver: Option<&SemVer>,
) -> Result<RcFile, (ExitCode, String)> {
    assert!(!rc_file_path.is_empty());
    RcFile::parse(
        rc_file_path,
        workspace,
        build_label,
        sem_ver,
        &read_file_default,
        &make_canonical,
    )
    .map_err(|(e, text)| (parse_error_to_exit_code(e), text))
}

// Java: src/main/cpp/option_processor.h:61 OptionProcessor
/// Splits the client command line, reads the rc files and builds the server command arguments.
pub struct OptionProcessor {
    startup_flags: StartupFlagRegistry,
    system_bazelrc_path: String,
    build_label: String,
    env: ClientEnvironment,
    cmd_line: Option<CommandLine>,
    parsed_blazercs: Vec<BlazercOption>,
    blazerc_and_env_command_args: Vec<String>,
    rcstartup_flags: Vec<RcStartupFlag>,
    /// Messages the client logs with BAZEL_LOG(WARNING), in order.
    warnings: Vec<String>,
}

impl OptionProcessor {
    // Java: src/main/cpp/option_processor.cc:67 OptionProcessor::OptionProcessor
    pub fn new(
        startup_flags: StartupFlagRegistry,
        system_bazelrc_path: &str,
        build_label: &str,
        env: ClientEnvironment,
    ) -> Self {
        OptionProcessor {
            startup_flags,
            system_bazelrc_path: system_bazelrc_path.to_string(),
            build_label: build_label.to_string(),
            env,
            cmd_line: None,
            parsed_blazercs: Vec::new(),
            blazerc_and_env_command_args: Vec::new(),
            rcstartup_flags: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    // Java: src/main/cpp/option_processor.cc:80 OptionProcessor::SplitCommandLine
    pub fn split_command_line(&self, mut args: Vec<String>) -> Result<CommandLine, String> {
        let lowercase_product_name = self.startup_flags.get_lowercase_product_name();
        if args.is_empty() {
            return Err("Unable to split command line, args is empty".to_string());
        }
        let path_to_binary = args[0].clone();
        let mut startup_args = Vec::new();
        let mut i = 1;
        while i < args.len() && is_arg(&args[i]) {
            let current_arg = args[i].clone();
            let is_nullary = self.startup_flags.maybe_check_valid_nullary(&current_arg)?;
            if is_nullary {
                startup_args.push(current_arg);
                i += 1;
            } else if self.startup_flags.is_unary(&current_arg) {
                if current_arg.contains('=') {
                    startup_args.push(current_arg);
                    i += 1;
                } else {
                    if i + 1 >= args.len() {
                        return Err(format!(
                            "Startup option '{0}' expects a value.\nUsage: '{0}=somevalue' or \
                             '{0} somevalue'.\n  For more info, run '{1} help startup_options'.",
                            current_arg, lowercase_product_name
                        ));
                    }
                    startup_args.push(format!("{}={}", current_arg, args[i + 1]));
                    i += 2;
                }
            } else {
                return Err(format!(
                    "Unknown startup option: '{}'.\n  For more info, run '{} help \
                     startup_options'.",
                    current_arg, lowercase_product_name
                ));
            }
        }
        if i == args.len() {
            return Ok(CommandLine {
                path_to_binary,
                startup_args,
                command: String::new(),
                command_args: vec![],
            });
        }
        let command = std::mem::take(&mut args[i]);
        if command.is_empty() {
            return Err(format!(
                "Command cannot be the empty string.\n  For more info, run '{} help'.",
                lowercase_product_name
            ));
        }
        let command_args = args.split_off(i + 1);
        Ok(CommandLine {
            path_to_binary,
            startup_args,
            command,
            command_args,
        })
    }

    // Java: src/main/cpp/option_processor.cc:374 OptionProcessor::GetRcFiles
    /// The rc files in priority order: system rc, workspace `.bazelrc`, `$HOME/.bazelrc`,
    /// `$BAZELRC` (comma separated), `--bazelrc` flags; deduplicated, each parsed with imports.
    pub fn get_rc_files(
        &mut self,
        workspace: &str,
        cwd: &str,
        cmd_line: &CommandLine,
    ) -> Result<Vec<RcFile>, (ExitCode, String)> {
        let bad = |e: String| (ExitCode::BadArgv, e);
        let mut rc_files: Vec<String> = Vec::new();

        // Get the system rc (unless --nosystem_rc).
        if search_nullary_option(&cmd_line.startup_args, "system_rc", true).map_err(bad)? {
            let system_rc = self
                .env
                .make_absolute_and_resolve_envvars(&self.system_bazelrc_path)
                .map_err(|e| (ExitCode::LocalEnvironmentalError, e))?;
            rc_files.push(system_rc);
        }

        // Get the workspace rc: %workspace%/.bazelrc (unless --noworkspace_rc), but only if we
        // are in a workspace.
        if !workspace.is_empty()
            && search_nullary_option(&cmd_line.startup_args, "workspace_rc", true).map_err(bad)?
        {
            rc_files.push(join_path(workspace, RC_BASENAME));
        }

        // Get the user rc: $HOME/.bazelrc (unless --nohome_rc)
        if search_nullary_option(&cmd_line.startup_args, "home_rc", true).map_err(bad)?
            && !self.env.home.is_empty()
        {
            rc_files.push(join_path(&self.env.home, RC_BASENAME));
        }

        // Get the env var provided rc.
        let env_var_rc_files = split(&self.env.bazelrc_env, ',');
        if let Err(e) =
            get_and_validate_existing_rc_files(&self.env, &env_var_rc_files, &mut rc_files)
        {
            return Err((ExitCode::BadArgv, e));
        }

        // Get the command-line provided rc, passed as --bazelrc or nothing if the flag is absent.
        let cmd_line_rc_files =
            get_all_unary_option_values(&cmd_line.startup_args, "--bazelrc", Some("/dev/null"));
        if let Err(e) =
            get_and_validate_existing_rc_files(&self.env, &cmd_line_rc_files, &mut rc_files)
        {
            return Err((ExitCode::BadArgv, e));
        }

        // BAZEL_LOG(INFO) << "Looking for the following rc files: " ...
        let rc_files = dedupe_blazerc_paths(&rc_files);

        let sem_ver = SemVer::parse(&self.build_label);
        let mut read_files_canonical_paths: BTreeSet<String> = BTreeSet::new();
        let mut result = Vec::new();
        for top_level_bazelrc_path in &rc_files {
            let parsed_rc = parse_rc_file(
                workspace,
                top_level_bazelrc_path,
                &self.build_label,
                sem_ver.as_ref(),
            )?;
            let sources = parsed_rc.canonical_source_paths();
            self.warnings.extend(warn_about_duplicate_rc_files(
                &read_files_canonical_paths,
                sources,
            ));
            read_files_canonical_paths.extend(sources.iter().cloned());
            result.push(parsed_rc);
        }

        let old_files = get_old_rc_paths(
            &self.env,
            workspace,
            cwd,
            &cmd_line.path_to_binary,
            &cmd_line.startup_args,
            &find_system_wide_rc(&self.env, &self.system_bazelrc_path),
        )
        .map_err(bad)?;
        let lost_files = get_lost_files(&old_files, &read_files_canonical_paths);
        if !lost_files.is_empty() {
            self.warnings.push(format!(
                "The following rc files are no longer being read, please transfer their contents \
                 or import their path into one of the standard rc files:\n{}",
                lost_files.join("\n")
            ));
        }
        Ok(result)
    }

    // Java: src/main/cpp/option_processor.cc:523 OptionProcessor::ParseOptions
    pub fn parse_options(
        &mut self,
        args: Vec<String>,
        workspace: &str,
        cwd: &str,
        startup_options: &mut dyn StartupArgsProcessor,
    ) -> Result<(), (ExitCode, String)> {
        assert!(self.cmd_line.is_none());
        let cmd_line = self
            .split_command_line(args)
            .map_err(|e| (ExitCode::BadArgv, e))?;

        let mut rc_files = Vec::new();
        if !search_nullary_option(&cmd_line.startup_args, "ignore_all_rc_files", false)
            .map_err(|e| (ExitCode::BadArgv, e))?
        {
            rc_files = self.get_rc_files(workspace, cwd, &cmd_line)?;
        }
        self.cmd_line = Some(cmd_line);

        self.parse_startup_options(&rc_files, startup_options)?;

        self.parsed_blazercs = self.get_blazerc_options(cwd, &rc_files);
        let env = self.env.processed_env.clone();
        self.blazerc_and_env_command_args =
            Self::get_blazerc_and_env_command_args(cwd, &self.parsed_blazercs, &env);
        Ok(())
    }

    // Java: src/main/cpp/option_processor.cc:603 OptionProcessor::ParseStartupOptions
    fn parse_startup_options(
        &mut self,
        rc_files: &[RcFile],
        startup_options: &mut dyn StartupArgsProcessor,
    ) -> Result<(), (ExitCode, String)> {
        let mut rcstartup_flags = Vec::new();
        for blazerc in rc_files {
            if let Some(opts) = blazerc.options().get("startup") {
                for option in opts {
                    rcstartup_flags.push(RcStartupFlag {
                        source: blazerc.canonical_source_paths()[option.source_index].clone(),
                        value: option.option.clone(),
                    });
                }
            }
        }
        // Linux only: "startup:linux".
        let platform_config = "linux";
        for blazerc in rc_files {
            if let Some(opts) = blazerc
                .options()
                .get(&format!("startup:{}", platform_config))
            {
                for option in opts {
                    rcstartup_flags.push(RcStartupFlag {
                        source: blazerc.canonical_source_paths()[option.source_index].clone(),
                        value: option.option.clone(),
                    });
                }
            }
        }
        for arg in &self.cmd_line.as_ref().unwrap().startup_args {
            if !is_arg(arg) {
                break;
            }
            rcstartup_flags.push(RcStartupFlag {
                source: String::new(),
                value: arg.clone(),
            });
        }
        self.rcstartup_flags = rcstartup_flags;
        startup_options.process_args(&self.rcstartup_flags)
    }

    /// The startup flags in processing order (rc files, `startup:linux`, command line).
    pub fn rc_startup_flags(&self) -> &[RcStartupFlag] {
        &self.rcstartup_flags
    }

    // Java: src/main/cpp/option_processor.cc:658 OptionProcessor::GetBlazercOptions
    pub fn get_blazerc_options(&self, _cwd: &str, blazercs: &[RcFile]) -> Vec<BlazercOption> {
        let mut result = Vec::new();
        // Provide terminal options as least important options.
        let terminal_rc_source = "client";
        let opt = |c: &str, o: String| BlazercOption {
            source_path: terminal_rc_source.to_string(),
            command: c.to_string(),
            option: o,
        };
        result.push(opt(
            "common",
            format!("--isatty={}", self.env.is_standard_terminal as i32),
        ));
        result.push(opt(
            "common",
            format!("--terminal_columns={}", self.env.terminal_columns),
        ));
        if self.env.is_emacs_terminal {
            result.push(opt("common", "--emacs".to_string()));
        }
        for blazerc in blazercs {
            for (command, options) in blazerc.options() {
                // Skip startup flags, which are already parsed by the client.
                if command == "startup" {
                    continue;
                }
                for rcoption in options {
                    result.push(BlazercOption {
                        source_path: blazerc.canonical_source_paths()[rcoption.source_index]
                            .clone(),
                        command: command.clone(),
                        option: rcoption.option.clone(),
                    });
                }
            }
        }
        result
    }

    // Java: src/main/cpp/option_processor.cc:696 OptionProcessor::GetBlazercAndEnvCommandArgs
    pub fn get_blazerc_and_env_command_args(
        cwd: &str,
        blazerc_options: &[BlazercOption],
        env: &[String],
    ) -> Vec<String> {
        let mut result = vec!["--rc_source=client".to_string()];
        let mut rcfile_index: HashMap<String, usize> = HashMap::new();
        rcfile_index.insert("client".to_string(), 0);
        let mut cur_index = 1;
        for option in blazerc_options {
            if rcfile_index.contains_key(&option.source_path) {
                continue;
            }
            rcfile_index.insert(option.source_path.clone(), cur_index);
            result.push(format!("--rc_source={}", option.source_path));
            cur_index += 1;
        }
        for option in blazerc_options {
            result.push(format!(
                "--default_override={}:{}={}",
                rcfile_index[&option.source_path], option.command, option.option
            ));
        }
        for env_var in env {
            result.push(format!("--client_env={}", env_var));
        }
        result.push(format!("--client_cwd={}", cwd));
        result
    }

    // Java: src/main/cpp/option_processor.cc:731 OptionProcessor::GetCommandArguments
    pub fn get_command_arguments(&self) -> Vec<String> {
        let cmd_line = self.cmd_line.as_ref().expect("ParseOptions not called");
        if cmd_line.command.is_empty() {
            return vec![];
        }
        let mut command_args = self.blazerc_and_env_command_args.clone();
        command_args.extend(cmd_line.command_args.iter().cloned());
        command_args
    }

    // Java: src/main/cpp/option_processor.cc:746 OptionProcessor::GetExplicitCommandArguments
    pub fn get_explicit_command_arguments(&self) -> Vec<String> {
        self.cmd_line
            .as_ref()
            .expect("ParseOptions not called")
            .command_args
            .clone()
    }

    // Java: src/main/cpp/option_processor.cc:751 OptionProcessor::GetCommand
    pub fn get_command(&self) -> String {
        self.cmd_line
            .as_ref()
            .expect("ParseOptions not called")
            .command
            .clone()
    }

    // Java: src/main/cpp/option_processor.cc:761 OptionProcessor::GetParsedBlazercOptions
    pub fn get_parsed_blazerc_options(&self) -> &[BlazercOption] {
        &self.parsed_blazercs
    }

    pub fn get_cmd_line(&self) -> Option<&CommandLine> {
        self.cmd_line.as_ref()
    }
}
