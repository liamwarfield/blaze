//! Registers the loading SkyFunctions on an [`EvaluatorBuilder`] and injects their precomputed
//! values.
//!
//! Java: the loading part of `SkyframeExecutor.skyFunctions()`
//! (com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760) and
//! `BazelSkyframeExecutorConstants`. The rest of `SkyframeExecutor` belongs to blaze_sky_top.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::repository_name::BAZEL_TOOLS;
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_skyframe::EvaluatorBuilder;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::{FileKind, StarlarkSemantics};
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root};
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::build_file_name::BuildFileName;
use crate::bzl_compile::{BzlCompileFunction, BzlCompileKey};
use crate::bzl_load::{BzlLoadConfig, BzlLoadFunction, BzlLoadKey};
use crate::containing_package_lookup::{
    ContainingPackageLookupKey, containing_package_lookup_function,
};
use crate::directories::LoadingDirectories;
use crate::directory_listing::{
    DirectoryListingKey, DirectoryListingStateFunction, DirectoryListingStateKey,
    directory_listing_function,
};
use crate::file::{FileFunction, FileKey};
use crate::file_chain_uniqueness::{
    FileSymlinkCycleUniquenessKey, FileSymlinkInfiniteExpansionUniquenessKey,
    file_symlink_cycle_uniqueness_function, file_symlink_infinite_expansion_uniqueness_function,
};
use crate::file_state::{ExternalFileAction, ExternalFilesHelper, FileStateFunction, FileStateKey};
use crate::globs::{GlobsKey, globs_function};
use crate::ignored_subdirectories::{IgnoredSubdirectoriesKey, ignored_subdirectories_function};
use crate::local_repository_lookup::{LocalRepositoryLookupKey, local_repository_lookup_function};
use crate::package_function::{
    self, PackageFunction, PackageFunctionConfig, PackageKey, SkyframePackageLocator,
};
use crate::package_lookup::{
    CrossRepositoryLabelViolationStrategy, PackageLookupFunction, PackageLookupKey,
};
use crate::pkgcache::PathPackageLocator;
use crate::precomputed::{
    AUTOLOAD_SYMBOLS, CONFIG_SETTING_VISIBILITY_POLICY, DEFAULT_VISIBILITY,
    LAZY_MACRO_EXPANSION_PACKAGES, PATH_PACKAGE_LOCATOR, REPOSITORY_OVERRIDES, STARLARK_SEMANTICS,
};
use crate::recursive_pkg::{
    PackageErrorKey, PackageErrorMessageKey, RecursivePkgFunction, RecursivePkgKey,
    package_error_function, package_error_message_function,
};
use crate::repo_file::{
    RepoFileFunction, RepoFileKey, RepoPackageArgsKey, repo_package_args_function,
};
use crate::repos::{
    ExternalRepos, RepositoryDirectoryFunction, RepositoryDirectoryKey, RepositoryMappingFunction,
    RepositoryMappingKey,
};
use crate::starlark_builtins::{StarlarkBuiltinsFunction, StarlarkBuiltinsKey};
use crate::target_pattern_function::{TargetPatternKey, target_pattern_function};
use crate::transitive_target::{TransitiveTargetKey, transitive_target_function};
use blaze_packages::build::package_factory::PackageFactory;
use blaze_packages::builtins::{AutoloadSymbols, BazelStarlarkEnvironment};
use blaze_packages::native_rules::RuleClassProvider;
use blaze_packages::package::{NoopValidator, PackageSettings};

/// Hardcoded constants describing bazel-on-skyframe behavior.
// Java: com/google/devtools/build/lib/skyframe/BazelSkyframeExecutorConstants.java:26 BazelSkyframeExecutorConstants
pub mod bazel_skyframe_executor_constants {
    use super::*;

    // Java: com/google/devtools/build/lib/skyframe/BazelSkyframeExecutorConstants.java:29 BazelSkyframeExecutorConstants.CROSS_REPOSITORY_LABEL_VIOLATION_STRATEGY
    pub const CROSS_REPOSITORY_LABEL_VIOLATION_STRATEGY: CrossRepositoryLabelViolationStrategy =
        CrossRepositoryLabelViolationStrategy::Error;

    // Java: com/google/devtools/build/lib/skyframe/BazelSkyframeExecutorConstants.java:32 BazelSkyframeExecutorConstants.BUILD_FILES_BY_PRIORITY
    pub const BUILD_FILES_BY_PRIORITY: [BuildFileName; 2] =
        [BuildFileName::BuildDotBazel, BuildFileName::Build];
}

/// What the loading functions are constructed with (Java: the `SkyframeExecutor` fields and
/// constructor arguments they read).
pub struct LoadingFunctionsConfig {
    pub directories: LoadingDirectories,
    /// Java `SkyframeExecutor.pkgLocator` (an `AtomicReference<PathPackageLocator>`).
    pub pkg_locator: Arc<RwLock<PathPackageLocator>>,
    /// Java `SkyframeExecutor.deletedPackages` (`--deleted_packages`).
    pub deleted_packages: Arc<RwLock<HashSet<PackageIdentifier>>>,
    pub external_file_action: ExternalFileAction,
    /// The repo contents cache directory, if enabled (Java `ExternalFilesHelper`'s supplier).
    pub repo_contents_cache_path: Option<Path>,
    pub cross_repository_label_violation_strategy: CrossRepositoryLabelViolationStrategy,
    pub build_files_by_priority: Vec<BuildFileName>,
    /// Java `ruleClassProvider.getToolsRepository()`: `@bazel_tools` in Bazel.
    pub tools_repository: Option<RepositoryName>,
    /// Bzlmod and repository fetching (REPOSITORY_MAPPING, REPOSITORY_DIRECTORY).
    pub external_repos: Arc<dyn ExternalRepos>,
    /// The natives of every file kind (Java `BazelStarlarkEnvironment`); REPO_FILE uses
    /// `FileKind::RepoFile`.
    pub native_registry: Arc<NativeRegistry>,
    /// The file system's digest function (BZL_COMPILE digests).
    pub digest_function: DigestHashFunction,
    /// Java `ruleClassProvider.getBazelStarlarkEnvironment()`.
    pub bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
    /// Java `ConfiguredRuleClassProvider` (native rule classes); `None` in tests that do not
    /// load packages.
    pub rule_class_provider: Option<Arc<RuleClassProvider>>,
    /// Java `SkyframeExecutor.pkgFactory`; `None` when `rule_class_provider` is `None` (PACKAGE
    /// then fails loudly).
    pub package_factory: Option<Arc<PackageFactory>>,
    /// Java `--show_loading_progress`.
    pub show_loading_progress: bool,
    /// Java `ruleClassProvider.getConfigurationFragmentMap().keySet()`.
    /// PORT: filled once the configuration fragments are ported (blaze_config).
    pub fragment_names: Vec<String>,
}

impl LoadingFunctionsConfig {
    /// Bazel's configuration (`SequencedSkyframeExecutor` with `BazelSkyframeExecutorConstants`
    /// and `BazelRuleClassProvider`): one package path entry, the workspace.
    pub fn for_bazel(
        directories: LoadingDirectories,
        external_repos: Arc<dyn ExternalRepos>,
        native_registry: Arc<NativeRegistry>,
        bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
        rule_class_provider: Option<Arc<RuleClassProvider>>,
    ) -> LoadingFunctionsConfig {
        let build_files_by_priority =
            bazel_skyframe_executor_constants::BUILD_FILES_BY_PRIORITY.to_vec();
        let pkg_locator = PathPackageLocator::new(
            Some(directories.get_output_base().clone()),
            vec![Root::from_path(directories.get_workspace().clone())],
            build_files_by_priority.clone(),
        );
        let digest_function = directories
            .get_output_base()
            .get_file_system()
            .get_digest_function();
        LoadingFunctionsConfig {
            directories,
            pkg_locator: Arc::new(RwLock::new(pkg_locator)),
            deleted_packages: Arc::new(RwLock::new(HashSet::new())),
            // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:190
            external_file_action: ExternalFileAction::DependOnExternalPkgForExternalRepoPaths,
            repo_contents_cache_path: None,
            cross_repository_label_violation_strategy:
                bazel_skyframe_executor_constants::CROSS_REPOSITORY_LABEL_VIOLATION_STRATEGY,
            build_files_by_priority,
            // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:214
            tools_repository: Some(BAZEL_TOOLS.clone()),
            external_repos,
            native_registry,
            digest_function,
            bazel_starlark_environment,
            package_factory: rule_class_provider.as_ref().map(|rcp| {
                // Java: BlazeRuntime's PackageFactory: Bazel modules install no package
                // validator and the default package settings.
                Arc::new(PackageFactory::new(
                    rcp.clone(),
                    PackageSettings::default(),
                    Arc::new(NoopValidator),
                ))
            }),
            rule_class_provider,
            show_loading_progress: false,
            fragment_names: bazel_fragment_names(),
        }
    }

    /// [`LoadingFunctionsConfig::for_bazel`] with Bazel's rule class provider and Starlark
    /// environment.
    pub fn for_bazel_default(
        directories: LoadingDirectories,
        external_repos: Arc<dyn ExternalRepos>,
    ) -> LoadingFunctionsConfig {
        let rule_class_provider = Arc::new(RuleClassProvider::create_bazel());
        let bazel_starlark_environment =
            crate::environment::bazel_starlark_environment(&rule_class_provider);
        Self::for_bazel(
            directories,
            external_repos,
            Arc::new(Self::default_native_registry()),
            bazel_starlark_environment,
            Some(rule_class_provider),
        )
    }

    /// The natives every loading file kind needs from this crate's owners: blaze_starlark's
    /// builtins plus REPO.bazel's `repo()` / `ignore_directories()`.
    pub fn default_native_registry() -> NativeRegistry {
        let mut registry = NativeRegistry::with_blaze_starlark_builtins();
        registry.register(
            &[FileKind::RepoFile],
            blaze_packages::repo_file::register_repo_file_globals,
        );
        registry
    }
}

/// The Starlark names of Bazel's configuration fragments (Java
/// `ConfiguredRuleClassProvider.getConfigurationFragmentMap().keySet()`: the `@StarlarkBuiltin`
/// name of each fragment registered by `BazelRuleClassProvider.RULE_SETS`; fragments without one,
/// `ConfigFeatureFlagConfiguration` and `TestConfiguration`, are not in the map).
// PORT: move to blaze_config with the fragments.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:825 ConfiguredRuleClassProvider.createFragmentMap
pub fn bazel_fragment_names() -> Vec<String> {
    [
        // AndroidConfiguration (BazelRuleClassProvider.ANDROID_RULES)
        "android",
        // AppleConfiguration (ObjcRules)
        "apple",
        // BazelAndroidConfiguration (ANDROID_RULES)
        "bazel_android",
        // BazelPythonConfiguration (PYTHON_RULES)
        "bazel_py",
        // CoverageConfiguration (CoreRules)
        "coverage",
        // CppConfiguration (CcRules)
        "cpp",
        // J2ObjcConfiguration (ObjcRules)
        "j2objc",
        // JavaConfiguration (JavaRules)
        "java",
        // ObjcConfiguration (ObjcRules)
        "objc",
        // PlatformConfiguration (PlatformRules)
        "platform",
        // ProtoConfiguration (PROTO_RULES)
        "proto",
        // PythonConfiguration (PYTHON_RULES)
        "py",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// Registers the loading SkyFunctions (Java `SkyframeExecutor.skyFunctions()`, loading part).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
pub fn register_loading_functions(
    builder: EvaluatorBuilder,
    config: &LoadingFunctionsConfig,
) -> EvaluatorBuilder {
    let external_files_helper = Arc::new(ExternalFilesHelper::create(
        config.pkg_locator.clone(),
        config.external_file_action,
        config.directories.clone(),
        config.repo_contents_cache_path.clone(),
    ));
    let file_state_function = FileStateFunction::new(external_files_helper.clone());
    let directory_listing_state_function =
        DirectoryListingStateFunction::new(external_files_helper);
    let file_function = FileFunction::new(config.pkg_locator.clone(), &config.directories);
    let package_lookup_function = PackageLookupFunction::new(
        config.deleted_packages.clone(),
        config.cross_repository_label_violation_strategy,
        config.build_files_by_priority.clone(),
    );
    let bzl_compile_function = BzlCompileFunction::new(config.digest_function);
    let repository_mapping_function = RepositoryMappingFunction::new(
        config.tools_repository.clone(),
        config.external_repos.clone(),
    );
    let repository_directory_function =
        RepositoryDirectoryFunction::new(config.external_repos.clone());
    let bzl_load_function = BzlLoadFunction::create(Arc::new(BzlLoadConfig {
        bazel_starlark_environment: config.bazel_starlark_environment.clone(),
        directories: config.directories.clone(),
        tools_repository: config
            .tools_repository
            .clone()
            .unwrap_or_else(|| BAZEL_TOOLS.clone()),
        fragment_names: config.fragment_names.clone(),
        hash_function: config.digest_function,
    }));
    let starlark_builtins_function =
        StarlarkBuiltinsFunction::new(config.bazel_starlark_environment.clone());
    let package_function = config.package_factory.clone().map(|package_factory| {
        PackageFunction::new(Arc::new(PackageFunctionConfig {
            package_factory,
            package_locator: Arc::new(SkyframePackageLocator {
                pkg_locator: config.pkg_locator.clone(),
                deleted_packages: config.deleted_packages.clone(),
            }),
            bazel_starlark_environment: config.bazel_starlark_environment.clone(),
            show_loading_progress: config.show_loading_progress,
            should_use_repo_dot_bazel: true,
        }))
    });
    let recursive_pkg_function = RecursivePkgFunction::new(config.directories.clone());
    let repo_file_function = RepoFileFunction::new(
        config.native_registry.clone(),
        Root::from_path(config.directories.get_workspace().clone()),
    );

    builder
        .typed_function::<FileStateKey, _>(move |k, env| file_state_function.compute(k, env))
        .typed_function::<DirectoryListingStateKey, _>(move |k, env| {
            directory_listing_state_function.compute(k, env)
        })
        .typed_function::<FileSymlinkCycleUniquenessKey, _>(file_symlink_cycle_uniqueness_function)
        .typed_function::<FileSymlinkInfiniteExpansionUniquenessKey, _>(
            file_symlink_infinite_expansion_uniqueness_function,
        )
        .typed_function::<FileKey, _>(move |k, env| file_function.compute(k, env))
        .typed_function::<DirectoryListingKey, _>(directory_listing_function)
        .typed_function::<PackageLookupKey, _>(move |k, env| {
            package_lookup_function.compute(k, env)
        })
        .typed_function::<ContainingPackageLookupKey, _>(containing_package_lookup_function)
        .typed_function::<BzlCompileKey, _>(move |k, env| bzl_compile_function.compute(k, env))
        .typed_function::<IgnoredSubdirectoriesKey, _>(ignored_subdirectories_function)
        .typed_function::<RepositoryDirectoryKey, _>(move |k, env| {
            repository_directory_function.compute(k, env)
        })
        .typed_function::<LocalRepositoryLookupKey, _>(local_repository_lookup_function)
        .typed_function::<RepositoryMappingKey, _>(move |k, env| {
            repository_mapping_function.compute(k, env)
        })
        .typed_function::<RepoPackageArgsKey, _>(repo_package_args_function)
        .typed_function::<RepoFileKey, _>(move |k, env| repo_file_function.compute(k, env))
        .typed_function::<BzlLoadKey, _>(move |k, env| bzl_load_function.compute(k, env))
        .typed_function::<StarlarkBuiltinsKey, _>(move |k, env| {
            starlark_builtins_function.compute(k, env)
        })
        .typed_function::<GlobsKey, _>(globs_function)
        .typed_function::<RecursivePkgKey, _>(move |k, env| recursive_pkg_function.compute(k, env))
        .typed_function::<PackageErrorMessageKey, _>(package_error_message_function)
        .typed_function::<PackageErrorKey, _>(package_error_function)
        .typed_function::<TargetPatternKey, _>(target_pattern_function)
        .typed_function::<TransitiveTargetKey, _>(transitive_target_function)
        .typed_function::<PackageKey, _>(move |k, env| match &package_function {
            Some(f) => f.compute(k, env),
            None => blaze_util::unported!(
                "PackageFunction without a PackageFactory (no rule class provider configured)"
            ),
        })
}

/// Injects the precomputed values the loading functions read (Java
/// `SkyframeExecutor.preparePackageLoading` / `setRepositoryOverrides` / `setStarlarkSemantics`).
pub fn inject_loading_precomputed(
    builder: EvaluatorBuilder,
    config: &LoadingFunctionsConfig,
    starlark_semantics: StarlarkSemantics,
    repository_overrides: IndexMap<String, PathFragment>,
) -> EvaluatorBuilder {
    let autoload_symbols = AutoloadSymbols::new(
        config
            .bazel_starlark_environment
            .get_uninjected_build_bzl_env(),
        &starlark_semantics,
    );
    let builder = STARLARK_SEMANTICS.set(builder, starlark_semantics);
    let builder = PATH_PACKAGE_LOCATOR.set(builder, config.pkg_locator.read().clone());
    let builder = AUTOLOAD_SYMBOLS.set(builder, autoload_symbols);
    // Java SkyframeExecutor.preparePackageLoading with the PackageOptions defaults.
    let builder = DEFAULT_VISIBILITY.set(builder, package_function::default_visibility());
    let builder = CONFIG_SETTING_VISIBILITY_POLICY.set(
        builder,
        package_function::config_setting_visibility_policy(true, false),
    );
    let builder = LAZY_MACRO_EXPANSION_PACKAGES.set(builder, HashSet::new());
    REPOSITORY_OVERRIDES.set(builder, repository_overrides)
}
