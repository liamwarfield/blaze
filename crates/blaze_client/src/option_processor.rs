//! The parts of `src/main/cpp/option_processor.cc` that `blaze_options::rc` does not port:
//! building the client environment, `PrintStartupOptionsProvenanceMessage`, and the log
//! statements `OptionProcessor::ParseOptions` emits (blaze_options returns warnings instead of
//! logging them).
//!
//! PORT: move to blaze_options (everything in this module).

use blaze_options::rc::option_processor::{
    ClientEnvironment, CommandLine, ExitCode, OptionProcessor, RC_BASENAME, can_read_file,
    get_all_unary_option_values, search_nullary_option,
};
use blaze_options::rc::tokenize::{join_path, split};

use crate::platform::{
    get_env, get_home_dir, get_processed_env, get_terminal_columns, is_emacs_terminal,
    is_standard_terminal,
};
use crate::startup_options::StartupOptions;
use crate::util::file::get_cwd;
use crate::util::logging::{LogLevel, bazel_log};

const OPTION_PROCESSOR_CC: &str = "src/main/cpp/option_processor.cc";

/// The environment `OptionProcessor` reads, taken from this process (C++ reads it directly).
pub fn client_environment() -> ClientEnvironment {
    let processed_env = get_processed_env();
    let mut env = std::collections::HashMap::new();
    for var_value in &processed_env {
        if let Some(eq) = var_value.find('=') {
            // getenv returns the first definition.
            env.entry(var_value[..eq].to_string())
                .or_insert_with(|| var_value[eq + 1..].to_string());
        }
    }
    ClientEnvironment {
        cwd: get_cwd(),
        home: get_home_dir(),
        bazelrc_env: get_env("BAZELRC"),
        env,
        is_standard_terminal: is_standard_terminal(),
        terminal_columns: get_terminal_columns(),
        is_emacs_terminal: is_emacs_terminal(),
        processed_env,
    }
}

// Java: src/main/cpp/option_processor.cc:566 PrintStartupOptions
fn print_startup_options(source: &str, options: &[String]) {
    if !source.is_empty() {
        let startup_args = options.join(" ");
        eprintln!(
            "INFO: Reading 'startup' options from {}: {}",
            source, startup_args
        );
    }
}

// Java: src/main/cpp/option_processor.cc:576 OptionProcessor::PrintStartupOptionsProvenanceMessage
/// Prints the startup flags in the order they are parsed, grouped by origin.
pub fn print_startup_options_provenance_message(parsed_startup_options: &StartupOptions) {
    // Print the startup flags in the order they are parsed, to keep the precedence clear. In
    // order to minimize the number of lines of output in the terminal, group sequential flags by
    // origin. Note that an rc file may turn up multiple times in this list, if, for example, it
    // imports another rc file and contains startup options on either side of the import
    // statement. This is done intentionally to make option priority clear.
    let mut most_recent_blazerc = String::new();
    let mut accumulated_options: Vec<String> = Vec::new();
    for flag in &parsed_startup_options.original_startup_options_ {
        if flag.source == most_recent_blazerc {
            accumulated_options.push(flag.value.clone());
        } else {
            print_startup_options(&most_recent_blazerc, &accumulated_options);
            // Start accumulating again.
            accumulated_options.clear();
            accumulated_options.push(flag.value.clone());
            most_recent_blazerc = flag.source.clone();
        }
    }
    // Don't forget to print out the last ones.
    print_startup_options(&most_recent_blazerc, &accumulated_options);
}

// Java: src/main/cpp/option_processor.cc:374 OptionProcessor::GetRcFiles
/// The list `GetRcFiles` logs as "Looking for the following rc files: ...", or `None` when
/// `GetRcFiles` fails before logging it (an unreadable `$BAZELRC` or `--bazelrc` file).
fn rc_files_looked_for(
    env: &ClientEnvironment,
    system_bazelrc_path: &str,
    workspace: &str,
    cmd_line: &CommandLine,
) -> Option<Vec<String>> {
    let mut rc_files: Vec<String> = Vec::new();
    // Get the system rc (unless --nosystem_rc).
    if search_nullary_option(&cmd_line.startup_args, "system_rc", true).ok()? {
        rc_files.push(
            env.make_absolute_and_resolve_envvars(system_bazelrc_path)
                .ok()?,
        );
    }
    // Get the workspace rc: %workspace%/.bazelrc (unless --noworkspace_rc), but only if we are
    // in a workspace.
    if !workspace.is_empty()
        && search_nullary_option(&cmd_line.startup_args, "workspace_rc", true).ok()?
    {
        rc_files.push(join_path(workspace, RC_BASENAME));
    }
    // Get the user rc: $HOME/.bazelrc (unless --nohome_rc)
    if search_nullary_option(&cmd_line.startup_args, "home_rc", true).ok()? && !env.home.is_empty()
    {
        rc_files.push(join_path(&env.home, RC_BASENAME));
    }
    // Get the env var provided rc, then the command-line provided rc.
    let env_var_rc_files = split(&env.bazelrc_env, ',');
    let cmd_line_rc_files =
        get_all_unary_option_values(&cmd_line.startup_args, "--bazelrc", Some("/dev/null"));
    for rc_file in env_var_rc_files.iter().chain(cmd_line_rc_files.iter()) {
        let absolute_rc = env.absolute_path_from_flag(rc_file);
        if !can_read_file(&absolute_rc) {
            return None;
        }
        rc_files.push(absolute_rc);
    }
    Some(rc_files)
}

/// Where `blaze_options`' `OptionProcessor` returns a warning, the C++ logs it with
/// BAZEL_LOG(WARNING) at one of these lines.
fn warning_line(warning: &str) -> u32 {
    if warning.starts_with("Duplicate rc file: ") {
        if warning.contains(" is read multiple times, it is a standard rc file location") {
            321
        } else if warning.contains(" is read multiple times, most recently imported from ") {
            326
        } else {
            336
        }
    } else {
        // "The following rc files are no longer being read, ..."
        482
    }
}

/// `OptionProcessor::ParseOptions` with the log statements the C++ emits while parsing.
/// Returns the C++ `(exit_code, error)` on failure.
pub fn parse_options_with_logging(
    option_processor: &mut OptionProcessor,
    env: &ClientEnvironment,
    system_bazelrc_path: &str,
    args: Vec<String>,
    workspace: &str,
    cwd: &str,
    startup_options: &mut StartupOptions,
) -> Result<(), (i32, String)> {
    let split = option_processor.split_command_line(args.clone()).ok();
    let result = option_processor.parse_options(args, workspace, cwd, startup_options);

    // Replay the logs of GetRcFiles, in the C++ order.
    if let Some(cmd_line) = &split {
        let ignore_all_rc_files =
            search_nullary_option(&cmd_line.startup_args, "ignore_all_rc_files", false)
                .unwrap_or(false);
        if !ignore_all_rc_files {
            if let Err((ExitCode::BadArgv, error)) = &result
                && error.starts_with("Error: Unable to read .bazelrc file '")
            {
                // Java: src/main/cpp/option_processor.cc:362 GetAndValidateExistingRcFiles
                bazel_log(LogLevel::Error, OPTION_PROCESSOR_CC, 362, error);
                // GetRcFiles returns BAD_ARGV and leaves `error` empty.
                return Err((ExitCode::BadArgv as i32, String::new()));
            }
            if let Some(rc_files) =
                rc_files_looked_for(env, system_bazelrc_path, workspace, cmd_line)
            {
                bazel_log(
                    LogLevel::Info,
                    OPTION_PROCESSOR_CC,
                    437,
                    format!("Looking for the following rc files: {}", rc_files.join(",")),
                );
            }
            // PORT: rc_file.cc's INFO logs ("Parsing the RcFile ...", optional import skips)
            // are not emitted: blaze_options' RcFile does not report them.
            for warning in option_processor.warnings() {
                bazel_log(
                    LogLevel::Warning,
                    OPTION_PROCESSOR_CC,
                    warning_line(warning),
                    warning,
                );
            }
        }
    }
    result.map_err(|(code, error)| (code as i32, error))
}
