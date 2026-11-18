//! Port of `com.google.devtools.build.lib.util.CommandFailureUtils`.

use blaze_cmdline::Label;
use indexmap::IndexMap;

use crate::java_file::JavaFile;
use crate::util::command_description_form::CommandDescriptionForm;
use crate::util::describable_execution_unit::DescribableExecutionUnit;
use crate::util::script_util::ScriptUtil;

const APPROXIMATE_MAXIMUM_MESSAGE_LENGTH: usize = 200;

/// Utility methods for describing command failures. See also `CommandUtils`. Unlike that one,
/// this class does not depend on `Command`; instead, it just manipulates command lines.
// Java: com/google/devtools/build/lib/util/CommandFailureUtils.java:31 CommandFailureUtils
pub struct CommandFailureUtils;

impl CommandFailureUtils {
    /// Construct a string that describes the command. Currently this returns a message of the
    /// form "foo bar baz", with shell meta-characters appropriately quoted and/or escaped,
    /// prefixed (if verbose is true) with an "env" command to set the environment.
    // Java: com/google/devtools/build/lib/util/CommandFailureUtils.java:44 CommandFailureUtils.describeCommand
    pub fn describe_command<S: AsRef<str>>(
        form: CommandDescriptionForm,
        pretty_print_args: bool,
        command_line_elements: &[S],
        environment: Option<&IndexMap<String, String>>,
        environment_variables_to_clear: Option<&[String]>,
        cwd: Option<&str>,
        configuration_checksum: Option<&str>,
        execution_platform_label: Option<&Label>,
        spawn_runner: Option<&str>,
    ) -> String {
        let mut message = String::new();
        let size = command_line_elements.len();
        let mut number_remaining = size;

        if form == CommandDescriptionForm::Complete {
            ScriptUtil::emit_begin_isolate(&mut message);
        }

        if form != CommandDescriptionForm::Abbreviated {
            if let Some(cwd) = cwd {
                ScriptUtil::emit_change_directory(&mut message, cwd);
            }
            // On Linux, insert an "exec" keyword to save a fork in "blaze run" generated scripts.
            // If we use "env" as a wrapper, the "exec" needs to be applied to the entire "env"
            // invocation.
            ScriptUtil::emit_exec(&mut message);
            // We *print* the environment variables here in sorted order, rather than in the
            // potentially nondeterministic order that will be actually used.
            if let Some(environment) = environment {
                ScriptUtil::emit_env_prefix(
                    &mut message,
                    /* ignore_environment= */ true,
                    environment,
                    environment_variables_to_clear,
                );
            }
        }

        let mut is_first_argument = true;
        for command_element in command_line_elements {
            let command_element = command_element.as_ref();
            // PORT: Java's String.length() of an internal (Latin-1) string is its UTF-8 byte
            // length, which is Rust's str::len().
            if form == CommandDescriptionForm::Abbreviated
                && message.len() + command_element.len() > APPROXIMATE_MAXIMUM_MESSAGE_LENGTH
            {
                message.push_str(" ... (remaining ");
                message.push_str(&number_remaining.to_string());
                message.push_str(if number_remaining == 1 {
                    " argument"
                } else {
                    " arguments"
                });
                message.push_str(" skipped)");
                break;
            } else {
                if number_remaining < size {
                    message.push_str(if pretty_print_args { " \\\n    " } else { " " });
                }
                ScriptUtil::emit_command_element(&mut message, command_element, is_first_argument);
                number_remaining -= 1;
            }
            is_first_argument = false;
        }

        if form == CommandDescriptionForm::Complete {
            ScriptUtil::emit_end_isolate(&mut message);
        }

        if form == CommandDescriptionForm::Complete {
            if let Some(configuration_checksum) = configuration_checksum {
                message.push('\n');
                message.push_str("# Configuration: ");
                message.push_str(configuration_checksum);
            }

            if let Some(execution_platform_label) = execution_platform_label {
                message.push('\n');
                message.push_str("# Execution platform: ");
                message.push_str(&execution_platform_label.to_string());
            }

            if let Some(spawn_runner) = spawn_runner {
                message.push('\n');
                message.push_str("# Runner: ");
                message.push_str(spawn_runner);
            }
        }

        message
    }

    /// Construct an error message that describes a failed command invocation. Currently this
    /// returns a message of the form "foo failed: error executing FooCompile command /dir/foo bar
    /// baz".
    // Java: com/google/devtools/build/lib/util/CommandFailureUtils.java:153 CommandFailureUtils.describeCommandFailure
    pub fn describe_command_failure_full<S: AsRef<str>>(
        verbose: bool,
        mnemonic: &str,
        command_line_elements: &[S],
        env: Option<&IndexMap<String, String>>,
        cwd: Option<&str>,
        configuration_checksum: Option<&str>,
        target_description: Option<&str>,
        execution_platform_label: Option<&Label>,
        spawn_runner: Option<&str>,
    ) -> String {
        let command_name = command_line_elements
            .first()
            .expect("NoSuchElementException")
            .as_ref();
        // Extract the part of the command name after the last "/", if any.
        let short_command_name = JavaFile::new(command_name).get_name().to_string();

        let form = if verbose {
            CommandDescriptionForm::Complete
        } else {
            CommandDescriptionForm::Abbreviated
        };

        let mut output = String::new();
        output.push_str("error executing ");
        output.push_str(mnemonic);
        output.push_str(" command ");
        if let Some(target_description) = target_description {
            output.push_str("(from ");
            output.push_str(target_description);
            output.push_str(") ");
        }
        if verbose {
            output.push_str("\n  ");
        }
        output.push_str(&CommandFailureUtils::describe_command(
            form,
            /* pretty_print_args= */ false,
            command_line_elements,
            env,
            None,
            cwd,
            configuration_checksum,
            execution_platform_label,
            spawn_runner,
        ));
        format!("{short_command_name} failed: {output}")
    }

    // Java: com/google/devtools/build/lib/util/CommandFailureUtils.java:196 CommandFailureUtils.describeCommandFailure
    pub fn describe_command_failure(
        verbose_failures: bool,
        cwd: Option<&str>,
        command: &dyn DescribableExecutionUnit,
    ) -> String {
        let env = command.get_environment();
        let configuration_checksum = command.get_configuration_checksum();
        let target_description = command.get_target_description();
        let execution_platform_label = command.get_execution_platform_label();
        CommandFailureUtils::describe_command_failure_full(
            verbose_failures,
            &command.get_mnemonic(),
            &command.get_arguments(),
            env.as_ref(),
            cwd,
            configuration_checksum.as_deref(),
            target_description.as_deref(),
            execution_platform_label.as_ref(),
            /* spawn_runner= */ None,
        )
    }
}
