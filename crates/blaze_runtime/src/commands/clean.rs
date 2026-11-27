//! `blaze clean`.

use blaze_events::{Event, EventHandler, Reporter};
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::clean_command::CleanCommandOptions;
use blaze_protos::failure_details::{
    self, FailureDetail, clean_command::Code, failure_detail::Category,
};
use blaze_shell::{CommandBuilder, CommandExecutionError, OS};
use blaze_vfs::{Path, PathFragment};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

/// Implements 'blaze clean'.
// Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:67 CleanCommand
pub struct CleanCommand {
    os: OS,
}

/// `CleanException`: a clean failure with its detailed code. The message is the cause's message
/// (`Strings.nullToEmpty(e.getMessage())`).
// Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:312 CleanCommand.CleanException
#[derive(Debug)]
struct CleanException {
    detailed_code: Code,
    message: String,
}

impl CleanException {
    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:315 CleanException.<init>
    fn new(detailed_code: Code, message: String) -> Self {
        CleanException {
            detailed_code,
            message,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:320 CleanException.getFailureDetail
    fn get_failure_detail(&self) -> FailureDetail {
        create_failure_detail(&self.message, self.detailed_code)
    }
}

/// What `actuallyClean` throws.
enum CleanError {
    Clean(CleanException),
    Interrupted,
}

/// What `asyncClean` throws.
enum AsyncCleanError {
    Io(std::io::Error),
    Command(String),
    Interrupted,
}

/// `IOException.getMessage()`: Blaze's file system errors already carry Java's message; plain OS
/// errors lose Rust's " (os error N)" suffix.
fn java_io_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => text[..i].to_string(),
        _ => text,
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:325 CleanCommand.createFailureDetail
fn create_failure_detail(message: &str, detailed_code: Code) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(Category::CleanCommand(failure_details::CleanCommand {
            code: detailed_code as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Interrupted(failure_details::Interrupted {
            code: failure_details::interrupted::Code::Interrupted as i32,
        })),
    })
}

impl CleanCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:112 CleanCommand.<init>
    pub fn new() -> Self {
        Self::with_os(OS::get_current())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:117 CleanCommand.<init>
    pub fn with_os(os: OS) -> Self {
        CleanCommand { os }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:136 CleanCommand.canUseAsync
    pub fn can_use_async(mut r#async: bool, expunge: bool, os: OS, reporter: &Reporter) -> bool {
        // TODO(bazel-team): Deactivate expunge_async on Windows or Unknown platforms as support
        // for daemonizing is done in daemonize.c and does not support those platforms.
        let async_support_missing = os == OS::Windows || os == OS::Unknown;
        if r#async && async_support_missing {
            let fallback_name = if expunge {
                "--expunge"
            } else {
                "synchronous clean"
            };
            reporter.handle(&Event::info(format!(
                "--async cannot be used on non-Linux platforms, falling back to {}",
                fallback_name
            )));
            r#async = false;
        }

        let clean_banner = if r#async || async_support_missing {
            "Starting clean."
        } else {
            "Starting clean (this may take a while). \
             Use --async if the clean takes more than several minutes."
        };
        reporter.handle(&Event::info(clean_banner));

        r#async
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:180 CleanCommand.asyncClean
    fn async_clean(
        env: &CommandEnvironment,
        path: &Path,
        path_item_name: &str,
    ) -> Result<(), AsyncCleanError> {
        let temp_base_name = format!(
            "{}_tmp_{}_{}",
            path.get_base_name(),
            std::process::id(),
            uuid::Uuid::new_v4()
        );

        // Keeping tempOutputBase in the same directory ensures it remains in the
        // same file system, and therefore the mv will be atomic and fast.
        let parent = path
            .get_parent_directory()
            .expect("NullPointerException: getParentDirectory()");
        let temp_path = parent.get_child(&temp_base_name);
        path.rename_to(&temp_path).map_err(AsyncCleanError::Io)?;
        env.get_reporter().handle(&Event::info(format!(
            "{} moved to {} for deletion",
            path_item_name, temp_path
        )));

        let command = format!(
            "/usr/bin/find {} -type d -not -perm -u=rwx -exec /bin/chmod -f u=rwx {{}} +; /bin/rm -rf {}",
            temp_base_name, temp_base_name
        );
        // PORT: logger.atInfo ("Executing daemonic shell command") is not written (no server
        // logger).

        // BinTools.getEmbeddedPath("daemonize"): the tool extracted into the install base.
        let daemonize = env
            .get_directories()
            .get_embedded_binaries_root()
            .get_relative_str("daemonize");
        if !daemonize.exists() {
            // BinTools.getEmbeddedPath returns null for a tool that is not in the install base,
            // and CleanCommand calls getPathString() on it.
            // Java: com/google/devtools/build/lib/exec/BinTools.java:155 BinTools.getEmbeddedPath
            panic!(
                "java.lang.NullPointerException: Cannot invoke \"com.google.devtools.build.lib.vfs.Path.getPathString()\" because the return value of \"com.google.devtools.build.lib.exec.BinTools.getEmbeddedPath(String)\" is null"
            );
        }

        // Daemonize the shell to ensure that the shell exits even while the "rm
        // -rf" command continues.
        let result = CommandBuilder::new(env.get_client_env())
            .add_arg(daemonize.get_path_string())
            .add_args(&["-l", "/dev/null"])
            .add_args(&["-p", "/dev/null"])
            .add_arg("--")
            .add_args(&["/bin/sh", "/bin/sh", "-c", command.as_str()])
            .set_working_dir(&temp_path.get_parent_directory().expect("parent"))
            .build()
            .execute();
        match result {
            // PORT: logger.atInfo ("Shell command status") is not written.
            Ok(_) => Ok(()),
            Err(CommandExecutionError::Interrupted(_)) => Err(AsyncCleanError::Interrupted),
            Err(CommandExecutionError::Command(e)) => {
                Err(AsyncCleanError::Command(e.get_message().to_string()))
            }
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:215 CleanCommand.actuallyClean
    fn actually_clean(
        env: &CommandEnvironment,
        output_base: &Path,
        expunge: bool,
        r#async: bool,
        symlink_prefix: &str,
    ) -> Result<BlazeCommandResult, CleanError> {
        // PORT: `env.getOutputService().clean()`: Bazel's default output service
        // (LocalOutputService) does nothing on clean; Blaze has no other output service.

        if let Err(e) = env.get_blaze_workspace().clear_caches() {
            return Err(CleanError::Clean(CleanException::new(
                Code::ActionCacheCleanFailure,
                java_io_message(&e),
            )));
        }

        // PORT: `DigestUtils.clearCache()`: Blaze has no global digest cache.

        if expunge && !r#async {
            // PORT: logger.atInfo ("Expunging...") is not written.
            // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:236 CleanCommand.actuallyClean
            env.get_runtime().prepare_for_abrupt_shutdown();
            // Close java.log.
            crate::server_log::close();
            // PORT: Java closes the server JVM's own stdout/stderr (FileDescriptor.out/err, which
            // the client redirected to <output_base>/server/jvm.out) and System.out/err. They are
            // left open: in batch mode and in the in-process server they are the user's
            // terminal; in the persistent server (BLAZE_SERVER_MODE=daemon) they are jvm.out,
            // which Linux deletes below while it is open, and closing fd 1 or 2 would let the
            // next file opened take that number and receive whatever the process prints to it.
            // The server shuts down after this command (shutdownOnSuccess below), as Java's does.

            // Delete the big subdirectories with the important content first--this
            // will take the most time. Then quickly delete the little locks, logs
            // and links right before we exit. Once the lock file is gone there will
            // be a small possibility of a server race if a client is waiting, but
            // all significant files will be gone by then.
            let deleted = output_base
                .delete_trees_below()
                .and_then(|()| output_base.delete_tree());
            if let Err(e) = deleted {
                return Err(CleanError::Clean(CleanException::new(
                    Code::OutputBaseDeleteFailure,
                    java_io_message(&e),
                )));
            }
        } else if expunge {
            // PORT: logger.atInfo ("Expunging asynchronously...") is not written.
            // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:266 CleanCommand.actuallyClean
            env.get_runtime().prepare_for_abrupt_shutdown();
            match Self::async_clean(env, output_base, "Output base") {
                Ok(()) => {}
                Err(AsyncCleanError::Io(e)) => {
                    return Err(CleanError::Clean(CleanException::new(
                        Code::OutputBaseTempMoveFailure,
                        java_io_message(&e),
                    )));
                }
                Err(AsyncCleanError::Command(message)) => {
                    return Err(CleanError::Clean(CleanException::new(
                        Code::AsyncOutputBaseDeleteFailure,
                        message,
                    )));
                }
                Err(AsyncCleanError::Interrupted) => return Err(CleanError::Interrupted),
            }
        } else {
            // PORT: logger.atInfo ("Output cleaning...") is not written.
            env.get_blaze_workspace().reset_evaluator();
            let execroot = output_base.get_relative_str("execroot");
            if execroot.exists() {
                if r#async {
                    match Self::async_clean(env, &execroot, "Output tree") {
                        Ok(()) => {}
                        Err(AsyncCleanError::Io(e)) => {
                            return Err(CleanError::Clean(CleanException::new(
                                Code::ExecrootTempMoveFailure,
                                java_io_message(&e),
                            )));
                        }
                        Err(AsyncCleanError::Command(message)) => {
                            return Err(CleanError::Clean(CleanException::new(
                                Code::AsyncExecrootDeleteFailure,
                                message,
                            )));
                        }
                        Err(AsyncCleanError::Interrupted) => return Err(CleanError::Interrupted),
                    }
                } else if let Err(e) = execroot.delete_trees_below() {
                    return Err(CleanError::Clean(CleanException::new(
                        Code::ExecrootDeleteFailure,
                        java_io_message(&e),
                    )));
                }
            }
        }
        // remove convenience links
        remove_output_directory_links(
            env.get_workspace().expect("clean runs in a workspace"),
            env.get_reporter(),
            symlink_prefix,
        );

        // shutdown on expunge cleans
        if expunge {
            return Ok(BlazeCommandResult::shutdown_on_success());
        }
        // PORT: System.gc() has no equivalent.
        Ok(BlazeCommandResult::success())
    }
}

impl BlazeCommand for CleanCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::CLEAN_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:124 CleanCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // Assert that there is no residue and warn about Starlark options.
        let residue = options.get_residue();
        if !residue.is_empty() {
            let message = format!("Unrecognized arguments: {}", residue.join(" "));
            env.get_reporter().handle(&Event::error(message.clone()));
            return BlazeCommandResult::failure_detail(create_failure_detail(
                &message,
                Code::ArgumentsNotRecognized,
            ));
        }

        // PORT: `env.getEventBus().post(new NoBuildEvent())` is skipped: Blaze has no event bus
        // yet. CleanStartingEvent is delivered to its subscribers (WorkerModule, SandboxModule) below.
        let clean_options = options
            .get_options::<CleanCommandOptions>()
            .expect("CleanCommand.Options");
        let r#async = Self::can_use_async(
            clean_options.async_,
            clean_options.expunge,
            self.os,
            env.get_reporter(),
        );
        // env.getEventBus().post(new CleanStartingEvent(options)): its subscriber
        // WorkerModule.cleanStarting shuts the worker pool down (the clean command inherits the
        // build command's options, WorkerOptions among them).
        // Java: com/google/devtools/build/lib/runtime/commands/CleanCommand.java:137 CleanCommand.exec
        // Java: com/google/devtools/build/lib/worker/WorkerModule.java:74 WorkerModule.cleanStarting
        if let Some(worker_options) =
            options.get_options::<blaze_optdefs::misc::worker_options::WorkerOptions>()
        {
            blaze_exec::worker::worker_module::WorkerModule::clean_starting(&worker_options);
        }
        // Its subscriber SandboxModule.cleanStarting removes the sandbox stash.
        // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:389 SandboxModule.cleanStarting
        blaze_exec::sandbox::sandbox_module::SandboxModule::clean_starting_for_server();

        let symlink_prefix = get_symlink_prefix(
            &options
                .get_options::<BuildRequestOptions>()
                .expect("BuildRequestOptions"),
            env.get_runtime().get_product_name(),
        );
        let output_base = env.get_output_base().clone();
        match Self::actually_clean(
            env,
            &output_base,
            clean_options.expunge,
            r#async,
            &symlink_prefix,
        ) {
            Ok(result) => result,
            Err(CleanError::Clean(e)) => {
                env.get_reporter().handle(&Event::error(e.message.clone()));
                BlazeCommandResult::failure_detail(e.get_failure_detail())
            }
            Err(CleanError::Interrupted) => {
                let message = "clean interrupted";
                env.get_reporter().handle(&Event::error(message));
                BlazeCommandResult::detailed_exit_code(interrupted_detailed_exit_code(message))
            }
        }
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:357 BuildRequestOptions.getSymlinkPrefix
fn get_symlink_prefix(options: &BuildRequestOptions, product_name: &str) -> String {
    match &options.symlink_prefix {
        None => format!("{}-", product_name),
        Some(prefix) => prefix.clone(),
    }
}

// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:64 OutputDirectoryLinksUtils.NO_CREATE_SYMLINKS_PREFIX
const NO_CREATE_SYMLINKS_PREFIX: &str = "/";

/// The names of all (types of) convenience symlinks (`getAllLinkDefinitions`):
/// `STANDARD_LINK_DEFINITIONS` (bin, testlogs, genfiles, out, the execroot named after the
/// workspace) followed by the rule class provider's extra definitions.
///
/// PORT: Bazel's `ConfiguredRuleClassProvider` has no extra symlink definitions (nothing in
/// Bazel calls `addSymlinkDefinition`), so only the standard ones are listed. Move to the
/// buildtool port of `OutputDirectoryLinksUtils` once it exists.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:56 OutputDirectoryLinksUtils.getAllLinkDefinitions
fn get_all_link_names(symlink_prefix: &str, workspace_base_name: &str) -> Vec<String> {
    vec![
        // ConfigSymlink.getLinkName: symlinkPrefix + suffix
        format!("{}bin", symlink_prefix),
        format!("{}testlogs", symlink_prefix),
        format!("{}genfiles", symlink_prefix),
        // output directory (bazel-out)
        format!("{}out", symlink_prefix),
        // execroot
        format!("{}{}", symlink_prefix, workspace_base_name),
    ]
}

/// Attempts to remove the convenience symlinks in the workspace directory.
///
/// Issues a warning if it fails, e.g. because workspaceDirectory is readonly. Also cleans up any
/// child directories created by a custom prefix.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:183 OutputDirectoryLinksUtils.removeOutputDirectoryLinks
fn remove_output_directory_links(workspace: &Path, event_handler: &Reporter, symlink_prefix: &str) {
    if symlink_prefix == NO_CREATE_SYMLINKS_PREFIX {
        return;
    }
    let mut failures: Vec<String> = Vec::new();

    let workspace_base_name = workspace.get_base_name();

    for link_name in get_all_link_names(symlink_prefix, workspace_base_name) {
        remove_link(workspace, &link_name, &mut failures);
    }

    remove_directory_and_parents(workspace, PathFragment::create(symlink_prefix));
    if !failures.is_empty() {
        event_handler.handle(&Event::warn(format!(
            "failed to remove one or more convenience symlinks for prefix '{}':\n  {}",
            symlink_prefix,
            failures.join("\n  ")
        )));
    }
}

/// PORT: the `ConvenienceSymlink` BEP entries are not built (no BEP); `logOnly` is false here.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:126 OutputDirectoryLinksUtils.removeLink
fn remove_link(base: &Path, name: &str, failures: &mut Vec<String>) {
    let link = base.get_relative_str(name);
    if link.is_symbolic_link() {
        // TODO(b/146885821): Consider also removing empty ancestor directories, to allow for
        //  cleaning up directories generated by --symlink_prefix=dir1/dir2/...
        //  Might be undesireable since it could also remove manually-created directories.
        if let Err(e) = link.delete() {
            failures.push(format!("{}: {}", name, java_io_message(&e)));
        }
    }
}

/// Attempts to remove a relative chain of directories under a given base. Returns true if the
/// removal was successful, and returns false if the removal fails because a directory was not
/// empty.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:641 FileSystemUtils.removeDirectoryAndParents
fn remove_directory_and_parents(base: &Path, mut to_remove: PathFragment) -> bool {
    if to_remove.is_absolute() {
        return false;
    }
    while !to_remove.is_empty() {
        let p = base.get_relative(&to_remove);
        if p.exists()
            && let Err(_e) = p.delete()
        {
            return false;
        }
        to_remove = match to_remove.get_parent_directory() {
            Some(parent) => parent,
            None => break,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_events::StoredEventHandler;
    use std::sync::Arc;

    const EXPECTED_SUGGESTION: &str = "Use --async";

    // Java: src/test/java/com/google/devtools/build/lib/runtime/commands/CleanCommandRecommendsAsyncTest.java
    fn check(async_on_command_line: bool, os: OS, expect_suggestion: bool) {
        let reporter = Reporter::new(None);
        let stored_event_handler = Arc::new(StoredEventHandler::new());
        reporter.add_handler(stored_event_handler.clone());

        let r#async = CleanCommand::can_use_async(async_on_command_line, false, os, &reporter);
        if os == OS::Windows || os == OS::Unknown {
            assert!(!r#async);
        }

        let matches = stored_event_handler
            .get_events()
            .iter()
            .any(|event| event.get_message().contains(EXPECTED_SUGGESTION));
        assert_eq!(
            matches, expect_suggestion,
            "async={} on OS {:?}",
            async_on_command_line, os
        );
    }

    #[test]
    fn test_clean_provides_expected_suggestion() {
        // When --async is provided, don't expect --async to be suggested.
        check(true, OS::Linux, false);
        check(true, OS::Windows, false);
        check(true, OS::Darwin, false);
        check(true, OS::Freebsd, false);
        check(true, OS::Openbsd, false);
        check(true, OS::Unknown, false);

        // When --async is not provided, expect the suggestion on platforms that support it.
        check(false, OS::Linux, true);
        check(false, OS::Windows, false);
        check(false, OS::Darwin, true);
        check(false, OS::Freebsd, true);
        check(false, OS::Openbsd, true);
        check(false, OS::Unknown, false);
    }

    /// Oracle: `bazel clean` prints `INFO: Starting clean (this may take a while). Use --async if
    /// the clean takes more than several minutes.`; `bazel clean --async` prints
    /// `INFO: Starting clean.`
    #[test]
    fn clean_banners_match_oracle() {
        for (r#async, banner) in [
            (
                false,
                "Starting clean (this may take a while). Use --async if the clean takes more than several minutes.",
            ),
            (true, "Starting clean."),
        ] {
            let reporter = Reporter::new(None);
            let stored = Arc::new(StoredEventHandler::new());
            reporter.add_handler(stored.clone());
            CleanCommand::can_use_async(r#async, false, OS::Linux, &reporter);
            let messages: Vec<String> = stored
                .get_events()
                .iter()
                .map(|e| e.get_message().to_string())
                .collect();
            assert_eq!(messages, [banner]);
        }
    }

    #[test]
    fn standard_link_names() {
        assert_eq!(
            get_all_link_names("bazel-", "re2"),
            [
                "bazel-bin",
                "bazel-testlogs",
                "bazel-genfiles",
                "bazel-out",
                "bazel-re2"
            ]
        );
    }

    #[test]
    fn remove_output_directory_links_removes_only_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = blaze_vfs::UnixFileSystem::create(blaze_vfs::DigestHashFunction::Sha256);
        let ws = Path::create_from_str(tmp.path().join("ws").to_str().unwrap(), &fs);
        std::fs::create_dir(ws.get_path_string()).unwrap();
        std::os::unix::fs::symlink("/nonexistent", tmp.path().join("ws/bazel-bin")).unwrap();
        std::os::unix::fs::symlink("/nonexistent", tmp.path().join("ws/bazel-ws")).unwrap();
        std::fs::create_dir(tmp.path().join("ws/bazel-out")).unwrap();
        let reporter = Reporter::new(None);
        remove_output_directory_links(&ws, &reporter, "bazel-");
        assert!(std::fs::symlink_metadata(tmp.path().join("ws/bazel-bin")).is_err());
        assert!(std::fs::symlink_metadata(tmp.path().join("ws/bazel-ws")).is_err());
        // Not a symlink: kept.
        assert!(tmp.path().join("ws/bazel-out").is_dir());
    }

    #[test]
    #[ignore = "needs a CommandEnvironment test fixture (BlazeRuntime + workspace) to run CleanCommand.exec end to end"]
    fn clean_command_exec_end_to_end() {}
}
