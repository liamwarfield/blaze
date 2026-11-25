//! Port of `runtime/commands/CoverageCommand.java`.
//!
//! Handles the 'coverage' command on the Bazel command line: the `test` command with
//! `--collect_code_coverage` required and the coverage test timeouts suggested. See the class
//! comment of the Java file for how coverage collection works (`collect_coverage.sh`, baseline
//! coverage, instrumented and metadata files, the combined report).

use blaze_optdefs::common::test_timeout::COVERAGE_CMD_TIMEOUT;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::COVERAGE_COMMAND;
use crate::commands::build::{TestCommand, run_build};
use crate::options_result::OptionsParsingResult;

/// Handles the 'coverage' command on the Bazel command line.
// Java: com/google/devtools/build/lib/runtime/commands/CoverageCommand.java:113 CoverageCommand
pub struct CoverageCommand;

impl CoverageCommand {
    /// Returns the name of the command to ask the project file for.
    // Java: com/google/devtools/build/lib/runtime/commands/CoverageCommand.java:115 CoverageCommand.commandName
    pub fn command_name(&self) -> &'static str {
        "coverage"
    }
}

impl BlazeCommand for CoverageCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &COVERAGE_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CoverageCommand.java:120 CoverageCommand.editOptions
    fn edit_options(&self, options_parser: &mut blaze_options::OptionsParser) {
        TestCommand.edit_options(options_parser);
        let result = options_parser
            .parse_with_priority(
                blaze_options::PriorityCategory::SOFTWARE_REQUIREMENT,
                Some("Options required by the coverage command"),
                vec!["--collect_code_coverage".to_string()],
            )
            .and_then(|_| {
                options_parser.parse_with_priority(
                    blaze_options::PriorityCategory::COMPUTED_DEFAULT,
                    Some("Options suggested for the coverage command"),
                    vec![COVERAGE_CMD_TIMEOUT.to_string()],
                )
            });
        if let Err(e) = result {
            // Should never happen.
            panic!(
                "IllegalStateException: Unexpected exception: {}",
                e.get_message()
            );
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:98 TestCommand.exec
    // (inherited; the BuildRequest's command name is the annotation's, "coverage").
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        run_build(env, options, true, None)
    }
}
