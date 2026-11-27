//! Wiring: the Starlark environment with `StarlarkRepositoryModule`, the repository and module
//! extension SkyFunctions, and their precomputed values.
//!
//! Java: `BazelRepositoryModule.initializeRuleClasses` (`addStarlarkBootstrap(new
//! RepositoryBootstrap(new StarlarkRepositoryModule()))`), `BazelRepositoryModule.getSkyFunctions`
//! and `beforeCommand` / `getPrecomputedValues`.

use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::SingleExtensionEvalKey;
use blaze_bzlmod::skyframe::keys::SingleExtensionKey;
use blaze_packages::builtins::BazelStarlarkEnvironment;
use blaze_packages::builtins::RuleClassProviderRegistrations;
use blaze_packages::builtins::StarlarkGlobals;
use blaze_packages::builtins::env::env_from_registration;
use blaze_packages::native_rules::RuleClassProvider;
use blaze_packages::native_rules::builtin_rule_function::add_rule_functions;
use blaze_skyframe::EvaluatorBuilder;

use crate::bzlmod::repo_mapping_entries::ModuleExtensionRepoMappingEntriesKey;
use crate::bzlmod::single_extension_eval_function::SingleExtensionEvalFunction;
use crate::bzlmod::single_extension_usages::SingleExtensionUsagesKey;
use crate::precomputed::FETCH_DISABLED;
use crate::precomputed::FORCE_FETCH;
use crate::precomputed::FORCE_FETCH_CONFIGURE;
use crate::precomputed::IS_VENDOR_COMMAND;
use crate::precomputed::REMOTE_EXECUTION_ENABLED;
use crate::precomputed::REQUIRE_REPO_EXTENSION_METADATA;
use crate::precomputed::RequireRepoExtensionMetadataMode;
use crate::repository::directory_tree_digest::DirectoryTreeDigestKey;
use crate::repository::repo_definition::RepoDefinitionKey;
use crate::repository::repo_environment_function::RepoEnvironmentKey;
use crate::repository::repository_fetch_function::RepositoryFetchFunction;
use crate::services::RepoServices;

/// Bazel's `BazelStarlarkEnvironment` (as `blaze_sky_loading::environment::
/// bazel_starlark_environment` builds it) with the real `repository_rule`, `module_extension`
/// and `tag_class` (Java `BazelRepositoryModule.initializeRuleClasses`) in place of
/// `blaze_packages`' placeholders.
pub fn bazel_starlark_environment(
    rule_class_provider: &RuleClassProvider,
) -> Arc<BazelStarlarkEnvironment> {
    let rule_functions =
        env_from_registration(&|b| add_rule_functions(b, rule_class_provider.get_rule_functions()));
    let mut registrations = RuleClassProviderRegistrations::bazel();
    add_repository_module_toplevels(&mut registrations);
    Arc::new(BazelStarlarkEnvironment::new(
        StarlarkGlobals::new(&blaze_sky_loading::environment::bazel_starlark_globals_fns()),
        blaze_sky_loading::environment::BAZEL_VERSION,
        rule_functions,
        &registrations,
    ))
}

/// Adds `StarlarkRepositoryModule`'s .bzl toplevels (`repository_rule`, `module_extension`,
/// `tag_class`) to `registrations`, replacing `blaze_packages`' placeholders.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java BazelRepositoryModule.initializeRuleClasses
pub fn add_repository_module_toplevels(registrations: &mut RuleClassProviderRegistrations) {
    for (name, entry) in
        env_from_registration(&crate::starlark::repository_module::register_repository_module)
    {
        registrations.registered_bzl_toplevels.insert(name, entry);
    }
}

/// The precomputed values of repository fetching (Java `BazelRepositoryModule.
/// getPrecomputedValues`, the ones not injected by `blaze_bzlmod` or `blaze_sky_loading`).
#[derive(Clone, Debug)]
pub struct RepoPrecomputedValues {
    pub fetch_disabled: bool,
    pub force_fetch: String,
    pub force_fetch_configure: String,
    pub is_vendor_command: bool,
    pub require_repo_extension_metadata: RequireRepoExtensionMetadataMode,
    pub remote_execution_enabled: bool,
}

impl Default for RepoPrecomputedValues {
    fn default() -> Self {
        RepoPrecomputedValues {
            fetch_disabled: false,
            // Java: RepositoryDirectoryValue.FORCE_FETCH_DISABLED / FORCE_FETCH_CONFIGURE_DISABLED
            force_fetch: String::new(),
            force_fetch_configure: String::new(),
            is_vendor_command: false,
            require_repo_extension_metadata: RequireRepoExtensionMetadataMode::False,
            remote_execution_enabled: false,
        }
    }
}

/// Injects [`RepoPrecomputedValues`].
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:745 BazelRepositoryModule.getPrecomputedValues
pub fn inject_repo_precomputed(
    builder: EvaluatorBuilder,
    values: &RepoPrecomputedValues,
) -> EvaluatorBuilder {
    let builder = FETCH_DISABLED.set(builder, values.fetch_disabled);
    let builder = FORCE_FETCH.set(builder, values.force_fetch.clone());
    let builder = FORCE_FETCH_CONFIGURE.set(builder, values.force_fetch_configure.clone());
    let builder = IS_VENDOR_COMMAND.set(builder, values.is_vendor_command);
    let builder =
        REQUIRE_REPO_EXTENSION_METADATA.set(builder, values.require_repo_extension_metadata);
    REMOTE_EXECUTION_ENABLED.set(builder, values.remote_execution_enabled)
}

/// Registers REPO_DEFINITION, REPOSITORY_ENVIRONMENT_VARIABLE, DIRECTORY_TREE_DIGEST,
/// SINGLE_EXTENSION_USAGES, SINGLE_EXTENSION_EVAL, SINGLE_EXTENSION and
/// MODULE_EXTENSION_REPO_MAPPING_ENTRIES. REPOSITORY_DIRECTORY and REPOSITORY_MAPPING are
/// `blaze_sky_loading`'s, over [`crate::external_repos::BzlmodExternalRepos`].
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java BazelRepositoryModule.getSkyFunctions
pub fn register_repo_rules_functions(
    builder: EvaluatorBuilder,
    services: Arc<RepoServices>,
) -> EvaluatorBuilder {
    let s = services.clone();
    let builder = builder.typed_function::<RepoDefinitionKey, _>(move |key, env| {
        crate::repository::repo_definition_function::compute(&s, key, env)
    });
    let repo_env = services.repo_env.clone();
    let builder = builder.typed_function::<RepoEnvironmentKey, _>(move |key, env| {
        crate::repository::repo_environment_function::compute(&repo_env, key, env)
    });
    let builder = builder.typed_function::<DirectoryTreeDigestKey, _>(|key, env| {
        crate::repository::directory_tree_digest::compute(key, env)
    });
    let builder = builder.typed_function::<SingleExtensionUsagesKey, _>(|key, env| {
        crate::bzlmod::single_extension_usages::compute(key, env)
    });
    let eval_function = Arc::new(SingleExtensionEvalFunction::new(services.clone()));
    let builder = builder.typed_function::<SingleExtensionEvalKey, _>(move |key, env| {
        eval_function.compute(key, env)
    });
    let builder =
        builder.typed_function::<SingleExtensionKey, _>(crate::bzlmod::single_extension::compute);
    let builder = builder.typed_function::<ModuleExtensionRepoMappingEntriesKey, _>(|key, env| {
        crate::bzlmod::repo_mapping_entries::compute(key, env)
    });
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:248 BazelRepositoryModule.workspaceInit
    let builder =
        builder.typed_function::<blaze_bzlmod::skyframe::keys::BazelModTidyKey, _>(|key, env| {
            crate::bzlmod::bazel_mod_tidy_function::compute(key, env)
        });
    let builder = builder
        .typed_function::<crate::bzlmod::bazel_fetch_all_function::BazelFetchAllKey, _>(
            crate::bzlmod::bazel_fetch_all_function::compute,
        );
    // SkyFunctions.VENDOR_FILE: new VendorFileFunction(runtime.getRuleClassProvider()
    // .getBazelStarlarkEnvironment()).
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:260 BazelRepositoryModule.workspaceInit
    let vendor_file_function = Arc::new(
        crate::bzlmod::vendor_file_function::VendorFileFunction::new(
            services.directories.workspace.get_file_system().clone(),
        ),
    );
    builder.typed_function::<crate::bzlmod::vendor_file_function::VendorFileKey, _>(
        move |key, env| vendor_file_function.compute(key, env),
    )
}

/// The REPOSITORY_DIRECTORY implementation for [`crate::external_repos::BzlmodExternalRepos`].
pub fn repository_fetch_function(services: Arc<RepoServices>) -> Arc<RepositoryFetchFunction> {
    Arc::new(RepositoryFetchFunction::new(services))
}
