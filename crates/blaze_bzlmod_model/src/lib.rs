//! Port of the Starlark-independent core of Bazel's `lib/bazel/bzlmod` (Bazel 9.3.2).
//!
//! - [`Version`], [`ModuleKey`] (canonical repo names), [`ModuleExtensionId`] and the extension
//!   usage data model ([`ModuleExtensionUsage`], [`Tag`] with opaque [`AttributeValues`])
//! - [`InterimModule`], [`Module`], overrides ([`ModuleOverride`])
//! - [`Discovery`] (over a [`ModuleFileLoader`]), [`Selection`], the checks of
//!   [`BazelModuleResolutionFunction`]
//! - [`BazelDepGraphFunction`] / [`BazelDepGraphValue`] (repo mappings, extension unique names)
//! - [`IndexRegistry`], [`RegistryFactoryImpl`] over a [`RegistryFetcher`]
//! - the lockfile: [`BazelLockFileValue`] with Gson-exact JSON, [`BazelLockFileFunction`],
//!   [`BazelLockFileModule`]
//!
//! MODULE.bazel evaluation (Starlark) lives in a later crate that produces [`InterimModule`]s.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod checksum;
pub mod dep_graph;
pub mod discovery;
pub mod external_deps_exception;
pub mod facts;
pub mod gson;
pub mod index_registry;
pub mod java_uri;
pub mod lockfile;
pub mod module;
pub mod module_extension;
pub mod module_key;
pub mod registry;
pub mod repo_spec;
pub mod resolution;
pub mod selection;
pub mod version;

pub use checksum::{Checksum, InvalidChecksumException, KeyType};
pub use dep_graph::{BazelDepGraphFunction, BazelDepGraphValue, Table};
pub use discovery::{
    Discovery, DiscoveryResult, ModuleFileLoader, NonRootModuleFileValue, RootModuleFileValue,
};
pub use external_deps_exception::{ExternalDepsCode, ExternalDepsException};
pub use facts::{FactValue, Facts};
pub use index_registry::{
    DEFAULT_REGISTRIES, IndexRegistry, KnownFileHashesMode, RegistryFactory, RegistryFactoryImpl,
};
pub use java_uri::{JavaUri, UriSyntaxException};
pub use lockfile::{
    BazelLockFileFunction, BazelLockFileModule, BazelLockFileValue, LOCK_FILE_VERSION,
    LockFileModuleExtension, LockFileModuleExtensionWithFactors, LockfileModuleExtensionMetadata,
    LockfileParseError, ModuleExtensionEvalFactors, RepoRecordedInputWithValue, UseAllRepos,
};
pub use module::{
    AbridgedModule, DepSpec, InterimModule, InterimModuleBuilder, Module, ModuleOverride,
    MultipleVersionOverride, NonRegistryOverride, SingleVersionOverride,
};
pub use module_extension::{
    IsolationKey, ModuleExtensionId, ModuleExtensionUsage, ModuleExtensionUsageProxy, RepoOverride,
    Tag,
};
pub use module_key::ModuleKey;
pub use registry::{
    FileRegistryFetcher, HttpRegistryFetcher, ModuleFile, Registry, RegistryException,
    RegistryFetcher, RegistryFileDownloadEvent, YankedVersionsValue,
};
pub use repo_spec::{
    ArchiveRepoSpecBuilder, AttributeValue, AttributeValues, AttributeValuesAdapter,
    GitRepoSpecBuilder, RemoteFile, RepoRuleId, RepoSpec, local_path_repo_spec,
};
pub use resolution::{
    BazelCompatibilityMode, BazelModuleResolutionFunction, BazelModuleResolutionValue,
    BazelVersion, CheckDirectDepsMode, DiscoverAndSelectResult, LockfileMode, YankedVersionsLookup,
    YankedVersionsUtil,
};
pub use selection::{Selection, SelectionResult};
pub use version::{Version, VersionParseException};
