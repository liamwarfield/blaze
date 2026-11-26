//! `ArtifactFunction` (source and tree artifacts) and `ArtifactNestedSetFunction`.

use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::event::Event;
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::as_action_template;
use blaze_exec::actions::{ActionExecutionException, FileArtifactValue, TreeArtifactValue};
use blaze_protos::failure_details::{self, FailureDetail, execution, failure_detail};
use blaze_skyframe::{
    ComputeError, Environment, SkyError, SkyFunctionException, SkyKey, SkyframeLookupResult,
    Transience,
};
use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::RootedPath;

use crate::action_execution_value::ActionExecutionValue;
use crate::action_template_expansion_value::ActionTemplateExpansionValue;
use crate::analysis_seam::{ActionLookupValue, AnalysisSeam};
use crate::keys::{ActionExecutionKey, ArtifactKey, ArtifactNestedSetKey, ArtifactNestedSetValue};
use crate::recursive_filesystem_traversal_function::{
    DirectTraversalRoot, DirectoryArtifactTraversalRequest, RecursiveFilesystemTraversalException,
    RecursiveFilesystemTraversalExceptionType, TrackSourceDirectoriesFlag,
};
use crate::skyframe_action_executor::SkyframeActionExecutor;

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

    /// [`SourceArtifactException::create`] for the `IOException` of a `FILE` dependency.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:522 SourceArtifactException.create
    pub fn create_from_message(artifact: &Artifact, message: &str) -> SourceArtifactException {
        SourceArtifactException {
            failure_detail: execution_failure(
                format!(
                    "{}: {}",
                    construct_error_message(artifact, "error reading file"),
                    message
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
    action_executor: &SkyframeActionExecutor,
    seam: &dyn AnalysisSeam,
    key: &ArtifactKey,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactValue>, ComputeError> {
    let artifact = &key.0;
    if !artifact.has_known_generating_action() {
        // If the artifact has no known generating action, it is a source artifact and is never
        // cached remotely.
        return create_source_value(action_executor, artifact, env);
    }
    // PORT: remote analysis caching (retrieveRemoteSkyValue) is not ported:
    // `RemoteAnalysisCachingDependenciesProvider.isRetrievalEnabled()` is false without it.

    let Some(artifact_dependencies) = ArtifactDependencies::discover_dependencies(
        artifact, seam, env, /* crash_if_action_owner_missing= */ true,
    )?
    else {
        return Ok(None);
    };

    // If the action is an ActionTemplate, we need to expand the ActionTemplate into concrete
    // actions, execute those actions in parallel and then aggregate the action execution results.
    if let Some(action_template) =
        artifact_dependencies.maybe_get_template_action_for_tree_artifact()
    {
        // mkdirForTreeArtifacts: () -> !skyframeActionExecutor.actionFileSystemType().inMemoryFileSystem()
        if !action_executor.action_file_system_type_in_memory_file_system() {
            mkdir_for_tree_artifact(artifact, env, action_template.as_ref())?;
        }
        return create_tree_artifact_value_from_action_key(&artifact_dependencies, env);
    }

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

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:186 ArtifactFunction.mkdirForTreeArtifact
fn mkdir_for_tree_artifact(
    artifact: &Artifact,
    env: &mut Environment<'_>,
    action_for_failure: &dyn Action,
) -> Result<(), ComputeError> {
    if let Err(e) = artifact.get_path().create_directory_and_parents() {
        let error_message = format!(
            "Failed to create output directory for TreeArtifact {}: {}",
            artifact.get_exec_path(),
            e
        );
        env.listener().handle(&Event::error_at(
            Some(action_for_failure.get_owner().get_location().clone()),
            error_message.clone(),
        ));
        // We could throw this as an IOException and expect our callers to catch and reprocess
        // it, but we know the action at fault, so we should be in charge.
        let code = execution_failure(
            error_message.clone(),
            execution::Code::TreeArtifactDirectoryCreationFailure,
        );
        // Java: ArtifactFunctionException(ActionExecutionException) is TRANSIENT.
        return Err(SkyFunctionException::new(
            ActionExecutionException::with_cause(
                error_message,
                &e,
                Some(action_for_failure),
                false,
                Some(code),
            ),
            Transience::Transient,
        )
        .into());
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:211 ArtifactFunction.createTreeArtifactValueFromActionKey
fn create_tree_artifact_value_from_action_key(
    artifact_dependencies: &ArtifactDependencies,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactValue>, ComputeError> {
    // Request the list of expanded action keys from the ActionTemplate.
    let Some(expanded_action_execution_keys) =
        artifact_dependencies.get_action_template_expansion_keys(env)?
    else {
        return Ok(None); // The expanded actions are not yet available.
    };

    let keys: Vec<SkyKey> = expanded_action_execution_keys
        .iter()
        .map(|k| SkyKey::new(ActionExecutionKey(k.clone())))
        .collect();
    let expanded_action_values = env.get_values_and_exceptions(&keys)?;
    if env.values_missing() {
        return Ok(None); // The execution values of the expanded actions are not yet all available.
    }

    // Aggregate the metadata for individual TreeFileArtifacts into a TreeArtifactValue for the
    // parent TreeArtifact.
    let parent = &artifact_dependencies.artifact;
    // TreeArtifactValue.newBuilder(parent) with putChild for each child, then build().
    let mut tree_children: Vec<(blaze_exec::actions::ActionInputRef, FileArtifactValue)> =
        Vec::new();

    for action_key in &expanded_action_execution_keys {
        let mut saw_tree_child = false;
        let Some(action_execution_value) =
            expanded_action_values.get_typed(&ActionExecutionKey(action_key.clone()))
        else {
            return Ok(None);
        };

        for (artifact, metadata) in action_execution_value.get_all_file_values() {
            blaze_util::check_state!(
                artifact.has_parent(),
                "Parentless artifact {} found in ActionExecutionValue for {}: {:?} {:?}",
                artifact,
                action_key,
                action_execution_value,
                artifact_dependencies
            );

            if artifact.get_parent() == Some(parent) {
                saw_tree_child = true;
                tree_children.push((artifact_input(artifact), metadata.clone()));
            }
        }

        blaze_util::check_state!(
            saw_tree_child,
            "Action denoted by {} does not output any TreeFileArtifacts from {:?}",
            action_key,
            artifact_dependencies
        );
    }

    let tree = TreeArtifactValue::build(tree_children, None, None);
    Ok(Some(ArtifactValue::Tree(Arc::new(tree))))
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:378 ArtifactFunction.getActionLookupValue
pub fn get_action_lookup_value(
    action_lookup_key: &ActionLookupKeyRef,
    seam: &dyn AnalysisSeam,
    env: &mut Environment<'_>,
    crash_if_action_owner_missing: bool,
) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError> {
    let Some(value) = seam.get_action_lookup_value(action_lookup_key, env)? else {
        blaze_util::check_state!(
            action_lookup_key.function_name() == "COVERAGE_REPORT"
                || !crash_if_action_owner_missing,
            "Not-yet-present artifact owner: {}",
            action_lookup_key
        );
        return Ok(None);
    };
    Ok(Some(value))
}

/// Describes dependencies of derived artifacts.
// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:439 ArtifactFunction.ArtifactDependencies
pub struct ArtifactDependencies {
    artifact: Artifact,
    action_lookup_value: Arc<dyn ActionLookupValue>,
}

impl ArtifactDependencies {
    /// Constructs an `ArtifactDependencies` for the provided `derived_artifact`. Returns `None` if
    /// any dependencies are not yet ready.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:452 ArtifactDependencies.discoverDependencies
    pub fn discover_dependencies(
        derived_artifact: &Artifact,
        seam: &dyn AnalysisSeam,
        env: &mut Environment<'_>,
        crash_if_action_owner_missing: bool,
    ) -> Result<Option<ArtifactDependencies>, ComputeError> {
        let generating_action_key = derived_artifact.get_generating_action_key();
        let Some(action_lookup_value) = get_action_lookup_value(
            generating_action_key.get_action_lookup_key(),
            seam,
            env,
            crash_if_action_owner_missing,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(ArtifactDependencies {
            artifact: derived_artifact.clone(),
            action_lookup_value,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:469 ArtifactDependencies.isTemplateActionForTreeArtifact
    pub fn is_template_action_for_tree_artifact(&self) -> bool {
        self.maybe_get_template_action_for_tree_artifact().is_some()
    }

    /// The generating action of the tree artifact when it is an `ActionTemplate`.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:473 ArtifactDependencies.maybeGetTemplateActionForTreeArtifact
    fn maybe_get_template_action_for_tree_artifact(&self) -> Option<ActionRef> {
        if !self.artifact.is_tree_artifact() {
            return None;
        }
        let result = self.action_lookup_value.get_actions()
            [self.artifact.get_generating_action_key().get_action_index() as usize]
            .clone();
        as_action_template(result.as_ref())
            .is_some()
            .then_some(result)
    }

    /// Returns action template expansion keys or `None` if that information is unavailable.
    ///
    /// Must only be called if [`Self::is_template_action_for_tree_artifact`] returns `true`.
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:488 ArtifactDependencies.getActionTemplateExpansionKeys
    pub fn get_action_template_expansion_keys(
        &self,
        env: &mut Environment<'_>,
    ) -> Result<Option<Vec<ActionLookupData>>, ComputeError> {
        blaze_util::check_state!(
            self.is_template_action_for_tree_artifact(),
            "Action is unexpectedly non-template: {:?}",
            self
        );
        let key = ActionTemplateExpansionValue::key(
            self.artifact
                .get_generating_action_key()
                .get_action_lookup_key()
                .clone(),
            self.artifact.get_generating_action_key().get_action_index(),
        );
        let Some(value) = env.get_value(key)? else {
            return Ok(None);
        };
        Ok(Some(value.get_generating_action_keys(&self.artifact)))
    }
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:503 ArtifactDependencies.toString
impl fmt::Debug for ArtifactDependencies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ArtifactDependencies{{artifact={}, generatingActionKey={}, actionLookupValue=<value>}}",
            self.artifact,
            self.artifact.get_generating_action_key()
        )
    }
}

// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:278 ArtifactFunction.createSourceValue
fn create_source_value(
    executor: &SkyframeActionExecutor,
    artifact: &Artifact,
    env: &mut Environment<'_>,
) -> Result<Option<ArtifactValue>, ComputeError> {
    env.check_interrupted()?;
    let path = artifact.get_path();
    let rooted_path =
        RootedPath::to_rooted_path_from_path(artifact.get_root().get_root().clone(), &path);
    let file_value = match env
        .get_value_or_throw::<blaze_sky_loading::file::FileKey, blaze_sky_loading::io::IOException>(
            blaze_sky_loading::file::FileKey::key(rooted_path),
        )? {
        Ok(v) => v,
        // PORT: Java's DetailedIOException branch (`createdDetailed`) is not reachable: FILE
        // throws plain IOExceptions here.
        Err(e) => {
            return Err(SkyFunctionException::new(
                SourceArtifactException::create_from_message(artifact, e.get_message()),
                Transience::Persistent,
            )
            .into());
        }
    };
    let Some(file_value) = file_value else {
        return Ok(None);
    };
    if !file_value.exists() {
        return Ok(Some(ArtifactValue::Missing(MissingArtifactValue::new(
            artifact,
        ))));
    }
    // PORT: `RegularFileStateValueWithMetadata` is never produced by Blaze's file systems.
    if file_value.is_directory() {
        // PORT: skipped, Google-internal (SourceDirectoryEvent has no subscriber in Bazel).
    }

    if !file_value.is_directory() || !TrackSourceDirectoriesFlag::track_source_directories() {
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:319 FileArtifactValue.createForSourceArtifact
        let is_file = file_value.is_file();
        let metadata = FileArtifactValue::create_for_source_artifact(
            &path,
            is_file,
            if is_file { file_value.get_size() } else { 0 },
            if is_file {
                file_value
                    .real_file_state_value()
                    .get_contents_proxy()
                    .map(|p| {
                        blaze_exec::actions::FileContentsProxy::new(
                            p.get_ctime(),
                            p.get_mtime(),
                            p.get_node_id(),
                        )
                    })
            } else {
                None
            },
            if is_file {
                file_value.get_digest().map(<[u8]>::to_vec)
            } else {
                None
            },
        );
        return match metadata {
            Ok(v) => {
                use crate::metadata_consumer_for_metrics::MetadataConsumerForMetrics as _;
                executor
                    .artifact_metrics()
                    .source_artifacts_seen
                    .accumulate_file(&v);
                Ok(Some(ArtifactValue::File(v)))
            }
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
                        artifact, file_value, request, e
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
