//! Options whose implementation lives in a module hook that Blaze has not ported
//! (`crate::bazel_modules` lists the hooks). Each is accepted by the options parser because its
//! options class is registered; a command that sets one of them to a value that needs the
//! missing behaviour fails loudly here, before it runs, instead of running without it.
//!
//! The checks follow the commands the Java hook acts on (the remote cache only matters to commands
//! that execute actions, for example), so that an rc file's `common` options do not stop
//! commands the missing behaviour would not change.
//!
//! Not checked here: options of hooks that PORTING.md allows to skip (profiling and tracing,
//! memory instrumentation, incremental state kept between commands), and options that only
//! select between ported behaviours.

use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::execution_graph_module::ExecutionGraphModuleExecutionGraphOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_optdefs::misc::debugging_options::DebuggingOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::starlark_debugger_options::StarlarkDebuggerOptions;

use crate::command::CommandAnnotation;
use crate::options_result::OptionsParsingResult;

/// Fails with `unported!` when `options` asks `command` for behaviour of a module hook that is
/// not ported. With the default value of every option this returns without effect.
pub(crate) fn check_unported_options(command: &CommandAnnotation, options: &OptionsParsingResult) {
    let command_name = command.name();

    // SkyframeActionExecutor.archivedTreeArtifactsEnabledForMnemonic: the executor does not
    // create archived tree artifacts, which is what the default filter (`-.*`) asks for.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:420 SkyframeActionExecutor.archivedTreeArtifactsEnabledForMnemonic
    if command.build_phase.executes()
        && let Some(core_options) =
            options.get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
    {
        let defaults = blaze_options::Options::<
            blaze_optdefs::fragment::core_options::CoreOptions,
        >::get_defaults();
        if core_options.archived_artifacts_mnemonics_filter
            != defaults.archived_artifacts_mnemonics_filter
        {
            blaze_util::unported!(
                "SkyframeActionExecutor.archivedTreeArtifactsEnabledForMnemonic",
                "--archived_tree_artifact_mnemonics_filter"
            );
        }
    }

    // RemoteModule.beforeCommand: the remote cache, remote execution, the disk cache and the
    // remote output service act on the actions a command executes; the remote downloader on the
    // repositories a command fetches. `build`, `test`, `coverage` and `run` run the remote module
    // (crate::commands::build::run_build); `mobile-install`, the other command that executes
    // actions, does not yet.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
    if let Some(remote_options) = options.get_options::<RemoteOptions>() {
        // Strings.isNullOrEmpty
        let set = |value: &Option<String>| value.as_ref().is_some_and(|v| !v.is_empty());
        let runs_remote_module = matches!(command_name, "build" | "test" | "coverage" | "run");
        if command.build_phase.executes() && !runs_remote_module {
            if set(&remote_options.remote_cache) {
                blaze_util::unported!("RemoteModule.beforeCommand", "--remote_cache");
            }
            if set(&remote_options.remote_executor) {
                blaze_util::unported!("RemoteModule.beforeCommand", "--remote_executor");
            }
            // CombinedCacheClientFactory.isDiskCache
            if remote_options
                .disk_cache
                .as_ref()
                .is_some_and(|path| !path.0.is_empty())
            {
                blaze_util::unported!("RemoteModule.beforeCommand", "--disk_cache");
            }
            if set(&remote_options.remote_output_service) {
                blaze_util::unported!(
                    "RemoteModule.beforeCommand",
                    "--experimental_remote_output_service"
                );
            }
        }
        if command.build_phase.loads() && set(&remote_options.remote_downloader) {
            blaze_util::unported!(
                "RemoteModule.beforeCommand",
                "--experimental_remote_downloader"
            );
        }
    }

    // The Build Event Protocol of the commands whose build events are not posted yet (the
    // module streams them: crate::bazel_modules::BazelBuildEventServiceModule).
    // Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:189 BazelBuildEventServiceModule.ALLOWED_COMMANDS
    if matches!(command_name, "mobile-install")
        && let Some(o) = options.get_options::<BuildEventStreamOptions>()
        && (!o.build_event_text_file.is_empty()
            || !o.build_event_binary_file.is_empty()
            || !o.build_event_json_file.is_empty()
            || options
                .get_options::<BuildEventServiceOptions>()
                .is_some_and(|o| !o.bes_backend.is_empty()))
    {
        blaze_util::unported!(
            "BuildEventServiceModule.beforeCommand",
            "the build events of the {} command",
            command_name
        );
    }

    // ExecutionGraphModule.beforeCommand: the execution graph log.
    // Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:222 ExecutionGraphModule.beforeCommand
    if options
        .get_options::<ExecutionGraphModuleExecutionGraphOptions>()
        .is_some_and(|o| o.enable_execution_graph_log)
    {
        blaze_util::unported!(
            "ExecutionGraphModule.beforeCommand",
            "--experimental_enable_execution_graph_log"
        );
    }

    // StarlarkDebuggerModule.beforeCommand: the Starlark debug server.
    // Java: com/google/devtools/build/lib/starlarkdebug/module/StarlarkDebuggerModule.java:36 StarlarkDebuggerModule.beforeCommand
    if options
        .get_options::<StarlarkDebuggerOptions>()
        .is_some_and(|o| o.debug_starlark)
    {
        blaze_util::unported!(
            "StarlarkDebuggerModule.beforeCommand",
            "--experimental_skylark_debug"
        );
    }

    // WorkspaceRuleModule.beforeCommand: the log of workspace rule events.
    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleModule.java:40 WorkspaceRuleModule.beforeCommand
    if options
        .get_options::<DebuggingOptions>()
        .is_some_and(|o| o.workspace_rules_log_file.is_some())
    {
        blaze_util::unported!(
            "WorkspaceRuleModule.beforeCommand",
            "--experimental_workspace_rules_log_file"
        );
    }

    if let Some(common_options) = options.get_options::<CommonCommandOptions>() {
        // ThreadDumpModule.beforeCommand: periodic thread dumps of the server.
        // Java: com/google/devtools/build/lib/runtime/ThreadDumpModule.java:64 ThreadDumpModule.beforeCommand
        if common_options.enable_thread_dump {
            blaze_util::unported!(
                "ThreadDumpModule.beforeCommand",
                "--experimental_enable_thread_dump"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use blaze_options::{OptionsClass, OptionsParser};

    use super::*;
    use crate::command_registry::{
        BUILD_COMMAND, MOBILE_INSTALL_COMMAND, QUERY_COMMAND, RUN_COMMAND, TEST_COMMAND,
    };

    fn parser(args: &[&str]) -> OptionsParser {
        let mut parser = OptionsParser::builder()
            .options_classes(&[
                RemoteOptions::class(),
                BuildEventServiceOptions::class(),
                BuildEventStreamOptions::class(),
                ExecutionGraphModuleExecutionGraphOptions::class(),
                StarlarkDebuggerOptions::class(),
                DebuggingOptions::class(),
                CommonCommandOptions::class(),
            ])
            .build()
            .unwrap_or_else(|e| e.throw());
        parser
            .parse(args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        parser
    }

    // The option defaults are Bazel's (transcribed options classes): a command without any of
    // these flags must not fail.
    #[test]
    fn default_options_are_accepted() {
        check_unported_options(&BUILD_COMMAND, &parser(&[]));
        check_unported_options(&QUERY_COMMAND, &parser(&[]));
    }

    #[test]
    fn build_event_files_are_accepted_for_build() {
        check_unported_options(
            &BUILD_COMMAND,
            &parser(&["--build_event_json_file=bep.json"]),
        );
    }

    #[test]
    fn build_event_files_are_accepted_for_query() {
        check_unported_options(
            &QUERY_COMMAND,
            &parser(&["--build_event_json_file=bep.json"]),
        );
    }

    #[test]
    #[should_panic(expected = "RemoteModule.beforeCommand")]
    fn disk_cache_fails() {
        check_unported_options(
            &MOBILE_INSTALL_COMMAND,
            &parser(&["--disk_cache=/tmp/cache"]),
        );
    }

    // build, test and run run the remote module (crate::commands::build::run_build).
    #[test]
    fn remote_flags_are_accepted_for_build_and_test() {
        for command in [&BUILD_COMMAND, &TEST_COMMAND, &RUN_COMMAND] {
            check_unported_options(command, &parser(&["--disk_cache=/tmp/cache"]));
            check_unported_options(command, &parser(&["--remote_cache=grpc://localhost:1"]));
            check_unported_options(command, &parser(&["--remote_executor=grpc://localhost:1"]));
        }
    }

    // RemoteModule treats an empty --disk_cache as no disk cache.
    #[test]
    fn empty_disk_cache_is_accepted() {
        check_unported_options(&BUILD_COMMAND, &parser(&["--disk_cache="]));
    }

    #[test]
    #[should_panic(expected = "RemoteModule.beforeCommand")]
    fn remote_cache_fails() {
        check_unported_options(
            &MOBILE_INSTALL_COMMAND,
            &parser(&["--remote_cache=grpc://localhost:1"]),
        );
    }

    /// The Build Event Service upload is ported (blaze_bes): `--bes_backend` passes.
    #[test]
    fn bes_backend_passes() {
        check_unported_options(
            &BUILD_COMMAND,
            &parser(&["--bes_backend=grpc://localhost:1"]),
        );
    }
}
