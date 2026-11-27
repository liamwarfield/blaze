//! Skyframe keys, values and functions of `lib/bazel/bzlmod`.
//!
//! # Registering and evaluating
//!
//! Build a [`BzlmodConfig`] (usually [`BzlmodConfig::from_repository_options`]) and register
//! the functions on an evaluator:
//!
//! ```ignore
//! let config = Arc::new(BzlmodConfig::from_repository_options(&repo_options, ...)?);
//! let builder = register_bzlmod_functions(EvaluatorBuilder::new(), config.clone());
//! // Until SingleExtensionEvalFunction is ported:
//! let builder = register_lockfile_only_extension_functions(builder);
//! let evaluator = builder.build();
//! ```
//!
//! [`register_bzlmod_functions`] registers MODULE_FILE, BAZEL_MODULE_RESOLUTION,
//! BAZEL_DEP_GRAPH, BAZEL_MODULE_INSPECTION, BAZEL_LOCK_FILE, REGISTRY, REPO_SPEC,
//! YANKED_VERSIONS and CLIENT_ENVIRONMENT_VARIABLE, and injects the precomputed values
//! ([`precomputed`]) from the config.
//!
//! `mod graph` evaluates [`BazelDepGraphKey`] and [`BazelModuleInspectorKey`] (with keep_going,
//! as `ModCommand` does). After the command, [`after_command`] writes the workspace and hidden
//! lockfiles from the evaluator's values.
//!
//! # The module extension seam
//!
//! SINGLE_EXTENSION_EVAL (`SingleExtensionEvalFunction`, which runs Starlark) is not ported here.
//! [`register_lockfile_only_extension_functions`] registers a placeholder that answers
//! [`SingleExtensionEvalKey`] from the lockfiles only (and fails with EXTENSION_EVAL_ERROR
//! otherwise), plus `SingleExtensionFunction` for [`SingleExtensionKey`]. The lead that ports
//! extension evaluation registers its functions for these same keys instead of calling it; the
//! values must be [`SingleExtensionValue`]s (`lock_file_info` set in UPDATE/REFRESH mode, as
//! [`after_command`] requires).
//!
//! # Other seams
//!
//! - [`RepositoryDirectoryLookup`] (in the config): where a repository is on disk, for modules
//!   with non-registry overrides. [`BuiltinRepositoryDirectoryLookup`] knows `@bazel_tools` and
//!   `local_path_override`s.
//! - [`PackageLookup`] (in the config): PACKAGE_LOOKUP for the packages of `include()`d files and
//!   of `single_version_override` patches. [`BuiltinPackageLookup`] looks for BUILD files under
//!   the workspace and fails loudly where it cannot be right.
//! - FILE is not ported in this crate; `ModuleFileFunction` reads files directly.
//! - [`precomputed`] holds local copies of `PrecomputedValue` and `ClientEnvironmentFunction`.

pub mod config;
pub mod dep_graph_function;
pub mod extension_placeholder;
pub mod keys;
pub mod lock_file;
pub mod module_file_function;
pub mod precomputed;
pub mod registry_functions;
pub mod resolution_function;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_util::string_encoding::{internal_to_platform, platform_to_internal};

use blaze_bzlmod_model::ExternalDepsException;
use blaze_skyframe::{ComputeError, EvaluatorBuilder, SkyFunctionException, Transience};

pub use config::{
    BAZEL_VERSION, BuiltinRepositoryDirectoryLookup, BzlmodConfig, BzlmodConfigError,
    RepositoryDirectoryLookup, builtin_modules,
};
pub use extension_placeholder::{
    check_single_extension, compute_single_extension, register_lockfile_only_extension_functions,
};
pub use keys::{
    BazelDepGraphKey, BazelLockFileKey, BazelModuleInspectorKey, BazelModuleResolutionKey,
    ModuleFileKey, ModuleFileValue, RegistryKey, RepoSpecKey, RepoSpecValue,
    SingleExtensionEvalKey, SingleExtensionKey, SingleExtensionValue, YankedVersionsKey,
};
pub use lock_file::{ENABLED_IN_MODES, after_command};
pub use module_file_function::{BuiltinPackageLookup, PackageLookup, PackageLookupResult};
pub use precomputed::{
    ClientEnvironmentKey, EnvironmentVariableValue, Precomputed, PrecomputedKey, PrecomputedValue,
};

/// `throw new XFunctionException(cause, transience)`.
pub(crate) fn throw(cause: ExternalDepsException, transience: Transience) -> ComputeError {
    SkyFunctionException::new(cause, transience).into()
}

/// The internal string of a platform path: Java's `Path.getPathString()`, which holds the file
/// name's bytes one char per byte (the server's platform charset is ISO-8859-1).
pub(crate) fn path_string(path: &Path) -> String {
    platform_to_internal(path.as_os_str()).into_owned()
}

/// The platform path of an internal path string (`StringEncoding.internalToPlatform`, as
/// `Path`'s file system calls apply it).
pub(crate) fn platform_path(path: &str) -> PathBuf {
    PathBuf::from(internal_to_platform(path).into_owned())
}

/// The message of the `IOException` Java's file system raises for `path`:
/// `<path> (<strerror>)`, e.g. `/ws/MODULE.bazel (No such file or directory)`.
///
/// PORT: Rust's `io::Error` prints `<strerror> (os error N)`; the strerror part is reused.
pub(crate) fn java_io_message(path: &Path, e: &std::io::Error) -> String {
    let text = e.to_string();
    let strerror = match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => &text[..i],
        _ => text.as_str(),
    };
    format!("{} ({})", path_string(path), strerror)
}

/// Registers the bzlmod SkyFunctions and injects the precomputed values of `config`.
///
/// The functions: MODULE_FILE, BAZEL_MODULE_RESOLUTION, BAZEL_DEP_GRAPH,
/// BAZEL_MODULE_INSPECTION, BAZEL_LOCK_FILE, REGISTRY, REPO_SPEC, YANKED_VERSIONS and
/// CLIENT_ENVIRONMENT_VARIABLE. SINGLE_EXTENSION_EVAL and SINGLE_EXTENSION are not registered;
/// see [`register_lockfile_only_extension_functions`].
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:745 BazelRepositoryModule.getPrecomputedValues
pub fn register_bzlmod_functions(
    builder: EvaluatorBuilder,
    config: Arc<BzlmodConfig>,
) -> EvaluatorBuilder {
    let mut builder = builder;
    builder = precomputed::STARLARK_SEMANTICS.set(builder, config.starlark_semantics.clone());
    builder = precomputed::INJECTED_REPOSITORIES.set(builder, config.injected_repositories.clone());
    builder = precomputed::MODULE_OVERRIDES.set(builder, config.module_overrides.clone());
    builder = precomputed::REGISTRIES.set(builder, config.registries.clone());
    builder = precomputed::MODULE_MIRRORS.set(builder, config.module_mirrors.clone());
    builder = precomputed::IGNORE_DEV_DEPS.set(builder, config.ignore_dev_deps);
    builder = precomputed::CHECK_DIRECT_DEPENDENCIES.set(builder, config.check_direct_dependencies);
    builder = precomputed::BAZEL_COMPATIBILITY_MODE.set(builder, config.bazel_compatibility_mode);
    builder = precomputed::LOCKFILE_MODE.set(builder, config.lockfile_mode);
    builder = precomputed::VENDOR_DIRECTORY.set(builder, config.vendor_directory.clone());
    builder =
        precomputed::ALLOWED_YANKED_VERSIONS.set(builder, config.allowed_yanked_versions.clone());
    builder = precomputed::LAST_INVALIDATION.set(builder, config.last_registry_invalidation);
    // PORT: REPOSITORY_OVERRIDES, FETCH_DISABLED, FORCE_FETCH, FORCE_FETCH_CONFIGURE,
    // REQUIRE_REPO_EXTENSION_METADATA and IS_VENDOR_COMMAND are read by repository fetching and
    // extension evaluation, which are not ported here.

    builder = precomputed::register_client_environment_function(builder, config.client_env.clone());

    let c = config.clone();
    builder = builder.typed_function::<ModuleFileKey, _>(move |key, env| {
        module_file_function::compute(&c, key, env)
    });
    let c = config.clone();
    builder = builder.typed_function::<BazelModuleResolutionKey, _>(move |_key, env| {
        resolution_function::compute(&c, env)
    });
    builder = builder.typed_function::<BazelDepGraphKey, _>(|_key, env| {
        dep_graph_function::compute_dep_graph(env)
    });
    builder = builder.typed_function::<BazelModuleInspectorKey, _>(|_key, env| {
        dep_graph_function::compute_inspector(env)
    });
    let c = config.clone();
    builder = builder.typed_function::<BazelLockFileKey, _>(move |key, env| {
        lock_file::compute(
            &c.workspace_root,
            &c.output_base,
            &*c.file_value_lookup,
            key,
            env,
        )
    });
    // `new RegistryFunction(registryFactory, workspaceRoot)`: the config holds both.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFunction.java:56 RegistryFunction.<init>
    let c = config.clone();
    builder = builder.typed_function::<RegistryKey, _>(move |key, env| {
        registry_functions::compute_registry(&c, key, env)
    });
    let c = config.clone();
    builder = builder.typed_function::<RepoSpecKey, _>(move |key, env| {
        registry_functions::compute_repo_spec(&c, key, env)
    });
    let c = config;
    builder = builder.typed_function::<YankedVersionsKey, _>(move |key, env| {
        registry_functions::compute_yanked_versions(&c, key, env)
    });
    builder
}
