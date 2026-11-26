//! `blaze vendor`: `bazel/commands/VendorCommand.java`.
//!
//! Fetches external repositories into a specified directory. It can be used to fetch all
//! external repositories, a specific list of repositories or the repositories needed to build a
//! specific list of targets. The command is used to create a vendor directory that can be used
//! to build the project offline.

use std::collections::{HashSet, VecDeque};

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_bzlmod::skyframe::keys::BazelModuleResolutionKey;
use blaze_bzlmod_model::java_uri::JavaUri;
use blaze_bzlmod_model::resolution::BazelModuleResolutionValue;
use blaze_bzlmod_model::vendor_manager::VendorManager;
use blaze_cmdline::RepositoryName;
use blaze_events::{Event, EventHandler as _};
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::command::vendor_options::VendorOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::OptionsParser;
use blaze_protos::failure_details;
use blaze_repo_rules::bzlmod::bazel_fetch_all_function::BazelFetchAllValue;
use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use blaze_sky_top::skyframe_executor::{DEFAULT_THREAD_COUNT, SkyframeExecutor};
use blaze_skyframe::{Key, SkyKey};
use blaze_vfs::Path;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::build::{RunBuild, RunBuildOutcome, run_build};
use crate::commands::fetch::{
    NoTargetValidator, RepositoryFetcher, RepositoryFetcherError, TargetFetcher,
    create_failed_blaze_command_result, create_failed_blaze_command_result_with_code,
    create_failed_blaze_command_result_with_exit_code, post_no_build_request_finished_event,
};
use crate::commands::query::{AfterCommand, create_skyframe_executor_with_repo_precomputed};
use crate::commands::target_patterns_helper;
use crate::options_result::OptionsParsingResult;

/// Fetches external repositories into a specified directory.
// Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:110 VendorCommand
pub struct VendorCommand;

/// The exceptions of `vendorAll`, `vendorRepos` and `vendorTargets`.
enum VendorError {
    /// `InterruptedException`.
    Interrupted,
    /// `IOException` with its message.
    Io(String),
}

impl BlazeCommand for VendorCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::VENDOR_COMMAND
    }

    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:131 VendorCommand.editOptions
    fn edit_options(&self, options_parser: &mut OptionsParser) {
        TargetFetcher::inject_no_build_option(options_parser);
    }

    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:136 VendorCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        if let Some(invalid_result) = Self::validate_options(env, options) {
            return invalid_result;
        }

        // The UI's subscriber of NoBuildEvent (showProgress = true). The build event stream is
        // not created for `vendor` (not in BazelBuildEventServiceModule.ALLOWED_COMMANDS).
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:777 UiEventHandler.noBuild
        if let Some(handler) = env.get_ui_event_handler() {
            handler.no_build(true);
        }

        // IS_VENDOR_COMMAND & VENDOR_DIR is already injected in "BazelRepositoryModule", we just
        // need to update this value for the delegator function to recognize this call is from
        // VendorCommand
        // PORT: the command's Skyframe executor is created with the value.
        let repo_precomputed = RepoPrecomputedValues {
            is_vendor_command: true,
            ..RepoPrecomputedValues::default()
        };

        let vendor_options: VendorOptions = options.get_options().expect("VendorOptions");
        let threads_option: LoadingPhaseThreadsOption =
            options.get_options().expect("LoadingPhaseThreadsOption");
        let threads = threads_option.threads.max(1) as usize;
        let vendor_directory = env
            .get_workspace()
            .expect("NullPointerException: getWorkspace()")
            .get_relative(
                options
                    .get_options::<RepositoryOptions>()
                    .expect("RepositoryOptions")
                    .vendor_directory
                    .as_ref()
                    .expect("validated")
                    .get(),
            );
        let vendor_manager = VendorManager::new(vendor_directory.as_fragment().clone());
        let targets = match target_patterns_helper::read_from(env, options) {
            Ok(targets) => targets,
            Err(e) => {
                env.get_reporter().handle(&Event::error(e.get_message()));
                return BlazeCommandResult::failure_detail(e.get_failure_detail());
            }
        };
        let result = if !targets.is_empty() {
            if !vendor_options.repos.is_empty() {
                return create_failed_blaze_command_result(
                    env.get_reporter(),
                    "Target patterns and --repo cannot both be specified",
                );
            }
            Self::vendor_targets(env, options, targets, &vendor_manager, repo_precomputed)
        } else {
            // The command's Skyframe executor (Java: env.getSkyframeExecutor()).
            let skyframe_executor =
                match create_skyframe_executor_with_repo_precomputed(env, repo_precomputed) {
                    Ok(e) => e,
                    Err(result) => return result,
                };
            let _after_command = AfterCommand(&skyframe_executor);
            // PORT: the dispatcher's computation of the main repository mapping on this
            // executor (see crate::commands::fetch).
            if let Err(RepositoryMappingResolutionOrInterrupted::Interrupted) = skyframe_executor
                .get_main_repo_mapping(false, DEFAULT_THREAD_COUNT, env.get_reporter().clone())
            {
                return vendor_interrupted(env);
            }
            if !vendor_options.repos.is_empty() {
                Self::vendor_repos(
                    env,
                    &skyframe_executor,
                    threads,
                    &vendor_options.repos,
                    &vendor_manager,
                )
            } else {
                Self::vendor_all(env, &skyframe_executor, threads, &vendor_manager)
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(VendorError::Interrupted) => return vendor_interrupted(env),
            Err(VendorError::Io(message)) => {
                return create_failed_blaze_command_result(
                    env.get_reporter(),
                    &format!("Error while vendoring repos: {}", message),
                );
            }
        };
        post_no_build_request_finished_event(env, result)
    }
}

/// `"Vendor interrupted: " + e.getMessage()` (the message of Blaze's interrupts is null).
fn vendor_interrupted(env: &CommandEnvironment) -> BlazeCommandResult {
    create_failed_blaze_command_result(env.get_reporter(), "Vendor interrupted: null")
}

impl VendorCommand {
    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:206 VendorCommand.validateOptions
    fn validate_options(
        env: &CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> Option<BlazeCommandResult> {
        let repository_options: RepositoryOptions =
            options.get_options().expect("RepositoryOptions");
        if repository_options.vendor_directory.is_none() {
            return Some(create_failed_blaze_command_result_with_code(
                env.get_reporter(),
                failure_details::fetch_command::Code::OptionsInvalid,
                "You cannot run the vendor command without specifying --vendor_dir",
            ));
        }
        let package_options: PackageOptions = options.get_options().expect("PackageOptions");
        if !package_options.fetch {
            return Some(create_failed_blaze_command_result_with_code(
                env.get_reporter(),
                failure_details::fetch_command::Code::OptionsInvalid,
                "You cannot run the vendor command with --nofetch",
            ));
        }
        None
    }

    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:222 VendorCommand.vendorAll
    fn vendor_all(
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        threads: usize,
        vendor_manager: &VendorManager,
    ) -> Result<BlazeCommandResult, VendorError> {
        let fetch_key = BazelFetchAllValue::key(/* configure_enabled= */ false);
        let evaluation_result = skyframe_executor
            .evaluate(
                &[SkyKey::new(fetch_key.clone())],
                /* keep_going= */ false,
                threads,
                env.get_reporter().clone(),
            )
            .map_err(|_| VendorError::Interrupted)?;
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

        let fetch_all_value: std::sync::Arc<BazelFetchAllValue> = evaluation_result
            .get(&fetch_key)
            .expect("BazelFetchAllValue without error");
        env.get_reporter()
            .handle(&Event::info("Vendoring all external repositories..."));
        Self::vendor(
            env,
            skyframe_executor,
            vendor_manager,
            &fetch_all_value.repos_to_vendor,
        )?;
        env.get_reporter().handle(&Event::info(
            "All external dependencies vendored successfully.",
        ));
        Ok(BlazeCommandResult::success())
    }

    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:247 VendorCommand.vendorRepos
    fn vendor_repos(
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        threads: usize,
        repos: &[String],
        vendor_manager: &VendorManager,
    ) -> Result<BlazeCommandResult, VendorError> {
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
                Err(RepositoryFetcherError::Interrupted) => return Err(VendorError::Interrupted),
            };

        // Split repos to found and not found, vendor found ones and report others
        let mut repos_to_vendor: Vec<RepositoryName> = Vec::new();
        let mut not_found_repo_errors: Vec<String> = Vec::new();
        for (name, value) in &repository_names_and_values {
            match value.as_deref() {
                Some(RepositoryDirectoryValue::Success {
                    exclude_from_vendoring,
                    ..
                }) => {
                    if !exclude_from_vendoring {
                        repos_to_vendor.push(name.clone());
                    }
                }
                Some(RepositoryDirectoryValue::Failure { error_msg }) => {
                    not_found_repo_errors.push(error_msg.clone());
                }
                None => panic!("NullPointerException: no RepositoryDirectoryValue for {name}"),
            }
        }

        env.get_reporter()
            .handle(&Event::info("Vendoring repositories..."));
        Self::vendor(env, skyframe_executor, vendor_manager, &repos_to_vendor)?;
        if !not_found_repo_errors.is_empty() {
            // "..." + notFoundRepoErrors: List.toString.
            return Ok(create_failed_blaze_command_result(
                env.get_reporter(),
                &format!(
                    "Vendoring some repos failed with errors: [{}]",
                    not_found_repo_errors.join(", ")
                ),
            ));
        }
        env.get_reporter()
            .handle(&Event::info("All requested repos vendored successfully."));
        Ok(BlazeCommandResult::success())
    }

    /// `vendorTargets`: `TargetFetcher.fetchTargets` (a `--nobuild` build of the targets), then
    /// the repositories its graph reaches.
    ///
    /// PORT: Java builds the request and runs `BuildTool.processRequest`; Blaze runs the build
    /// command's driver (crate::commands::build::run_build) with the hook that `fetch` uses and
    /// vendors in `after_build`, while the build's Skyframe graph exists.
    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:282 VendorCommand.vendorTargets
    fn vendor_targets(
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
        targets: Vec<String>,
        vendor_manager: &VendorManager,
        repo_precomputed: RepoPrecomputedValues,
    ) -> Result<BlazeCommandResult, VendorError> {
        let validator = NoTargetValidator;
        let mut vendor_error: Option<VendorError> = None;
        let result = run_build(
            env,
            options,
            false,
            Some(RunBuild {
                targets,
                validator: &validator,
                repo_precomputed,
                after_build: Box::new(|outcome: RunBuildOutcome<'_>| {
                    let env = &*outcome.env;
                    // Java: com/google/devtools/build/lib/bazel/commands/TargetFetcher.java:58 TargetFetcher.fetchTargets
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
                            env.get_reporter(),
                            failure_details::fetch_command::Code::QueryEvaluationError,
                            &message,
                        );
                    }

                    // Traverse the graph created from build to collect repos and vendor them
                    let target_keys: Vec<SkyKey> = outcome
                        .build_result
                        .get_actual_targets()
                        .unwrap_or_default()
                        .iter()
                        .map(|view| {
                            let key = view.get_lookup_key();
                            let ct_key = key
                                .as_any()
                                .downcast_ref::<ConfiguredTargetKey>()
                                .unwrap_or_else(|| {
                                    panic!("ClassCastException: {key} is not a ConfiguredTargetKey")
                                });
                            SkyKey::new(ct_key.clone())
                        })
                        .collect();
                    let repos_to_vendor =
                        collect_repos_from_targets(outcome.skyframe_executor, &target_keys);

                    env.get_reporter()
                        .handle(&Event::info("Vendoring dependencies for targets..."));
                    if let Err(e) = Self::vendor(
                        env,
                        outcome.skyframe_executor,
                        vendor_manager,
                        &repos_to_vendor,
                    ) {
                        vendor_error = Some(e);
                        return BlazeCommandResult::success();
                    }
                    env.get_reporter().handle(&Event::info(
                        "All external dependencies for the requested targets vendored \
                         successfully.",
                    ));
                    BlazeCommandResult::success()
                }),
            }),
        );
        match vendor_error {
            Some(e) => Err(e),
            None => Ok(result),
        }
    }

    /// Copies the fetched repos from the external cache into the vendor directory, unless the
    /// repo is ignored or was already vendored and up-to-date
    // Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:335 VendorCommand.vendor
    fn vendor(
        env: &CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        vendor_manager: &VendorManager,
        repos_to_vendor: &[RepositoryName],
    ) -> Result<(), VendorError> {
        // 1. Vendor registry files
        let module_resolution_value = skyframe_executor
            .get_evaluator()
            .get_existing_value(&SkyKey::new(BazelModuleResolutionKey))
            .expect("NullPointerException: BazelModuleResolutionValue");
        let module_resolution_value = module_resolution_value
            .downcast_ref::<BazelModuleResolutionValue>()
            .expect("ClassCastException: BazelModuleResolutionValue");
        let registry_files = &module_resolution_value.registry_file_hashes;

        // vendorPathToURL is a map of
        //  key: a vendor path string converted to lower case
        //  value: a URL string
        // This map is for detecting potential rare vendor path conflicts, such as:
        //  http://foo.bar.com/BCR vs http://foo.bar.com/bcr => conflict vendor paths on
        // case-insensitive system
        //  http://foo.bar.com/bcr vs http://foo.bar.com:8081/bcr => conflict vendor path because
        // port number is ignored in vendor path
        // The user has to update the Bazel registries this if such conflicts occur.
        let mut vendor_path_to_url: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (raw_url, checksum) in registry_files {
            let url = JavaUri::new(raw_url)
                .unwrap_or_else(|e| panic!("IllegalArgumentException: {}", e.get_message()));
            if url.get_scheme() == Some("file") {
                continue;
            }

            let vendor_path = |url: &JavaUri| -> Result<String, VendorError> {
                vendor_manager
                    .get_vendor_path_for_url(url)
                    .map(|p| p.get_path_string().to_string())
                    .map_err(|e| VendorError::Io(e.0))
            };
            let output_path = vendor_path(&url)?;
            // toLowerCase(Locale.ROOT)
            let output_path_lower_case = output_path.to_lowercase();
            if let Some(previous_url) = vendor_path_to_url.get(&output_path_lower_case) {
                let previous_path = vendor_path(
                    &JavaUri::new(previous_url).expect("a URL that was parsed before"),
                )?;
                return Err(VendorError::Io(format!(
                    "Vendor paths conflict detected for registry URLs:\n    {} => {}\n    {} => \
                     {}\nTheir output paths are either the same or only differ by case, which \
                     will cause conflict on case insensitive file systems, please fix by \
                     changing the registry URLs!",
                    previous_url, previous_path, raw_url, output_path
                )));
            }

            // Only vendor a registry URL when its checksum exists, otherwise the URL should be
            // recorded as "not found" in moduleResolutionValue.getRegistryFileHashes()
            if !vendor_manager
                .is_url_vendored(&url)
                .map_err(|e| VendorError::Io(e.0))?
                && checksum.is_some()
            {
                let vendored = skyframe_executor
                    .get_registry_fetcher()
                    .download_and_read_one_url_for_bzlmod(
                        &url,
                        env.get_nonstrict_repo_env(),
                        checksum.as_ref(),
                    )
                    .map_err(|e| match e {
                        blaze_bzlmod_model::registry::RegistryException::Interrupted => {
                            VendorError::Interrupted
                        }
                        e => VendorError::Io(e.get_message().to_string()),
                    })
                    .and_then(|content| {
                        vendor_manager
                            .vendor_registry_url(&url, &content)
                            .map_err(|e| VendorError::Io(java_io_message(e)))
                    });
                if let Err(e) = vendored {
                    return Err(match e {
                        VendorError::Io(message) => VendorError::Io(format!(
                            "Failed to vendor registry URL {} at {}: {}",
                            url, output_path, message
                        )),
                        e => e,
                    });
                }
            }

            vendor_path_to_url.insert(output_path_lower_case, raw_url.clone());
        }

        // 2. Vendor repos
        let external_path = env
            .get_output_base()
            .get_relative(&blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION);
        let workspace: &Path = env
            .get_workspace()
            .expect("NullPointerException: getWorkspace()");
        blaze_repo_rules::bzlmod::vendor_manager::vendor_repos(
            vendor_manager,
            &external_path,
            workspace,
            repos_to_vendor,
        )
        .map_err(|e| VendorError::Io(java_io_message(e)))?;

        // 3. Invalidate RepositoryDirectoryValue for vendored repos.
        skyframe_executor.get_evaluator().delete(|k, _| {
            k.function_name() == RepositoryDirectoryKey::FUNCTION
                && k.downcast::<RepositoryDirectoryKey>()
                    .is_some_and(|k| repos_to_vendor.contains(&k.0))
        });
        Ok(())
    }
}

/// Java `IOException.getMessage()` of a file system error.
fn java_io_message(e: std::io::Error) -> String {
    blaze_repo_fetch::errors::IoException::from(e).get_message()
}

/// The repositories (`RepositoryDirectoryValue.Success` not excluded from vendoring) the graph
/// reaches from `target_keys`, in breadth-first order.
// Java: com/google/devtools/build/lib/bazel/commands/VendorCommand.java:309 VendorCommand.collectReposFromTargets
fn collect_repos_from_targets(
    skyframe_executor: &SkyframeExecutor,
    target_keys: &[SkyKey],
) -> Vec<RepositoryName> {
    let graph = skyframe_executor.get_walkable_graph();
    // ImmutableSet.Builder: first occurrence wins.
    let mut repos: indexmap::IndexSet<RepositoryName> = indexmap::IndexSet::new();
    let mut nodes: VecDeque<SkyKey> = target_keys.iter().cloned().collect();
    // Mark nodes as visited when they are enqueued.
    let mut visited: HashSet<SkyKey> = target_keys.iter().cloned().collect();
    while let Some(key) = nodes.pop_front() {
        if let Some(value) = graph.get_value(&key)
            && let Some(RepositoryDirectoryValue::Success {
                exclude_from_vendoring: false,
                ..
            }) = value.downcast_ref::<RepositoryDirectoryValue>()
        {
            repos.insert(
                key.downcast::<RepositoryDirectoryKey>()
                    .expect("ClassCastException: RepositoryName")
                    .0
                    .clone(),
            );
        }
        for dep_key in graph.get_direct_deps(&key) {
            if visited.insert(dep_key.clone()) {
                nodes.push_back(dep_key);
            }
        }
    }
    repos.into_iter().collect()
}
