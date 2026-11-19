//! `ArtifactFunction` (source and tree artifacts) and `ArtifactNestedSetFunction`.

use std::fmt;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_exec::actions::{ActionExecutionException, FileArtifactValue, TreeArtifactValue};
use blaze_protos::failure_details::{self, FailureDetail, execution, failure_detail};
use blaze_skyframe::{
    ComputeError, Environment, SkyError, SkyFunctionException, SkyKey, SkyframeLookupResult,
    Transience,
};
use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::{RootedPath, Symlinks};

use crate::action_execution_value::ActionExecutionValue;
use crate::keys::{ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, ArtifactNestedSetValue};
use crate::recursive_filesystem_traversal_function::{
    DirectTraversalRoot, DirectoryArtifactTraversalRequest, RecursiveFilesystemTraversalException,
    RecursiveFilesystemTraversalExceptionType, TrackSourceDirectoriesFlag,
};

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

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:546 SourceArtifactException.create
    pub fn create_for_traversal(
        artifact: &Artifact,
        e: &RecursiveFilesystemTraversalException,
    ) -> SourceArtifactException {
        SourceArtifactException {
            failure_detail: execution_failure(
                format!(
                    "{}: {}",
                    construct_error_message(artifact, "error traversing directory"),
                    e.get_message()
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
    // PORT: skipped, tracing (BugReport.sendBugReport writes the Java log).
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
        // PORT: skipped, Google-internal (SourceDirectoryEvent has no subscriber in Bazel).
    }

    if !stat.is_directory() || !TrackSourceDirectoriesFlag::track_source_directories() {
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:183 FileArtifactValue.createForSourceArtifact
        return match FileArtifactValue::create_from_stat(&path, &stat) {
            Ok(v) => Ok(Some(ArtifactValue::File(v))),
            Err(e) => Err(SkyFunctionException::new(
                SourceArtifactException::create(artifact, &e),
                Transience::Transient,
            )
            .into()),
        };
    }
    // For directory artifacts that are not Filesets, we initiate a directory traversal here, and
    // compute a hash from the directory structure.
    // We rely on the guarantees of RecursiveFilesystemTraversalFunction for correctness.
    //
    // This approach may have unexpected interactions with --package_path. In particular, the exec
    // root is set up from the loading / analysis phase, and it is now too late to change it;
    // therefore, this may traverse a different set of files depending on which targets are built
    // at the same time and what the package-path layout is (this may be moot if there is only one
    // entry). Or this may return a set of files that's inconsistent with those actually available
    // to the action (for local execution).
    //
    // In the future, we need to make this result the source of truth for the files available to
    // the action so that we at least have consistency.
    let rooted_path =
        RootedPath::to_rooted_path_from_path(artifact.get_root().get_root().clone(), &path);
    let request = DirectoryArtifactTraversalRequest::create(
        DirectTraversalRoot::for_rooted_path(&rooted_path),
        /* skip_testing_for_subpackage= */ true,
        artifact.clone(),
    );
    let value = match env
        .get_value_or_throw::<DirectoryArtifactTraversalRequest, RecursiveFilesystemTraversalException>(
            request.clone(),
        )? {
        Ok(v) => v,
        Err(e) => {
            use RecursiveFilesystemTraversalExceptionType as Type;
            // Use a match to guarantee that if a new type is added, this stops compiling.
            match e.get_type() {
                Type::DanglingSymlink
                | Type::FileOperationFailure
                | Type::SymlinkCycleOrInfiniteExpansion
                | Type::CannotTraverseSourceDirectory
                | Type::CannotCrossPackageBoundary => {
                    return Err(SkyFunctionException::new(
                        SourceArtifactException::create_for_traversal(artifact, &e),
                        Transience::Persistent,
                    )
                    .into());
                }
                Type::InconsistentFilesystem | Type::DetailedIoException => {
                    return Err(SkyFunctionException::new(
                        SourceArtifactException::create_for_traversal(artifact, &e),
                        Transience::Transient,
                    )
                    .into());
                }
                Type::GeneratedPathConflict => {
                    panic!(
                        "IllegalStateException: Generated conflict in source tree: {} {:?} {:?}: {}",
                        artifact, stat, request, e
                    );
                }
            }
        }
    };
    let Some(value) = value else {
        return Ok(None);
    };
    let mut fp = Fingerprint::new();
    for file in value.get_transitive_files().to_list().iter() {
        fp.add_string(file.get_name_in_symlink_tree().get_path_string());
        fp.add_bytes(
            file.get_metadata()
                .get_digest()
                .unwrap_or_else(|| panic!("NullPointerException: digest of {file:?}")),
        );
    }
    Ok(Some(ArtifactValue::File(
        FileArtifactValue::create_for_directory_with_hash(fp.digest_and_reset()),
    )))
}

/// One `Pair<SkyKey, Exception>` of [`ArtifactNestedSetEvalException::get_nested_exceptions`]: a
/// dependency key and the `SourceArtifactException` or `ActionExecutionException` it failed with.
///
/// Equality is Java's `Pair.equals` over a key and an exception: exceptions compare by identity,
/// here the identity of the error object Skyframe holds for the dependency.
#[derive(Clone, Debug)]
pub struct SkyKeyAndException {
    key: SkyKey,
    exception: SkyError,
}

impl SkyKeyAndException {
    /// `getFirst()`.
    pub fn get_first(&self) -> &SkyKey {
        &self.key
    }

    /// `getSecond()` when it is a `SourceArtifactException`.
    pub fn get_source_artifact_exception(&self) -> Option<&SourceArtifactException> {
        self.exception.downcast_ref::<SourceArtifactException>()
    }

    /// `getSecond()` when it is an `ActionExecutionException`.
    pub fn get_action_execution_exception(&self) -> Option<&ActionExecutionException> {
        self.exception.downcast_ref::<ActionExecutionException>()
    }

    /// `Throwable.toString()` of `getSecond()`: the class name and the message.
    ///
    /// PORT: an `ActionExecutionException` that was already reported is Java's subclass
    /// `AlreadyReportedActionExecutionException`; the port keeps one type, so the base class name
    /// is printed.
    fn exception_to_string(&self) -> String {
        if let Some(e) = self.get_source_artifact_exception() {
            return format!(
                "com.google.devtools.build.lib.skyframe.ArtifactFunction$SourceArtifactException: {e}"
            );
        }
        if let Some(e) = self.get_action_execution_exception() {
            return format!(
                "com.google.devtools.build.lib.actions.ActionExecutionException: {}",
                e.get_message()
            );
        }
        panic!(
            "IllegalStateException: Unexpected exception type: {}, key: {:?}",
            self.exception, self.key
        )
    }

    fn exception_identity(&self) -> *const () {
        Arc::as_ptr(&self.exception) as *const ()
    }
}

impl PartialEq for SkyKeyAndException {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.exception_identity() == other.exception_identity()
    }
}

impl Eq for SkyKeyAndException {}

impl std::hash::Hash for SkyKeyAndException {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.exception_identity().hash(state);
    }
}

/// Bundles the exceptions from the evaluation of the children keys together.
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:145 ArtifactNestedSetFunction.ArtifactNestedSetEvalException
#[derive(Clone, Debug)]
pub struct ArtifactNestedSetEvalException {
    message: String,
    nested_exceptions: NestedSet<SkyKeyAndException>,
    catastrophic: bool,
}

impl ArtifactNestedSetEvalException {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:150 ArtifactNestedSetEvalException.<init>
    fn new(
        message: String,
        nested_exceptions: NestedSet<SkyKeyAndException>,
        catastrophic: bool,
    ) -> ArtifactNestedSetEvalException {
        ArtifactNestedSetEvalException {
            message,
            nested_exceptions,
            catastrophic,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:157 ArtifactNestedSetEvalException.getNestedExceptions
    pub fn get_nested_exceptions(&self) -> &NestedSet<SkyKeyAndException> {
        &self.nested_exceptions
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:161 ArtifactNestedSetEvalException.isCatastrophic
    pub fn is_catastrophic(&self) -> bool {
        self.catastrophic
    }
}

impl fmt::Display for ArtifactNestedSetEvalException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ArtifactNestedSetEvalException {}

/// What `getOrThrow(key, SourceArtifactException.class, ActionExecutionException.class,
/// ArtifactNestedSetEvalException.class)` throws for the key of an action input.
pub(crate) enum InputDepException {
    Source(SourceArtifactException, SkyError),
    Action(ActionExecutionException, SkyError),
    NestedSetEval(ArtifactNestedSetEvalException),
}

fn catch_input_dep_exception(e: &SkyError) -> Option<InputDepException> {
    if let Some(x) = e.downcast_ref::<SourceArtifactException>() {
        return Some(InputDepException::Source(x.clone(), e.clone()));
    }
    if let Some(x) = e.downcast_ref::<ActionExecutionException>() {
        return Some(InputDepException::Action(x.clone(), e.clone()));
    }
    e.downcast_ref::<ArtifactNestedSetEvalException>()
        .map(|x| InputDepException::NestedSetEval(x.clone()))
}

/// The value of the key of an action input.
pub(crate) enum InputDepValue {
    /// The value of an `ARTIFACT` key.
    Artifact(Arc<ArtifactValue>),
    /// The value of an `ACTION_EXECUTION` key (callers that need it read it from their own
    /// lookup result; here only its presence matters).
    #[allow(dead_code)]
    Action(Arc<ActionExecutionValue>),
    /// The value of an `ARTIFACT_NESTED_SET` key.
    NestedSet(ArtifactNestedSetValue),
}

/// `result.getOrThrow(key, SourceArtifactException.class, ActionExecutionException.class,
/// ArtifactNestedSetEvalException.class)` for the key of an action input (`Artifact.key(input)`
/// or an `ArtifactNestedSetKey`).
pub(crate) fn get_input_dep_or_throw(
    result: &SkyframeLookupResult,
    key: &SkyKey,
) -> Result<Option<InputDepValue>, InputDepException> {
    if let Some(k) = key.downcast::<ArtifactKey>() {
        return result
            .get_or_throw_with(k, catch_input_dep_exception)
            .map(|v| v.map(InputDepValue::Artifact));
    }
    if let Some(k) = key.downcast::<ActionExecutionKey>() {
        return result
            .get_or_throw_with(k, catch_input_dep_exception)
            .map(|v| v.map(InputDepValue::Action));
    }
    let k = key
        .downcast::<ArtifactNestedSetKey>()
        .unwrap_or_else(|| panic!("ClassCastException: {key:?} is not the key of an action input"));
    result
        .get_or_throw_with(k, catch_input_dep_exception)
        .map(|v| v.map(|v| InputDepValue::NestedSet(*v)))
}

/// `ArtifactNestedSetFunction.compute`: evaluates the direct elements of the nested set as
/// artifacts and the transitive ones as `ArtifactNestedSetKey`s.
// Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:54 ArtifactNestedSetFunction.compute
pub fn compute_artifact_nested_set(
    key: &ArtifactNestedSetKey,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactNestedSetValue>, ComputeError> {
    // consumedArtifactsTrackerSupplier.get() is null unless a remote output checker initializes
    // it (SkyframeExecutor.initializeConsumedArtifactsTracker); none is ported.
    let dep_keys: Vec<SkyKey> = key.get_direct_dep_keys();
    let deps_eval_result = env.get_values_and_exceptions(&dep_keys)?;

    let mut transitive_exceptions_builder: NestedSetBuilder<SkyKeyAndException> =
        NestedSetBuilder::stable_order();
    let mut catastrophic = false;
    let mut result = ArtifactNestedSetValue::ALL_PRESENT;

    // Throw a SkyFunctionException when a dep evaluation results in an exception.
    for key in &dep_keys {
        // Trigger the exception, if any.
        match get_input_dep_or_throw(&deps_eval_result, key) {
            Ok(None) => continue,
            Ok(Some(InputDepValue::NestedSet(value))) => {
                if value == ArtifactNestedSetValue::SOME_MISSING {
                    result = ArtifactNestedSetValue::SOME_MISSING;
                }
                continue;
            }
            Ok(Some(InputDepValue::Artifact(value))) => {
                if matches!(*value, ArtifactValue::Missing(_)) {
                    result = ArtifactNestedSetValue::SOME_MISSING;
                }
            }
            Ok(Some(InputDepValue::Action(_))) => {}
            Err(InputDepException::Source(_, e)) => {
                // SourceArtifactException is never catastrophic.
                transitive_exceptions_builder.add(SkyKeyAndException {
                    key: key.clone(),
                    exception: e,
                });
            }
            Err(InputDepException::Action(action_exception, e)) => {
                transitive_exceptions_builder.add(SkyKeyAndException {
                    key: key.clone(),
                    exception: e,
                });
                catastrophic |= action_exception.is_catastrophe();
            }
            Err(InputDepException::NestedSetEval(e)) => {
                catastrophic |= e.is_catastrophic();
                transitive_exceptions_builder.add_transitive(e.get_nested_exceptions());
            }
        }
    }

    if !transitive_exceptions_builder.is_empty() {
        let transitive_exceptions = transitive_exceptions_builder.build();
        // The NestedSet of exceptions is usually small, hence flattening won't be too costly.
        let first_sky_key_and_exception = transitive_exceptions.to_list()[0].clone();
        // Java: com/google/devtools/build/lib/skyframe/ArtifactNestedSetFunction.java:131 ArtifactNestedSetFunctionException
        return Err(SkyFunctionException {
            catastrophic,
            ..SkyFunctionException::new(
                ArtifactNestedSetEvalException::new(
                    format!(
                        "Error evaluating artifact nested set. First exception: {}, SkyKey: {:?}",
                        first_sky_key_and_exception.exception_to_string(),
                        first_sky_key_and_exception.get_first()
                    ),
                    transitive_exceptions,
                    catastrophic,
                ),
                Transience::Persistent,
            )
        }
        .into());
    }

    // This should only happen when all error handling is done.
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(result))
}
