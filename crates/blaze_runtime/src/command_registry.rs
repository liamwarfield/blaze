//! The `@Command` annotations of every command a Bazel 9.3.2 server registers, stub
//! [`BlazeCommand`]s for the commands that are not ported yet, and `BlazeCommandUtils.getOptions`.
//!
//! Each `<NAME>_COMMAND` static transcribes the `@Command` annotation on the Java class of that
//! command. The commands are registered by the modules in `crate::bazel_modules`
//! (`BazelBuiltinCommandModule`, `MobileInstallModule`, `BazelRepositoryModule`).

use std::sync::Arc;

use blaze_options::OptionsClass;
use blaze_options::definition::OptionsClassRef;
use blaze_options::rc::handler::Command as RcCommand;

use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::canonicalize_command::CanonicalizeCommandOptions;
use blaze_optdefs::command::clean_command::CleanCommandOptions;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_optdefs::command::dump_command::DumpCommandDumpOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::fetch_options::FetchOptions;
use blaze_optdefs::command::help_command::HelpCommandOptions;
use blaze_optdefs::command::info_command::InfoCommandOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::mobile_install_command::MobileInstallCommandOptions;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::command::print_action_command::PrintActionCommandPrintActionOptions;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_optdefs::command::remote_analysis_caching_options::RemoteAnalysisCachingOptions;
use blaze_optdefs::command::run_command::RunCommandRunOptions;
use blaze_optdefs::command::shutdown_command::ShutdownCommandOptions;
use blaze_optdefs::command::skyfocus_options::SkyfocusOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::command::vendor_options::VendorOptions;
use blaze_optdefs::command::version_command::VersionCommandVersionOptions;
use blaze_optdefs::command::write_adb_args_action::WriteAdbArgsActionOptions;
use blaze_optdefs::fragment::config_command::ConfigCommandConfigOptions;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::mod_options::ModOptions;

use crate::command::{BlazeCommand, BlazeCommandResult, BuildPhase, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::module::BlazeModule;
use crate::options_result::OptionsParsingResult;

// ---------------------------------------------------------------------------------------------
// BlazeCommandUtils
// ---------------------------------------------------------------------------------------------

/// The set of option-classes that are common to all Blaze commands.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:37 BlazeCommandUtils.COMMON_COMMAND_OPTIONS
fn common_command_options() -> Vec<OptionsClassRef> {
    vec![
        UiOptions::class(),
        CommonCommandOptions::class(),
        // PORT: Java's runtime `ClientOptions` is the class the rc handler reads, which Blaze
        // keeps in blaze_options.
        blaze_options::rc::client_options::ClientOptions::class(),
        // Starlark options aren't applicable to all commands, but making them a common option
        // allows users to put them in the common section of the bazelrc. See issue #3538.
        BuildLanguageOptions::class(),
    ]
}

/// Adds `class` to `options` unless it is already there (identity, as Java compares `Class`
/// objects).
fn add_option_class(options: &mut Vec<OptionsClassRef>, class: OptionsClassRef) {
    if !options.iter().any(|c| std::ptr::eq(*c, class)) {
        options.push(class);
    }
}

/// The common options of all commands: Blaze core's plus every module's
/// `getCommonCommandOptions`, in insertion order (Java: `ImmutableSet`).
// Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:58 BlazeCommandUtils.getCommonOptions
pub fn get_common_options(suppliers: &[Arc<dyn BlazeModule>]) -> Vec<OptionsClassRef> {
    let mut builder = Vec::new();
    for class in common_command_options() {
        add_option_class(&mut builder, class);
    }
    for supplier in suppliers {
        for class in supplier.get_common_command_options() {
            add_option_class(&mut builder, class);
        }
    }
    builder
}

/// Returns the set of all options (including those inherited directly and transitively) for this
/// command's `@Command` annotation.
///
/// PORT: java-hash-order. Java collects the classes in a `HashSet` and returns them in hash
/// order; Blaze keeps the first-seen order. The order only affects which class an
/// `OptionsParser` lists first, which is not observable for the W1 commands.
/// PORT: the rule class provider argument is replaced by
/// [`crate::fragment_options::fragment_options_classes`] (Bazel's fragment registry, from `blaze_build_config`).
// Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:74 BlazeCommandUtils.getOptions
pub fn get_options(
    annotation: &CommandAnnotation,
    modules: &[Arc<dyn BlazeModule>],
) -> Vec<OptionsClassRef> {
    let mut options = get_common_options(modules);
    for class in (annotation.options)() {
        add_option_class(&mut options, class);
    }

    if annotation.uses_configuration_options {
        for class in crate::fragment_options::fragment_options_classes() {
            add_option_class(&mut options, class);
        }
    }

    for supplier in modules {
        for class in supplier.get_command_options(annotation.name()) {
            add_option_class(&mut options, class);
        }
    }

    for base in annotation.inherits_options_from {
        for class in get_options(base, modules) {
            add_option_class(&mut options, class);
        }
    }
    options
}

// ---------------------------------------------------------------------------------------------
// @Command annotations, in BuiltinCommandModule.serverInit order, then MobileInstallModule and
// BazelRepositoryModule.
// ---------------------------------------------------------------------------------------------

fn no_options() -> Vec<OptionsClassRef> {
    Vec::new()
}

fn build_command_options() -> Vec<OptionsClassRef> {
    vec![
        BuildRequestOptions::class(),
        ExecutionOptions::class(),
        LocalExecutionOptions::class(),
        PackageOptions::class(),
        AnalysisOptions::class(),
        LoadingOptions::class(),
        KeepGoingOption::class(),
        LoadingPhaseThreadsOption::class(),
        BuildEventProtocolOptions::class(),
        SkyfocusOptions::class(),
        RemoteAnalysisCachingOptions::class(),
    ]
}

/// `@Command` of `BuildCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/BuildCommand.java:68 BuildCommand
pub static BUILD_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "build",
        inherits_options_from: &[],
    },
    options: build_command_options,
    inherits_options_from: &[],
    short_description: "Builds the specified targets.",
    uses_configuration_options: true,
    build_phase: BuildPhase::EXECUTES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:build.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn canonicalize_command_options() -> Vec<OptionsClassRef> {
    vec![CanonicalizeCommandOptions::class(), PackageOptions::class()]
}

/// `@Command` of `CanonicalizeCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:70 CanonicalizeCommand
pub static CANONICALIZE_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "canonicalize-flags",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: canonicalize_command_options,
    // inherits from build to get proper package loading options and rc flag aliases.
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Canonicalizes a list of %{product} options.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: concat!(
        "This command canonicalizes a list of %{product} options. Don't forget to prepend  '--' to",
        " end option parsing before the flags to canonicalize. This command doesn't support",
        " the strategy policies under --invocation_policy flag.\n%{options}",
    ),
    must_run_in_workspace: false,
    completion: "",
};

fn clean_command_options() -> Vec<OptionsClassRef> {
    vec![CleanCommandOptions::class()]
}

/// `@Command` of `CleanCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:67 CleanCommand
pub static CLEAN_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "clean",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: clean_command_options,
    // TODO(bazel-team): Remove this - we inherit a huge number of unused options.
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Removes output files and optionally stops the server.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: true, // Does not, but need to allow so we can ignore Starlark options.
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: false, // Do not create a command.log, otherwise we couldn't delete it.
    help: "resource:clean.txt",
    must_run_in_workspace: true,
    completion: "",
};

/// `@Command` of `CoverageCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/CoverageCommand.java:113 CoverageCommand
pub static COVERAGE_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "coverage",
        inherits_options_from: &[&TEST_COMMAND.rc],
    },
    options: no_options,
    inherits_options_from: &[&TEST_COMMAND],
    short_description: "Generates code coverage report for specified test targets.",
    uses_configuration_options: false,
    build_phase: BuildPhase::EXECUTES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:coverage.txt",
    must_run_in_workspace: true,
    completion: "label-test",
};

fn dump_command_options() -> Vec<OptionsClassRef> {
    vec![DumpCommandDumpOptions::class()]
}

/// `@Command` of `DumpCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:99 DumpCommand
pub static DUMP_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "dump",
        inherits_options_from: &[],
    },
    options: dump_command_options,
    inherits_options_from: &[],
    short_description: "Dumps the internal state of the %{product} server process.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: false,
    has_sensitive_residue: false,
    binary_std_out: true,
    binary_std_err: false,
    write_command_log: true,
    help: concat!(
        "Usage: %{product} dump <options>\n",
        "Dumps the internal state of the %{product} server process.  This command is provided",
        " as an aid to debugging, not as a stable interface, so users should not try to parse",
        " the output; instead, use 'query' or 'info' for this purpose.\n",
        "%{options}",
    ),
    must_run_in_workspace: false,
    completion: "",
};

fn help_command_options() -> Vec<OptionsClassRef> {
    vec![HelpCommandOptions::class()]
}

/// `@Command` of `HelpCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:92 HelpCommand
pub static HELP_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "help",
        inherits_options_from: &[],
    },
    options: help_command_options,
    inherits_options_from: &[],
    short_description: "Prints help for commands, or the index.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:help.txt",
    must_run_in_workspace: false,
    completion: "command|{startup_options,target-syntax,info-keys}",
};

fn info_command_options() -> Vec<OptionsClassRef> {
    vec![InfoCommandOptions::class()]
}

/// `@Command` of `InfoCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:110 InfoCommand
pub static INFO_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "info",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: info_command_options,
    // We have InfoCommand inherit from {@link BuildCommand} because we want all
    // configuration defaults specified in ~/.blazerc for {@code build} to apply to
    // {@code info} too, even though it doesn't actually do a build.
    //
    // (Ideally there would be a way to make {@code info} inherit just the bare
    // minimum of relevant options from {@code build}, i.e. those that affect the
    // values it prints.  But there's no such mechanism.)
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Displays runtime info about the %{product} server.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: true,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:info.txt",
    must_run_in_workspace: true,
    completion: "info-key",
};

fn print_action_command_options() -> Vec<OptionsClassRef> {
    vec![PrintActionCommandPrintActionOptions::class()]
}

/// `@Command` of `PrintActionCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/PrintActionCommand.java:80 PrintActionCommand
pub static PRINT_ACTION_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "print_action",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: print_action_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Prints the command line args for compiling a file.",
    uses_configuration_options: false,
    build_phase: BuildPhase::ANALYZES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:print_action.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn query_command_options() -> Vec<OptionsClassRef> {
    vec![
        CoreOptions::class(), // for --action_env, which affects the repo env
        PackageOptions::class(),
        QueryOptions::class(),
        KeepGoingOption::class(),
        LoadingPhaseThreadsOption::class(),
    ]
}

/// `@Command` of `QueryCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/QueryCommand.java:73 QueryCommand
pub static QUERY_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "query",
        inherits_options_from: &[],
    },
    options: query_command_options,
    inherits_options_from: &[],
    short_description: "Executes a dependency graph query.",
    uses_configuration_options: false,
    build_phase: BuildPhase::LOADS,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: true,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:query.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn run_command_options() -> Vec<OptionsClassRef> {
    vec![RunCommandRunOptions::class()]
}

/// `@Command` of `RunCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:120 RunCommand
pub static RUN_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "run",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: run_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Runs the specified target.",
    uses_configuration_options: false,
    build_phase: BuildPhase::EXECUTES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: true,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:run.txt",
    must_run_in_workspace: true,
    completion: "label-bin",
};

fn shutdown_command_options() -> Vec<OptionsClassRef> {
    vec![ShutdownCommandOptions::class()]
}

/// `@Command` of `ShutdownCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java:39 ShutdownCommand
pub static SHUTDOWN_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "shutdown",
        inherits_options_from: &[],
    },
    options: shutdown_command_options,
    inherits_options_from: &[],
    short_description: "Stops the %{product} server.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: false,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: concat!(
        "This command shuts down the memory resident %{product} server process.\n",
        "%{options}",
    ),
    must_run_in_workspace: false,
    completion: "",
};

fn test_command_options() -> Vec<OptionsClassRef> {
    vec![TestSummaryOptions::class()]
}

/// `@Command` of `TestCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:75 TestCommand
pub static TEST_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "test",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: test_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Builds and runs the specified test targets.",
    uses_configuration_options: false,
    build_phase: BuildPhase::EXECUTES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:test.txt",
    must_run_in_workspace: true,
    completion: "label-test",
};

fn version_command_options() -> Vec<OptionsClassRef> {
    vec![VersionCommandVersionOptions::class()]
}

/// `@Command` of `VersionCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/VersionCommand.java:46 VersionCommand
pub static VERSION_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "version",
        inherits_options_from: &[],
    },
    options: version_command_options,
    inherits_options_from: &[],
    short_description: "Prints version information for %{product}.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: false,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:version.txt",
    must_run_in_workspace: false,
    completion: "",
};

fn aquery_command_options() -> Vec<OptionsClassRef> {
    vec![AqueryOptions::class()]
}

/// `@Command` of `AqueryCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:66 AqueryCommand
pub static AQUERY_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "aquery",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: aquery_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Analyzes the given targets and queries the action graph.",
    uses_configuration_options: true,
    build_phase: BuildPhase::ANALYZES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: true,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:aquery.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn cquery_command_options() -> Vec<OptionsClassRef> {
    vec![CqueryOptions::class()]
}

/// `@Command` of `CqueryCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:77 CqueryCommand
pub static CQUERY_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "cquery",
        inherits_options_from: &[&TEST_COMMAND.rc],
    },
    options: cquery_command_options,
    // We inherit from TestCommand so that we pick up changes like `test --test_arg=foo` in
    // .bazelrc files.
    // Without doing this, there is no easy way to use the output of cquery to determine whether a
    // test has changed between two invocations, because the testrunner action is not easily
    // introspectable.
    inherits_options_from: &[&TEST_COMMAND],
    short_description: "Loads, analyzes, and queries the specified targets w/ configurations.",
    uses_configuration_options: true,
    build_phase: BuildPhase::ANALYZES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: true,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:cquery.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn config_command_options() -> Vec<OptionsClassRef> {
    vec![ConfigCommandConfigOptions::class()]
}

/// `@Command` of `ConfigCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/ConfigCommand.java:84 ConfigCommand
pub static CONFIG_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "config",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: config_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Displays details of configurations.",
    uses_configuration_options: true,
    build_phase: BuildPhase::NONE,
    hidden: true,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:config.txt",
    must_run_in_workspace: true,
    completion: "string",
};

/// `@Command` of `LicenseCommand`.
// Java: com/google/devtools/build/lib/runtime/commands/LicenseCommand.java:43 LicenseCommand
pub static LICENSE_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "license",
        inherits_options_from: &[],
    },
    options: no_options,
    inherits_options_from: &[],
    short_description: "Prints the license of this software.",
    uses_configuration_options: false,
    build_phase: BuildPhase::NONE,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "Prints the license of this software.\n\n%{options}",
    must_run_in_workspace: false,
    completion: "",
};

fn mobile_install_command_options() -> Vec<OptionsClassRef> {
    vec![
        MobileInstallCommandOptions::class(),
        WriteAdbArgsActionOptions::class(),
    ]
}

/// `@Command` of `MobileInstallCommand`.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:79 MobileInstallCommand
pub static MOBILE_INSTALL_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "mobile-install",
        inherits_options_from: &[&BUILD_COMMAND.rc],
    },
    options: mobile_install_command_options,
    inherits_options_from: &[&BUILD_COMMAND],
    short_description: "Installs targets to mobile devices.",
    uses_configuration_options: false,
    build_phase: BuildPhase::EXECUTES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:mobile-install.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn fetch_command_options() -> Vec<OptionsClassRef> {
    vec![
        FetchOptions::class(),
        PackageOptions::class(),
        KeepGoingOption::class(),
        LoadingPhaseThreadsOption::class(),
    ]
}

/// `@Command` of `FetchCommand`.
// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:74 FetchCommand
pub static FETCH_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "fetch",
        inherits_options_from: &[&TEST_COMMAND.rc],
    },
    options: fetch_command_options,
    inherits_options_from: &[&TEST_COMMAND],
    short_description: "Fetches external repositories that are prerequisites to the targets.",
    uses_configuration_options: true,
    build_phase: BuildPhase::ANALYZES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:fetch.txt",
    must_run_in_workspace: true,
    completion: "label",
};

fn mod_command_options() -> Vec<OptionsClassRef> {
    vec![
        CoreOptions::class(), // for --action_env, which affects the repo env
        ModOptions::class(),
        PackageOptions::class(),
        LoadingPhaseThreadsOption::class(),
    ]
}

/// `@Command` of `ModCommand`.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:119 ModCommand
pub static MOD_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "mod",
        inherits_options_from: &[],
    },
    options: mod_command_options,
    inherits_options_from: &[],
    short_description: "Queries the Bzlmod external dependency graph",
    uses_configuration_options: false,
    build_phase: BuildPhase::LOADS,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:mod.txt",
    must_run_in_workspace: true,
    completion: "",
};

fn vendor_command_options() -> Vec<OptionsClassRef> {
    vec![
        VendorOptions::class(),
        PackageOptions::class(),
        KeepGoingOption::class(),
        LoadingPhaseThreadsOption::class(),
    ]
}

/// `@Command` of `VendorCommand`.
// Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:110 VendorCommand
pub static VENDOR_COMMAND: CommandAnnotation = CommandAnnotation {
    rc: RcCommand {
        name: "vendor",
        inherits_options_from: &[&TEST_COMMAND.rc],
    },
    options: vendor_command_options,
    inherits_options_from: &[&TEST_COMMAND],
    short_description: "Fetches external repositories into a folder specified by the flag --vendor_dir.",
    uses_configuration_options: true,
    build_phase: BuildPhase::ANALYZES,
    hidden: false,
    allow_residue: true,
    has_sensitive_residue: false,
    binary_std_out: false,
    binary_std_err: false,
    write_command_log: true,
    help: "resource:vendor.txt",
    must_run_in_workspace: true,
    completion: "",
};

// ---------------------------------------------------------------------------------------------
// Commands that are not ported yet: their annotation is complete (so rc parsing, `help` and
// option inheritance see them) and `exec` fails loudly.
// ---------------------------------------------------------------------------------------------

macro_rules! unported_command {
    ($(#[$meta:meta])* $name:ident, $annotation:ident, $exec:literal) => {
        $(#[$meta])*
        pub struct $name;

        impl BlazeCommand for $name {
            fn annotation(&self) -> &'static CommandAnnotation {
                &$annotation
            }

            fn exec(
                &self,
                _env: &mut CommandEnvironment,
                _options: &OptionsParsingResult,
            ) -> BlazeCommandResult {
                blaze_util::unported!($exec)
            }
        }
    };
}

// Integration lead: implemented in crate::commands::build.
pub use crate::commands::build::BuildCommand;
// cmds implementer: implemented in crate::commands::canonicalize.
pub use crate::commands::canonicalize::CanonicalizeCommand;
// Implemented in crate::commands::coverage.
pub use crate::commands::coverage::CoverageCommand;
// Implemented in crate::commands::dump.
pub use crate::commands::dump::DumpCommand;
// Implemented in crate::commands::print_action.
pub use crate::commands::print_action::PrintActionCommand;
// W4 analysis lead: implemented in crate::commands::query.
pub use crate::commands::query::QueryCommand;
// Implemented in crate::commands::run.
pub use crate::commands::run::RunCommand;
// Integration lead: implemented in crate::commands::build.
pub use crate::commands::build::TestCommand;
// W4 analysis lead: implemented in crate::commands::aquery.
pub use crate::commands::aquery::AqueryCommand;
// W4 analysis lead: implemented in crate::commands::cquery.
pub use crate::commands::cquery::CqueryCommand;
// Implemented in crate::commands::config.
pub use crate::commands::config::ConfigCommand;
// cmds implementer: implemented in crate::commands::license.
pub use crate::commands::license::LicenseCommand;
unported_command!(
    /// Implementation of the 'mobile-install' command.
    // Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:79 MobileInstallCommand
    MobileInstallCommand,
    MOBILE_INSTALL_COMMAND,
    "MobileInstallCommand.exec"
);
// cmds implementer: implemented in crate::commands::fetch.
pub use crate::commands::fetch::FetchCommand;
// Integration lead: implemented in crate::commands::mod_command.
pub use crate::commands::mod_command::ModCommand;
// Implemented in crate::commands::vendor.
pub use crate::commands::vendor::VendorCommand;

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_options::OptionsParser;

    /// Every annotation in this file, in registration order.
    const ALL_COMMANDS: &[&CommandAnnotation] = &[
        &MOBILE_INSTALL_COMMAND,
        &FETCH_COMMAND,
        &MOD_COMMAND,
        &VENDOR_COMMAND,
        &BUILD_COMMAND,
        &CANONICALIZE_COMMAND,
        &CLEAN_COMMAND,
        &COVERAGE_COMMAND,
        &DUMP_COMMAND,
        &HELP_COMMAND,
        &INFO_COMMAND,
        &PRINT_ACTION_COMMAND,
        &QUERY_COMMAND,
        &RUN_COMMAND,
        &SHUTDOWN_COMMAND,
        &TEST_COMMAND,
        &VERSION_COMMAND,
        &AQUERY_COMMAND,
        &CQUERY_COMMAND,
        &CONFIG_COMMAND,
        &LICENSE_COMMAND,
    ];

    fn registered_commands() -> Vec<Arc<dyn BlazeCommand>> {
        let startup_options = OptionsParser::builder()
            .build()
            .unwrap_or_else(|e| e.throw());
        let mut builder = crate::module::ServerBuilder::new();
        for module in crate::bazel_modules::bazel_modules() {
            module
                .server_init(&startup_options, &mut builder)
                .unwrap_or_else(|e| panic!("{}", e.get_message()));
        }
        builder.get_commands().to_vec()
    }

    fn parser_for(annotation: &CommandAnnotation) -> OptionsParser {
        let modules = crate::bazel_modules::bazel_modules();
        let classes = get_options(annotation, &modules);
        OptionsParser::builder()
            .options_classes(&classes)
            .allow_residue(annotation.allow_residue)
            .build()
            .unwrap_or_else(|e| panic!("{}: {}", annotation.name(), e.get_message()))
    }

    #[test]
    fn every_command_builds_an_options_parser() {
        for annotation in ALL_COMMANDS {
            parser_for(annotation);
        }
    }

    #[test]
    fn union_of_all_command_options_builds_an_options_parser() {
        let modules = crate::bazel_modules::bazel_modules();
        let mut all = Vec::new();
        for annotation in ALL_COMMANDS {
            for class in get_options(annotation, &modules) {
                add_option_class(&mut all, class);
            }
        }
        // BlazeOptionHandler uses the union as fallback data, which allows duplicates that parse
        // equivalently (e.g. --repo of mod and vendor).
        OptionsParser::get_fallback_options_data(&all)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    #[test]
    fn registered_commands_match_the_annotations_in_order() {
        let names: Vec<&str> = registered_commands()
            .iter()
            .map(|c| c.annotation().name())
            .collect();
        let expected: Vec<&str> = ALL_COMMANDS.iter().map(|a| a.name()).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn help_lists_the_oracle_commands() {
        // `bazel help` of the oracle (Bazel 9.3.2): sorted, %{product} expanded, hidden omitted.
        let expected: &[(&str, &str)] = &[
            (
                "aquery",
                "Analyzes the given targets and queries the action graph.",
            ),
            ("build", "Builds the specified targets."),
            (
                "canonicalize-flags",
                "Canonicalizes a list of bazel options.",
            ),
            (
                "clean",
                "Removes output files and optionally stops the server.",
            ),
            (
                "coverage",
                "Generates code coverage report for specified test targets.",
            ),
            (
                "cquery",
                "Loads, analyzes, and queries the specified targets w/ configurations.",
            ),
            (
                "dump",
                "Dumps the internal state of the bazel server process.",
            ),
            (
                "fetch",
                "Fetches external repositories that are prerequisites to the targets.",
            ),
            ("help", "Prints help for commands, or the index."),
            ("info", "Displays runtime info about the bazel server."),
            ("license", "Prints the license of this software."),
            ("mobile-install", "Installs targets to mobile devices."),
            ("mod", "Queries the Bzlmod external dependency graph"),
            (
                "print_action",
                "Prints the command line args for compiling a file.",
            ),
            ("query", "Executes a dependency graph query."),
            ("run", "Runs the specified target."),
            ("shutdown", "Stops the bazel server."),
            ("test", "Builds and runs the specified test targets."),
            (
                "vendor",
                "Fetches external repositories into a folder specified by the flag --vendor_dir.",
            ),
            ("version", "Prints version information for bazel."),
        ];
        let mut listed: Vec<(String, String)> = registered_commands()
            .iter()
            .map(|c| c.annotation())
            .filter(|a| !a.hidden)
            .map(|a| {
                (
                    a.name().to_string(),
                    a.short_description.replace("%{product}", "bazel"),
                )
            })
            .collect();
        listed.sort();
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(n, d)| (n.to_string(), d.to_string()))
            .collect();
        assert_eq!(listed, expected);
    }

    #[test]
    fn info_accepts_re2_rc_options() {
        let mut parser = parser_for(&INFO_COMMAND);
        parser
            .parse(&[
                "--features=layering_check",
                "--features=parse_headers",
                "--enable_platform_specific_config",
                "--cxxopt=-std=c++17",
            ])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    #[test]
    fn test_accepts_test_output() {
        let mut parser = parser_for(&TEST_COMMAND);
        parser
            .parse(&["--test_output=errors"])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    #[test]
    fn version_rejects_cxxopt() {
        let mut parser = parser_for(&VERSION_COMMAND);
        let err = parser.parse(&["--cxxopt=x"]).unwrap_err();
        assert_eq!(err.get_message(), "Unrecognized option: --cxxopt=x");
    }

    #[test]
    fn rc_inheritance_matches_annotation_inheritance() {
        for annotation in ALL_COMMANDS {
            let rc: Vec<&str> = annotation
                .rc
                .inherits_options_from
                .iter()
                .map(|c| c.name)
                .collect();
            let full: Vec<&str> = annotation
                .inherits_options_from
                .iter()
                .map(|c| c.name())
                .collect();
            assert_eq!(rc, full, "{}", annotation.name());
        }
    }

    #[test]
    fn get_options_includes_inherited_and_fragment_options() {
        let modules = crate::bazel_modules::bazel_modules();
        let info = get_options(&INFO_COMMAND, &modules);
        // From build (inherited), its configuration options and its module options.
        for class in [
            BuildRequestOptions::class(),
            CoreOptions::class(),
            blaze_optdefs::fragment::cpp_options::CppOptions::class(),
            blaze_optdefs::misc::worker_options::WorkerOptions::class(),
            InfoCommandOptions::class(),
            UiOptions::class(),
        ] {
            assert!(info.iter().any(|c| std::ptr::eq(*c, class)));
        }
        let version = get_options(&VERSION_COMMAND, &modules);
        assert!(
            !version
                .iter()
                .any(|c| std::ptr::eq(*c, CoreOptions::class()))
        );
    }
}
