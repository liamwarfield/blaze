//! Project files (`PROJECT.scl`): ports of `skyframe/ProjectFilesLookupFunction.java`,
//! `skyframe/ProjectFilesLookupValue.java`, `analysis/Project.findProjectFiles` and the part of
//! `buildtool/AnalysisPhaseRunner.evaluateProjectFile` that decides whether a build has project
//! files.
//!
//! PORT: finding the project files of a build is ported. Reading them (`ProjectFunction`,
//! `ProjectValue`, aliases, `Project.applySclConfig`, `FlagSetFunction`, the active directories
//! matcher) is not: a build whose targets resolve to a project file fails with `unported!`. A
//! build without project files goes on unchanged, as in Java.

use std::fmt;
use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::remote_analysis_caching_options::{
    RemoteAnalysisCacheMode, RemoteAnalysisCachingOptions,
};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_sky_loading::actions::FileValue;
use blaze_sky_loading::containing_package_lookup::ContainingPackageLookupKey;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::io::IOException;
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Key, SkyFunctionException, SkyFunctionName, SkyKey,
};
use blaze_vfs::RootedPath;
use indexmap::IndexSet;

use crate::skyframe_executor::{DEFAULT_THREAD_COUNT, SkyframeExecutor};

/// Name of project metadata files. See `Project`.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:35 ProjectFilesLookupFunction.PROJECT_FILE_NAME
pub const PROJECT_FILE_NAME: &str = "PROJECT.scl";

/// `SkyFunctions.PROJECT_FILES_LOOKUP`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:45 SkyFunctions.PROJECT_FILES_LOOKUP
pub const PROJECT_FILES_LOOKUP: SkyFunctionName = SkyFunctionName("PROJECT_FILES_LOOKUP");

/// The key of a `ProjectFilesLookupValue`: a package.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:77 ProjectFilesLookupValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectFilesLookupKey(pub PackageIdentifier);

impl ProjectFilesLookupKey {
    // Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:59 ProjectFilesLookupValue.key
    pub fn key(id: PackageIdentifier) -> Self {
        ProjectFilesLookupKey(id)
    }
}

impl fmt::Debug for ProjectFilesLookupKey {
    // Java: com/google/devtools/build/skyframe/AbstractSkyKey.java:59 AbstractSkyKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", PROJECT_FILES_LOOKUP, self.0)
    }
}

impl Key for ProjectFilesLookupKey {
    type Value = ProjectFilesLookupValue;
    const FUNCTION: SkyFunctionName = PROJECT_FILES_LOOKUP;
}

/// The project files of a package: every `PROJECT.scl` in the package and in the packages above
/// it.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:33 ProjectFilesLookupValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFilesLookupValue {
    project_files: Vec<Label>,
}

impl ProjectFilesLookupValue {
    /// Returns the project files for the corresponding key's package.
    ///
    /// Order is innermost to outermost: if both `a/PROJECT.scl` and `a/b/c/PROJECT.scl` are
    /// included, `a/b/c/PROJECT.scl` appears first.
    // Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:49 ProjectFilesLookupValue.getProjectFiles
    pub fn get_project_files(&self) -> &[Label] {
        &self.project_files
    }
}

/// A project file exists but isn't a file.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:118 ProjectFilesLookupFunction.UnexpectedProjectFileTypeException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct UnexpectedProjectFileTypeException(pub String);

/// `SkyFunction` for `ProjectFilesLookupValue`.
///
/// PORT: Java keeps the directory being examined and the files found so far in a
/// `SkyKeyComputeState`, to go on where it stopped after a restart. A dependency request blocks
/// here, so the loop runs through; a second run (error bubbling) starts again from the key's
/// package and asks for the same dependencies.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:46 ProjectFilesLookupFunction.compute
fn project_files_lookup_function(
    key: &ProjectFilesLookupKey,
    env: &mut Environment<'_>,
) -> Result<Option<ProjectFilesLookupValue>, ComputeError> {
    // Which project files have we discovered so far?
    let mut project_files: Vec<Label> = Vec::new();
    // Tracks the current directory we're checking for BUILD and PROJECT.scl files. According to
    // the ProjectFilesLookupValue API contract, the original value from the skykey should be a
    // package (i.e. has a BUILD file). But this code doesn't require that: if the directory
    // doesn't have a BUILD file the code still gracefully finds the nearest enclosing package.
    // This is especially important when walking up the directory path, since the parent directory
    // of a package isn't necessarily another package.
    let mut current_dir: PackageIdentifier = key.0.clone();
    loop {
        let Some(innermost_pkg_lookup_value) =
            env.get_value(ContainingPackageLookupKey(current_dir.clone()))?
        else {
            return Ok(None);
        };
        if !innermost_pkg_lookup_value.has_containing_package() {
            // We've reached the root directory: nothing left. This list may be empty but that's
            // okay. That just means the input package has no associated project files.
            return Ok(Some(ProjectFilesLookupValue { project_files }));
        }

        // Now that we've found a BUILD file, determine the project file path to look for.
        let innermost_pkg_id = innermost_pkg_lookup_value
            .get_containing_package_name()
            .clone();
        let project_file_label =
            match Label::create_in_package(&innermost_pkg_id, PROJECT_FILE_NAME) {
                Ok(label) => label,
                Err(e) => panic!(
                    "IllegalStateException: Unexpected failure parsing {}: {}",
                    PROJECT_FILE_NAME, e
                ),
            };

        // Lookup the project file.
        let project_file_path = innermost_pkg_id
            .get_package_fragment()
            .get_relative_str(PROJECT_FILE_NAME);
        let file_sky_key = FileKey::key(RootedPath::to_rooted_path(
            innermost_pkg_lookup_value
                .get_containing_package_root()
                .clone(),
            &project_file_path,
        ));
        let file_value: Option<Arc<FileValue>> =
            match env.get_value_or_throw::<FileKey, IOException>(file_sky_key)? {
                Ok(v) => v,
                // ProjectFilesLookupException(IOException)
                Err(e) => return Err(SkyFunctionException::persistent(e).into()),
            };
        let Some(file_value) = file_value else {
            return Ok(None);
        };

        if file_value.is_file() {
            project_files.push(project_file_label);
        } else if file_value.exists() {
            // ProjectFilesLookupException(UnexpectedProjectFileTypeException)
            return Err(
                SkyFunctionException::persistent(UnexpectedProjectFileTypeException(format!(
                    "{} isn't a file",
                    project_file_path.get_path_string()
                )))
                .into(),
            );
        }

        let Some(parent_dir) = innermost_pkg_id
            .get_package_fragment()
            .get_parent_directory()
        else {
            // Hit the root directory. Returns the results we've collected.
            return Ok(Some(ProjectFilesLookupValue { project_files }));
        };
        current_dir = PackageIdentifier::create(current_dir.get_repository().clone(), parent_dir);
    }
}

/// Registers `PROJECT_FILES_LOOKUP`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder.typed_function::<ProjectFilesLookupKey, _>(project_files_lookup_function)
}

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
