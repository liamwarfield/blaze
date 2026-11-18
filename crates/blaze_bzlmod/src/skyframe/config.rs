//! What the bzlmod SkyFunctions need from the server and the command: directories, the
//! precomputed values `BazelRepositoryModule` injects, the client environment, the downloader
//! and the repository-directory seam.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use blaze_bzlmod_model::repo_spec::LOCAL_REPOSITORY;
use blaze_bzlmod_model::{
    AttributeValue, BazelCompatibilityMode, CheckDirectDepsMode, LockfileMode, ModuleOverride,
    NonRegistryOverride, RegistryFactory, RegistryFactoryImpl, RegistryFetcher,
    local_path_repo_spec,
};
use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::BAZEL_TOOLS;
use blaze_optdefs::misc::repository_options as ro;
use blaze_skyframe::{ComputeError, Environment};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::keys::ModuleFileKey;
use super::module_file_function::{BuiltinPackageLookup, PackageLookup};

/// The Bazel version modules' `bazel_compatibility` is checked against
/// (`BlazeVersionInfo.instance().getVersion()`): the release at the pin (`UPSTREAM.json`).
// Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:132 BlazeVersionInfo.getVersion
pub const BAZEL_VERSION: &str = "9.3.2";

/// Finds the directory of a fetched repository (Java: `RepositoryDirectoryValue` of
/// `RepositoryDirectoryValue.key(name)`, cast to `Success`, `.root()`). `Ok(None)` when a
/// dependency is unavailable (Java returns null).
///
/// PORT: `RepositoryFetchFunction` (REPOSITORY_DIRECTORY) is not ported yet. This seam lets
/// `ModuleFileFunction` read the MODULE.bazel of modules with non-registry overrides; the repo
/// fetching lead replaces [`BuiltinRepositoryDirectoryLookup`] with a lookup that requests
/// REPOSITORY_DIRECTORY.
pub trait RepositoryDirectoryLookup: Send + Sync {
    fn repo_root(
        &self,
        name: &RepositoryName,
        env: &mut Environment<'_>,
    ) -> Result<Option<PathBuf>, ComputeError>;
}

/// The repositories Blaze can locate without fetching: `@bazel_tools` (the install base's
/// `embedded_tools`) and modules with a `local_path_override` (from MODULE.bazel or
/// `--override_module`, whose `local_repository` path is relative to the workspace root).
/// Every other repository is `unported!`.
#[derive(Debug, Clone)]
pub struct BuiltinRepositoryDirectoryLookup {
    /// `<install_base>/embedded_tools`.
    pub embedded_tools: PathBuf,
    pub workspace_root: PathBuf,
}

impl RepositoryDirectoryLookup for BuiltinRepositoryDirectoryLookup {
    fn repo_root(
        &self,
        name: &RepositoryName,
        env: &mut Environment<'_>,
    ) -> Result<Option<PathBuf>, ComputeError> {
        if name == &*BAZEL_TOOLS {
            return Ok(Some(self.embedded_tools.clone()));
        }
        // Java: RepoDefinitionFunction looks the repo up in the root module's non-registry
        // overrides (ModuleFileValue.KEY_FOR_ROOT_MODULE) to avoid a cycle through
        // BAZEL_DEP_GRAPH. The overrides include --override_module.
        let Some(root) = env.get_value(ModuleFileKey::key_for_root_module())? else {
            return Ok(None);
        };
        let root = root.as_root();
        let override_ = root
            .non_registry_override_canonical_repo_to_module_name
            .get(name)
            .and_then(|module_name| root.overrides.get(module_name));
        let Some(ModuleOverride::NonRegistry(non_registry_override)) = override_ else {
            blaze_util::unported!("RepositoryFetchFunction.compute", "repository {}", name);
        };
        let repo_spec = non_registry_override.repo_spec();
        if repo_spec.repo_rule_id() != Some(&*LOCAL_REPOSITORY) {
            blaze_util::unported!(
                "RepositoryFetchFunction.compute",
                "repository {} defined by {:?}",
                name,
                repo_spec.repo_rule_id().map(|id| id.to_string())
            );
        }
        let Some(AttributeValue::String(path)) = repo_spec.attributes().get("path") else {
            blaze_util::unported!(
                "RepositoryFetchFunction.compute",
                "local_repository {} without a string path",
                name
            );
        };
        // local_repository: a relative path is relative to the main repository's root.
        Ok(Some(self.workspace_root.join(path)))
    }
}

/// Everything the bzlmod SkyFunctions need. Build it with
/// [`BzlmodConfig::from_repository_options`] (what `BazelRepositoryModule.beforeCommand` and
/// `getPrecomputedValues` compute) or fill it in directly (tests).
pub struct BzlmodConfig {
    /// The workspace directory (`BlazeDirectories.getWorkspace()`).
    pub workspace_root: PathBuf,
    /// The output base (holds the hidden lockfile).
    pub output_base: PathBuf,
    /// The install base (holds `embedded_tools`, the `@bazel_tools` repo).
    pub install_base: PathBuf,
    /// `PrecomputedValue.STARLARK_SEMANTICS`.
    pub starlark_semantics: StarlarkSemantics,
    /// `ModuleFileFunction.REGISTRIES` (never empty).
    pub registries: IndexSet<String>,
    /// `RegistryFunction.MODULE_MIRRORS`.
    pub module_mirrors: IndexMap<String, IndexSet<String>>,
    /// `ModuleFileFunction.IGNORE_DEV_DEPS`.
    pub ignore_dev_deps: bool,
    /// `ModuleFileFunction.MODULE_OVERRIDES` (from `--override_module`).
    pub module_overrides: IndexMap<String, ModuleOverride>,
    /// `ModuleFileFunction.INJECTED_REPOSITORIES` (from `--inject_repository`).
    pub injected_repositories: IndexMap<String, PathFragment>,
    /// `BazelModuleResolutionFunction.CHECK_DIRECT_DEPENDENCIES`.
    pub check_direct_dependencies: CheckDirectDepsMode,
    /// `BazelModuleResolutionFunction.BAZEL_COMPATIBILITY_MODE`.
    pub bazel_compatibility_mode: BazelCompatibilityMode,
    /// `BazelLockFileFunction.LOCKFILE_MODE`.
    pub lockfile_mode: LockfileMode,
    /// `YankedVersionsUtil.ALLOWED_YANKED_VERSIONS`.
    pub allowed_yanked_versions: Vec<String>,
    /// `RepositoryDirectoryValue.VENDOR_DIRECTORY` (absolute).
    pub vendor_directory: Option<PathBuf>,
    /// `RegistryFunction.LAST_INVALIDATION`.
    pub last_registry_invalidation: SystemTime,
    /// The client environment (`ClientEnvironmentFunction`).
    pub client_env: Arc<IndexMap<String, String>>,
    /// Downloads registry files (`DownloadManager.downloadAndReadOneUrlForBzlmod`).
    pub registry_fetcher: Arc<dyn RegistryFetcher>,
    /// Creates registries for `RegistryFunction` (`RegistryFactoryImpl` in Bazel).
    pub registry_factory: Arc<dyn RegistryFactory>,
    /// `BlazeVersionInfo.instance().getVersion()`; empty disables `bazel_compatibility` checks.
    pub bazel_version: String,
    /// `ModuleFileFunction`'s built-in modules (see [`builtin_modules`]).
    pub builtin_modules: IndexMap<String, NonRegistryOverride>,
    /// Where fetched repositories are (REPOSITORY_DIRECTORY seam).
    pub repository_directory_lookup: Arc<dyn RepositoryDirectoryLookup>,
    /// Whether a package exists and under which root (PACKAGE_LOOKUP seam). The default,
    /// [`BuiltinPackageLookup`], is only right with the default `--package_path` and no
    /// `--deleted_packages`.
    pub package_lookup: Arc<dyn PackageLookup>,
}

/// An `AbruptExitException` raised while computing the configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BzlmodConfigError {
    pub message: String,
    /// `FailureDetails.ExternalRepository.Code` name, e.g. "UNKNOWN_REGISTRY".
    pub code: &'static str,
}

/// The modules treated as implicit dependencies of every other module, as non-registry
/// overrides.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:1011 ModuleFileFunction.getBuiltinModules
pub fn builtin_modules() -> IndexMap<String, NonRegistryOverride> {
    IndexMap::from([(
        "bazel_tools".to_string(),
        NonRegistryOverride::bazel_tools_override(),
    )])
}

/// `RepositoryOptions.CheckDirectDepsMode` as the model's enum.
pub fn check_direct_deps_mode(mode: ro::CheckDirectDepsMode) -> CheckDirectDepsMode {
    match mode {
        ro::CheckDirectDepsMode::OFF => CheckDirectDepsMode::Off,
        ro::CheckDirectDepsMode::WARNING => CheckDirectDepsMode::Warning,
        ro::CheckDirectDepsMode::ERROR => CheckDirectDepsMode::Error,
    }
}

/// `RepositoryOptions.BazelCompatibilityMode` as the model's enum.
pub fn bazel_compatibility_mode(mode: ro::BazelCompatibilityMode) -> BazelCompatibilityMode {
    match mode {
        ro::BazelCompatibilityMode::ERROR => BazelCompatibilityMode::Error,
        ro::BazelCompatibilityMode::WARNING => BazelCompatibilityMode::Warning,
        ro::BazelCompatibilityMode::OFF => BazelCompatibilityMode::Off,
    }
}

/// `RepositoryOptions.LockfileMode` as the model's enum.
pub fn lockfile_mode(mode: ro::LockfileMode) -> LockfileMode {
    match mode {
        ro::LockfileMode::OFF => LockfileMode::Off,
        ro::LockfileMode::UPDATE => LockfileMode::Update,
        ro::LockfileMode::REFRESH => LockfileMode::Refresh,
        ro::LockfileMode::ERROR => LockfileMode::Error,
    }
}

/// Ensures that base URLs aren't duplicated even after /-delimited segments are appended.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:683 BazelRepositoryModule.normalizeBaseUrl
fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:689 BazelRepositoryModule.normalizeBaseUrls
fn normalize_base_urls(base_urls: &[String]) -> IndexSet<String> {
    base_urls.iter().map(|u| normalize_base_url(u)).collect()
}

/// If the given path is absolute, leave it as it is. If relative, it is relative to the current
/// working directory. If it starts with `%workspace%`, it is relative to the workspace root.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:700 BazelRepositoryModule.getAbsolutePath
fn get_absolute_path(path: &str, workspace_root: &Path, working_directory: &Path) -> String {
    // PORT: byte-strings (workspace and working directory paths are converted lossily; Java keeps the bytes)
    let mut path = path.replace("%workspace%", &workspace_root.to_string_lossy());
    if !PathFragment::is_absolute_str(&path) {
        // PORT: byte-strings (workspace and working directory paths are converted lossily; Java keeps the bytes)
        path = PathFragment::create(&working_directory.to_string_lossy())
            .get_relative_str(&path)
            .get_path_string()
            .to_string();
    }
    path
}

impl BzlmodConfig {
    /// The parts of `BazelRepositoryModule.beforeCommand` and `getPrecomputedValues` that
    /// produce the values the bzlmod functions read.
    ///
    /// `nonstrict_repo_env` is `CommandEnvironment.getNonstrictRepoEnv()` (the environment
    /// registries see); `client_env` is the client environment.
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:276 BazelRepositoryModule.beforeCommand
    pub fn from_repository_options(
        repo_options: &ro::RepositoryOptions,
        workspace_root: &Path,
        working_directory: &Path,
        output_base: &Path,
        install_base: &Path,
        starlark_semantics: StarlarkSemantics,
        client_env: IndexMap<String, String>,
        nonstrict_repo_env: IndexMap<String, String>,
        registry_fetcher: Arc<dyn RegistryFetcher>,
    ) -> Result<BzlmodConfig, BzlmodConfigError> {
        // PORT: --override_repository (REPOSITORY_OVERRIDES) belongs to RepoDefinitionFunction,
        // which is not ported here.
        let mut injection_map: IndexMap<String, PathFragment> = IndexMap::new();
        for injection in &repo_options.repository_injections {
            if injection.path.is_empty() {
                injection_map.shift_remove(&injection.apparent_name);
                continue;
            }
            let repo_path = get_absolute_path(&injection.path, workspace_root, working_directory);
            injection_map.insert(
                injection.apparent_name.clone(),
                PathFragment::create(&repo_path),
            );
        }

        let mut module_override_map: IndexMap<String, ModuleOverride> = IndexMap::new();
        for override_ in &repo_options.module_overrides {
            if override_.path.is_empty() {
                module_override_map.shift_remove(&override_.module_name);
                continue;
            }
            let module_path = get_absolute_path(&override_.path, workspace_root, working_directory);
            module_override_map.insert(
                override_.module_name.clone(),
                ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec(
                    &module_path,
                ))),
            );
        }

        let vendor_directory = repo_options
            .vendor_directory
            .as_ref()
            .map(|v| workspace_root.join(v.get().get_path_string()));
        // PORT: the external/vendor symlinks beforeCommand creates belong to repository fetching
        // and vendoring (not ported).

        let registries: IndexSet<String> = if !repo_options.registries.is_empty() {
            normalize_base_urls(&repo_options.registries)
        } else {
            blaze_bzlmod_model::DEFAULT_REGISTRIES
                .iter()
                .map(|s| s.to_string())
                .collect()
        };
        let module_mirrors: IndexMap<String, IndexSet<String>> =
            if !repo_options.module_mirrors.is_empty() {
                // Last wins semantics allow previous settings to be overridden.
                let mut registry_to_mirrors: IndexMap<String, IndexSet<String>> = IndexMap::new();
                for (registry, mirrors) in &repo_options.module_mirrors {
                    registry_to_mirrors
                        .insert(normalize_base_url(registry), normalize_base_urls(mirrors));
                }
                // The key "" returned by the option conversion defines mirrors for all registries
                // that don't have their own explicitly defined.
                let unknown_registries: Vec<&str> = registry_to_mirrors
                    .keys()
                    .filter(|k| !k.is_empty() && !registries.contains(*k))
                    .map(|k| k.as_str())
                    .collect();
                if !unknown_registries.is_empty() {
                    return Err(BzlmodConfigError {
                        message: format!(
                            "--module_mirrors references registries not listed in --registries: {}",
                            unknown_registries.join(", ")
                        ),
                        code: "UNKNOWN_REGISTRY",
                    });
                }
                let default_mirrors = registry_to_mirrors.get("").cloned().unwrap_or_default();
                registries
                    .iter()
                    .map(|registry| {
                        (
                            registry.clone(),
                            registry_to_mirrors
                                .get(registry)
                                .cloned()
                                .unwrap_or_else(|| default_mirrors.clone()),
                        )
                    })
                    .collect()
            } else {
                // DEFAULT_MODULE_MIRRORS
                IndexMap::new()
            };

        let nonstrict_repo_env = Arc::new(nonstrict_repo_env);
        let registry_factory: Arc<dyn RegistryFactory> =
            Arc::new(RegistryFactoryImpl::new(Box::new(move || {
                (*nonstrict_repo_env).clone()
            })));

        Ok(BzlmodConfig {
            workspace_root: workspace_root.to_path_buf(),
            output_base: output_base.to_path_buf(),
            install_base: install_base.to_path_buf(),
            starlark_semantics,
            registries,
            module_mirrors,
            ignore_dev_deps: repo_options.ignore_dev_dependency,
            module_overrides: module_override_map,
            injected_repositories: injection_map,
            check_direct_dependencies: check_direct_deps_mode(
                repo_options.check_direct_dependencies,
            ),
            bazel_compatibility_mode: bazel_compatibility_mode(
                repo_options.bazel_compatibility_mode,
            ),
            lockfile_mode: lockfile_mode(repo_options.lockfile_mode),
            allowed_yanked_versions: repo_options.allowed_yanked_versions.clone(),
            vendor_directory,
            // PORT: Java keeps the previous command's value unless an hour has passed; one
            // command per process always takes `clock.now()`.
            last_registry_invalidation: SystemTime::now(),
            client_env: Arc::new(client_env),
            registry_fetcher,
            registry_factory,
            bazel_version: BAZEL_VERSION.to_string(),
            builtin_modules: builtin_modules(),
            repository_directory_lookup: Arc::new(BuiltinRepositoryDirectoryLookup {
                embedded_tools: install_base.join("embedded_tools"),
                workspace_root: workspace_root.to_path_buf(),
            }),
            package_lookup: Arc::new(BuiltinPackageLookup {
                workspace_root: workspace_root.to_path_buf(),
                repository_directory_lookup: Arc::new(BuiltinRepositoryDirectoryLookup {
                    embedded_tools: install_base.join("embedded_tools"),
                    workspace_root: workspace_root.to_path_buf(),
                }),
            }),
        })
    }
}
