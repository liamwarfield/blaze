//! `blaze shutdown`.

use blaze_optdefs::command::shutdown_command::ShutdownCommandOptions;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

/// The 'blaze shutdown' command.
// Java: com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java:39 ShutdownCommand
pub struct ShutdownCommand;

impl ShutdownCommand {
    pub fn new() -> Self {
        ShutdownCommand
    }
}

impl BlazeCommand for ShutdownCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::SHUTDOWN_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java:56 ShutdownCommand.editOptions
    fn edit_options(&self, _options_parser: &mut blaze_options::OptionsParser) {}

    // Java: com/google/devtools/build/lib/runtime/commands/ShutdownCommand.java:59 ShutdownCommand.exec
    fn exec(
        &self,
        _env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let limit = options
            .get_options::<ShutdownCommandOptions>()
            .expect("ShutdownCommand.Options")
            .heap_size_limit;

        // Iff limit is non-zero, shut down the server if total memory exceeds the
        // limit. totalMemory is the actual heap size that the VM currently uses
        // *from the OS perspective*. That is, it's not the size occupied by all
        // objects (which is totalMemory() - freeMemory()), and not the -Xmx
        // (which is maxMemory()). It's really how much memory this process
        // currently consumes, in addition to the JVM code and C heap.

        if limit == 0 {
            return BlazeCommandResult::shutdown_on_success();
        }
        // PORT: `Runtime.getRuntime().totalMemory() > limit * 1000L * 1000` needs the JVM heap
        // size; Blaze has no JVM.
        blaze_util::unported!(
            "ShutdownCommand.exec",
            "--iff_heap_size_greater_than needs Runtime.totalMemory (no JVM)"
        )
    }
}
