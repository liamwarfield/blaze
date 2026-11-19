//! `blaze canonicalize-flags`.

use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::canonicalize_command::{
    CanonicalizeCommandFlagClashCanaryOptions, CanonicalizeCommandOptions,
};
use blaze_optdefs::fragment::converters::BLAZE_ALIASING_FLAG;
use blaze_options::OptionsClass;
use blaze_options::OptionsParser;
use blaze_options::definition::OptionsClassRef;
use blaze_options::invocation_policy::{InvocationPolicyEnforcer, InvocationPolicyParser};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::query::{AfterCommand, create_skyframe_executor, reporter};
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;
use crate::starlark_options_parser::{
    SkyframeExecutorTargetLoader, StarlarkOptionsParser, StarlarkOptionsParsingError,
};

/// The 'blaze canonicalize-flags' command.
// Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:70 CanonicalizeCommand
pub struct CanonicalizeCommand;

impl CanonicalizeCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:262 CanonicalizeCommand.handleInterruptedException
    fn handle_interrupted_exception(env: &CommandEnvironment) -> BlazeCommandResult {
        let message = "canonicalization interrupted";
        env.get_reporter().handle(&Event::error(message));
        BlazeCommandResult::detailed_exit_code(interrupted_detailed_exit_code(message))
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:268 CanonicalizeCommand.reportAndCreateCommandFailure
    fn report_and_create_command_failure(
        env: &CommandEnvironment,
        message: &str,
        detailed_code: failure_details::command::Code,
    ) -> BlazeCommandResult {
        env.get_reporter()
            .handle(&Event::error(message.to_string()));
        BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
            message: message.to_string(),
            category: Some(Category::Command(failure_details::Command {
                code: detailed_code as i32,
            })),
        }))
    }
}

/// `InterruptedFailureDetails.detailedExitCode(message)`.
// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::Interrupted(failure_details::Interrupted {
            code: failure_details::interrupted::Code::Interrupted as i32,
        })),
    })
}

impl BlazeCommand for CanonicalizeCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::CANONICALIZE_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:142 CanonicalizeCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let runtime = env.get_runtime().clone();
        let canonicalize_options = options
            .get_options::<CanonicalizeCommandOptions>()
            .expect("CanonicalizeCommand.Options");
        let command_name = canonicalize_options.for_command.clone();
        let Some(command) = runtime.get_command_map().get(&command_name) else {
            let message = format!(
                "Not a valid command: '{}' (should be one of {})",
                command_name,
                runtime
                    .get_command_map()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            env.get_reporter().handle(&Event::error(message.clone()));
            return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
                message,
                category: Some(Category::CanonicalizeFlags(
                    failure_details::CanonicalizeFlags {
                        code: failure_details::canonicalize_flags::Code::ForCommandInvalid as i32,
                    },
                )),
            }));
        };
        let mut options_classes: Vec<OptionsClassRef> =
            crate::command_registry::get_options(command.annotation(), runtime.get_blaze_modules());
        options_classes.push(CanonicalizeCommandFlagClashCanaryOptions::class());

        // set up the command environment for starlark options parsing
        // PORT: env.syncPackageLoading(options) is the creation of the command's Skyframe
        // executor (crate::commands::query::create_skyframe_executor), which reports its own
        // errors.
        let skyframe_executor = match create_skyframe_executor(env, None) {
            Ok(e) => e,
            Err(result) => return result,
        };
        let _after_command = AfterCommand(&skyframe_executor);
        // getMainRepoMapping(eventHandler) is getMainRepoMapping(false, DEFAULT_THREAD_COUNT,
        // eventHandler).
        let main_repo_mapping = match skyframe_executor.get_main_repo_mapping(
            false,
            DEFAULT_THREAD_COUNT,
            reporter(env),
        ) {
            Ok(m) => m,
            Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
                return Self::handle_interrupted_exception(env);
            }
            Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
                env.get_reporter().handle(&Event::error(e.to_string()));
                return BlazeCommandResult::detailed_exit_code(
                    crate::commands::query::to_runtime_detailed_exit_code(
                        blaze_query::compat::failure_details::from_packages(&e.detailed_exit_code),
                    ),
                );
            }
        };

        let mut parser = OptionsParser::builder()
            .options_classes(&options_classes)
            .skip_starlark_option_prefixes()
            .allow_residue(true)
            .with_alias_flag(Some(BLAZE_ALIASING_FLAG))
            .with_aliases(&options.get_aliases())
            .with_conversion_context(Arc::new(main_repo_mapping))
            .build()
            .unwrap_or_else(|e| e.throw());

        if let Err(e) = parser.parse_list(options.get_residue()) {
            return Self::report_and_create_command_failure(
                env,
                e.get_message(),
                failure_details::command::Code::OptionsParseFailure,
            );
        }

        let build_setting_loader = SkyframeExecutorTargetLoader::new(
            &skyframe_executor,
            env.get_relative_working_directory().clone(),
            reporter(env),
        );
        let (starlark_result, starlark_canonical) = {
            let mut starlark_options_parser = StarlarkOptionsParser::builder()
                .build_setting_loader(&build_setting_loader)
                .native_options_parser(&mut parser)
                .include_default_values(canonicalize_options.include_default_values)
                .build();
            let result = starlark_options_parser.parse();
            let canonical = match &result {
                Ok(_) => starlark_options_parser.canonicalize(),
                Err(_) => Vec::new(),
            };
            (result, canonical)
        };
        match starlark_result {
            Ok(parsed) => blaze_util::check_state!(parsed),
            Err(StarlarkOptionsParsingError::OptionsParsing(e)) => {
                return Self::report_and_create_command_failure(
                    env,
                    e.get_message(),
                    failure_details::command::Code::StarlarkOptionsParseFailure,
                );
            }
            Err(StarlarkOptionsParsingError::Interrupted) => {
                return Self::handle_interrupted_exception(env);
            }
        }

        if !parser.get_residue().is_empty() {
            return Self::report_and_create_command_failure(
                env,
                &format!("Unrecognized arguments: {}", parser.get_residue().join(" ")),
                failure_details::command::Code::ArgumentsNotRecognized,
            );
        }

        // Java: com/google/devtools/build/lib/runtime/commands/CanonicalizeCommand.java:225 CanonicalizeCommand.exec
        let policy = match InvocationPolicyParser::parse_policy(Some(
            canonicalize_options.invocation_policy.as_str(),
        )) {
            Ok(policy) => policy,
            Err(e) => {
                return Self::report_and_create_command_failure(
                    env,
                    e.get_message(),
                    failure_details::command::Code::InvocationPolicyParseFailure,
                );
            }
        };

        let result = (|| -> Result<(), blaze_options::OptionsParsingException> {
            let invocation_policy_enforcer = InvocationPolicyEnforcer::new(
                policy.clone(),
                parser.get_conversion_context().cloned(),
            );
            invocation_policy_enforcer.enforce(
                &mut parser,
                &command_name,
                /* invocation_policy_flag_list_builder= */ &mut Vec::new(),
            )?;

            let out_err = env.get_reporter().get_out_err().clone();
            // Print out the canonical invocation policy if requested.
            if canonicalize_options.canonicalize_policy {
                let effective_policy = InvocationPolicyEnforcer::get_effective_invocation_policy(
                    &policy,
                    &mut parser,
                    &command_name,
                )?;
                out_err.print_out_ln(&blaze_protos::text_format::message_to_string(
                    &effective_policy,
                ));
            } else {
                // Otherwise, print out the canonical command line
                // PORT: the Starlark half was computed above, before the native parser was
                // borrowed for the enforcement; the enforcer does not touch Starlark options.
                let native_result = parser.canonicalize();
                for piece in native_result.iter().chain(starlark_canonical.iter()) {
                    out_err.print_out_ln(piece);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            return Self::report_and_create_command_failure(
                env,
                e.get_message(),
                failure_details::command::Code::InvocationPolicyInvalid,
            );
        }

        BlazeCommandResult::success()
    }
}
