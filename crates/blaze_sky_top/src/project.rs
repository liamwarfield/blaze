//! Project files (`PROJECT.scl`): ports of `skyframe/ProjectFilesLookupFunction.java`,
//! `skyframe/ProjectFilesLookupValue.java`, `analysis/Project.findProjectFiles` and the part of
//! `buildtool/AnalysisPhaseRunner.evaluateProjectFile` that decides whether a build has project
//! files.
//!
//! PORT: PROJECT_FILES_LOOKUP and PROJECT (`ProjectFunction`, `ProjectValue`) are ported in
//! `blaze_sky_loading::project_function` (BUILD_OPTIONS_SCOPE needs them below this crate).
//! Applying a project's configuration to a build (`Project.applySclConfig`, `FlagSetFunction`)
//! is not: a build whose targets resolve to a project file fails with `unported!`. A build
//! without project files goes on unchanged, as in Java.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::Label;
use blaze_events::Event;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::remote_analysis_caching_options::{
    RemoteAnalysisCacheMode, RemoteAnalysisCachingOptions,
};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_sky_analysis::flag_set_function::{ConfigFlagDefinitions, FlagSetKey, FlagSetValue};
use blaze_sky_loading::project_function::ProjectKey;
use blaze_skyframe::SkyKey;
use indexmap::{IndexMap, IndexSet};

use crate::skyframe_executor::{DEFAULT_THREAD_COUNT, SkyframeExecutor};

pub use blaze_sky_loading::project_function::{
    PROJECT_FILE_NAME, PROJECT_FILES_LOOKUP, ProjectFilesLookupKey, ProjectFilesLookupValue,
    UnexpectedProjectFileTypeException, register,
};

/// Thrown when project data can't be read.
// Java: com/google/devtools/build/lib/analysis/Project.java:80 Project.ProjectResolutionException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProjectResolutionException(pub String);

/// Finds and returns the project files for a set of build targets.
///
/// This walks up each target's package path looking for `PROJECT_FILE_NAME` files. For example,
/// for `//foo/bar/baz:mytarget`, this might look in `foo/bar/baz`, `foo/bar`, and `foo` ("might"
/// because it skips directories that don't have BUILD files - those directories aren't
/// packages).
///
/// This method doesn't read project file content so it doesn't resolve project file aliases.
///
/// Returns a map from each target to its set of project files, ordered by reverse package
/// depth. So a project file in `foo/bar` appears before a project file in `foo`. Targets with
/// no matching project files aren't in the map.
// Java: com/google/devtools/build/lib/analysis/Project.java:272 Project.findProjectFiles
pub fn find_project_files(
    targets: &IndexSet<Label>,
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<Vec<(Label, Vec<Label>)>, ProjectResolutionException> {
    // TODO: b/324127050 - Support other repos.
    let targets_to_sky_keys: Vec<(Label, ProjectFilesLookupKey)> = targets
        .iter()
        .map(|target| {
            (
                target.clone(),
                ProjectFilesLookupKey::key(target.get_package_identifier().clone()),
            )
        })
        .collect();
    // evaluateSkyKeys(eventHandler, keys, keepGoing=false): distinct keys.
    let sky_keys: IndexSet<SkyKey> = targets_to_sky_keys
        .iter()
        .map(|(_, key)| SkyKey::new(key.clone()))
        .collect();
    let sky_keys: Vec<SkyKey> = sky_keys.into_iter().collect();
    let eval_result = match skyframe_executor.evaluate(
        &sky_keys,
        /* keep_going= */ false,
        DEFAULT_THREAD_COUNT,
        event_handler,
    ) {
        Ok(result) => result,
        // Java: evaluateSkyKeys turns an interrupt into an IllegalStateException.
        Err(_) => panic!("IllegalStateException: java.lang.InterruptedException"),
    };
    if eval_result.has_error() {
        // ProjectResolutionException(String msg, Throwable cause): getMessage() is msg.
        return Err(ProjectResolutionException(
            "Error finding project files".to_owned(),
        ));
    }

    // ImmutableMultimap.Builder: keys in first-insertion order; a target without project files
    // has no entry.
    let mut ans: Vec<(Label, Vec<Label>)> = Vec::new();
    for (target, key) in &targets_to_sky_keys {
        let containing_projects = eval_result
            .get(key)
            .unwrap_or_else(|| panic!("NullPointerException: {:?}", key));
        if !containing_projects.get_project_files().is_empty() {
            ans.push((
                target.clone(),
                containing_projects.get_project_files().to_vec(),
            ));
        }
    }
    Ok(ans)
}

/// What `evaluateProjectFile` throws: `LoadingFailedException`.
// Java: com/google/devtools/build/lib/pkgcache/LoadingFailedException.java:27 LoadingFailedException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct LoadingFailedException {
    pub message: String,
    /// `getDetailedExitCode().getExitCode()`.
    pub exit_code: i32,
    /// `getDetailedExitCode().getFailureDetail()`.
    pub failure_detail: blaze_protos::failure_details::FailureDetail,
}

/// The inputs of `Project.applySclConfig` that come from the command: every native option name
/// (`"name"` and, for boolean options, `"noname"`), the options users set (canonical form ->
/// the `--config` it was expanded from, or empty), and the `--config` definitions of the rc
/// files.
#[derive(Clone, Default)]
pub struct ProjectOptionsContext {
    pub all_option_names: Arc<IndexSet<String>>,
    pub user_options: Arc<IndexMap<String, String>>,
    pub config_flag_definitions: Arc<ConfigFlagDefinitions>,
}

/// The active PROJECT.scls for this build.
// Java: com/google/devtools/build/lib/analysis/Project.java:93 Project.ActiveProjects
#[derive(Clone, Debug)]
pub struct ActiveProjects {
    /// PROJECT.scls to the targets that resolve to them.
    pub project_files_to_target_labels: IndexMap<Label, Vec<Label>>,
    /// True if some of this build's targets have PROJECT.scls and others don't.
    pub partial_project_build: bool,
    /// A descriptive message explaining how different targets resolve to different PROJECT.scls.
    pub different_projects_details: String,
}

impl ActiveProjects {
    // Java: com/google/devtools/build/lib/analysis/Project.java:97 Project.ActiveProjects.isEmpty
    pub fn is_empty(&self) -> bool {
        self.project_files_to_target_labels.is_empty()
    }

    /// User-friendly description of this build type, for consumer info/error messaging.
    // Java: com/google/devtools/build/lib/analysis/Project.java:102 Project.ActiveProjects.buildType
    pub fn build_type(&self) -> &'static str {
        if self.project_files_to_target_labels.len() > 1 {
            "multi-project build"
        } else if self.partial_project_build {
            "build where only some targets have projects"
        } else if self.project_files_to_target_labels.len() == 1 {
            "single-project build"
        } else {
            "build with no projects"
        }
    }
}

/// Returns the canonical project files for a set of targets.
///
/// If a target matches multiple project files (like `a/PROJECT.scl` and `a/b/PROJECT.scl`),
/// only the innermost is considered.
// TODO: b/324127375 - Support hierarchical project files: [foo/project.scl, foo/bar/project.scl].
// Java: com/google/devtools/build/lib/analysis/Project.java:129 Project.getProjectFiles
pub fn get_project_files(
    targets: &IndexSet<Label>,
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<ActiveProjects, ProjectResolutionException> {
    // Map targets to their innermost matching project file. Omits targets with no project files.
    // findProjectFiles returns all project files up a target's path (and omits targets with
    // no project files). We just use the first entry, which is the innermost file.
    let targets_to_project_files: IndexMap<Label, Label> =
        find_project_files(targets, skyframe_executor, event_handler.clone())?
            .into_iter()
            .map(|(target, files)| (target, files[0].clone()))
            .collect();
    if targets_to_project_files.is_empty() {
        // None of the targets have project files.
        return Ok(ActiveProjects {
            project_files_to_target_labels: IndexMap::new(),
            partial_project_build: false,
            different_projects_details: String::new(),
        });
    }
    // Sets.difference(ImmutableSet.copyOf(targets), targetsToProjectFiles.keySet())
    let targets_with_no_project_files: Vec<Label> = targets
        .iter()
        .filter(|t| !targets_to_project_files.contains_key(*t))
        .cloned()
        .collect();

    // Since project files can be aliases to other files, we need to parse them to potentially
    // remap to their references. Also remember the targets that resolved to that project file
    // for clean error reporting.
    // Multimaps.invertFrom(forMap(target -> key), LinkedListMultimap.create())
    let mut project_file_keys_to_targets: IndexMap<ProjectKey, Vec<Label>> = IndexMap::new();
    for (target, project_file) in &targets_to_project_files {
        project_file_keys_to_targets
            .entry(ProjectKey::new(project_file.clone()))
            .or_default()
            .push(target.clone());
    }
    // Load project file content from Skyframe.
    let sky_keys: Vec<SkyKey> = project_file_keys_to_targets
        .keys()
        .map(|k| SkyKey::new(k.clone()))
        .collect();
    let eval_result = match skyframe_executor.evaluate(
        &sky_keys,
        /* keep_going= */ false,
        DEFAULT_THREAD_COUNT,
        event_handler.clone(),
    ) {
        Ok(result) => result,
        // Java: evaluateSkyKeys turns an interrupt into an IllegalStateException.
        Err(_) => panic!("IllegalStateException: java.lang.InterruptedException"),
    };
    if eval_result.has_error() {
        let message = eval_result
            .get_first_error()
            .and_then(|e| e.get_exception())
            .map(|e| e.to_string())
            .unwrap_or_default();
        return Err(ProjectResolutionException(format!(
            "error loading project files: {}",
            message
        )));
    }

    // De-duplicate the projectFileKeysToTargets keys by resolving project aliases.
    let mut canonical_projects_to_targets: IndexMap<Label, Vec<Label>> = IndexMap::new();
    for (key, key_targets) in &project_file_keys_to_targets {
        let value = eval_result
            .get(key)
            .unwrap_or_else(|| panic!("NullPointerException: {:?}", key));
        let actual = value
            .get_actual_project_file()
            .expect("NullPointerException: getActualProjectFile()")
            .clone();
        // LinkedHashMap.put replaces the value of an existing key.
        canonical_projects_to_targets.insert(actual, key_targets.clone());
    }

    if canonical_projects_to_targets.len() != 1 {
        // Targets resolve to different project files.
        let details = different_project_files_error(
            &canonical_projects_to_targets,
            &targets_with_no_project_files,
        );
        let partial =
            !canonical_projects_to_targets.is_empty() && !targets_with_no_project_files.is_empty();
        return Ok(ActiveProjects {
            project_files_to_target_labels: canonical_projects_to_targets,
            partial_project_build: partial,
            different_projects_details: details,
        });
    } else {
        let project_file = canonical_projects_to_targets.keys().next().unwrap();
        event_handler.handle(&Event::info(format!(
            "Reading project settings from {}.",
            project_file
        )));
    }

    if targets_with_no_project_files.is_empty() {
        // All targets resolve to the same canonical project file.
        Ok(ActiveProjects {
            project_files_to_target_labels: canonical_projects_to_targets,
            partial_project_build: false,
            different_projects_details: String::new(),
        })
    } else {
        // Some targets have project files and some don't.
        Ok(ActiveProjects {
            project_files_to_target_labels: canonical_projects_to_targets,
            partial_project_build: true,
            different_projects_details: String::new(),
        })
    }
}

/// User-friendly error message for when targets resolve to different project files or only some
/// targets have project files.
// Java: com/google/devtools/build/lib/analysis/Project.java:205 Project.differentProjectFilesError
fn different_project_files_error(
    canonical_projects_to_targets: &IndexMap<Label, Vec<Label>>,
    targets_with_no_project_files: &[Label],
) -> String {
    let mut msg_builder = String::from("Targets have different project settings:");
    // Maximum number of "//foo:target -> //foo:PROJECT.scl" entries to show.
    const MAX_TO_SHOW: usize = 5;
    // Iterate through each project file group (and also the "no project file" group), adding one
    // entry from each group until we reach the max. This samples each group as evenly as
    // possible.
    // (group key `None` is the "no project file" group; iterators as positions.)
    let mut groups: Vec<(Option<&Label>, &[Label])> = canonical_projects_to_targets
        .iter()
        .map(|(k, v)| (Some(k), v.as_slice()))
        .collect();
    if !targets_with_no_project_files.is_empty() {
        groups.push((None, targets_with_no_project_files));
    }
    let mut grouped_results: IndexMap<Option<&Label>, Vec<&Label>> = IndexMap::new();
    let mut positions: IndexMap<usize, usize> = (0..groups.len()).map(|i| (i, 0)).collect();
    let mut total_results = 0;
    let mut next_group_iteration = positions.clone();
    while total_results < MAX_TO_SHOW && !next_group_iteration.is_empty() {
        positions = next_group_iteration;
        next_group_iteration = IndexMap::new();
        let keys: Vec<usize> = positions.keys().copied().collect();
        let mut broke = false;
        for g in keys {
            let pos = positions[&g];
            let (key, targets) = groups[g];
            grouped_results.entry(key).or_default().push(&targets[pos]);
            positions.insert(g, pos + 1);
            if pos + 1 < targets.len() {
                next_group_iteration.insert(g, pos + 1);
            }
            total_results += 1;
            if total_results == MAX_TO_SHOW {
                broke = true;
                break;
            }
        }
        if broke {
            break;
        }
    }
    // Report results grouped by PROJECT file.
    for (key, group_targets) in &grouped_results {
        let project_file = match key {
            None => "no project file".to_owned(),
            Some(l) => l.to_string(),
        };
        for target in group_targets {
            msg_builder.push_str(&format!("\n  - {} -> {}", target, project_file));
        }
    }
    // projectFileToNextTarget.values().stream().mapToInt(Iterators::size).sum(): the targets
    // left in the last iteration's iterators.
    let results_left: usize = positions
        .iter()
        .map(|(g, pos)| groups[*g].1.len() - pos)
        .sum();
    if results_left > 0 {
        msg_builder.push_str(&format!("\n  (...and {} more)", results_left));
    }
    msg_builder
}

/// Returns the build options to add to this invocation from its active project files and
/// `--scl_config` setting.
// Java: com/google/devtools/build/lib/analysis/Project.java:324 Project.applySclConfig
pub fn apply_scl_config(
    from_options: &BuildOptions,
    active_projects: &ActiveProjects,
    scl_config: &str,
    context: &ProjectOptionsContext,
    enforce_canonical_configs: bool,
    event_handler: Arc<dyn ExtendedEventHandler>,
    skyframe_executor: &SkyframeExecutor,
) -> Result<IndexSet<String>, InvalidConfigurationException> {
    let from_options = Arc::new(from_options.clone());
    let flag_set_keys: Vec<FlagSetKey> = active_projects
        .project_files_to_target_labels
        .iter()
        .map(|(p, targets)| FlagSetKey {
            targets: targets.iter().cloned().collect(),
            project_file: p.clone(),
            scl_config: scl_config.to_owned(),
            target_options: from_options.clone(),
            all_option_names: context.all_option_names.clone(),
            user_options: context.user_options.clone(),
            config_flag_definitions: context.config_flag_definitions.clone(),
            enforce_canonical: enforce_canonical_configs,
        })
        .collect();
    let sky_keys: Vec<SkyKey> = flag_set_keys
        .iter()
        .map(|k| SkyKey::new(k.clone()))
        .collect();
    let result = match skyframe_executor.evaluate(
        &sky_keys,
        /* keep_going= */ false,
        DEFAULT_THREAD_COUNT,
        event_handler.clone(),
    ) {
        Ok(result) => result,
        Err(_) => panic!("IllegalStateException: java.lang.InterruptedException"),
    };
    if result.has_error() {
        let message = result
            .get_first_error()
            .and_then(|e| e.get_exception())
            .map(|e| e.to_string())
            .unwrap_or_default();
        return Err(InvalidConfigurationException::with_code(
            format!("Cannot parse options: {}", message),
            "INVALID_BUILD_OPTIONS",
        ));
    }

    // Permit multiple configs as long as they all produce the same value, ignoring projects
    // with no project files.
    // ImmutableSet<ImmutableSet<String>>: sets compare without order.
    // PORT: java-hash-order. `result.values()` iterates the `EvaluationResult`'s `HashMap`
    // keyed by `FlagSetValue.Key`, so the value whose messages are printed below is the first
    // key in that map's order. The key's `userOptions` hold the client's
    // `--startup_time=<ms>` (blaze.cc), so with several projects the oracle's choice varies from
    // run to run (12 oracle runs of two-project builds, 2026-11-19: both projects seen); any
    // order is one the oracle can print.
    // Java: com/google/devtools/build/skyframe/EvaluationResult.java:157 EvaluationResult.Builder
    let ordered_keys = blaze_util::profiler::java_hash_order::java_hash_map_order(
        &flag_set_keys,
        |k| k.java_hash_code(),
        blaze_util::profiler::java_hash_order::Insertion::Put,
    );
    let values: Vec<Arc<FlagSetValue>> = ordered_keys
        .iter()
        .map(|k| result.get(k).expect("FLAG_SET value"))
        .collect();
    let mut unique_configs: Vec<&IndexSet<String>> = Vec::new();
    for v in &values {
        let flags = v.get_options_from_flagset();
        if !unique_configs.iter().any(|u| **u == *flags) {
            unique_configs.push(flags);
        }
    }
    if unique_configs.len() > 1 {
        if !scl_config.is_empty() {
            return Err(InvalidConfigurationException::with_code(
                format!(
                    "--scl_config={} resolves to conflicting flagsets: {}",
                    scl_config, active_projects.different_projects_details
                ),
                "INVALID_BUILD_OPTIONS",
            ));
        }
        return Err(InvalidConfigurationException::with_code(
            format!(
                "Mismatching default configs for a {}. {}",
                active_projects.build_type(),
                active_projects.different_projects_details
            ),
            "INVALID_BUILD_OPTIONS",
        ));
    }

    // result.values().iterator().next()
    let value = &values[0];
    for event in value.get_persistent_messages() {
        event_handler.handle(event);
    }
    // Options from the selected project config.
    Ok(value.get_options_from_flagset().clone())
}

/// `AnalysisPhaseRunner.ProjectEvaluationResult` (without the active directories matcher, which
/// only remote analysis caching and Skyfocus read).
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:183 AnalysisPhaseRunner.ProjectEvaluationResult
// (the builder is `Default` and the fields)
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:194 AnalysisPhaseRunner.ProjectEvaluationResult.builder
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:200 AnalysisPhaseRunner.ProjectEvaluationResult.Builder.buildOptions
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:204 AnalysisPhaseRunner.ProjectEvaluationResult.Builder.projectFile
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:206 AnalysisPhaseRunner.ProjectEvaluationResult.Builder.build
// PORT: skipped, the matcher only serves remote analysis caching and Skyfocus (see above): AnalysisPhaseRunner.ProjectEvaluationResult.Builder.activeDirectoriesMatcher
// PORT: skipped, the private constructor of a static class: AnalysisPhaseRunner.AnalysisPhaseRunner
#[derive(Clone, Debug, Default)]
pub struct ProjectEvaluationResult {
    pub build_options: IndexSet<String>,
    pub project_file: Option<Label>,
}

/// What `evaluateProjectFile` throws.
#[derive(Debug)]
pub enum ProjectEvaluationError {
    LoadingFailed(LoadingFailedException),
    InvalidConfiguration(InvalidConfigurationException),
}

/// Evaluates project files and sets up `ProjectEvaluationResult`.
///
/// PORT: `SKYFOCUS` is not a feature here (Skyfocus is not ported; its flags fail where they are
/// read), and remote analysis caching fails loudly, so the active directories matcher is never
/// computed.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:226 AnalysisPhaseRunner.evaluateProjectFile
pub fn evaluate_project_file(
    remote_analysis_caching_options: Option<&RemoteAnalysisCachingOptions>,
    build_request_options: &BuildRequestOptions,
    build_options: &BuildOptions,
    non_expanded_labels: &IndexSet<Label>,
    context: &ProjectOptionsContext,
    skyframe_executor: &SkyframeExecutor,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<ProjectEvaluationResult, ProjectEvaluationError> {
    // The remote analysis caching features (ANALYSIS_CACHING_UPLOAD, ANALYSIS_CACHING_DOWNLOAD).
    // PORT: Java reads the mode only for a command whose build phase executes; another command
    // that sets the mode fails here too.
    if let Some(remote_analysis_caching_options) = remote_analysis_caching_options
        && !matches!(
            remote_analysis_caching_options.mode,
            RemoteAnalysisCacheMode::OFF
        )
    {
        blaze_util::unported!(
            "AnalysisPhaseRunner.evaluateProjectFile",
            "--experimental_remote_analysis_cache_mode (remote analysis caching)"
        );
    }

    let mut scl_config_feature = false;
    let scl_config: Option<String> = build_options
        .get::<CoreOptions>()
        .expect("CoreOptions")
        .scl_config
        .clone();
    // request.getBuildOptions().enforceProjectConfigs
    let enforce_project_configs = build_request_options.enforce_project_configs;
    // !Strings.isNullOrEmpty(sclConfig) || enforceProjectConfigs
    if scl_config.as_deref().is_some_and(|s| !s.is_empty()) || enforce_project_configs {
        scl_config_feature = true;
    }

    if !scl_config_feature {
        return Ok(ProjectEvaluationResult::default());
    }

    let active_projects =
        match get_project_files(non_expanded_labels, skyframe_executor, reporter.clone()) {
            Ok(p) => p,
            Err(e) => {
                // DetailedExitCode.of(ExitCode.PARSING_FAILURE, FailureDetail.getDefaultInstance())
                return Err(ProjectEvaluationError::LoadingFailed(
                    LoadingFailedException {
                        message: e.0,
                        exit_code: 1,
                        failure_detail: blaze_protos::failure_details::FailureDetail::default(),
                    },
                ));
            }
        };

    let mut result = ProjectEvaluationResult::default();
    if !active_projects.is_empty() {
        // Do not apply canonical configurations if there are no project files.
        let options = apply_scl_config(
            build_options,
            &active_projects,
            scl_config.as_deref().unwrap_or(""),
            context,
            enforce_project_configs,
            reporter,
            skyframe_executor,
        )
        .map_err(ProjectEvaluationError::InvalidConfiguration)?;
        result.build_options = options;
        result.project_file = active_projects
            .project_files_to_target_labels
            .keys()
            .next()
            .cloned();
    }
    Ok(result)
}
