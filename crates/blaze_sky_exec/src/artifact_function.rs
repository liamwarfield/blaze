//! `ArtifactFunction` (source and tree artifacts) and `ArtifactNestedSetFunction`.

use std::fmt;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_exec::actions::{FileArtifactValue, TreeArtifactValue};
use blaze_protos::failure_details::{self, FailureDetail, execution, failure_detail};
use blaze_skyframe::{ComputeError, Environment, SkyFunctionException, SkyKey, Transience};
use blaze_vfs::Symlinks;

use crate::keys::{ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, ArtifactNestedSetValue};

/// The value of an [`ArtifactKey`]: a `FileArtifactValue` (source files), `MissingArtifactValue`
/// or a `TreeArtifactValue`.
#[derive(Clone, Debug)]
pub enum ArtifactValue {
    File(FileArtifactValue),
    Missing(MissingArtifactValue),
    Tree(Arc<TreeArtifactValue>),
}

/// A `SkyValue` representing a missing input file.
// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:80 ArtifactFunction.MissingArtifactValue
#[derive(Clone, Debug)]
pub struct MissingArtifactValue {
    failure_detail: FailureDetail,
}

impl MissingArtifactValue {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:83 MissingArtifactValue.<init>
    pub fn new(missing_artifact: &Artifact) -> MissingArtifactValue {
        MissingArtifactValue {
            failure_detail: execution_failure(
                construct_error_message(missing_artifact, "missing input file"),
                execution::Code::SourceInputMissing,
            ),
        }
    }

    /// `getDetailedExitCode().getFailureDetail()`.
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

/// An exception thrown representing a source input `IOException`.
// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:514 ArtifactFunction.SourceArtifactException
#[derive(Clone, Debug)]
pub struct SourceArtifactException {
    failure_detail: FailureDetail,
}

impl SourceArtifactException {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:522 SourceArtifactException.create
    pub fn create(artifact: &Artifact, e: &std::io::Error) -> SourceArtifactException {
        SourceArtifactException {
            failure_detail: execution_failure(
                format!(
                    "{}: {}",
                    construct_error_message(artifact, "error reading file"),
                    e
                ),
                execution::Code::SourceInputIoException,
            ),
        }
    }

    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }
}

impl fmt::Display for SourceArtifactException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.failure_detail.message)
    }
}

impl std::error::Error for SourceArtifactException {}

/// A `FailureDetail` with an `Execution` code.
pub fn execution_failure(message: String, code: execution::Code) -> FailureDetail {
    FailureDetail {
        message,
        category: Some(failure_detail::Category::Execution(
            failure_details::Execution { code: code as i32 },
        )),
    }
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:403 ArtifactFunction.constructErrorMessage
pub fn construct_error_message(artifact: &Artifact, error: &str) -> String {
    let owner_label = artifact.get_owner();
    let owner_label = match owner_label {
        Some(l) if l.get_name() != "." => l,
        // Discovered inputs may not have an owner. Directory source artifacts may be owned by a
        // label ':.' which will crash toPathFragment below.
        _ => return format!("{} '{}'", error, artifact.get_exec_path_string()),
    };
    let label_fragment = owner_label.to_path_fragment();
    if owner_label.get_repository().is_main() {
        if &label_fragment == artifact.get_exec_path() {
            // No additional useful information from path.
            return format!("{error} '{owner_label}'");
        }
    } else {
        // Not worth threading sibling repository layout config value all the way here: if either
        // match, we know the label isn't useful.
        for sibling_repository_layout in [false, true] {
            if &owner_label
                .get_repository()
                .get_exec_path(sibling_repository_layout)
                .get_relative(&label_fragment)
                == artifact.get_exec_path()
            {
                return format!("{error} '{owner_label}'");
            }
        }
    }
    // PORT: BugReport.sendBugReport is not ported (logging only).
    format!(
        "{} '{}', owner: '{}'",
        error,
        artifact.get_exec_path_string(),
        owner_label
    )
}

/// `ArtifactFunction.compute`.
// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:131 ArtifactFunction.compute
pub fn compute_artifact(
    key: &ArtifactKey,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactValue>, ComputeError> {
    let artifact = &key.0;
    if !artifact.has_known_generating_action() {
        // If the artifact has no known generating action, it is a source artifact and is never
        // cached remotely.
        return create_source_value(artifact, env);
    }
    // PORT: remote analysis caching (retrieveRemoteSkyValue) and action templates
    // (ArtifactDependencies.maybeGetTemplateActionForTreeArtifact) are not ported.
    let generating_action_key = artifact.get_generating_action_key().clone();
    let Some(action_value) = env.get_value(ActionExecutionKey(generating_action_key))? else {
        return Ok(None);
    };
    blaze_util::check_state!(
        artifact.is_tree_artifact(),
        "'{}' used as a key for ArtifactFunction is not a tree artifact or a source artifact",
        artifact
    );
    // We got a request for the whole tree artifact. We can just return the associated
    // TreeArtifactValue.
    let tree = action_value
        .get_tree_artifact_value(artifact)
        .unwrap_or_else(|| panic!("NullPointerException: {artifact}"))
        .clone();
    Ok(Some(ArtifactValue::Tree(tree)))
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:278 ArtifactFunction.createSourceValue
fn create_source_value(
    artifact: &Artifact,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactValue>, ComputeError> {
    env.check_interrupted()?;
    // PORT: Java requests FileValue.key(rootedPath) (FILE, in blaze_sky_loading) and builds the
    // metadata from it. Within a single non-incremental build the FILE value is a stat of the
    // same path, so the path is stat'ed here directly (following symlinks, as FileValue does).
    let path = artifact.get_path();
    let stat = match path.stat_if_found(Symlinks::Follow) {
        Ok(s) => s,
        Err(e) => {
            return Err(SkyFunctionException::new(
                SourceArtifactException::create(artifact, &e),
                Transience::Persistent,
            )
            .into());
        }
    };
    let Some(stat) = stat else {
        return Ok(Some(ArtifactValue::Missing(MissingArtifactValue::new(
            artifact,
        ))));
    };
    if stat.is_directory() {
        // PORT: SourceDirectoryEvent is posted for BEP/metrics only; not ported.
    }
    // TrackSourceDirectoriesFlag.trackSourceDirectories() is false by default: directories get a
    // plain directory value.
    // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:183 FileArtifactValue.createForSourceArtifact
    match FileArtifactValue::create_from_stat(&path, &stat) {
        Ok(v) => Ok(Some(ArtifactValue::File(v))),
        Err(e) => Err(SkyFunctionException::new(
            SourceArtifactException::create(artifact, &e),
            Transience::Transient,
        )
        .into()),
    }
}

/// `ArtifactNestedSetFunction.compute`: requests the direct leaves and non-leaf children of the
/// nested set; their values are read by the consuming action from their own keys.
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:94 ArtifactNestedSetFunction.compute
pub fn compute_artifact_nested_set(
    key: &ArtifactNestedSetKey,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactNestedSetValue>, ComputeError> {
    let dep_keys: Vec<SkyKey> = key.get_direct_dep_keys();
    let result = env.get_values_and_exceptions(&dep_keys)?;
    // PORT: Java's ArtifactNestedSetFunction propagates the first ActionExecutionException /
    // SourceArtifactException of a child (getOrThrow) and returns null when values are missing.
    // Here a failed child makes the value missing, and its error reaches the parent through the
    // evaluator's error propagation (in_error_bubbling() is always false).
    for k in &dep_keys {
        let _ = result.get(k);
    }
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(ArtifactNestedSetValue))
}
