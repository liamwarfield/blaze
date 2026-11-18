//! `blaze help`.

use blaze_events::{Event, EventHandler, OutErr};
use blaze_optdefs::command::help_command::HelpCommandOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::info::InfoCommand;
use crate::options_result::OptionsParsingResult;
use crate::runtime::BlazeRuntime;
use crate::version_info::BlazeVersionInfo;

/// The 'blaze help' command, which prints all available commands as well as specific help pages.
///
/// PORT: the index, `help info-keys` and the unknown-command error are ported; the help pages
/// that need `BlazeCommandUtils` (command usage, `startup_options`, `target-syntax`), the
/// completion script, `flags-as-proto` and `everything-as-html` are not.
// Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:92 HelpCommand
pub struct HelpCommand;

impl HelpCommand {
    pub fn new() -> Self {
        HelpCommand
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:140 HelpCommand.emitBlazeVersionInfo
    fn emit_blaze_version_info(out_err: &OutErr, product_name: &str) {
        let release_info = BlazeVersionInfo::instance().get_release_name();
        let line = format!("[{} {}]", product_name, release_info);
        out_err.print_out(&format!("{:>80}\n", line));
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:164 HelpCommand.emitStartupOptions
    fn emit_startup_options(_out_err: &OutErr, _runtime: &BlazeRuntime) {
        blaze_util::unported!("BlazeCommandUtils.expandHelpTopic", "help startup_options")
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:219 HelpCommand.emitCompletionHelp
    fn emit_completion_help(_shell: Option<&str>, _runtime: &BlazeRuntime) -> BlazeCommandResult {
        blaze_util::unported!(
            "HelpCommand.emitCompletionHelp",
            "needs OptionsParser.getOptionsCompletion and the completion scripts"
        )
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:168 HelpCommand.emitTargetSyntaxHelp
    fn emit_target_syntax_help(_out_err: &OutErr, _product_name: &str) {
        blaze_util::unported!("BlazeCommandUtils.expandHelpTopic", "help target-syntax")
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:172 HelpCommand.emitInfoKeysHelp
    fn emit_info_keys_help(env: &CommandEnvironment, out_err: &OutErr) {
        let empty = blaze_options::OptionsParser::builder()
            .build()
            .expect("OptionsParser.builder().build()");
        for item in InfoCommand::get_info_item_map(env, &empty).values() {
            out_err.print_out(&format!(
                "{:<23} {}\n",
                item.get_name(),
                item.get_description()
            ));
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:141 HelpCommand.emitGenericHelp
    fn emit_generic_help(out_err: &OutErr, runtime: &BlazeRuntime) {
        let commands: Vec<(String, &'static CommandAnnotation)> = runtime
            .get_command_map()
            .iter()
            .map(|(name, command)| (name.clone(), command.annotation()))
            .collect();
        out_err.print_out(&Self::generic_help(runtime.get_product_name(), &commands));
    }

    /// The text `emitGenericHelp` prints, given the command map's names and annotations.
    fn generic_help(
        product_name: &str,
        commands: &[(String, &'static CommandAnnotation)],
    ) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Usage: {} <command> <options> ...\n\n",
            product_name
        ));
        out.push_str("Available commands:\n");

        let mut names_in_order: Vec<&(String, &'static CommandAnnotation)> =
            commands.iter().collect();
        names_in_order.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, annotation) in names_in_order {
            if annotation.hidden {
                continue;
            }
            let short_description = annotation
                .short_description
                .replace("%{product}", product_name);
            out.push_str(&format!("  {:<19} {}\n", name, short_description));
        }

        out.push('\n');
        out.push_str("Getting more help:\n");
        out.push_str(&format!("  {} help <command>\n", product_name));
        out.push_str("                   Prints help and options for <command>.\n");
        out.push_str(&format!("  {} help startup_options\n", product_name));
        out.push_str(&format!(
            "                   Options for the JVM hosting {}.\n",
            product_name
        ));
        out.push_str(&format!("  {} help target-syntax\n", product_name));
        out.push_str("                   Explains the syntax for specifying targets.\n");
        out.push_str(&format!("  {} help info-keys\n", product_name));
        out.push_str("                   Displays a list of keys used by the info command.\n");
        out
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:636 HelpCommand.createFailureResult
    fn create_failure_result(
        message: &str,
        detailed_code: failure_details::help_command::Code,
    ) -> BlazeCommandResult {
        BlazeCommandResult::failure_detail(FailureDetail {
            message: message.to_string(),
            category: Some(Category::HelpCommand(failure_details::HelpCommand {
                code: detailed_code as i32,
            })),
        })
    }
}

impl BlazeCommand for HelpCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::HELP_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/HelpCommand.java:133 HelpCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // PORT: `env.getEventBus().post(new NoBuildEvent())` only reaches the Build Event
        // Protocol, which is ported for build and test; a BEP file requested for this command
        // fails loudly before it runs (crate::unported_options).
        let runtime = env.get_runtime().clone();
        let out_err = env.get_reporter().get_out_err().clone();
        let _help_options = options
            .get_options::<HelpCommandOptions>()
            .expect("HelpCommand.Options");
        let residue = options.get_residue();
        if residue.is_empty() {
            Self::emit_blaze_version_info(&out_err, runtime.get_product_name());
            Self::emit_generic_help(&out_err, &runtime);
            return BlazeCommandResult::success();
        }
        if residue[0] == "completion" {
            if residue.len() > 2 {
                let message = "The completion command takes at most one argument";
                env.get_reporter().handle(&Event::error(message));
                return Self::create_failure_result(
                    message,
                    failure_details::help_command::Code::MissingArgument,
                );
            }
            let shell = residue.get(1).map(String::as_str);
            return Self::emit_completion_help(shell, &runtime);
        }
        if residue.len() != 1 {
            let message = "You must specify exactly one command";
            env.get_reporter().handle(&Event::error(message));
            return Self::create_failure_result(
                message,
                failure_details::help_command::Code::MissingArgument,
            );
        }
        let help_subject = residue[0].as_str();
        let product_name = runtime.get_product_name();
        // Go through the custom subjects before going through Bazel commands.
        match help_subject {
            "startup_options" => {
                Self::emit_blaze_version_info(&out_err, product_name);
                Self::emit_startup_options(&out_err, &runtime);
                return BlazeCommandResult::success();
            }
            "target-syntax" => {
                Self::emit_blaze_version_info(&out_err, product_name);
                Self::emit_target_syntax_help(&out_err, product_name);
                return BlazeCommandResult::success();
            }
            "info-keys" => {
                Self::emit_info_keys_help(env, &out_err);
                return BlazeCommandResult::success();
            }
            "flags-as-proto" => {
                blaze_util::unported!("HelpCommand.emitFlagsAsProtoHelp")
            }
            "everything-as-html" => {
                blaze_util::unported!("HelpCommand.HtmlEmitter.emit")
            }
            _ => {} // fall out
        }

        if !runtime.get_command_map().contains_key(help_subject) {
            let message = format!("'{}' is not a known command", help_subject);
            env.get_reporter().handle(&Event::error(message.clone()));
            return Self::create_failure_result(
                &message,
                failure_details::help_command::Code::CommandNotFound,
            );
        }
        Self::emit_blaze_version_info(&out_err, product_name);
        blaze_util::unported!("BlazeCommandUtils.getUsage", "help {}", help_subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotation_for_test(
        hidden: bool,
        short_description: &'static str,
    ) -> &'static CommandAnnotation {
        Box::leak(Box::new(CommandAnnotation {
            rc: blaze_options::rc::handler::Command {
                name: "x",
                inherits_options_from: &[],
            },
            options: Vec::new,
            inherits_options_from: &[],
            short_description,
            uses_configuration_options: false,
            build_phase: crate::command::BuildPhase::NONE,
            hidden,
            allow_residue: false,
            has_sensitive_residue: false,
            binary_std_out: false,
            binary_std_err: false,
            write_command_log: true,
            help: "",
            must_run_in_workspace: false,
            completion: "",
        }))
    }

    /// Format of the oracle's `bazel help` index (Bazel 9.3.2): the version line right-aligned to
    /// 80 columns, sorted non-hidden commands padded to 20, then the help topics.
    #[test]
    fn generic_help_format_matches_oracle() {
        let commands = vec![
            (
                "version".to_string(),
                annotation_for_test(false, "Prints version information for %{product}."),
            ),
            (
                "aquery".to_string(),
                annotation_for_test(
                    false,
                    "Analyzes the given targets and queries the action graph.",
                ),
            ),
            (
                "hidden-cmd".to_string(),
                annotation_for_test(true, "Hidden."),
            ),
        ];
        assert_eq!(
            HelpCommand::generic_help("bazel", &commands),
            "\
Usage: bazel <command> <options> ...

Available commands:
  aquery              Analyzes the given targets and queries the action graph.
  version             Prints version information for bazel.

Getting more help:
  bazel help <command>
                   Prints help and options for <command>.
  bazel help startup_options
                   Options for the JVM hosting bazel.
  bazel help target-syntax
                   Explains the syntax for specifying targets.
  bazel help info-keys
                   Displays a list of keys used by the info command.
"
        );
    }

    #[test]
    fn version_line_right_aligned_to_80() {
        let line = format!("[{} {}]", "bazel", "release 9.3.2");
        assert_eq!(
            format!("{:>80}\n", line),
            "                                                           [bazel release 9.3.2]\n"
        );
    }
}
