//! `blaze fetch`: `bazel/commands/FetchCommand.java`, with `RepositoryFetcher` and
//! `TargetFetcher` of the same package.

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_events::{Event, EventHandler, Reporter};
use blaze_optdefs::command::fetch_options::FetchOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_options::{OptionsParser, PriorityCategory};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_repo_rules::bzlmod::bazel_fetch_all_function::BazelFetchAllValue;
use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use blaze_sky_top::skyframe_executor::{DEFAULT_THREAD_COUNT, SkyframeExecutor};
use blaze_skyframe::SkyKey;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::build::{RunBuild, RunBuildOutcome, run_build};
use crate::commands::query::{AfterCommand, create_skyframe_executor_with_repo_precomputed};
use crate::commands::target_patterns_helper;
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

/// Fetches external repositories. Which is so fetch.
// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:74 FetchCommand
pub struct FetchCommand;

impl BlazeCommand for FetchCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::FETCH_COMMAND
    }

    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:79 FetchCommand.editOptions
    fn edit_options(&self, options_parser: &mut OptionsParser) {
        TargetFetcher::inject_no_build_option(options_parser);
    }

    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:84 FetchCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        if let Some((code, message)) = Self::validate_options(options) {
            return create_failed_blaze_command_result_with_code(env.get_reporter(), code, message);
        }

        if let Some(bep) = env.get_bep() {
            bep.post_no_build_event(
                Some(env.get_command_name().to_string()),
                Some(env.get_command_start_time()),
                /* separate_finished_event= */ true,
                /* show_progress= */ true,
                Some(env.get_command_id().to_string()),
            );
        }
        // The UI's subscriber of NoBuildEvent (showProgress = true).
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:777 UiEventHandler.noBuild
        if let Some(handler) = env.get_ui_event_handler() {
            handler.no_build(true);
        }

        let fetch_options: FetchOptions = options.get_options().expect("FetchOptions");
        let mut repo_precomputed = RepoPrecomputedValues::default();
        if fetch_options.force {
            // Using commandId as the value -instead of true/false- to make sure to invalidate
            // skyframe and to actually force fetch each time
            if fetch_options.configure {
                repo_precomputed.force_fetch_configure = env.get_command_id().to_string();
            } else {
                repo_precomputed.force_fetch = env.get_command_id().to_string();
            }
        }

        let threads_option: LoadingPhaseThreadsOption =
            options.get_options().expect("LoadingPhaseThreadsOption");
        let targets = match target_patterns_helper::read_from(env, options) {
            Ok(targets) => targets,
            Err(e) => {
                env.get_reporter().handle(&Event::error(e.get_message()));
                return BlazeCommandResult::failure_detail(e.get_failure_detail());
            }
        };
        let threads = threads_option.threads.max(1) as usize;
        if !targets.is_empty() {
            if !fetch_options.repos.is_empty() {
                return create_failed_blaze_command_result(
                    env.get_reporter(),
                    "Target patterns and --repo cannot both be specified",
                );
            }
            // PORT: the injected FORCE_FETCH value reaches the build's Skyframe executor
            // (crate::commands::build creates it) through the RunBuild hook.
            let result = Self::fetch_target(env, options, targets, repo_precomputed);
            return post_no_build_request_finished_event(env, result);
        }

        // The command's Skyframe executor (Java: env.getSkyframeExecutor()), with the precomputed
        // values injected above.
        let skyframe_executor =
            match create_skyframe_executor_with_repo_precomputed(env, repo_precomputed) {
                Ok(e) => e,
                Err(result) => return result,
            };
        let _after_command = AfterCommand(&skyframe_executor);
        // PORT: the dispatcher's computation of the main repository mapping on this executor; it
        // succeeded in the dispatcher already, which reported its events (crate::dispatcher,
        // MainRepoMappingEventHandler).
        if let Err(RepositoryMappingResolutionOrInterrupted::Interrupted) = skyframe_executor
            .get_main_repo_mapping(false, DEFAULT_THREAD_COUNT, env.get_reporter().clone())
        {
            return fetch_interrupted(env);
        }
        let result = if !fetch_options.repos.is_empty() {
            Self::fetch_repos(env, &skyframe_executor, threads, &fetch_options.repos)
        } else {
            // --all or just 'fetch' (equivalent) or --configure
            Self::fetch_all(env, &skyframe_executor, threads, fetch_options.configure)
        };
        // catch (InterruptedException e): returns before the NoBuildRequestFinishedEvent.
        // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:133 FetchCommand.exec
        let result = match result {
            Ok(result) => result,
            Err(FetchInterrupted) => return fetch_interrupted(env),
        };
        post_no_build_request_finished_event(env, result)
    }
}

impl FetchCommand {
    /// The failure `validateOptions` reports, if any (code and message; the caller reports it).
    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:149 FetchCommand.validateOptions
    fn validate_options(
        options: &OptionsParsingResult,
    ) -> Option<(failure_details::fetch_command::Code, &'static str)> {
        let pkg_options: PackageOptions = options.get_options().expect("PackageOptions");
        if !pkg_options.fetch {
            return Some((
                failure_details::fetch_command::Code::OptionsInvalid,
                "You cannot run fetch with --nofetch",
            ));
        }
        let fetch_options: FetchOptions = options.get_options().expect("FetchOptions");
        let options_count = [
            fetch_options.all,
            fetch_options.configure,
            !fetch_options.repos.is_empty(),
            !options.get_residue().is_empty(),
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if options_count > 1 {
            return Some((
                failure_details::fetch_command::Code::OptionsInvalid,
                "Only one fetch option can be provided for fetch command",
            ));
        }
        None
    }

    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:172 FetchCommand.fetchAll
    fn fetch_all(
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        threads: usize,
        configure_enabled: bool,
    ) -> Result<BlazeCommandResult, FetchInterrupted> {
        let evaluation_result = match skyframe_executor.evaluate(
            &[SkyKey::new(BazelFetchAllValue::key(configure_enabled))],
            /* keep_going= */ false,
            threads,
            env.get_reporter().clone(),
        ) {
            Ok(r) => r,
            Err(_) => return Err(FetchInterrupted),
        };
        if evaluation_result.has_error() {
            let message = evaluation_result
                .get_first_error()
                .and_then(|e| e.get_exception())
                .map(|e| e.to_string());
            return Ok(create_failed_blaze_command_result(
                env.get_reporter(),
                message
                    .as_deref()
                    .unwrap_or("Unexpected error during fetching all external deps."),
            ));
        }

        env.get_reporter().handle(&Event::info(
            "All external dependencies fetched successfully.",
        ));
        Ok(BlazeCommandResult::success())
    }

    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:195 FetchCommand.fetchRepos
    fn fetch_repos(
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        threads: usize,
        repos: &[String],
    ) -> Result<BlazeCommandResult, FetchInterrupted> {
        let repository_names_and_values =
            match RepositoryFetcher::fetch_repos(repos, env, skyframe_executor, threads) {
                Ok(values) => values,
                Err(RepositoryFetcherError::RepositoryMappingResolution(message, exit_code)) => {
                    return Ok(create_failed_blaze_command_result_with_exit_code(
                        env.get_reporter(),
                        &format!("Invalid repo name: {}", message),
                        exit_code,
                    ));
                }
                Err(RepositoryFetcherError::RepositoryFetcher(message)) => {
                    return Ok(create_failed_blaze_command_result(
                        env.get_reporter(),
                        &message,
                    ));
                }
                Err(RepositoryFetcherError::Interrupted) => return Err(FetchInterrupted),
            };

        let not_found_repos = repository_names_and_values
            .iter()
            .filter_map(|(_, value)| match value.as_deref() {
                Some(RepositoryDirectoryValue::Failure { error_msg }) => Some(error_msg.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("; ");
        if !not_found_repos.is_empty() {
            return Ok(create_failed_blaze_command_result(
                env.get_reporter(),
                &format!(
                    "Fetching some repos failed with errors: {}",
                    not_found_repos
                ),
            ));
        }
        env.get_reporter()
            .handle(&Event::info("All requested repos fetched successfully."));
        Ok(BlazeCommandResult::success())
    }

    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:225 FetchCommand.fetchTarget
    fn fetch_target(
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
        targets: Vec<String>,
        repo_precomputed: RepoPrecomputedValues,
    ) -> BlazeCommandResult {
        TargetFetcher::fetch_targets(env, options, targets, repo_precomputed)
    }
}

/// `env.getEventBus().post(new NoBuildRequestFinishedEvent(result.getExitCode(), now))`, then
/// `return result`.
// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:139 FetchCommand.exec
pub(crate) fn post_no_build_request_finished_event(
    env: &CommandEnvironment,
    result: BlazeCommandResult,
) -> BlazeCommandResult {
    if let Some(bep) = env.get_bep() {
        bep.post_no_build_request_finished_event(result.get_exit_code(), {
            use blaze_events::Clock as _;
            blaze_events::JavaClock.current_time_millis()
        });
    }
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:788 UiEventHandler.noBuildFinished
    if let Some(handler) = env.get_ui_event_handler() {
        handler.no_build_finished();
    }
    result
}

/// The `InterruptedException` `fetchAll` and `fetchRepos` throw.
struct FetchInterrupted;

/// `"Fetch interrupted: " + e.getMessage()` (the message of Blaze's interrupts is null).
fn fetch_interrupted(env: &CommandEnvironment) -> BlazeCommandResult {
    create_failed_blaze_command_result(env.get_reporter(), "Fetch interrupted: null")
}

// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:240 FetchCommand.createFailedBlazeCommandResult
pub(crate) fn create_failed_blaze_command_result_with_code(
    reporter: &Reporter,
    fetch_command_code: failure_details::fetch_command::Code,
    message: &str,
) -> BlazeCommandResult {
    create_failed_blaze_command_result_with_exit_code(
        reporter,
        message,
        DetailedExitCode::of(FailureDetail {
            message: message.to_string(),
            category: Some(Category::FetchCommand(failure_details::FetchCommand {
                code: fetch_command_code as i32,
            })),
        }),
    )
}

// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:253 FetchCommand.createFailedBlazeCommandResult
pub(crate) fn create_failed_blaze_command_result(
    reporter: &Reporter,
    error_message: &str,
) -> BlazeCommandResult {
    create_failed_blaze_command_result_with_exit_code(
        reporter,
        error_message,
        interrupted_detailed_exit_code(error_message),
    )
}

// Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:259 FetchCommand.createFailedBlazeCommandResult
pub(crate) fn create_failed_blaze_command_result_with_exit_code(
    reporter: &Reporter,
    message: &str,
    exit_code: DetailedExitCode,
) -> BlazeCommandResult {
    reporter.handle(&Event::error(message.to_string()));
    BlazeCommandResult::detailed_exit_code(exit_code)
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

/// The exceptions of `RepositoryFetcher.fetchRepos`.
pub(crate) enum RepositoryFetcherError {
    /// `RepositoryMappingResolutionException`: message and detailed exit code.
    RepositoryMappingResolution(String, DetailedExitCode),
    /// `RepositoryFetcherException`.
    RepositoryFetcher(String),
    /// `InterruptedException`.
    Interrupted,
}

/// Fetches repositories for commands.
// Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:44 RepositoryFetcher
pub(crate) struct RepositoryFetcher<'a> {
    env: &'a CommandEnvironment,
    skyframe_executor: &'a SkyframeExecutor,
    threads: usize,
}

impl RepositoryFetcher<'_> {
    // Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:57 RepositoryFetcher.fetchRepos
    pub(crate) fn fetch_repos(
        repos: &[String],
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        threads: usize,
    ) -> Result<Vec<(RepositoryName, Option<Arc<RepositoryDirectoryValue>>)>, RepositoryFetcherError>
    {
        RepositoryFetcher {
            env,
            skyframe_executor,
            threads,
        }
        .fetch_repos_internal(repos)
    }

    // Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:67 RepositoryFetcher.fetchRepos
    fn fetch_repos_internal(
        &self,
        repos: &[String],
    ) -> Result<Vec<(RepositoryName, Option<Arc<RepositoryDirectoryValue>>)>, RepositoryFetcherError>
    {
        let reposnames = self.collect_repository_names(repos)?;
        let evaluation_result = self.evaluate_fetch(&reposnames)?;
        Ok(reposnames
            .into_iter()
            .map(|repo_name| {
                let value = evaluation_result.get(&RepositoryDirectoryKey::key(repo_name.clone()));
                (repo_name, value)
            })
            .collect())
    }

    // Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:82 RepositoryFetcher.evaluateFetch
    fn evaluate_fetch(
        &self,
        reposnames: &[RepositoryName],
    ) -> Result<blaze_skyframe::EvaluationResult, RepositoryFetcherError> {
        let repo_delegator_keys: Vec<SkyKey> = reposnames
            .iter()
            .map(|r| SkyKey::new(RepositoryDirectoryKey::key(r.clone())))
            .collect();
        let evaluation_result = self
            .skyframe_executor
            .evaluate(
                &repo_delegator_keys,
                /* keep_going= */ false,
                self.threads,
                self.env.get_reporter().clone(),
            )
            .map_err(|_| RepositoryFetcherError::Interrupted)?;
        if evaluation_result.has_error() {
            let message = evaluation_result
                .get_first_error()
                .and_then(|e| e.get_exception())
                .map(|e| e.to_string());
            return Err(RepositoryFetcherError::RepositoryFetcher(
                message.unwrap_or_else(|| "Unexpected error during repository fetching.".into()),
            ));
        }
        Ok(evaluation_result)
    }

    // Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:100 RepositoryFetcher.collectRepositoryNames
    fn collect_repository_names(
        &self,
        repos: &[String],
    ) -> Result<Vec<RepositoryName>, RepositoryFetcherError> {
        // ImmutableSet.Builder: duplicates collapse, first occurrence wins.
        let mut reposnames: Vec<RepositoryName> = Vec::new();
        for repo in repos {
            let name = self.get_repository_name(repo)?;
            if !reposnames.contains(&name) {
                reposnames.push(name);
            }
        }
        Ok(reposnames)
    }

    // Java: com/google/devtools/build/lib/bazel/commands/RepositoryFetcher.java:115 RepositoryFetcher.getRepositoryName
    fn get_repository_name(
        &self,
        repo_name: &str,
    ) -> Result<RepositoryName, RepositoryFetcherError> {
        let invalid = |message: String| {
            RepositoryFetcherError::RepositoryFetcher(format!("Invalid repo name: {}", message))
        };
        if let Some(canonical) = repo_name.strip_prefix("@@") {
            // canonical RepoName
            RepositoryName::create(canonical).map_err(|e| invalid(e.get_message().to_string()))
        } else if let Some(apparent) = repo_name.strip_prefix('@') {
            // apparent RepoName
            RepositoryName::validate_user_provided_repo_name(apparent)
                .map_err(|e| invalid(e.get_message().to_string()))?;
            let keep_going = self
                .env
                .get_options()
                .get_options::<KeepGoingOption>()
                .expect("KeepGoingOption")
                .keep_going;
            let repo_mapping = match self.skyframe_executor.get_main_repo_mapping(
                keep_going,
                self.threads,
                self.env.get_reporter().clone(),
            ) {
                Ok(m) => m,
                Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
                    return Err(RepositoryFetcherError::Interrupted);
                }
                Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
                    return Err(RepositoryFetcherError::RepositoryMappingResolution(
                        e.to_string(),
                        crate::commands::query::to_runtime_detailed_exit_code(
                            blaze_query::compat::failure_details::from_packages(
                                &e.detailed_exit_code,
                            ),
                        ),
                    ));
                }
            };
            Ok(repo_mapping.get(apparent))
        } else {
            Err(invalid(
                "The repo value has to be either apparent '@repo' or canonical '@@repo' repo name"
                    .to_string(),
            ))
        }
    }
}

/// Fetches all repos needed for building a given set of targets.
// Java: com/google/devtools/build/lib/bazel/commands/TargetFetcher.java:33 TargetFetcher
pub(crate) struct TargetFetcher;

/// `BuildTool.processRequest(request, null, ...)`: no target validator.
pub(crate) struct NoTargetValidator;

impl blaze_sky_top::build_tool::TargetValidator for NoTargetValidator {
    fn validate_targets(
        &self,
        _targets: &[blaze_packages::target::Target],
        _keep_going: bool,
    ) -> Result<(), blaze_sky_top::project::LoadingFailedException> {
        Ok(())
    }
}

impl TargetFetcher {
    /// Creates a no-build build request to fetch all repos needed to build these targets.
    // Java: com/google/devtools/build/lib/bazel/commands/TargetFetcher.java:41 TargetFetcher.fetchTargets
    // Java: com/google/devtools/build/lib/bazel/commands/FetchCommand.java:225 FetchCommand.fetchTarget
    fn fetch_targets(
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
        targets: Vec<String>,
        repo_precomputed: RepoPrecomputedValues,
    ) -> BlazeCommandResult {
        // PORT: Java builds the BuildRequest here and runs `new BuildTool(env).processRequest`;
        // Blaze runs the build command's driver (crate::commands::build::run_build) on the
        // request's targets with the hook that RunCommand uses, and handles the BuildResult in
        // `after_build`.
        let validator = NoTargetValidator;
        run_build(
            env,
            options,
            false,
            Some(RunBuild {
                targets,
                validator: &validator,
                repo_precomputed,
                post_build_callback: None,
                after_build: Box::new(|outcome: RunBuildOutcome<'_>| {
                    let reporter = outcome.env.get_reporter().clone();
                    if !outcome.build_result.get_success() {
                        let message = format!(
                            "Fetching some target dependencies failed with errors: {}",
                            outcome
                                .detailed_exit_code
                                .get_failure_detail()
                                .map(|d| d.message.as_str())
                                .unwrap_or("")
                        );
                        return create_failed_blaze_command_result_with_code(
                            &reporter,
                            failure_details::fetch_command::Code::QueryEvaluationError,
                            &message,
                        );
                    }
                    reporter.handle(&Event::info(
                        "All external dependencies for the requested targets fetched \
                         successfully.",
                    ));
                    BlazeCommandResult::success()
                }),
            }),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/commands/TargetFetcher.java:68 TargetFetcher.injectNoBuildOption
    pub(crate) fn inject_no_build_option(options_parser: &mut OptionsParser) {
        if let Err(e) = options_parser.parse_with_priority(
            PriorityCategory::COMPUTED_DEFAULT,
            Some("Options required to fetch target"),
            vec!["--nobuild".to_string()],
        ) {
            panic!(
                "IllegalStateException: Fetch target needed option failed to parse: {}",
                e.get_message()
            );
        }
    }
}
