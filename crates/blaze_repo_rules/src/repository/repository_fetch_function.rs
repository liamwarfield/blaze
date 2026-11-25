//! REPOSITORY_DIRECTORY: fetches the given repository.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java, plus the
//! events it posts (lib/repository/RepositoryFetchProgress.java,
//! lib/repository/RepositoryFailedEvent.java,
//! bazel/repository/starlark/StarlarkRepositoryDefinitionLocationEvent.java).
//!
//! PORT: Java runs `computeInternal` in a worker thread (`WorkerSkyKeyComputeState`) so that
//! Skyframe restarts do not rerun the repository rule. Blaze's evaluator blocks in
//! `Environment::get_value` instead of restarting, so `compute_internal` runs directly on the
//! SkyFunction's thread; the memory-pressure `CancellationException` retry loop has nothing to
//! retry.
//!
//! PORT: the remote repo contents cache (`RemoteRepoContentsCache`) is always null without
//! remote cache options; its lookup and `addToCache` are skipped. The remote repository executor
//! and the syscall cache are not passed to the context.

use std::sync::Arc;

use blaze_bzlmod::skyframe::precomputed::Precomputed as BzlmodPrecomputed;
use blaze_bzlmod::skyframe::precomputed::VENDOR_DIRECTORY;
use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::module::BOOTSTRAP_REPO_RULES;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::label_constants::REPO_FILE_NAME;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::Event;
use blaze_events::FetchProgress;
use blaze_events::Postable;
use blaze_repo_fetch::fs_utils;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::file_state::FileStateKey;
use blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectoriesKey;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::precomputed::PATH_PACKAGE_LOCATOR;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_sky_loading::repos::RepositoryDirectoryValue;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::starlark_util::event_location;
use blaze_skyframe::ComputeError;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::Transience;
use blaze_starlark::EvalError;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::EvalErrorKind;
use blaze_util::unported;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::Root;
use blaze_vfs::RootedPath;
use blaze_vfs::Symlinks;
use indexmap::IndexMap;
use starlark::eval::Evaluator;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;

use super::digest_writer::DigestWriter;
use super::digest_writer::DigestWriterError;
use super::exceptions::AlreadyReportedRepositoryAccessException;
use super::exceptions::RepositoryFunctionException;
use super::exceptions::RepositoryFunctionExceptionCause;
use super::repo_definition::RepoDefinition;
use super::repo_definition::RepoDefinitionKey;
use super::repo_definition::RepoDefinitionValue;
use super::repo_environment_function::get_environment_view;
use super::repo_recorded_input::RepoRecordedInputWithValue;
use super::repository_resolved_event::RepositoryResolvedEvent;
use super::repository_utils::get_external_repository_directory;
use super::repository_utils::is_valid_repo_root;
use super::repository_utils::replant_symlinks;
use crate::attr_values::starlark_to_attribute_value;
use crate::env_handle::SkyEnv;
use crate::precomputed::FETCH_DISABLED;
use crate::precomputed::FORCE_FETCH;
use crate::precomputed::FORCE_FETCH_CONFIGURE;
use crate::precomputed::REMOTE_EXECUTION_ENABLED;
use crate::precomputed::REQUIRE_REPO_EXTENSION_METADATA;
use crate::precomputed::RequireRepoExtensionMetadataMode;
use crate::services::RepoServices;
use crate::starlark::base_external_context::CloseError;
use crate::starlark::base_external_context::StarlarkBaseExternalContext;
use crate::starlark::context::EvaluationAbort;
use crate::starlark::context::ExternalContext;
use crate::starlark::context::ExternalContextKind;
use crate::starlark::context::RepositoryCtx;
use crate::starlark::repo_metadata::RepoMetadata;
use crate::starlark::repo_metadata::RepoMetadataValue;
use crate::starlark::repo_metadata::Reproducibility;
use crate::starlark::repository_context::StarlarkRepositoryContext;
use crate::starlark::thread::ImplementationOutcome;
use crate::starlark::thread::call_implementation;

// ---------------------------------------------------------------------------------------------
// Events

/// Returns the unique identifying string for a repository fetching event.
///
/// PORT: move to blaze_events or the runtime with `RepositoryFetchProgress`.
// Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:30 RepositoryFetchProgress.repositoryFetchContextString
pub fn repository_fetch_context_string(repo_name: &RepositoryName) -> String {
    format!("repository {}", repo_name)
}

/// Reports the progress of a repository fetch.
///
/// PORT: move to blaze_events or the runtime (Java lib/repository).
// Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:24 RepositoryFetchProgress
#[derive(Debug, Clone)]
pub struct RepositoryFetchProgress {
    repo_name: RepositoryName,
    finished: bool,
    message: String,
}

impl RepositoryFetchProgress {
    // Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:34 RepositoryFetchProgress.ongoing
    pub fn ongoing(repo_name: RepositoryName, message: impl Into<String>) -> Self {
        RepositoryFetchProgress {
            repo_name,
            finished: false,
            message: message.into(),
        }
    }

    // Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:38 RepositoryFetchProgress.finished
    pub fn finished(repo_name: RepositoryName) -> Self {
        RepositoryFetchProgress {
            repo_name,
            finished: true,
            message: "finished.".to_owned(),
        }
    }
}

impl Postable for RepositoryFetchProgress {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl FetchProgress for RepositoryFetchProgress {
    // Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:48 RepositoryFetchProgress.getResourceIdentifier
    fn get_resource_identifier(&self) -> String {
        repository_fetch_context_string(&self.repo_name)
    }

    // Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:53 RepositoryFetchProgress.getProgress
    fn get_progress(&self) -> String {
        self.message.clone()
    }

    // Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:58 RepositoryFetchProgress.isFinished
    fn is_finished(&self) -> bool {
        self.finished
    }
}

/// Event indicating that a failure is related to a given external repository; this is in
/// particular the case, if fetching that repository failed.
///
/// PORT: move to the build event stream port (Java lib/repository); only the fields are ported.
// Java: com/google/devtools/build/lib/repository/RepositoryFailedEvent.java:33 RepositoryFailedEvent
#[derive(Debug, Clone)]
pub struct RepositoryFailedEvent {
    repo: RepositoryName,
    message: String,
}

impl RepositoryFailedEvent {
    // Java: com/google/devtools/build/lib/repository/RepositoryFailedEvent.java:37 RepositoryFailedEvent.<init>
    pub fn new(repo: RepositoryName, message: impl Into<String>) -> Self {
        RepositoryFailedEvent {
            repo,
            message: message.into(),
        }
    }

    // Java: com/google/devtools/build/lib/repository/RepositoryFailedEvent.java:42 RepositoryFailedEvent.getRepo
    pub fn get_repo(&self) -> &RepositoryName {
        &self.repo
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl Postable for RepositoryFailedEvent {}

/// Event reporting about the place where a Starlark repository rule was defined.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDefinitionLocationEvent.java:20 StarlarkRepositoryDefinitionLocationEvent
#[derive(Debug, Clone)]
pub struct StarlarkRepositoryDefinitionLocationEvent {
    pub name: String,
    pub definition_information: String,
}

impl Postable for StarlarkRepositoryDefinitionLocationEvent {}

// ---------------------------------------------------------------------------------------------
// Errors

/// What the internal steps of [`RepositoryFetchFunction`] throw: Java's
/// `RepositoryFunctionException` or `InterruptedException`.
#[derive(Debug)]
pub enum FetchFunctionError {
    Repository(RepositoryFunctionException),
    Interrupted(Interrupted),
}

impl From<Interrupted> for FetchFunctionError {
    fn from(e: Interrupted) -> Self {
        FetchFunctionError::Interrupted(e)
    }
}

impl From<RepositoryFunctionException> for FetchFunctionError {
    fn from(e: RepositoryFunctionException) -> Self {
        FetchFunctionError::Repository(e)
    }
}

impl From<DigestWriterError> for FetchFunctionError {
    fn from(e: DigestWriterError) -> Self {
        match e {
            DigestWriterError::Repository(e) => FetchFunctionError::Repository(e),
            DigestWriterError::Interrupted(e) => FetchFunctionError::Interrupted(e),
        }
    }
}

impl From<FetchFunctionError> for ComputeError {
    fn from(e: FetchFunctionError) -> Self {
        match e {
            FetchFunctionError::Repository(e) => e.into(),
            FetchFunctionError::Interrupted(e) => ComputeError::Interrupted(e),
        }
    }
}

/// `P.get(env)` of a precomputed value that is always injected (Java unboxes it or calls
/// `checkNotNull`).
fn required<T: PartialEq + Send + Sync + 'static>(
    precomputed: &BzlmodPrecomputed<T>,
    env: &mut Environment<'_>,
) -> Result<Arc<T>, Interrupted> {
    Ok(precomputed.get(env)?.unwrap_or_else(|| {
        panic!(
            "NullPointerException: precomputed value {:?}",
            precomputed.get_key()
        )
    }))
}

/// `new Success(Root.fromPath(repoRoot), excludeFromVendoring)`.
fn success(repo_root: &Path, exclude_from_vendoring: bool) -> RepositoryDirectoryValue {
    RepositoryDirectoryValue::Success {
        root: Root::from_path(repo_root.clone()),
        exclude_from_vendoring,
    }
}

// ---------------------------------------------------------------------------------------------
// The implementation function's return value

/// What the repository rule implementation returned (Java's `switch (result)` cases), extracted
/// while the Starlark heap is alive.
enum ImplReturn {
    /// `case Dict<?, ?> dict`, after `Dict.cast(dict, String.class, Object.class, "return
    /// value")`.
    Dict(IndexMap<String, AttributeValue>),
    /// `Dict.cast` failed: Java's `EvalException` message (it has no call stack).
    DictCastError(String),
    /// `case RepoMetadata rm`.
    Metadata(RepoMetadata),
    /// `default`.
    Other,
}

/// Marks the context successful (Java calls `markSuccessful()` right after
/// `Starlark.positionalOnlyCall` returns) and classifies the return value.
fn extract_return_value<'v>(
    value: Value<'v>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> Result<ImplReturn, EvalError> {
    if let Some(mut context) = ExternalContext::from_eval_or_none(eval) {
        context.base.mark_successful();
    }
    if let Some(dict) = DictRef::from_value(value) {
        // Java: net/starlark/java/eval/Dict.java:659 Dict.cast
        for (k, v) in dict.iter() {
            if k.unpack_str().is_none() {
                return Ok(ImplReturn::DictCastError(format!(
                    "got dict<{}, {}> for 'return value', want dict<string, unknown>",
                    k.get_type(),
                    v.get_type()
                )));
            }
        }
        let mut attrs = IndexMap::new();
        for (k, v) in dict.iter() {
            let key = k.unpack_str().expect("checked above").to_owned();
            match starlark_to_attribute_value(v) {
                Ok(value) => {
                    attrs.insert(key, value);
                }
                Err(message) => unported!(
                    "RepositoryFetchFunction.fetch",
                    "a repository rule returned a dict whose value for '{key}' is not an \
                     attribute value: {message}"
                ),
            }
        }
        return Ok(ImplReturn::Dict(attrs));
    }
    if let Some(metadata) = value.downcast_ref::<RepoMetadataValue>() {
        return Ok(ImplReturn::Metadata(metadata.0.clone()));
    }
    Ok(ImplReturn::Other)
}

/// How the body of Java's `try (Mutability ...; StarlarkRepositoryContext ...)` block in `fetch`
/// ended abnormally; mapped to its `catch` clauses after the context is closed.
enum FetchBodyError {
    /// `NeedsSkyframeRestartException`.
    Restart,
    /// `InterruptedException`.
    Interrupted,
    /// `EvalException` with a call stack.
    Eval(EvalError),
    /// `EvalException` without a call stack (`getMessageWithStack()` is the message).
    EvalMessage(String),
    /// `IOException`.
    Io(IOException),
    /// `RepositoryFunctionException` (not caught by `fetch`).
    Repository(RepositoryFunctionException),
}

/// What the body of the try-with-resources block in `fetch` computes.
type FetchBody = (RepoMetadata, Vec<RepoRecordedInputWithValue>);

/// Java `StarlarkFunction.getLocation()` of the implementation function, as
/// `Location.toString()`: the start of the function's name (starlark-rust BLAZE PATCH 0024,
/// `DefModule::declaration_span`).
// Java: net/starlark/java/eval/StarlarkFunction.java:172 StarlarkFunction.getLocation
pub(crate) fn implementation_location(implementation: &OwnedFrozenValue) -> String {
    let span = blaze_starlark::defining_module::DefModule::declaration_span(
        implementation.value().to_value(),
    )
    .expect("ClassCastException: the implementation is a StarlarkFunction");
    blaze_starlark::error::Location::of_span_start(&span).to_string()
}

/// The result of [`RepositoryFetchFunction::fetch`].
///
/// `recorded_input_values`: any recorded inputs (and their values) encountered during the fetch
/// of the repo. Changes to these inputs will result in the repo being refetched in the future.
/// `reproducible`: whether the fetched repo contents are reproducible, hence cacheable.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:580 RepositoryFetchFunction.FetchResult
struct FetchResult {
    recorded_input_values: Vec<RepoRecordedInputWithValue>,
    reproducible: Reproducibility,
}

// ---------------------------------------------------------------------------------------------
// RepositoryFetchFunction

/// A SkyFunction that fetches the given repository.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:90 RepositoryFetchFunction
pub struct RepositoryFetchFunction {
    services: Arc<RepoServices>,
}

impl RepositoryFetchFunction {
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:104 RepositoryFetchFunction.<init>
    pub fn new(services: Arc<RepoServices>) -> Self {
        RepositoryFetchFunction { services }
    }

    fn directories(&self) -> &LoadingDirectories {
        &self.services.directories
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:141 RepositoryFetchFunction.compute
    pub fn compute(
        &self,
        repository_name: &RepositoryName,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryDirectoryValue>, ComputeError> {
        let Some(starlark_semantics) = STARLARK_SEMANTICS.get(env)? else {
            return Ok(None);
        };

        if !repository_name.is_visible() {
            return Ok(Some(RepositoryDirectoryValue::Failure {
                error_msg: format!(
                    "No repository visible as '@{}' from {}",
                    repository_name.get_name(),
                    repository_name.get_context_repo_display_string()
                ),
            }));
        }

        // PORT: skipped, profiling (ProfilerTask.REPOSITORY_FETCH).
        let repo_root = get_external_repository_directory(self.directories())
            .get_relative_str(repository_name.get_name());

        let Some(repo_definition_value) =
            env.get_value(RepoDefinitionKey::key(repository_name.clone()))?
        else {
            return Ok(None);
        };
        let repo_definition = match &*repo_definition_value {
            RepoDefinitionValue::NotFound => {
                return Ok(Some(RepositoryDirectoryValue::Failure {
                    error_msg: format!("Repository '{}' is not defined", repository_name),
                }));
            }
            RepoDefinitionValue::RepoOverride(repo_path) => {
                return Ok(self.setup_override(repo_path, env, &repo_root, repository_name)?);
            }
            RepoDefinitionValue::Found(rd) => rd.clone(),
        };

        // PORT: see the module comment (WorkerSkyKeyComputeState).
        Ok(self.compute_internal(
            env,
            repository_name,
            &starlark_semantics,
            &repo_root,
            repo_definition,
        )?)
    }

    /// The actual SkyFunction logic. Although Blaze's evaluator never restarts,
    /// `env.values_missing()` can still be true due to deps in error. So this function still needs
    /// to return `None` when appropriate.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:215 RepositoryFetchFunction.computeInternal
    fn compute_internal(
        &self,
        env: &mut Environment<'_>,
        repository_name: &RepositoryName,
        starlark_semantics: &Arc<StarlarkSemantics>,
        repo_root: &Path,
        repo_definition: Arc<RepoDefinition>,
    ) -> Result<Option<RepositoryDirectoryValue>, FetchFunctionError> {
        let Some(digest_writer) = DigestWriter::create(
            env,
            self.directories(),
            repository_name,
            &repo_definition,
            starlark_semantics,
        )?
        else {
            return Ok(None);
        };

        let exclude_repo_from_vendoring = true;
        if required(&VENDOR_DIRECTORY, env)?.is_some() {
            // If vendor mode is on
            let _exclude_repo_by_default =
                Self::is_repo_excluded_from_vendoring_by_default(&repo_definition);
            unported!(
                "RepositoryFetchFunction.tryGettingValueUsingVendoredRepo",
                "vendor mode (VendorFileValue) for {}",
                repository_name
            );
        }

        if Self::should_use_cached_repo_contents(env, &repo_definition)? {
            // Make sure marker file is up-to-date; correctly describes the current repository
            // state
            if digest_writer
                .are_repository_and_marker_file_consistent(env)?
                .is_none()
            {
                return Ok(Some(success(repo_root, exclude_repo_from_vendoring)));
            }
            if env.values_missing() {
                return Ok(None);
            }

            // Then check if the local repo contents cache has this.
            let repo_contents_cache = &self.services.repo_contents_cache;
            if repo_contents_cache.is_enabled() {
                let candidate_repos =
                    repo_contents_cache.get_candidate_repos(&digest_writer.predeclared_input_hash);
                for candidate in candidate_repos {
                    if digest_writer
                        .are_repository_and_marker_file_consistent_at(
                            env,
                            &candidate.recorded_inputs_file,
                        )?
                        .is_none()
                    {
                        if self
                            .setup_override(
                                candidate.contents_dir.as_fragment(),
                                env,
                                repo_root,
                                repository_name,
                            )?
                            .is_none()
                        {
                            return Ok(None);
                        }
                        candidate.touch();
                        return Ok(Some(success(repo_root, exclude_repo_from_vendoring)));
                    }
                    if env.values_missing() {
                        return Ok(None);
                    }
                }
            }

            // PORT: remoteRepoContentsCache is always null (see the module comment).
        }

        /* At this point: This is a force fetch, a local repository, OR The repository cache is old
        or didn't exist. In any of those cases, we initiate the fetching process UNLESS this is
        offline mode (fetching is disabled) */
        if !*required(&FETCH_DISABLED, env)? {
            // Fetching a repository is a long-running operation that can easily be interrupted. If
            // it is and the marker file exists on disk, a new call of this method may treat this
            // repository as valid even though it is in an inconsistent state. Clear the marker
            // file and only recreate it after fetching is done to prevent this scenario.
            DigestWriter::clear_marker_file(self.directories(), repository_name)?;
            let Some(result) =
                self.fetch_and_handle_events(&repo_definition, repo_root, env, repository_name)?
            else {
                return Ok(None);
            };
            digest_writer.write_marker_file(&result.recorded_input_values)?;
            if matches!(result.reproducible, Reproducibility::Yes)
                && !repo_definition.repo_rule.local
            {
                // This repo may be eligible for the local and remote repo contents cache.
                // Replant symlinks before caching to convert absolute symlinks relative if
                // possible, which can make more repos eligible.
                let external_repo_root = get_external_repository_directory(self.directories());
                let replant_symlinks_result = match replant_symlinks(
                    repo_root,
                    self.directories().get_workspace(),
                    &external_repo_root,
                    &PathFragment::empty_fragment(),
                    // The local repo contents cache can't handle any cross-repo symlinks and while
                    // the remote repo contents cache could in theory handle main repo symlinks,
                    // this would add a lot of complexity for little gain (local files are always
                    // available).
                    /* replant_symlinks_into_main_repo= */
                    false,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        return Err(RepositoryFunctionException::io_message(
                            format!(
                                "error replanting symlinks in repo {} before caching: {}",
                                repository_name,
                                e.get_message()
                            ),
                            Transience::Transient,
                        )
                        .into());
                    }
                };
                // PORT: remoteRepoContentsCache is always null (see the module comment).
                let repo_contents_cache = &self.services.repo_contents_cache;
                if repo_contents_cache.is_enabled()
                    && replant_symlinks_result.safe_for_local_cache()
                {
                    let new_cache_entry = match repo_contents_cache.move_to_cache(
                        repo_root,
                        &digest_writer.marker_path,
                        &digest_writer.predeclared_input_hash,
                    ) {
                        Ok(entry) => entry,
                        Err(e) => {
                            return Err(RepositoryFunctionException::io_message(
                                format!(
                                    "error moving repo {} into the repo contents cache: {}",
                                    repository_name,
                                    e.get_message()
                                ),
                                Transience::Transient,
                            )
                            .into());
                        }
                    };
                    let cached_repo_dir = &new_cache_entry.contents_dir;
                    let cached_repo_dir_rooted_path = RootedPath::to_rooted_path(
                        Root::absolute_root(cached_repo_dir.get_file_system()),
                        cached_repo_dir.as_fragment(),
                    );
                    // Don't forget to register a FileStateValue on the cache repo dir, so that we
                    // know to refetch if the cache entry gets GC'd from under us or the entire
                    // cache is deleted.
                    //
                    // Note that registering a FileValue dependency instead would lead to subtly
                    // incorrect behavior when the repo contents cache directory is deleted between
                    // builds:
                    // 1. We register a FileValue dependency on the cache entry.
                    // 2. Before the next build, the repo contents cache directory is deleted.
                    // 3. On the next build, FileSystemValueChecker invalidates the underlying
                    //    FileStateValue, which in turn results in the FileValue and the current
                    //    RepositoryDirectoryValue being marked as dirty.
                    // 4. Skyframe visits the dirty nodes bottom up to check for actual changes. In
                    //    particular, it reevaluates FileFunction before RepositoryFetchFunction
                    //    and thus the FileValue of the repo contents cache directory is locked in
                    //    as non-existent before RepositoryFetchFunction can recreate it.
                    // 5. Any other SkyFunction that depends on the FileValue of a file in the repo
                    //    (e.g. PackageFunction) will report that file as missing since the
                    //    resolved path has a parent that is non-existent.
                    // By using FileStateValue directly, which benefits from special logic built
                    // into DirtinessCheckerUtils that recognizes the repo contents cache
                    // directories with non-UUID names and prevents locking in their value during
                    // dirtiness checking, we avoid 4. and thus the incorrect missing file errors
                    // in 5.
                    if env
                        .get_value(FileStateKey::key(cached_repo_dir_rooted_path))?
                        .is_none()
                    {
                        return Ok(None);
                    }
                }
            }
            return Ok(Some(success(repo_root, exclude_repo_from_vendoring)));
        }

        if !repo_root.exists() {
            // The repository isn't on the file system, there is nothing we can do.
            return Err(RepositoryFunctionException::io_message(
                format!(
                    "to fix, run\n\tbazel fetch //...\nExternal repository {} not found and \
                     fetching repositories is disabled.",
                    repository_name
                ),
                Transience::Transient,
            )
            .into());
        }

        // Try to build with whatever is on the file system and emit a warning.
        env.listener().handle(&Event::warn(format!(
            "External repository '{}' is not up-to-date and fetching is disabled. To update, run \
             the build without the '--nofetch' command line option.",
            repository_name
        )));

        Ok(Some(success(repo_root, exclude_repo_from_vendoring)))
    }

    /// Determines whether we should use cache repo contents (either the one in
    /// `$outputBase/external` or any matching entry in the repo contents cache).
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:502 RepositoryFetchFunction.shouldUseCachedRepoContents
    fn should_use_cached_repo_contents(
        env: &mut Environment<'_>,
        repo_definition: &RepoDefinition,
    ) -> Result<bool, Interrupted> {
        /* If fetching is enabled & this is a local repo: do NOT use cache!
         * Local repository are generally fast and do not rely on non-local data, making caching
         * them across server instances impractical. */
        if !*required(&FETCH_DISABLED, env)? && repo_definition.repo_rule.local {
            return Ok(false);
        }

        let force_fetch_enabled = !required(&FORCE_FETCH, env)?.is_empty();
        let force_fetch_configure_enabled = repo_definition.repo_rule.configure
            && !required(&FORCE_FETCH_CONFIGURE, env)?.is_empty();

        /* For the non-local repositories, do NOT use cache if:
         * 1) Force fetch is enabled (bazel sync, or bazel fetch --force), OR
         * 2) Force fetch configure is enabled (bazel sync --configure) */
        if force_fetch_enabled || force_fetch_configure_enabled {
            return Ok(false);
        }

        Ok(true)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:526 RepositoryFetchFunction.isRepoExcludedFromVendoringByDefault
    fn is_repo_excluded_from_vendoring_by_default(repo_definition: &RepoDefinition) -> bool {
        repo_definition.repo_rule.local || repo_definition.repo_rule.configure
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:531 RepositoryFetchFunction.fetchAndHandleEvents
    fn fetch_and_handle_events(
        &self,
        repo_definition: &Arc<RepoDefinition>,
        repo_root: &Path,
        env: &mut Environment<'_>,
        repo_name: &RepositoryName,
    ) -> Result<Option<FetchResult>, FetchFunctionError> {
        env.listener()
            .post(Arc::new(RepositoryFetchProgress::ongoing(
                repo_name.clone(),
                "starting",
            )));

        let result = match self.fetch(repo_definition, repo_root, env, repo_name) {
            Ok(result) => result,
            Err(FetchFunctionError::Interrupted(e)) => return Err(e.into()),
            Err(FetchFunctionError::Repository(e)) => {
                // Upon an exceptional exit, the fetching of that repository is over as well.
                env.listener()
                    .post(Arc::new(RepositoryFetchProgress::finished(
                        repo_name.clone(),
                    )));
                env.listener().post(Arc::new(RepositoryFailedEvent::new(
                    repo_name.clone(),
                    e.get_message(),
                )));

                if matches!(
                    e.get_cause(),
                    RepositoryFunctionExceptionCause::AlreadyReported(_)
                ) {
                    return Err(e.into());
                }
                let cause = match e.get_cause() {
                    RepositoryFunctionExceptionCause::Io(io) => {
                        AlreadyReportedRepositoryAccessException::from_io(io)
                    }
                    RepositoryFunctionExceptionCause::Eval(eval) => {
                        AlreadyReportedRepositoryAccessException::from_eval(eval)
                    }
                    RepositoryFunctionExceptionCause::AlreadyReported(_) => unreachable!(),
                };
                env.listener().handle(&Event::error(format!(
                    "fetching {}: {}",
                    repo_definition.name,
                    e.get_message()
                )));

                // Rewrap the underlying exception to signal callers not to re-report this error.
                return Err(RepositoryFunctionException::already_reported(
                    cause,
                    if e.is_transient() {
                        Transience::Transient
                    } else {
                        Transience::Persistent
                    },
                )
                .into());
            }
        };

        if env.values_missing() {
            return Ok(None);
        }
        env.listener()
            .post(Arc::new(RepositoryFetchProgress::finished(
                repo_name.clone(),
            )));
        Ok(Some(result.expect("NullPointerException: fetch result")))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:564 RepositoryFetchFunction.setupRepoRoot
    fn setup_repo_root(repo_root: &Path) -> Result<(), RepositoryFunctionException> {
        let result = (|| -> std::io::Result<()> {
            repo_root.delete_tree()?;
            repo_root
                .get_parent_directory()
                .expect("NullPointerException: repo root parent")
                .create_directory_and_parents()
        })();
        result.map_err(|e| RepositoryFunctionException::from_io_error(&e, Transience::Transient))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:585 RepositoryFetchFunction.fetch
    fn fetch(
        &self,
        repo_definition: &Arc<RepoDefinition>,
        output_directory: &Path,
        env: &mut Environment<'_>,
        // PORT: Java names the StarlarkThread and its SymbolGenerator after the repo; Blaze's
        // threads have no names.
        _repo_name: &RepositoryName,
    ) -> Result<Option<FetchResult>, FetchFunctionError> {
        Self::setup_repo_root(output_directory)?;

        let def_info = RepositoryResolvedEvent::get_rule_definition_information(repo_definition);
        env.listener()
            .post(Arc::new(StarlarkRepositoryDefinitionLocationEvent {
                name: repo_definition.name.clone(),
                definition_information: def_info.clone(),
            }));

        let repo_rule = &repo_definition.repo_rule;
        let env_var_values = get_environment_view(env, &repo_rule.environ)?;
        if env_var_values.is_none() {
            return Ok(None);
        }
        let starlark_semantics = STARLARK_SEMANTICS.get(env)?;
        if env.values_missing() {
            return Ok(None);
        }
        let starlark_semantics = starlark_semantics.expect("values are not missing");
        let require_repo_extension_metadata_mode =
            *required(&REQUIRE_REPO_EXTENSION_METADATA, env)?;

        let package_locator = PATH_PACKAGE_LOCATOR.get(env)?;
        if env.values_missing() {
            return Ok(None);
        }
        let package_locator = package_locator.expect("values are not missing");

        let main_repo_mapping: Option<RepositoryMapping> =
            if BOOTSTRAP_REPO_RULES.contains(&repo_rule.id) {
                // Avoid a cycle.
                None
            } else {
                let Some(main_repo_mapping_value) =
                    env.get_value(RepositoryMappingKey::key(MAIN.clone()))?
                else {
                    return Ok(None);
                };
                Some(
                    main_repo_mapping_value
                        .repository_mapping()
                        .expect("NullPointerException: main repository mapping")
                        .clone(),
                )
            };

        let ignored_subdirectories = env.get_value(IgnoredSubdirectoriesKey::key_main())?;
        if env.values_missing() {
            return Ok(None);
        }
        let ignored_subdirectories = ignored_subdirectories
            .expect("values are not missing")
            .as_ignored_subdirectories()
            .clone();

        let services = &self.services;
        let body = SkyEnv::scope(
            env,
            |sky_env| -> Result<Option<FetchBody>, FetchFunctionError> {
                // Java: new StarlarkRepositoryContext(...) (its constructor calls the base
                // constructor with these arguments).
                let base = StarlarkBaseExternalContext::new(
                    output_directory.clone(),
                    services.directories.clone(),
                    sky_env,
                    services.repo_env.clone(),
                    services.nonstrict_repo_env.clone(),
                    services.download_manager.clone(),
                    services.timeout_scaling,
                    services.process_wrapper.clone(),
                    starlark_semantics.clone(),
                    repository_fetch_context_string(&RepositoryName::create_unvalidated(
                        &repo_definition.name,
                    )),
                    /* allow_watching_paths_outside_workspace= */ true,
                )
                .with_process_wrapper_options(
                    services.process_wrapper_kill_delay,
                    services.process_wrapper_graceful_sigterm,
                );
                let mut context = ExternalContext {
                    base,
                    kind: ExternalContextKind::Repository(StarlarkRepositoryContext::new(
                        repo_definition.clone(),
                        package_locator.clone(),
                        ignored_subdirectories,
                    )),
                    abort: None,
                };
                // PORT: the thread name ("repository " + display form) and the SymbolGenerator are
                // not observable; the print handler (which reports to this environment's
                // listener at once), the repo mapping recorder and the StarlarkThreadContext with
                // the main repo mapping are set up by `call_implementation`.
                if context.is_remotable() {
                    // If a rule is declared remotable then invalidate it if remote execution gets
                    // enabled or disabled.
                    if let Err(e) = REMOTE_EXECUTION_ENABLED.get(context.base.env.get()) {
                        Self::close_after_failure(&mut context);
                        return Err(FetchFunctionError::Interrupted(e));
                    }
                }

                // This rule is mainly executed for its side effect. Nevertheless, the return
                // value is of importance, as it provides information on how the call has to be
                // modified to be a reproducible rule.
                // PORT: skipped, profiling (ProfilerTask.STARLARK_REPOSITORY_FN).
                let ctx_name = repo_definition.name.clone();
                let ctx_os = context.base.get_os();
                let ctx_workspace_root = self.services.directories.workspace.clone();
                let ctx_repo_definition = repo_definition.clone();
                let ImplementationOutcome {
                    result,
                    mut context,
                    ..
                } = call_implementation(
                    &repo_rule.impl_,
                    repo_rule.module_context.as_ref(),
                    Phase::Repository,
                    starlark_semantics.clone(),
                    main_repo_mapping.clone(),
                    context,
                    |heap| {
                        heap.alloc(RepositoryCtx {
                            name: ctx_name,
                            repo_definition: ctx_repo_definition,
                            workspace_root: ctx_workspace_root,
                            os: ctx_os,
                        })
                    },
                    extract_return_value,
                );
                // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(env.getListener())):
                // `call_implementation` has reported print() output as it happened.

                let body = (|| -> Result<FetchBody, FetchBodyError> {
                    match context.abort {
                        Some(EvaluationAbort::NeedsSkyframeRestart) => {
                            return Err(FetchBodyError::Restart);
                        }
                        Some(EvaluationAbort::Interrupted) => {
                            return Err(FetchBodyError::Interrupted);
                        }
                        None => {}
                    }
                    let returned = match result {
                        Ok(returned) => returned,
                        Err(e) if e.kind == EvalErrorKind::Cancelled => {
                            return Err(FetchBodyError::Interrupted);
                        }
                        Err(e) => return Err(FetchBodyError::Eval(e)),
                    };
                    let repo_metadata = match returned {
                        ImplReturn::Dict(attrs) => RepoMetadata {
                            reproducible: Reproducibility::No,
                            attrs_for_reproducibility: attrs,
                        },
                        ImplReturn::DictCastError(message) => {
                            return Err(FetchBodyError::EvalMessage(message));
                        }
                        ImplReturn::Metadata(rm) => rm,
                        ImplReturn::Other => {
                            if Self::should_require_repo_metadata(
                                require_repo_extension_metadata_mode,
                                repo_definition,
                            ) {
                                return Err(FetchBodyError::Repository(
                                    Self::throw_default_repo_metadata_error(
                                        repo_definition,
                                        require_repo_extension_metadata_mode,
                                        context.base.env.get(),
                                    ),
                                ));
                            }
                            RepoMetadata::nonreproducible()
                        }
                    };
                    let resolved = RepositoryResolvedEvent::new(
                        repo_definition,
                        &repo_metadata.attrs_for_reproducibility,
                    );
                    if resolved.is_new_information_returned() {
                        // TODO: https://github.com/bazelbuild/bazel/issues/26511 - printing this
                        //  information isn't super useful, as it's often not actionable. Figure out
                        //  what to do instead.
                        let listener = context.base.env.get().listener();
                        listener.handle(&Event::debug(resolved.get_message()));
                        listener.handle(&Event::debug(def_info.clone()));
                    }

                    let recorded_input_values = context.base.get_recorded_inputs();
                    Ok((repo_metadata, recorded_input_values))
                })();

                // Java: try-with-resources closes the context before the catch clauses run; an
                // exception from close() is suppressed if the body threw.
                let should_delete =
                    context.should_delete_working_directory_on_close(context.base.was_successful());
                let close_result = context.base.close(should_delete);
                let body = match (body, close_result) {
                    (Ok(v), Ok(())) => Ok(v),
                    (Ok(_), Err(CloseError::Eval(message))) => {
                        Err(FetchBodyError::EvalMessage(message))
                    }
                    (Ok(_), Err(CloseError::Io(e))) => Err(FetchBodyError::Io(e)),
                    (Err(e), _) => Err(e),
                };

                match body {
                    Ok(v) => Ok(Some(v)),
                    Err(FetchBodyError::Restart) => Ok(None),
                    Err(FetchBodyError::Interrupted) => {
                        Err(FetchFunctionError::Interrupted(Interrupted))
                    }
                    Err(FetchBodyError::Eval(e)) => {
                        let listener = context.base.env.get().listener();
                        listener.handle(&Event::error_at(
                            e.location().map(event_location),
                            format!(
                                "An error occurred during the fetch of repository '{}':\n   {}",
                                repo_definition.name,
                                e.message_with_stack()
                            ),
                        ));
                        listener.handle(&Event::info(
                            RepositoryResolvedEvent::get_rule_definition_information(
                                repo_definition,
                            ),
                        ));
                        Err(RepositoryFunctionException::already_reported(
                            AlreadyReportedRepositoryAccessException::from_eval(&e),
                            Transience::Transient,
                        )
                        .into())
                    }
                    Err(FetchBodyError::EvalMessage(message)) => {
                        let listener = context.base.env.get().listener();
                        listener.handle(&Event::error_at(
                            None,
                            format!(
                                "An error occurred during the fetch of repository '{}':\n   {}",
                                repo_definition.name, message
                            ),
                        ));
                        listener.handle(&Event::info(
                            RepositoryResolvedEvent::get_rule_definition_information(
                                repo_definition,
                            ),
                        ));
                        Err(RepositoryFunctionException::already_reported(
                            AlreadyReportedRepositoryAccessException::from_eval_message(message),
                            Transience::Transient,
                        )
                        .into())
                    }
                    Err(FetchBodyError::Io(e)) => {
                        Err(RepositoryFunctionException::io(e, Transience::Transient).into())
                    }
                    Err(FetchBodyError::Repository(e)) => Err(e.into()),
                }
            },
        )?;
        let Some((repo_metadata, recorded_input_values)) = body else {
            return Ok(None);
        };

        if !output_directory.is_directory() {
            return Err(RepositoryFunctionException::io_message(
                format!("{} must create a directory", repo_definition.name),
                Transience::Transient,
            )
            .into());
        }

        // Make sure the fetched repo has a boundary file.
        if !is_valid_repo_root(output_directory) {
            if output_directory.is_symbolic_link() {
                // The created repo is actually just a symlink to somewhere else (think
                // local_repository). In this case, we shouldn't try to create the repo boundary
                // file ourselves, but report an error instead.
                return Err(RepositoryFunctionException::io_message(
                    format!(
                        "No MODULE.bazel, REPO.bazel, or WORKSPACE file found in {}",
                        output_directory
                    ),
                    Transience::Transient,
                )
                .into());
            }
            // Otherwise, we can just create an empty REPO.bazel file.
            if let Err(e) =
                fs_utils::create_empty_file(&output_directory.get_relative(&REPO_FILE_NAME))
            {
                return Err(
                    RepositoryFunctionException::from_io_error(&e, Transience::Transient).into(),
                );
            }
        }

        Ok(Some(FetchResult {
            recorded_input_values,
            reproducible: repo_metadata.reproducible,
        }))
    }

    /// Closes `context` after the body of Java's try-with-resources threw (exceptions from
    /// `close()` are suppressed).
    fn close_after_failure(context: &mut ExternalContext) {
        let should_delete =
            context.should_delete_working_directory_on_close(context.base.was_successful());
        let _ = context.base.close(should_delete);
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:755 RepositoryFetchFunction.shouldRequireRepoMetadata
    fn should_require_repo_metadata(
        require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
        repo_definition: &RepoDefinition,
    ) -> bool {
        match require_repo_extension_metadata_mode {
            RequireRepoExtensionMetadataMode::False => false,
            RequireRepoExtensionMetadataMode::All => true,
            RequireRepoExtensionMetadataMode::Root => repo_definition
                .repo_rule
                .id
                .bzl_file_label()
                .expect("NullPointerException: RepoRuleId.bzlFileLabel is null")
                .get_repository()
                .is_main(),
        }
    }

    /// Reports the error and returns the exception Java throws.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:765 RepositoryFetchFunction.throwDefaultRepoMetadataError
    fn throw_default_repo_metadata_error(
        repo_definition: &RepoDefinition,
        require_repo_extension_metadata_mode: RequireRepoExtensionMetadataMode,
        env: &mut Environment<'_>,
    ) -> RepositoryFunctionException {
        let definition_information =
            RepositoryResolvedEvent::get_rule_definition_information(repo_definition);
        let message = format!(
            "repository rule for repo '{}' did not return repo_metadata (implementation at {}), \
             but --incompatible_require_repo_extension_metadata={} requires it",
            repo_definition.name,
            implementation_location(&repo_definition.repo_rule.impl_),
            require_repo_extension_metadata_mode
        );
        env.listener().handle(&Event::error(message.clone()));
        env.listener().handle(&Event::info(definition_information));
        RepositoryFunctionException::already_reported(
            AlreadyReportedRepositoryAccessException::from_io(&IOException::new(message)),
            Transience::Persistent,
        )
    }

    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:787 RepositoryFetchFunction.setupOverride
    fn setup_override(
        &self,
        source_path: &PathFragment,
        env: &mut Environment<'_>,
        repo_root: &Path,
        repo_name: &RepositoryName,
    ) -> Result<Option<RepositoryDirectoryValue>, FetchFunctionError> {
        DigestWriter::clear_marker_file(self.directories(), repo_name)?;
        Self::symlink_repo_root(
            self.directories(),
            repo_root,
            &self.directories().get_workspace().get_relative(source_path),
            repo_name.get_name(),
            env,
        )
    }

    /// Makes `source` a symlink to `destination` and checks that `destination` is a repository
    /// root.
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:800 RepositoryFetchFunction.symlinkRepoRoot
    pub fn symlink_repo_root(
        directories: &LoadingDirectories,
        source: &Path,
        destination: &Path,
        user_defined_path: &str,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryDirectoryValue>, FetchFunctionError> {
        if source.is_directory_with(Symlinks::Nofollow)
            && let Err(e) = source.delete_tree()
        {
            return Err(
                RepositoryFunctionException::from_io_error(&e, Transience::Transient).into(),
            );
        }
        if let Err(e) = fs_utils::ensure_symbolic_link(source, destination.as_fragment()) {
            return Err(RepositoryFunctionException::io_message(
                format!(
                    "Could not create symlink to repository \"{}\" (absolute path: \"{}\"): {}",
                    user_defined_path,
                    destination,
                    IOException::from_io(&e).get_message()
                ),
                Transience::Transient,
            )
            .into());
        }

        // Check that the target directory exists and is a directory.
        // Note that we have to check `destination` and not `source` here, otherwise we'd have a
        // circular dependency between SkyValues.
        let target_dir_rooted_path = if destination.starts_with(directories.get_install_base()) {
            // The install base only changes with the Bazel binary so it's acceptable not to add
            // its ancestors as Skyframe dependencies.
            RootedPath::to_rooted_path(
                Root::from_path(destination.clone()),
                &PathFragment::empty_fragment(),
            )
        } else {
            RootedPath::to_rooted_path(
                Root::absolute_root(destination.get_file_system()),
                destination.as_fragment(),
            )
        };

        let target_dir_value = match env
            .get_value_or_throw::<FileKey, IOException>(FileKey::key(target_dir_rooted_path))?
        {
            Ok(v) => v,
            Err(e) => {
                return Err(RepositoryFunctionException::io_message(
                    format!("Could not access {}: {}", destination, e.get_message()),
                    Transience::Persistent,
                )
                .into());
            }
        };
        let Some(target_dir_value) = target_dir_value else {
            // TODO(bazel-team): If this returns null, we unnecessarily recreate the symlink above
            // on the second execution.
            return Ok(None);
        };

        if !target_dir_value.is_directory() {
            return Err(RepositoryFunctionException::io_message(
                format!(
                    "The repository's path is \"{}\" (absolute: \"{}\") but it does not exist or \
                     is not a directory.",
                    user_defined_path, destination
                ),
                Transience::Persistent,
            )
            .into());
        }

        // Check that the directory contains a repo boundary file.
        // Note that we need to do this here since we're not creating a repo boundary file
        // ourselves, but entrusting the entire contents of the repo root to this target directory.
        if !is_valid_repo_root(destination) {
            return Err(RepositoryFunctionException::io_message(
                format!(
                    "No MODULE.bazel, REPO.bazel, or WORKSPACE file found in {}",
                    destination
                ),
                Transience::Transient,
            )
            .into());
        }
        Ok(Some(RepositoryDirectoryValue::Success {
            root: Root::from_path(source.clone()),
            exclude_from_vendoring: true,
        }))
    }
}
