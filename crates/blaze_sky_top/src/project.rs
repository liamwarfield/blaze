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
use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::remote_analysis_caching_options::{
    RemoteAnalysisCacheMode, RemoteAnalysisCachingOptions,
};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_skyframe::SkyKey;
use indexmap::IndexSet;

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

/// Evaluates project files and sets up `ProjectEvaluationResult`.
///
/// PORT: returns nothing: the result of a build without project files is the empty
/// `ProjectEvaluationResult` (no build options, no active directories matcher, no project file),
/// and a build with project files is not ported (module comment). `SKYFOCUS` is not a feature
/// here (Skyfocus is not ported; its flags fail where they are read).
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:226 AnalysisPhaseRunner.evaluateProjectFile
pub fn evaluate_project_file(
    remote_analysis_caching_options: Option<&RemoteAnalysisCachingOptions>,
    build_request_options: &BuildRequestOptions,
    build_options: &BuildOptions,
    non_expanded_labels: &IndexSet<Label>,
    skyframe_executor: &SkyframeExecutor,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<(), LoadingFailedException> {
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
        return Ok(());
    }

    // Project.getProjectFiles(targetPatternPhaseValue.getNonExpandedLabels(), ...)
    // Java: com/google/devtools/build/lib/analysis/Project.java:129 Project.getProjectFiles
    // Map targets to their innermost matching project file. Omits targets with no project files.
    let targets_to_project_files =
        match find_project_files(non_expanded_labels, skyframe_executor, reporter) {
            Ok(files) => files,
            Err(e) => {
                // DetailedExitCode.of(ExitCode.PARSING_FAILURE, FailureDetail.getDefaultInstance())
                return Err(LoadingFailedException {
                    message: e.0,
                    exit_code: 1,
                    failure_detail: blaze_protos::failure_details::FailureDetail::default(),
                });
            }
        };

    if targets_to_project_files.is_empty() {
        // None of the targets have project files: `ActiveProjects.isEmpty()`, so no canonical
        // configuration is applied ("Do not apply canonical configurations if there are no
        // project files").
        return Ok(());
    }

    blaze_util::unported!(
        "Project.getProjectFiles",
        "project files of this build's targets (ProjectFunction, Project.applySclConfig): {}",
        targets_to_project_files
            .iter()
            .map(|(target, files)| format!("{} -> {}", target, files[0]))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
