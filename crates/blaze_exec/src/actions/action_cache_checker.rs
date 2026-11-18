//! Port of `lib/actions/ActionCacheChecker.java` (with `ActionCacheUtils`, `ActionCacheAwareAction`,
//! `OutputChecker` and `ProxyMetadataFactory`), and the `Action` view it needs.
//!
//! # Seam with `blaze_actions`
//!
//! `ActionCacheChecker` reads actions through [`CacheCheckAction`], the exact subset of
//! `lib/actions/Action.java` it calls (key, inputs, outputs, client env, flags). `blaze_actions`
//! (or an adapter in `blaze_exec`, once it depends on `blaze_actions`) implements it for every
//! action. The adapter carries the `ActionKeyContext` (Java passes it to `getKey`). Artifacts are
//! [`ActionInputRef`]s whose `as_artifact()` is `Some`; tree artifacts (Java `SpecialArtifact`)
//! are those with `is_tree_artifact()`.
//!
//! Output metadata comes from an [`OutputMetadataStore`], input metadata from an
//! [`InputMetadataProvider`]. Constructors that live in other packages (`TreeFileArtifact
//! .createTreeOutput`, `ArchivedTreeArtifact.createForTree`, `TreeArtifactValue.Builder`) are
//! reached through [`TreeArtifactFactory`]; the default one fails with `unported!`.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler, EventKind};
/// `InterruptedException` as the checker throws it (re-exported for callers that do not depend on
/// `blaze_shell`).
pub use blaze_shell::InterruptedException;
use blaze_util::{check_argument, check_state, unported};
use blaze_vfs::{PathFragment, Root};
use indexmap::{IndexMap, IndexSet};

use super::action_input::{ActionInput, ActionInputRef};
use super::cache::action_cache::{
    ActionCache, ActionCacheEntry, ActionCacheEntryBuilder, MissReason, OutputPermissions,
};
use super::cache::output_metadata_store::{IoOrInterruptedException, OutputMetadataStore};
use super::file_artifact_value::FileArtifactValue;
use super::input_metadata_provider::{InputMetadataProvider, TreeArtifactValue};

/// Port of `ActionCacheAwareAction`: actions that require special handling by the ActionCache.
/// TODO(b/159326450): Remove this interface once Starlark supports mandatory inputs
/// specification.
// Java: com/google/devtools/build/lib/actions/ActionCacheAwareAction.java:20 ActionCacheAwareAction
pub trait ActionCacheAwareAction {
    /// By default, the action cache only stores non-mandatory inputs' execPaths.
    // Java: com/google/devtools/build/lib/actions/ActionCacheAwareAction.java:23 ActionCacheAwareAction.storeInputsExecPathsInActionCache
    fn store_inputs_exec_paths_in_action_cache(&self) -> bool;
}

/// The members of `lib/actions/Action.java` that [`ActionCacheChecker`] uses (seam with
/// `blaze_actions`).
// Java: com/google/devtools/build/lib/actions/Action.java Action
pub trait CacheCheckAction: Send + Sync {
    /// `getKey(actionKeyContext, inputMetadataProvider)`. The implementation supplies the
    /// `ActionKeyContext`.
    // Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:39 ActionKeyComputer.getKey
    fn get_key(
        &self,
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<String, InterruptedException>;

    /// `getInputs()`.
    fn get_inputs(&self) -> NestedSet<ActionInputRef>;

    /// `getMandatoryInputs()`.
    fn get_mandatory_inputs(&self) -> NestedSet<ActionInputRef>;

    /// `getAllowedDerivedInputs()`.
    fn get_allowed_derived_inputs(&self) -> NestedSet<ActionInputRef>;

    /// `getOutputs()`, in the order of Java's `ImmutableSet`.
    fn get_outputs(&self) -> Vec<ActionInputRef>;

    /// `getPrimaryOutput()`.
    fn get_primary_output(&self) -> ActionInputRef;

    /// `inputsKnown()`.
    fn inputs_known(&self) -> bool;

    /// `discoversInputs()`.
    fn discovers_inputs(&self) -> bool;

    /// `updateInputs(NestedSet<Artifact>)`: records the inputs of an action that discovers them.
    fn update_inputs(&self, inputs: NestedSet<ActionInputRef>);

    /// `executeUnconditionally()`.
    fn execute_unconditionally(&self) -> bool;

    /// `isVolatile()`.
    fn is_volatile(&self) -> bool;

    /// `getClientEnvironmentVariables()`.
    fn get_client_environment_variables(&self) -> Vec<String>;

    /// `prettyPrint()`; also used where Java formats the action with `toString()`.
    fn pretty_print(&self) -> String;

    /// `action instanceof ActionCacheAwareAction`.
    fn as_action_cache_aware_action(&self) -> Option<&dyn ActionCacheAwareAction> {
        None
    }

    /// `action instanceof TestRunnerAction` (read by `CheckUpToDateFilter.apply`).
    fn is_test_runner_action(&self) -> bool {
        false
    }

    /// Downcasting for the execution filter and adapters.
    fn as_any(&self) -> &dyn Any;
}

/// Port of `OutputChecker`: checks whether an output should be downloaded or its metadata
/// revalidated.
// Java: com/google/devtools/build/lib/actions/OutputChecker.java:20 OutputChecker
pub trait OutputChecker: Send + Sync {
    /// Returns whether the given output should be downloaded.
    // Java: com/google/devtools/build/lib/actions/OutputChecker.java:25 OutputChecker.shouldDownloadOutput
    fn should_download_output(
        &self,
        output: &dyn ActionInput,
        metadata: &FileArtifactValue,
    ) -> bool {
        !self.should_trust_metadata(output, metadata)
    }

    /// Returns whether the given metadata should be trusted.
    // Java: com/google/devtools/build/lib/actions/OutputChecker.java:30 OutputChecker.shouldTrustMetadata
    fn should_trust_metadata(&self, file: &dyn ActionInput, metadata: &FileArtifactValue) -> bool;
}

/// `OutputChecker.TRUST_ALL`.
// Java: com/google/devtools/build/lib/actions/OutputChecker.java:21 OutputChecker.TRUST_ALL
#[derive(Debug, Default, Clone, Copy)]
pub struct TrustAll;

impl OutputChecker for TrustAll {
    fn should_trust_metadata(
        &self,
        _file: &dyn ActionInput,
        _metadata: &FileArtifactValue,
    ) -> bool {
        true
    }
}

/// `OutputChecker.TRUST_LOCAL_ONLY`.
// Java: com/google/devtools/build/lib/actions/OutputChecker.java:22 OutputChecker.TRUST_LOCAL_ONLY
#[derive(Debug, Default, Clone, Copy)]
pub struct TrustLocalOnly;

impl OutputChecker for TrustLocalOnly {
    fn should_trust_metadata(&self, _file: &dyn ActionInput, metadata: &FileArtifactValue) -> bool {
        !metadata.is_remote()
    }
}

/// Port of `ProxyMetadataFactory`.
// Java: com/google/devtools/build/lib/actions/ProxyMetadataFactory.java:26 ProxyMetadataFactory
pub trait ProxyMetadataFactory: Send + Sync {
    /// `createProxyMetadata(Artifact)`: a `ProxyFileArtifactValue` ([`FileArtifactValue::Proxy`])
    /// or `None`.
    // Java: com/google/devtools/build/lib/actions/ProxyMetadataFactory.java:29 ProxyMetadataFactory.createProxyMetadata
    fn create_proxy_metadata(
        &self,
        artifact: &ActionInputRef,
    ) -> io::Result<Option<FileArtifactValue>>;
}

/// `ProxyMetadataFactory.NO_PROXIES`: factory suitable for builds that never expect to use
/// `ProxyFileArtifactValue`.
// Java: com/google/devtools/build/lib/actions/ProxyMetadataFactory.java:32 ProxyMetadataFactory.NO_PROXIES
#[derive(Debug, Default, Clone, Copy)]
pub struct NoProxies;

impl ProxyMetadataFactory for NoProxies {
    fn create_proxy_metadata(
        &self,
        _artifact: &ActionInputRef,
    ) -> io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }
}

/// Represents logic that evaluates the root of the package containing an exec path, for
/// constructing artifacts out of the action cache. Passed through to
/// [`ArtifactResolver::resolve_source_artifacts`].
///
/// PORT: not `Send + Sync`: the Skyframe implementation (`ActionExecutionFunction
/// .PackageRootResolverWithEnvironment`) holds the function's `Environment` for one call.
// Java: com/google/devtools/build/lib/actions/PackageRootResolver.java:31 PackageRootResolver
pub trait PackageRootResolver {
    /// Returns mapping from exec path to root. The root is `None` if the path has no containing
    /// package. Returns `None` if for some reason the result cannot be determined at this time
    /// (such as when used within a SkyFunction with missing dependencies).
    // Java: com/google/devtools/build/lib/actions/PackageRootResolver.java:42 PackageRootResolver.findPackageRootsForFiles
    #[allow(clippy::type_complexity)]
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
    ) -> Result<Option<HashMap<PathFragment, Option<Root>>>, PackageRootExceptionOrInterrupted>;
}

/// Port of `PackageRootResolver.PackageRootException` (message only).
// Java: com/google/devtools/build/lib/actions/PackageRootResolver.java:51 PackageRootResolver.PackageRootException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PackageRootException {
    pub message: String,
}

/// `throws PackageRootResolver.PackageRootException, InterruptedException`.
#[derive(Debug, thiserror::Error)]
pub enum PackageRootExceptionOrInterrupted {
    #[error(transparent)]
    PackageRoot(#[from] PackageRootException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

/// The part of `ArtifactResolver` that [`ActionCacheChecker`] uses (seam with
/// `blaze_actions`).
// Java: com/google/devtools/build/lib/actions/ArtifactResolver.java:30 ArtifactResolver
pub trait ArtifactResolver: Send + Sync {
    /// `resolveSourceArtifacts(execPaths, resolver)`: exec path to source artifact (absent if the
    /// root cannot be determined), or `None` if any dependencies are missing.
    // Java: com/google/devtools/build/lib/actions/ArtifactResolver.java:83 ArtifactResolver.resolveSourceArtifacts
    fn resolve_source_artifacts(
        &self,
        exec_paths: &[PathFragment],
        resolver: &dyn PackageRootResolver,
    ) -> Result<Option<HashMap<PathFragment, ActionInputRef>>, PackageRootExceptionOrInterrupted>;
}

/// Constructors of tree-artifact values the checker needs that live in other packages (seam:
/// `blaze_actions` implements them).
pub trait TreeArtifactFactory: Send + Sync {
    /// `TreeFileArtifact.createTreeOutput(parent, parentRelativePath)`.
    // Java: com/google/devtools/build/lib/actions/Artifact.java TreeFileArtifact.createTreeOutput
    fn create_tree_output(
        &self,
        parent: &ActionInputRef,
        parent_relative_path: &PathFragment,
    ) -> ActionInputRef;

    /// `ArchivedTreeArtifact.createForTree(parent)`.
    // Java: com/google/devtools/build/lib/actions/Artifact.java ArchivedTreeArtifact.createForTree
    fn create_archived_tree_artifact(&self, parent: &ActionInputRef) -> ActionInputRef;

    /// `TreeArtifactValue.newBuilder(parent)`, `putChild` for each child,
    /// `setArchivedRepresentation`, `build()` (which computes the aggregate metadata).
    // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java TreeArtifactValue.Builder.build
    fn build_tree_artifact_value(
        &self,
        parent: &ActionInputRef,
        children: Vec<(ActionInputRef, FileArtifactValue)>,
        archived_representation: Option<(ActionInputRef, FileArtifactValue)>,
    ) -> TreeArtifactValue;

    /// `constructProxyTreeMetadata(parent)`: visits the tree on disk
    /// (`TreeArtifactValue.visitTree`) and builds proxy metadata for each non-directory child.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:421 ActionCacheChecker.constructProxyTreeMetadata
    fn construct_proxy_tree_metadata(
        &self,
        parent: &ActionInputRef,
        proxy_metadata_factory: &dyn ProxyMetadataFactory,
    ) -> Result<TreeArtifactValue, IoOrInterruptedException>;
}

/// The default [`TreeArtifactFactory`]: only `TreeArtifactValue.Builder.build` is ported.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnportedTreeArtifactFactory;

impl TreeArtifactFactory for UnportedTreeArtifactFactory {
    fn create_tree_output(&self, _parent: &ActionInputRef, _path: &PathFragment) -> ActionInputRef {
        unported!("TreeFileArtifact.createTreeOutput")
    }

    fn create_archived_tree_artifact(&self, _parent: &ActionInputRef) -> ActionInputRef {
        unported!("ArchivedTreeArtifact.createForTree")
    }

    fn build_tree_artifact_value(
        &self,
        _parent: &ActionInputRef,
        children: Vec<(ActionInputRef, FileArtifactValue)>,
        archived_representation: Option<(ActionInputRef, FileArtifactValue)>,
    ) -> TreeArtifactValue {
        // Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:686 TreeArtifactValue.Builder.build
        TreeArtifactValue::build(children, archived_representation, None)
    }

    fn construct_proxy_tree_metadata(
        &self,
        _parent: &ActionInputRef,
        _proxy_metadata_factory: &dyn ProxyMetadataFactory,
    ) -> Result<TreeArtifactValue, IoOrInterruptedException> {
        unported!("ActionCacheChecker.constructProxyTreeMetadata")
    }
}

/// `Entry<String, ActionCache.Entry>` of [`ActionCacheUtils::get_cache_entry_with_key`].
pub type CacheEntryWithKey = (String, ActionCacheEntry);

/// Utility functions for [`ActionCache`].
// Java: com/google/devtools/build/lib/actions/ActionCacheUtils.java:22 ActionCacheUtils
pub struct ActionCacheUtils;

impl ActionCacheUtils {
    // Java: com/google/devtools/build/lib/actions/ActionCacheUtils.java:26 ActionCacheUtils.getCacheEntryWithKey
    pub fn get_cache_entry_with_key(
        action_cache: &dyn ActionCache,
        action: &dyn CacheCheckAction,
    ) -> Option<CacheEntryWithKey> {
        for output in action.get_outputs() {
            if let Some(entry) = action_cache.get(&output.get_exec_path_string()) {
                return Some((output.get_exec_path_string(), entry));
            }
        }
        None
    }

    /// Checks whether one of existing output paths is already used as a key.
    // Java: com/google/devtools/build/lib/actions/ActionCacheUtils.java:39 ActionCacheUtils.getCacheEntry
    pub fn get_cache_entry(
        action_cache: &dyn ActionCache,
        action: &dyn CacheCheckAction,
    ) -> Option<ActionCacheEntry> {
        for output in action.get_outputs() {
            if let Some(entry) = action_cache.get(&output.get_exec_path_string()) {
                return Some(entry);
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheUtils.java:49 ActionCacheUtils.removeCacheEntry
    pub fn remove_cache_entry(action_cache: &dyn ActionCache, action: &dyn CacheCheckAction) {
        for output in action.get_outputs() {
            action_cache.remove(&output.get_exec_path_string());
        }
    }
}

/// Cache config parameters for ActionCacheChecker.
// Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:81 ActionCacheChecker.CacheConfig
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionCacheCheckerCacheConfig {
    enabled: bool,
    store_output_metadata: bool,
}

impl ActionCacheCheckerCacheConfig {
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:82 ActionCacheChecker.CacheConfig.enabled
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:84 ActionCacheChecker.CacheConfig.storeOutputMetadata
    pub fn store_output_metadata(&self) -> bool {
        self.store_output_metadata
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:86 ActionCacheChecker.CacheConfig.builder
    pub fn builder() -> ActionCacheCheckerCacheConfigBuilder {
        ActionCacheCheckerCacheConfigBuilder::default()
    }
}

/// Builder for ActionCacheChecker.CacheConfig.
// Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:92 ActionCacheChecker.CacheConfig.Builder
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionCacheCheckerCacheConfigBuilder {
    enabled: Option<bool>,
    store_output_metadata: Option<bool>,
}

impl ActionCacheCheckerCacheConfigBuilder {
    pub fn set_enabled(&mut self, value: bool) -> &mut Self {
        self.enabled = Some(value);
        self
    }

    pub fn set_store_output_metadata(&mut self, value: bool) -> &mut Self {
        self.store_output_metadata = Some(value);
        self
    }

    pub fn build(&self) -> ActionCacheCheckerCacheConfig {
        // AutoValue: unset properties are an error.
        let mut missing = Vec::new();
        if self.enabled.is_none() {
            missing.push(" enabled");
        }
        if self.store_output_metadata.is_none() {
            missing.push(" storeOutputMetadata");
        }
        check_state!(
            missing.is_empty(),
            "Missing required properties:{}",
            missing.concat()
        );
        ActionCacheCheckerCacheConfig {
            enabled: self.enabled.unwrap(),
            store_output_metadata: self.store_output_metadata.unwrap(),
        }
    }
}

/// The currently cached outputs when output metadata is stored (i.e.,
/// `CacheConfig.storeOutputMetadata`).
///
/// Metadata retrieved from the filesystem overrides the cached metadata. This way, an action will
/// not be rerun if the cached metadata is still valid, unless the filesystem state needs to be
/// updated.
// Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:311 ActionCacheChecker.CachedOutputMetadata
struct CachedOutputMetadata {
    file_metadata: IndexMap<ActionInputRef, FileArtifactValue>,
    tree_metadata: IndexMap<ActionInputRef, TreeArtifactValue>,
}

/// Wrapper for all context needed by the ActionCacheChecker to handle a single action.
// Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:867 ActionCacheChecker.Token
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionCacheCheckerToken {
    /// The primary output's path, used as the key for [`ActionCache`].
    cache_key: String,

    /// The result of calling `Action.getKey`, or `None` if it was not called.
    action_key: Option<String>,
}

/// `ActionCacheChecker.Token`.
pub type Token = ActionCacheCheckerToken;

impl ActionCacheCheckerToken {
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:875 ActionCacheChecker.Token.<init>
    fn new(action: &dyn CacheCheckAction) -> ActionCacheCheckerToken {
        ActionCacheCheckerToken {
            cache_key: action.get_primary_output().get_exec_path_string(),
            action_key: None,
        }
    }
}

/// The predicate deciding whether an action may be executed (`Predicate<? super Action>`).
pub type ExecutionFilter = Box<dyn Fn(&dyn CacheCheckAction) -> bool + Send + Sync>;

/// Checks whether an action needs to be executed, or whether it has not changed since it was last
/// stored in the action cache. Must be informed of the new Action data after execution as well.
///
/// The fingerprint, input files names, and metadata (either mtimes or MD5sums) of each action are
/// cached in the action cache to avoid unnecessary rebuilds.
///
/// While instances of this class hold references to action and metadata cache instances, they
/// are otherwise lightweight, and should be constructed anew and discarded for each build
/// request.
// Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:66 ActionCacheChecker
pub struct ActionCacheChecker {
    // PORT: the ActionKeyContext is carried by the CacheCheckAction implementation.
    execution_filter: ExecutionFilter,
    artifact_resolver: Arc<dyn ArtifactResolver>,
    proxy_metadata_factory: Arc<dyn ProxyMetadataFactory>,
    cache_config: ActionCacheCheckerCacheConfig,
    total_cache_check_semaphore_wait_millis: AtomicI64,
    tree_artifact_factory: Arc<dyn TreeArtifactFactory>,

    action_cache: Option<Arc<dyn ActionCache>>, // None when not enabled.
}

fn is_tree_artifact(artifact: &ActionInputRef) -> bool {
    artifact.as_artifact().is_some_and(|a| a.is_tree_artifact())
}

fn is_constant_metadata(artifact: &ActionInputRef) -> bool {
    artifact
        .as_artifact()
        .is_some_and(|a| a.is_constant_metadata())
}

fn is_source_artifact(artifact: &ActionInputRef) -> bool {
    artifact
        .as_artifact()
        .is_some_and(|a| a.is_source_artifact())
}

impl ActionCacheChecker {
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:101 ActionCacheChecker.<init>
    pub fn new(
        action_cache: Option<Arc<dyn ActionCache>>,
        artifact_resolver: Arc<dyn ArtifactResolver>,
        execution_filter: ExecutionFilter,
        proxy_metadata_factory: Arc<dyn ProxyMetadataFactory>,
        cache_config: Option<ActionCacheCheckerCacheConfig>,
    ) -> ActionCacheChecker {
        let cache_config = cache_config.unwrap_or_else(|| {
            ActionCacheCheckerCacheConfig::builder()
                .set_enabled(true)
                .set_store_output_metadata(false)
                .build()
        });
        let action_cache = if cache_config.enabled() {
            Some(action_cache.expect("NullPointerException"))
        } else {
            None
        };
        ActionCacheChecker {
            execution_filter,
            artifact_resolver,
            proxy_metadata_factory,
            cache_config,
            total_cache_check_semaphore_wait_millis: AtomicI64::new(0),
            tree_artifact_factory: Arc::new(UnportedTreeArtifactFactory),
            action_cache,
        }
    }

    /// Sets the constructors of tree-artifact values (see [`TreeArtifactFactory`]).
    pub fn set_tree_artifact_factory(
        &mut self,
        tree_artifact_factory: Arc<dyn TreeArtifactFactory>,
    ) {
        self.tree_artifact_factory = tree_artifact_factory;
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:123 ActionCacheChecker.isActionExecutionProhibited
    pub fn is_action_execution_prohibited(&self, action: &dyn CacheCheckAction) -> bool {
        !(self.execution_filter)(action)
    }

    /// Whether the action cache is enabled.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:128 ActionCacheChecker.enabled
    pub fn enabled(&self) -> bool {
        self.cache_config.enabled()
    }

    fn action_cache(&self) -> &dyn ActionCache {
        &**self
            .action_cache
            .as_ref()
            .expect("NullPointerException: action cache disabled")
    }

    /// Checks whether one of existing output paths is already used as a key. If yes, returns it -
    /// otherwise uses first output file as a key
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:137 ActionCacheChecker.getCacheEntry
    fn get_cache_entry(&self, action: &dyn CacheCheckAction) -> Option<ActionCacheEntry> {
        if !self.cache_config.enabled() {
            return None; // ignore existing cache when disabled.
        }
        ActionCacheUtils::get_cache_entry(self.action_cache(), action)
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:144 ActionCacheChecker.removeCacheEntry
    pub fn remove_cache_entry(&self, action: &dyn CacheCheckAction) {
        check_state!(self.enabled(), "Action cache disabled");
        ActionCacheUtils::remove_cache_entry(self.action_cache(), action);
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:150 ActionCacheChecker.getCachedMetadata
    fn get_cached_metadata<'a>(
        cached_output_metadata: Option<&'a CachedOutputMetadata>,
        artifact: &ActionInputRef,
    ) -> Option<&'a FileArtifactValue> {
        check_argument!(!is_tree_artifact(artifact));
        cached_output_metadata?.file_metadata.get(artifact)
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:162 ActionCacheChecker.getCachedTreeMetadata
    fn get_cached_tree_metadata<'a>(
        cached_output_metadata: Option<&'a CachedOutputMetadata>,
        artifact: &ActionInputRef,
    ) -> Option<&'a TreeArtifactValue> {
        check_argument!(is_tree_artifact(artifact));
        cached_output_metadata?.tree_metadata.get(artifact)
    }

    /// Returns whether an action cache entry is up to date.
    ///
    /// `action_key`: the action key previously obtained from `action.get_key()`;
    /// `action_inputs`: the action inputs; usually `action.get_inputs()`, but might be a
    /// previously cached set of discovered inputs for actions that discover them;
    /// `cached_output_metadata`: cached metadata that should be used instead of
    /// `output_metadata_store`; `output_checker`: used to check whether remote metadata should be
    /// trusted; `effective_environment`: the effective client environment for the action.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:191 ActionCacheChecker.isUpToDate
    fn is_up_to_date(
        entry: &ActionCacheEntry,
        action: &dyn CacheCheckAction,
        action_key: &str,
        action_inputs: &NestedSet<ActionInputRef>,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        cached_output_metadata: Option<&CachedOutputMetadata>,
        output_checker: Option<&dyn OutputChecker>,
        effective_environment: IndexMap<String, String>,
        action_execution_salt: &str,
        output_permissions: OutputPermissions,
        use_archived_tree_artifacts: bool,
    ) -> Result<bool, InterruptedException> {
        let mut builder = ActionCacheEntryBuilder::new(
            action_key,
            action.discovers_inputs(),
            effective_environment,
            action_execution_salt,
            output_permissions,
            use_archived_tree_artifacts,
        );

        for artifact in action.get_outputs() {
            if is_tree_artifact(&artifact) {
                let mut tree_metadata =
                    ActionCacheChecker::get_cached_tree_metadata(cached_output_metadata, &artifact)
                        .cloned();
                if tree_metadata.is_none() {
                    tree_metadata = ActionCacheChecker::get_output_tree_metadata_maybe(
                        output_metadata_store,
                        &artifact,
                    )?;
                }
                match tree_metadata {
                    Some(tree_metadata)
                        if ActionCacheChecker::should_trust_tree_metadata(
                            &artifact,
                            &tree_metadata,
                            output_checker,
                        ) =>
                    {
                        builder.add_output_tree(&*artifact, &tree_metadata);
                    }
                    _ => return Ok(false),
                }
            } else {
                let mut metadata =
                    ActionCacheChecker::get_cached_metadata(cached_output_metadata, &artifact)
                        .cloned();
                if metadata.is_none() {
                    metadata = ActionCacheChecker::get_output_metadata_maybe(
                        output_metadata_store,
                        &artifact,
                    )?;
                }
                match metadata {
                    Some(metadata)
                        if ActionCacheChecker::should_trust_metadata(
                            &artifact,
                            &metadata,
                            output_checker,
                        ) =>
                    {
                        builder.add_output_file(&*artifact, metadata);
                    }
                    _ => return Ok(false),
                }
            }
        }
        for artifact in action_inputs.to_list().iter() {
            let input_metadata =
                ActionCacheChecker::get_input_metadata_maybe(input_metadata_provider, artifact);
            builder.add_input_file(&**artifact, input_metadata);
        }
        Ok(entry.get_digest() == builder.build().get_digest())
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:246 ActionCacheChecker.shouldTrustMetadata
    fn should_trust_metadata(
        artifact: &ActionInputRef,
        metadata: &FileArtifactValue,
        output_checker: Option<&dyn OutputChecker>,
    ) -> bool {
        check_argument!(!is_tree_artifact(artifact));
        match output_checker {
            None => true,
            Some(output_checker) => output_checker.should_trust_metadata(&**artifact, metadata),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:255 ActionCacheChecker.shouldTrustTreeMetadata
    fn should_trust_tree_metadata(
        artifact: &ActionInputRef,
        tree_metadata: &TreeArtifactValue,
        output_checker: Option<&dyn OutputChecker>,
    ) -> bool {
        check_argument!(is_tree_artifact(artifact));
        let Some(output_checker) = output_checker else {
            return true;
        };
        if let Some((archived_artifact, archived_metadata)) = &tree_metadata.archived_representation
            && !output_checker.should_trust_metadata(&**archived_artifact, archived_metadata)
        {
            return false;
        }
        for (child, child_metadata) in &tree_metadata.child_values {
            if !output_checker.should_trust_metadata(&**child, child_metadata) {
                return false;
            }
        }
        true
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:287 ActionCacheChecker.unconditionalExecution
    fn unconditional_execution(&self, action: &dyn CacheCheckAction) -> bool {
        !self.is_action_execution_prohibited(action) && action.execute_unconditionally()
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:291 ActionCacheChecker.computeEffectiveEnvironment
    fn compute_effective_environment(
        action: &dyn CacheCheckAction,
        client_env: &IndexMap<String, String>,
    ) -> IndexMap<String, String> {
        let mut builder = IndexMap::new();
        for var in action.get_client_environment_variables() {
            if let Some(value) = client_env.get(&var) {
                // buildKeepingLast
                builder.insert(var, value.clone());
            }
        }
        builder
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:315 ActionCacheChecker.loadCachedOutputMetadata
    fn load_cached_output_metadata(
        &self,
        action: &dyn CacheCheckAction,
        entry: &ActionCacheEntry,
        output_metadata_store: &dyn OutputMetadataStore,
    ) -> Result<CachedOutputMetadata, InterruptedException> {
        let mut merged_file_metadata: IndexMap<ActionInputRef, FileArtifactValue> = IndexMap::new();
        let mut merged_tree_metadata: IndexMap<ActionInputRef, TreeArtifactValue> = IndexMap::new();
        let proxy_outputs: IndexSet<&str> = entry
            .get_proxy_outputs()
            .iter()
            .map(String::as_str)
            .collect();

        for artifact in action.get_outputs() {
            if is_tree_artifact(&artifact) {
                let parent = &artifact;

                if proxy_outputs.contains(parent.get_exec_path_string().as_str()) {
                    match self
                        .tree_artifact_factory
                        .construct_proxy_tree_metadata(parent, &*self.proxy_metadata_factory)
                    {
                        Ok(metadata) => {
                            put_or_throw(&mut merged_tree_metadata, parent.clone(), metadata);
                        }
                        Err(IoOrInterruptedException::Io(_)) => {
                            // Ignore - we'll get an action cache miss.
                        }
                        Err(IoOrInterruptedException::Interrupted(e)) => return Err(e),
                    }
                    continue;
                }

                let Some(cached_tree_metadata) = entry.get_output_tree(&**parent) else {
                    continue;
                };

                // PORT: Java uses a HashMap here; TreeArtifactValue.Builder sorts the children, so
                // its iteration order is not observable.
                let mut child_values: IndexMap<ActionInputRef, FileArtifactValue> = IndexMap::new();
                // Load remote child file metadata from cache.
                for (key, value) in cached_tree_metadata.child_values() {
                    child_values.insert(
                        self.tree_artifact_factory
                            .create_tree_output(parent, &PathFragment::create(key)),
                        value.clone(),
                    );
                }

                let mut archived_representation =
                    cached_tree_metadata
                        .archived_file_value()
                        .map(|file_artifact_value| {
                            (
                                self.tree_artifact_factory
                                    .create_archived_tree_artifact(parent),
                                file_artifact_value.clone(),
                            )
                        });

                let filesystem_tree_metadata =
                    match output_metadata_store.get_tree_artifact_value(parent) {
                        Ok(tree) => Some(tree),
                        Err(e) if e.is_file_not_found() => None,
                        Err(IoOrInterruptedException::Interrupted(e)) => return Err(e),
                        Err(IoOrInterruptedException::Io(_)) => {
                            // Ignore the cached metadata if we encountered an error when loading
                            // its counterpart from the filesystem.
                            // PORT: logging dropped ("Failed to load metadata for %s").
                            continue;
                        }
                    };

                if let Some(filesystem_tree_metadata) = filesystem_tree_metadata {
                    // Filesystem metadata overrides the cached metadata.
                    for (child, value) in filesystem_tree_metadata.child_values {
                        child_values.insert(child, value);
                    }
                    if filesystem_tree_metadata.archived_representation.is_some() {
                        archived_representation = filesystem_tree_metadata.archived_representation;
                    }
                }

                let merged = self.tree_artifact_factory.build_tree_artifact_value(
                    parent,
                    child_values.into_iter().collect(),
                    archived_representation,
                );

                put_or_throw(&mut merged_tree_metadata, parent.clone(), merged);
            } else {
                let cached_metadata =
                    if proxy_outputs.contains(artifact.get_exec_path_string().as_str()) {
                        self.proxy_metadata_factory
                            .create_proxy_metadata(&artifact)
                            .unwrap_or_default()
                    } else {
                        entry.get_output_file(&*artifact).cloned()
                    };

                let Some(cached_metadata) = cached_metadata else {
                    continue;
                };

                let filesystem_metadata = match ActionCacheChecker::get_output_metadata_or_constant(
                    output_metadata_store,
                    &artifact,
                ) {
                    Ok(metadata) => metadata,
                    Err(e) if e.is_file_not_found() => None,
                    Err(IoOrInterruptedException::Interrupted(e)) => return Err(e),
                    Err(IoOrInterruptedException::Io(_)) => {
                        // Ignore the cached metadata if we encountered an error when loading its
                        // counterpart from the filesystem.
                        // PORT: logging dropped ("Failed to load metadata for %s").
                        continue;
                    }
                };

                // Filesystem metadata overrides the cached metadata.
                put_or_throw(
                    &mut merged_file_metadata,
                    artifact.clone(),
                    filesystem_metadata.unwrap_or(cached_metadata),
                );
            }
        }

        Ok(CachedOutputMetadata {
            file_metadata: merged_file_metadata,
            tree_metadata: merged_tree_metadata,
        })
    }

    /// Checks whether `action` needs to be executed and returns a token if so.
    ///
    /// The method checks if any of the action's inputs or outputs have changed. Returns a token
    /// if the action needs to be executed, and `None` otherwise.
    ///
    /// If this method returns a token, indicating that the action will be executed, the
    /// `output_metadata_store` must have any cached metadata cleared so that it does not serve
    /// stale metadata for the action's outputs after the action is executed.
    // Note: the handler should only be used for DEPCHECKER events; there's no
    // guarantee it will be available for other events.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:452 ActionCacheChecker.getTokenIfNeedToExecute
    pub fn get_token_if_need_to_execute(
        &self,
        action: &dyn CacheCheckAction,
        resolved_cache_artifacts: Option<&[ActionInputRef]>,
        client_env: &IndexMap<String, String>,
        output_permissions: OutputPermissions,
        handler: Option<&dyn EventHandler>,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        action_execution_salt: &str,
        output_checker: Option<&dyn OutputChecker>,
        use_archived_tree_artifacts: bool,
    ) -> Result<Option<Token>, InterruptedException> {
        // TODO(bazel-team): (2010) For RunfilesAction/SymlinkAction and similar actions that
        // produce only symlinks we should not check whether inputs are valid at all - all that
        // matters that inputs and outputs are still exist (and new inputs have not appeared). All
        // other checks are unnecessary. In other words, the only metadata we should check for
        // them is file existence itself.

        if !self.cache_config.enabled() {
            return Ok(Some(Token::new(action)));
        }
        let mut action_inputs = action.get_inputs();
        // Resolve action inputs from cache, if necessary.
        let inputs_known = action.inputs_known();
        if !inputs_known && let Some(resolved_cache_artifacts) = resolved_cache_artifacts {
            // The action doesn't know its inputs, but the caller has a good idea of what they
            // are.
            check_state!(
                action.discovers_inputs(),
                "Actions that don't know their inputs must discover them: {}",
                action.pretty_print()
            );
            if action
                .as_action_cache_aware_action()
                .is_some_and(|a| a.store_inputs_exec_paths_in_action_cache())
            {
                action_inputs = NestedSetBuilder::wrap(
                    Order::STABLE_ORDER,
                    resolved_cache_artifacts.iter().cloned(),
                );
            } else {
                action_inputs = NestedSetBuilder::stable_order()
                    .add_transitive(&action.get_mandatory_inputs())
                    .add_all(resolved_cache_artifacts.iter().cloned())
                    .build();
            }
        }
        let entry = self.get_cache_entry(action);
        let mut cached_output_metadata = None;
        if let Some(entry) = &entry
            && !entry.is_corrupted()
            && self.cache_config.store_output_metadata()
        {
            cached_output_metadata =
                Some(self.load_cached_output_metadata(action, entry, output_metadata_store)?);
        }

        let mut token = Token::new(action);
        if self.must_execute(
            action,
            entry.as_ref(),
            &mut token,
            handler,
            input_metadata_provider,
            output_metadata_store,
            &action_inputs,
            client_env,
            output_permissions,
            action_execution_salt,
            cached_output_metadata.as_ref(),
            output_checker,
            use_archived_tree_artifacts,
        )? {
            if entry.is_some() {
                self.remove_cache_entry(action);
            }
            return Ok(Some(token));
        }

        if !inputs_known {
            action.update_inputs(action_inputs);
        }

        // Inject cached output metadata if we have an action cache hit.
        if let Some(cached_output_metadata) = cached_output_metadata {
            for (artifact, metadata) in cached_output_metadata.file_metadata {
                output_metadata_store.inject_file(&artifact, metadata);
            }
            for (artifact, tree) in cached_output_metadata.tree_metadata {
                output_metadata_store.inject_tree(&artifact, tree);
            }
        }

        Ok(None)
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:533 ActionCacheChecker.mustExecute
    fn must_execute(
        &self,
        action: &dyn CacheCheckAction,
        entry: Option<&ActionCacheEntry>,
        token: &mut Token,
        handler: Option<&dyn EventHandler>,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        action_inputs: &NestedSet<ActionInputRef>,
        client_env: &IndexMap<String, String>,
        output_permissions: OutputPermissions,
        action_execution_salt: &str,
        cached_output_metadata: Option<&CachedOutputMetadata>,
        output_checker: Option<&dyn OutputChecker>,
        use_archived_tree_artifacts: bool,
    ) -> Result<bool, InterruptedException> {
        // Unconditional execution can be applied only for actions that are allowed to be
        // executed.
        if self.unconditional_execution(action) {
            check_state!(action.is_volatile());
            ActionCacheChecker::report_unconditional_execution(handler, action);
            self.action_cache()
                .account_miss(MissReason::UnconditionalExecution);
            return Ok(true);
        }

        let Some(entry) = entry else {
            ActionCacheChecker::report_new_action(handler, action);
            self.action_cache().account_miss(MissReason::NotCached);
            return Ok(true);
        };

        if entry.is_corrupted() {
            ActionCacheChecker::report_corrupted_cache_entry(handler, action);
            self.action_cache()
                .account_miss(MissReason::CorruptedCacheEntry);
            return Ok(true);
        }

        let action_key = action.get_key(input_metadata_provider)?;
        token.action_key = Some(action_key.clone()); // Save the action key for reuse in updateActionCache().

        let effective_environment =
            ActionCacheChecker::compute_effective_environment(action, client_env);

        if !ActionCacheChecker::is_up_to_date(
            entry,
            action,
            &action_key,
            action_inputs,
            input_metadata_provider,
            output_metadata_store,
            cached_output_metadata,
            output_checker,
            effective_environment,
            action_execution_salt,
            output_permissions,
            use_archived_tree_artifacts,
        )? {
            ActionCacheChecker::report_digest_mismatch(handler, action);
            self.action_cache().account_miss(MissReason::DigestMismatch);
            return Ok(true);
        }

        self.action_cache().account_hit();
        Ok(false)
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:596 ActionCacheChecker.getInputMetadataOrConstant
    fn get_input_metadata_or_constant(
        input_metadata_provider: &dyn InputMetadataProvider,
        artifact: &ActionInputRef,
    ) -> io::Result<Option<FileArtifactValue>> {
        let metadata = input_metadata_provider.get_input_metadata(artifact)?;
        Ok(if metadata.is_some() && is_constant_metadata(artifact) {
            Some(FileArtifactValue::ConstantMetadata)
        } else {
            metadata
        })
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:604 ActionCacheChecker.getOutputMetadataOrConstant
    fn get_output_metadata_or_constant(
        output_metadata_store: &dyn OutputMetadataStore,
        artifact: &ActionInputRef,
    ) -> Result<Option<FileArtifactValue>, IoOrInterruptedException> {
        let metadata = output_metadata_store.get_output_metadata(artifact)?;
        Ok(if metadata.is_some() && is_constant_metadata(artifact) {
            Some(FileArtifactValue::ConstantMetadata)
        } else {
            metadata
        })
    }

    // TODO(ulfjack): It's unclear to me why we're ignoring all IOExceptions. In some cases, we want
    // to trigger a re-execution, so we should catch the IOException explicitly there. In others,
    // we should propagate the exception, because it is unexpected (e.g., bad file system state).
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:617 ActionCacheChecker.getInputMetadataMaybe
    fn get_input_metadata_maybe(
        input_metadata_provider: &dyn InputMetadataProvider,
        artifact: &ActionInputRef,
    ) -> Option<FileArtifactValue> {
        ActionCacheChecker::get_input_metadata_or_constant(input_metadata_provider, artifact)
            .unwrap_or_default()
    }

    // TODO(ulfjack): It's unclear to me why we're ignoring all IOExceptions. In some cases, we want
    // to trigger a re-execution, so we should catch the IOException explicitly there. In others,
    // we should propagate the exception, because it is unexpected (e.g., bad file system state).
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:630 ActionCacheChecker.getOutputMetadataMaybe
    fn get_output_metadata_maybe(
        output_metadata_store: &dyn OutputMetadataStore,
        artifact: &ActionInputRef,
    ) -> Result<Option<FileArtifactValue>, InterruptedException> {
        check_argument!(!is_tree_artifact(artifact));
        match ActionCacheChecker::get_output_metadata_or_constant(output_metadata_store, artifact) {
            Ok(metadata) => Ok(metadata),
            Err(IoOrInterruptedException::Io(_)) => Ok(None),
            Err(IoOrInterruptedException::Interrupted(e)) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:641 ActionCacheChecker.getOutputTreeMetadataMaybe
    fn get_output_tree_metadata_maybe(
        output_metadata_store: &dyn OutputMetadataStore,
        artifact: &ActionInputRef,
    ) -> Result<Option<TreeArtifactValue>, InterruptedException> {
        check_argument!(is_tree_artifact(artifact));
        match output_metadata_store.get_tree_artifact_value(artifact) {
            Ok(tree) => Ok(Some(tree)),
            Err(IoOrInterruptedException::Io(_)) => Ok(None),
            Err(IoOrInterruptedException::Interrupted(e)) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:651 ActionCacheChecker.updateActionCache
    pub fn update_action_cache(
        &self,
        action: &dyn CacheCheckAction,
        token: &Token,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        client_env: &IndexMap<String, String>,
        output_permissions: OutputPermissions,
        action_execution_salt: &str,
        use_archived_tree_artifacts: bool,
    ) -> Result<(), IoOrInterruptedException> {
        check_state!(
            self.cache_config.enabled(),
            "cache unexpectedly disabled, action: {}",
            action.pretty_print()
        );
        let key = &token.cache_key;
        if self.action_cache().get(key).is_some() {
            // This cache entry has already been updated by a shared action. We don't need to do
            // it again.
            return Ok(());
        }
        let effective_environment =
            ActionCacheChecker::compute_effective_environment(action, client_env);

        // We may already have the action key stored in the token if there was a previous (but out
        // of date) cache entry for this action. If not, there's no need to store the action key in
        // the token since we won't need it again.
        let action_key = match &token.action_key {
            Some(action_key) => action_key.clone(),
            None => action.get_key(input_metadata_provider)?,
        };

        let mut builder = ActionCacheEntryBuilder::new(
            action_key,
            action.discovers_inputs(),
            effective_environment,
            action_execution_salt,
            output_permissions,
            use_archived_tree_artifacts,
        );

        for output in action.get_outputs() {
            // Remove old records from the cache if they used different key.
            let exec_path = output.get_exec_path_string();
            if *key != exec_path {
                self.action_cache().remove(&exec_path);
            }
            if !output_metadata_store.artifact_omitted(&output) {
                if is_tree_artifact(&output) {
                    let metadata = output_metadata_store.get_tree_artifact_value(&output)?;
                    builder.add_output_tree_save_metadata(
                        &*output,
                        &metadata,
                        self.cache_config.store_output_metadata(),
                    );
                } else {
                    // Output files *must* exist and be accessible after successful action
                    // execution. We use the 'constant' metadata for the volatile workspace status
                    // output. The volatile output contains information such as timestamps, and
                    // even when --stamp is enabled, we don't want to rebuild everything if only
                    // that file changes.
                    let metadata = ActionCacheChecker::get_output_metadata_or_constant(
                        output_metadata_store,
                        &output,
                    )?;
                    check_state!(metadata.is_some());
                    builder.add_output_file_save_metadata(
                        &*output,
                        metadata.unwrap(),
                        self.cache_config.store_output_metadata(),
                    );
                }
            }
        }

        let store_all_inputs_in_action_cache = action
            .as_action_cache_aware_action()
            .is_some_and(|a| a.store_inputs_exec_paths_in_action_cache());
        let exclude_paths_from_action_cache: IndexSet<ActionInputRef> =
            if !store_all_inputs_in_action_cache && action.discovers_inputs() {
                action.get_mandatory_inputs().to_set()
            } else {
                IndexSet::new()
            };

        for input in action.get_inputs().to_list().iter() {
            builder.add_input_file_save_exec_path(
                &**input,
                ActionCacheChecker::get_input_metadata_maybe(input_metadata_provider, input),
                /* save_exec_path= */ !exclude_paths_from_action_cache.contains(input),
            );
        }

        self.action_cache().put(key, builder.build());
        Ok(())
    }

    /// The inputs an action that discovers inputs used in its cached execution, or `None` if
    /// dependencies are missing.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:730 ActionCacheChecker.getCachedInputs
    pub fn get_cached_inputs(
        &self,
        action: &dyn CacheCheckAction,
        resolver: &dyn PackageRootResolver,
    ) -> Result<Option<Vec<ActionInputRef>>, PackageRootExceptionOrInterrupted> {
        let entry = self.get_cache_entry(action);
        let Some(entry) = entry.filter(|entry| !entry.is_corrupted()) else {
            return Ok(Some(Vec::new()));
        };

        let mut outputs: Vec<PathFragment> = Vec::new();
        for output in action.get_outputs() {
            outputs.push(output.get_exec_path().clone());
        }
        let mut input_exec_paths: Vec<PathFragment> = Vec::new();
        if let Some(discovered_input_paths) = entry.get_discovered_input_paths() {
            for path in discovered_input_paths {
                let exec_path = PathFragment::create(path);
                // Code assumes that action has only 1-2 outputs and ArrayList.contains() will be
                // most efficient.
                if !outputs.contains(&exec_path) {
                    input_exec_paths.push(exec_path);
                }
            }
        }

        // Note that this method may trigger a violation of the desirable invariant that
        // getInputs() is a superset of getMandatoryInputs(). See bug about an "action not in
        // canonical form" error message and the integration test
        // test_crosstool_change_and_failure().
        let mut allowed_derived_inputs_map: HashMap<PathFragment, ActionInputRef> = HashMap::new();
        for derived_input in action.get_allowed_derived_inputs().to_list().iter() {
            if !is_source_artifact(derived_input) {
                allowed_derived_inputs_map
                    .insert(derived_input.get_exec_path().clone(), derived_input.clone());
            }
        }

        let mut input_artifacts_builder: Vec<ActionInputRef> = Vec::new();
        let mut unresolved_paths: Vec<PathFragment> = Vec::new();
        for exec_path in input_exec_paths {
            match allowed_derived_inputs_map.get(&exec_path) {
                Some(artifact) => input_artifacts_builder.push(artifact.clone()),
                None => {
                    // Remember this execPath, we will try to resolve it as a source artifact.
                    unresolved_paths.push(exec_path);
                }
            }
        }

        let Some(resolved_artifacts) = self
            .artifact_resolver
            .resolve_source_artifacts(&unresolved_paths, resolver)?
        else {
            // We are missing some dependencies. We need to rerun this update later.
            return Ok(None);
        };

        for exec_path in &unresolved_paths {
            // If PathFragment cannot be resolved into the artifact, ignore it. This could happen
            // if the rule has changed and the action no longer depends on, e.g., an additional
            // source file in a separate package and that package is no longer referenced anywhere
            // else. It is safe to ignore such paths because dependency checker would identify
            // changes in inputs (ignored path was used before) and will force action execution.
            if let Some(artifact) = resolved_artifacts.get(exec_path) {
                input_artifacts_builder.push(artifact.clone());
            }
        }
        Ok(Some(input_artifacts_builder))
    }

    /// Only call if action requires execution because there was a failure to record action cache
    /// hit.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:799 ActionCacheChecker.getTokenUnconditionallyAfterFailureToRecordActionCacheHit
    pub fn get_token_unconditionally_after_failure_to_record_action_cache_hit(
        &self,
        action: &dyn CacheCheckAction,
        resolved_cache_artifacts: Option<&[ActionInputRef]>,
        client_env: &IndexMap<String, String>,
        output_permissions: OutputPermissions,
        handler: Option<&dyn EventHandler>,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &dyn OutputMetadataStore,
        action_execution_salt: &str,
        output_checker: Option<&dyn OutputChecker>,
        use_archived_tree_artifacts: bool,
    ) -> Result<Option<Token>, InterruptedException> {
        self.remove_cache_entry(action);
        self.get_token_if_need_to_execute(
            action,
            resolved_cache_artifacts,
            client_env,
            output_permissions,
            handler,
            input_metadata_provider,
            output_metadata_store,
            action_execution_salt,
            output_checker,
            use_archived_tree_artifacts,
        )
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:827 ActionCacheChecker.addCacheCheckSemaphoreWaitTime
    pub fn add_cache_check_semaphore_wait_time(&self, wait_time_ms: i64) {
        self.total_cache_check_semaphore_wait_millis
            .fetch_add(wait_time_ms, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:831 ActionCacheChecker.getTotalCacheCheckSemaphoreWaitMillis
    pub fn get_total_cache_check_semaphore_wait_millis(&self) -> i64 {
        self.total_cache_check_semaphore_wait_millis
            .load(Ordering::SeqCst)
    }

    /// In most cases, this method should not be called directly - reportXXX() methods should be
    /// used instead. This is done to avoid cost associated with building the message.
    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:839 ActionCacheChecker.reportRebuild
    fn report_rebuild(
        handler: Option<&dyn EventHandler>,
        action: &dyn CacheCheckAction,
        message: &str,
    ) {
        // For RunfilesTreeAction, do not report rebuild.
        if let Some(handler) = handler {
            handler.handle(&Event::of(
                EventKind::Depchecker,
                format!("Executing {}: {}.", action.pretty_print(), message),
            ));
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:850 ActionCacheChecker.reportUnconditionalExecution
    fn report_unconditional_execution(
        handler: Option<&dyn EventHandler>,
        action: &dyn CacheCheckAction,
    ) {
        ActionCacheChecker::report_rebuild(handler, action, "unconditional execution is requested");
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:854 ActionCacheChecker.reportDigestMismatch
    fn report_digest_mismatch(handler: Option<&dyn EventHandler>, action: &dyn CacheCheckAction) {
        ActionCacheChecker::report_rebuild(
            handler,
            action,
            "action changed since cached execution",
        );
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:858 ActionCacheChecker.reportNewAction
    fn report_new_action(handler: Option<&dyn EventHandler>, action: &dyn CacheCheckAction) {
        ActionCacheChecker::report_rebuild(
            handler,
            action,
            "no entry in the cache (action is new)",
        );
    }

    // Java: com/google/devtools/build/lib/actions/ActionCacheChecker.java:862 ActionCacheChecker.reportCorruptedCacheEntry
    fn report_corrupted_cache_entry(
        handler: Option<&dyn EventHandler>,
        action: &dyn CacheCheckAction,
    ) {
        ActionCacheChecker::report_rebuild(handler, action, "cache entry is corrupted");
    }
}

impl fmt::Debug for ActionCacheChecker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionCacheChecker")
            .field("cache_config", &self.cache_config)
            .finish_non_exhaustive()
    }
}

/// Guava `ImmutableMap.Builder.put` + `buildOrThrow`: duplicate keys are an
/// `IllegalArgumentException`.
fn put_or_throw<V>(map: &mut IndexMap<ActionInputRef, V>, key: ActionInputRef, value: V) {
    if map.contains_key(&key) {
        panic!(
            "IllegalArgumentException: Multiple entries with same key: {}",
            key
        );
    }
    map.insert(key, value);
}
