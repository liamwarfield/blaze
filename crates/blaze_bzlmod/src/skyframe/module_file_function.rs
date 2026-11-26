//! Port of `ModuleFileFunction` (MODULE_FILE): takes a [`ModuleKey`] and its override (if any),
//! retrieves the module file from a registry or as directed by the override, and evaluates it.
//!
//! The Starlark part (`execModuleFile`, `injectRepos`) is [`crate::module_file::exec_module_file`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_bzlmod_model::{
    ExternalDepsCode, ExternalDepsException, InterimModule, ModuleFile, ModuleKey, ModuleOverride,
    NonRootModuleFileValue, Registry, RegistryException, RegistryFileDownloadEvent,
    RootModuleFileValue, Version, resolution::BZLMOD_ALLOWED_YANKED_VERSIONS_ENV,
};
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::repository_mapping::EMPTY as EMPTY_REPOSITORY_MAPPING;
use blaze_cmdline::{Label, LabelRepoContextImpl, PackageIdentifier, RepositoryName};
use blaze_events::{Event, ExtendedEventHandler, StoredEventHandler};
use blaze_skyframe::{ComputeError, Environment, Transience};
use blaze_starlark::StarlarkSemantics;
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::config::{BzlmodConfig, RepositoryDirectoryLookup};
use super::keys::{ModuleFileKey, ModuleFileValue, RegistryKey};
use super::precomputed::{
    ClientEnvironmentKey, IGNORE_DEV_DEPS, INJECTED_REPOSITORIES, MODULE_OVERRIDES, REGISTRIES,
    STARLARK_SEMANTICS,
};
use super::{java_io_message, path_string, platform_path, throw};
use crate::module_file::{
    CompiledModuleFile, IncludeStatement, ModuleThreadContext, exec_module_file,
};

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:121 ModuleFileFunction.BZLMOD_REMINDER
const BZLMOD_REMINDER: &str = "\
###############################################################################
# Bazel now uses Bzlmod by default to manage external dependencies.
# Please consider migrating your external dependencies from WORKSPACE to MODULE.bazel.
#
# For more details, please check https://github.com/bazelbuild/bazel/issues/18958
###############################################################################
";

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:130 ModuleFileFunction.INCLUDE_FILENAME_SUFFIX
const INCLUDE_FILENAME_SUFFIX: &str = ".MODULE.bazel";

/// `errorf(code, format, args)`: a `ModuleFileFunctionException` with the one-argument
/// constructor, which is TRANSIENT.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:989 ModuleFileFunction.errorf
fn errorf(code: ExternalDepsCode, message: String) -> ComputeError {
    throw(
        ExternalDepsException::with_message(code, message),
        Transience::Transient,
    )
}

/// `ModuleFileMetadata`: the registry (null for non-registry overrides) and the registry file
/// hashes collected while fetching the module file.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:146 ModuleFileFunction.ModuleFileMetadata
struct ModuleFileMetadata {
    registry: Option<Arc<dyn Registry>>,
    registry_file_hashes: IndexMap<String, Option<blaze_bzlmod_model::Checksum>>,
}

/// Result of a [`get_module_file`] call; `registry` is `None` if this module has a
/// non-registry override.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:741 ModuleFileFunction.GetModuleFileResult
struct GetModuleFileResult {
    module_file: ModuleFile,
    registry: Option<Arc<dyn Registry>>,
    download_event_handler: StoredEventHandler,
}

/// PORT: Java keeps the compiled module file and the include horizon in a `SkyKeyComputeState`
/// to survive Skyframe restarts; the blocking evaluator never restarts, so they are locals.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:164 ModuleFileFunction.compute
pub fn compute(
    config: &BzlmodConfig,
    sky_key: &ModuleFileKey,
    env: &mut Environment<'_>,
) -> Result<Option<ModuleFileValue>, ComputeError> {
    let Some(starlark_semantics) = STARLARK_SEMANTICS.get(env)? else {
        return Ok(None);
    };

    if *sky_key == ModuleFileKey::key_for_root_module() {
        return Ok(
            compute_for_root_module(config, &starlark_semantics, env)?.map(ModuleFileValue::Root)
        );
    }

    let allowed_yanked_versions_from_env = env.get_value(ClientEnvironmentKey::key(
        BZLMOD_ALLOWED_YANKED_VERSIONS_ENV,
    ))?;
    if allowed_yanked_versions_from_env.is_none() {
        return Ok(None);
    }

    let Some(root_module_file_value) = env.get_value(ModuleFileKey::key_for_root_module())? else {
        return Ok(None);
    };
    let root_module_file_value = root_module_file_value.as_root();

    let module_key = sky_key.module_key();
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:194 ModuleFileFunction.compute
    let get_module_file_result = {
        let _c = Profiler::instance().profile_with(ProfilerTask::BZLMOD, || {
            format!("fetch module file: {module_key}")
        });
        get_module_file(
            config,
            module_key,
            root_module_file_value.overrides.get(module_key.name()),
            env,
        )?
    };
    let Some(get_module_file_result) = get_module_file_result else {
        return Ok(None);
    };

    let module_file_metadata = ModuleFileMetadata {
        registry: get_module_file_result.registry.clone(),
        registry_file_hashes: RegistryFileDownloadEvent::collect_to_map(
            &get_module_file_result.download_event_handler.get_posts(),
        ),
    };
    let compiled_module_file = CompiledModuleFile::parse_and_compile(
        get_module_file_result.module_file,
        module_key,
        &starlark_semantics,
        env.listener(),
    )
    .map_err(|e| throw(e, Transience::Persistent))?;

    let mut module_thread_context: ModuleThreadContext;
    if module_file_metadata.registry.is_some() {
        let non_dev_include = compiled_module_file
            .include_statements
            .iter()
            .find(|include| !include.dev_dependency);
        if let Some(non_dev_include) = non_dev_include {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "include() directive found at {}, but it can only be used in the root module \
                     or in modules with non-registry overrides",
                    non_dev_include.location
                ),
            ));
        }
        module_thread_context = exec_module_file(
            &compiled_module_file,
            /* include_label_to_parsed_module_file= */ None,
            module_key,
            // Dev dependencies should always be ignored if the current module isn't the root
            // module.
            /* ignore_dev_deps= */
            true,
            &config.builtin_modules,
            /* injected_repositories= */ &IndexMap::new(),
            // Disable printing for modules from registries. We don't want them to be able to spam
            // the console during resolution.
            /* print_is_noop= */
            true,
            &starlark_semantics,
            env.listener(),
        )
        .map_err(|e| throw(e, Transience::Transient))?;
    } else {
        let Some((context, _)) = exec_non_registry_module_file(
            config,
            module_key,
            &compiled_module_file,
            &starlark_semantics,
            env,
        )?
        else {
            return Ok(None);
        };
        module_thread_context = context;
    }

    // Perform some sanity checks.
    let module: InterimModule =
        match module_thread_context.build_module(module_file_metadata.registry.clone()) {
            Ok(module) => module,
            Err(e) => {
                env.listener().handle(&Event::error_at(
                    e.innermost_location.clone(),
                    e.message_with_stack.clone(),
                ));
                return Err(errorf(
                    ExternalDepsCode::BadModule,
                    format!("error executing MODULE.bazel file for {module_key}"),
                ));
            }
        };
    if module.name != module_key.name() {
        return Err(errorf(
            ExternalDepsCode::BadModule,
            format!(
                "the MODULE.bazel file of {} declares a different name ({})",
                module_key, module.name
            ),
        ));
    }
    if !module_key.version().is_empty() && module.version != *module_key.version() {
        return Err(errorf(
            ExternalDepsCode::BadModule,
            format!(
                "the MODULE.bazel file of {} declares a different version ({})",
                module_key, module.version
            ),
        ));
    }

    Ok(Some(ModuleFileValue::NonRoot(NonRootModuleFileValue {
        module,
        registry_file_hashes: module_file_metadata.registry_file_hashes,
    })))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:290 ModuleFileFunction.computeForRootModule
fn compute_for_root_module(
    config: &BzlmodConfig,
    starlark_semantics: &Arc<StarlarkSemantics>,
    env: &mut Environment<'_>,
) -> Result<Option<RootModuleFileValue>, ComputeError> {
    let module_file_path = get_module_file_path(&config.workspace_root);
    if config
        .file_value_lookup
        .file_value(&config.workspace_root, &MODULE_DOT_BAZEL_FILE_NAME, env)?
        .is_none()
    {
        return Ok(None);
    }
    let module_file_contents: Vec<u8> = if module_file_path.exists() {
        read_module_file(&module_file_path)?
    } else {
        let contents = BZLMOD_REMINDER.as_bytes().to_vec();
        create_module_file(&module_file_path, &contents)?;
        env.listener().handle(&Event::warn(
            "--enable_bzlmod is set, but no MODULE.bazel file was found at the workspace root. \
             Bazel will create an empty MODULE.bazel file. Please consider migrating your \
             external dependencies from WORKSPACE to MODULE.bazel. For more details, please \
             refer to https://github.com/bazelbuild/bazel/issues/18958.",
        ));
        contents
    };
    let compiled_module_file = CompiledModuleFile::parse_and_compile(
        ModuleFile::create(module_file_contents, path_string(&module_file_path)),
        &ModuleKey::root(),
        starlark_semantics,
        env.listener(),
    )
    .map_err(|e| throw(e, Transience::Persistent))?;

    let Some((mut module_thread_context, include_label_to_compiled_module_file)) =
        exec_non_registry_module_file(
            config,
            &ModuleKey::root(),
            &compiled_module_file,
            starlark_semantics,
            env,
        )?
    else {
        return Ok(None);
    };
    let command_overrides = MODULE_OVERRIDES.get_required(env)?;
    Ok(Some(build_root_module_file_value(
        &mut module_thread_context,
        &include_label_to_compiled_module_file,
        &command_overrides,
        env.listener(),
    )?))
}

/// `compiled_module_file` must be the module's compiled MODULE.bazel. Returns the evaluated
/// context and the compiled includes.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:340 ModuleFileFunction.execNonRegistryModuleFile
/// The evaluated context and the compiled included files.
type ExecutedModuleFile = (
    ModuleThreadContext,
    Arc<IndexMap<String, CompiledModuleFile>>,
);

fn exec_non_registry_module_file(
    config: &BzlmodConfig,
    module_key: &ModuleKey,
    compiled_module_file: &CompiledModuleFile,
    starlark_semantics: &Arc<StarlarkSemantics>,
    env: &mut Environment<'_>,
) -> Result<Option<ExecutedModuleFile>, ComputeError> {
    let is_root = module_key.is_root();
    let ignore_dev_deps = if is_root {
        *IGNORE_DEV_DEPS.get_required(env)?
    } else {
        true
    };
    let mut include_label_to_compiled_module_file: IndexMap<String, CompiledModuleFile> =
        IndexMap::new();
    let mut horizon =
        active_include_statements(&compiled_module_file.include_statements, ignore_dev_deps);
    while !horizon.is_empty() {
        let Some(new_horizon) = advance_horizon(
            config,
            module_key,
            &mut include_label_to_compiled_module_file,
            &horizon,
            env,
            starlark_semantics,
            ignore_dev_deps,
        )?
        else {
            return Ok(None);
        };
        horizon = new_horizon;
    }
    let injected_repositories = if is_root {
        (*INJECTED_REPOSITORIES.get_required(env)?).clone()
    } else {
        IndexMap::new()
    };
    let include_label_to_compiled_module_file = Arc::new(include_label_to_compiled_module_file);
    let context = exec_module_file(
        compiled_module_file,
        Some(include_label_to_compiled_module_file.clone()),
        module_key,
        ignore_dev_deps,
        &config.builtin_modules,
        &injected_repositories,
        // Allow printing to aid in debugging non-registry overrides, which are often edited by the
        // user.
        /* print_is_noop= */
        false,
        starlark_semantics,
        env.listener(),
    )
    .map_err(|e| throw(e, Transience::Transient))?;
    Ok(Some((context, include_label_to_compiled_module_file)))
}

/// Reads, parses, and compiles all module files named by `horizon` that haven't been compiled
/// yet, stores the result in `include_label_to_compiled_module_file`, and returns the include
/// statements of these newly compiled module files as a new "horizon".
///
/// Skipping files that have already been compiled isn't just an optimization: without it, an
/// `include()` cycle would keep the horizon non-empty forever and a "diamond" include structure
/// would result in a number of compilations exponential in the include depth. Cycles are
/// reported as an error when the module file is executed (`ModuleThreadContext.include`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:395 ModuleFileFunction.advanceHorizon
fn advance_horizon(
    config: &BzlmodConfig,
    module_key: &ModuleKey,
    include_label_to_compiled_module_file: &mut IndexMap<String, CompiledModuleFile>,
    horizon: &[IncludeStatement],
    env: &mut Environment<'_>,
    starlark_semantics: &Arc<StarlarkSemantics>,
    ignore_dev_deps: bool,
) -> Result<Option<Vec<IncludeStatement>>, ComputeError> {
    let mut seen_include_labels: HashSet<String> = include_label_to_compiled_module_file
        .keys()
        .cloned()
        .collect();
    let pending: Vec<IncludeStatement> = horizon
        .iter()
        .filter(|s| seen_include_labels.insert(s.include_label.clone()))
        .cloned()
        .collect();
    if pending.is_empty() {
        return Ok(Some(Vec::new()));
    }
    // Includes are only allowed in the root module as well as those with non-registry overrides,
    // so their repo name never contains a version.
    let repo_context = LabelRepoContextImpl::of(
        module_key.get_canonical_repo_name_without_version(),
        EMPTY_REPOSITORY_MAPPING.clone(),
    );
    let mut include_labels: Vec<Label> = Vec::with_capacity(pending.len());
    for include_statement in &pending {
        if !include_statement.include_label.starts_with("//") {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "bad include label '{}' at {}: include() must be called with repo-relative \
                     labels (starting with double slashes)",
                    include_statement.include_label, include_statement.location
                ),
            ));
        }
        let include_label =
            match Label::parse_with_repo_context(&include_statement.include_label, &repo_context) {
                Ok(l) => l,
                Err(e) => {
                    return Err(errorf(
                        ExternalDepsCode::BadModule,
                        format!(
                            "bad include label '{}' at {}: {}",
                            include_statement.include_label,
                            include_statement.location,
                            e.get_message()
                        ),
                    ));
                }
            };
        let name = include_label.get_name();
        let basename = &name[name.rfind('/').map_or(0, |i| i + 1)..];
        if !basename.ends_with(INCLUDE_FILENAME_SUFFIX) {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "bad include label '{}' at {}: the file to be included must have a name \
                     ending in '{}'",
                    include_statement.include_label,
                    include_statement.location,
                    INCLUDE_FILENAME_SUFFIX
                ),
            ));
        }
        if basename.starts_with('.') {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "bad include label '{}' at {}: the name of the file to be included must not \
                     start with '.'",
                    include_statement.include_label, include_statement.location
                ),
            ));
        }
        include_labels.push(include_label);
    }
    // Java: env.getValuesAndExceptions(PackageLookupValue.key(l.getPackageIdentifier()) ...)
    let mut pkg_lookup_values: Vec<Option<PackageLookupResult>> =
        Vec::with_capacity(include_labels.len());
    for include_label in &include_labels {
        pkg_lookup_values.push(
            config
                .package_lookup
                .lookup(include_label.get_package_identifier(), env)?,
        );
    }
    let mut rooted_paths: Vec<PathBuf> = Vec::with_capacity(pending.len());
    // The (root, root-relative path) of each rooted path, for its FileValue key.
    let mut rooted_path_parts: Vec<(PathBuf, PathFragment)> = Vec::with_capacity(pending.len());
    for (i, include_label) in include_labels.iter().enumerate() {
        let Some(pkg_lookup_value) = &pkg_lookup_values[i] else {
            return Ok(None);
        };
        if !pkg_lookup_value.package_exists {
            let mut message = pkg_lookup_value.get_error_msg().to_owned();
            if pkg_lookup_value.is_no_build_file_value {
                let Some(explanation) = config
                    .package_lookup
                    .explain_no_build_file_value(include_label.get_package_identifier(), env)?
                else {
                    return Ok(None);
                };
                message = explanation;
            }
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "unable to load package for '{}' included at {}: {}",
                    pending[i].include_label, pending[i].location, message
                ),
            ));
        }
        // RootedPath.toRootedPath(pkgLookupValue.getRoot(), includeLabel.toPathFragment())
        rooted_paths.push(pkg_lookup_value.get_root().join(platform_path(
            include_label.to_path_fragment().get_path_string(),
        )));
        rooted_path_parts.push((
            pkg_lookup_value.get_root().to_path_buf(),
            include_label.to_path_fragment(),
        ));
    }
    // Resolve all file dependencies before compiling so a restart cannot retain a compiled file
    // while discarding its nested includes from the next horizon.
    // PORT: Java's batch request (getValuesAndExceptions) is one request per key here.
    let mut file_values = Vec::with_capacity(pending.len());
    for (root, relative) in &rooted_path_parts {
        file_values.push(config.file_value_lookup.file_value(root, relative, env)?);
    }
    let mut checked_file_values = Vec::with_capacity(pending.len());
    for file_value in file_values {
        let Some(file_value) = file_value else {
            return Ok(None);
        };
        checked_file_values.push(file_value);
    }
    let mut new_horizon: Vec<IncludeStatement> = Vec::new();
    for (i, rooted_path) in rooted_paths.iter().enumerate() {
        if !checked_file_values[i].is_file {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "error reading '{}' included at {}: not a regular file",
                    pending[i].include_label, pending[i].location
                ),
            ));
        }
        let bytes = match std::fs::read(rooted_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(errorf(
                    ExternalDepsCode::BadModule,
                    format!(
                        "error reading '{}' included at {}: {}",
                        pending[i].include_label,
                        pending[i].location,
                        java_io_message(rooted_path, &e)
                    ),
                ));
            }
        };
        let compiled_module_file = CompiledModuleFile::parse_and_compile(
            // `RootedPath.asPath().getPathString()`.
            ModuleFile::create(bytes, path_string(rooted_path)),
            &ModuleKey::root(),
            starlark_semantics,
            env.listener(),
        )
        .map_err(|e| throw(e, Transience::Persistent))?;
        new_horizon.extend(active_include_statements(
            &compiled_module_file.include_statements,
            ignore_dev_deps,
        ));
        include_label_to_compiled_module_file
            .insert(pending[i].include_label.clone(), compiled_module_file);
    }
    Ok(Some(new_horizon))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:545 ModuleFileFunction.activeIncludeStatements
fn active_include_statements(
    include_statements: &[IncludeStatement],
    ignore_dev_deps: bool,
) -> Vec<IncludeStatement> {
    if !ignore_dev_deps {
        return include_statements.to_vec();
    }
    include_statements
        .iter()
        .filter(|include| !include.dev_dependency)
        .cloned()
        .collect()
}

/// What `ModuleFileFunction` reads from a `PackageLookupValue` (the value of PACKAGE_LOOKUP for
/// one package).
// Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:47 PackageLookupValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLookupResult {
    /// `packageExists()`.
    pub package_exists: bool,
    /// `getRoot()`: the package path entry (or repository directory) the package was found
    /// under. `None` for an unsuccessful lookup (Java throws `IllegalStateException`).
    pub root: Option<PathBuf>,
    /// `getBuildFileName().getFilenameFragment()`, e.g. `BUILD.bazel`. `None` for an
    /// unsuccessful lookup (Java throws `IllegalStateException`).
    pub build_file_name: Option<String>,
    /// `this == PackageLookupValue.NO_BUILD_FILE_VALUE`.
    pub is_no_build_file_value: bool,
    /// `getErrorMsg()`. `None` for a successful lookup (Java throws `IllegalStateException`).
    pub error_msg: Option<String>,
}

impl PackageLookupResult {
    /// `PackageLookupValue.success(root, buildFileName)`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:160 SuccessfulPackageLookupValue
    pub fn success(root: PathBuf, build_file_name: impl Into<String>) -> PackageLookupResult {
        PackageLookupResult {
            package_exists: true,
            root: Some(root),
            build_file_name: Some(build_file_name.into()),
            is_no_build_file_value: false,
            error_msg: None,
        }
    }

    /// `PackageLookupValue.NO_BUILD_FILE_VALUE`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:239 NoBuildFilePackageLookupValue
    pub fn no_build_file() -> PackageLookupResult {
        PackageLookupResult {
            package_exists: false,
            root: None,
            build_file_name: None,
            is_no_build_file_value: true,
            error_msg: Some("BUILD file not found on package path".to_owned()),
        }
    }

    /// Any other unsuccessful value (`DELETED_PACKAGE_VALUE`, `invalidPackageName`,
    /// `incorrectRepositoryReference`, `NoRepositoryPackageLookupValue`) with its `getErrorMsg()`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:220 UnsuccessfulPackageLookupValue
    pub fn unsuccessful(error_msg: impl Into<String>) -> PackageLookupResult {
        PackageLookupResult {
            package_exists: false,
            root: None,
            build_file_name: None,
            is_no_build_file_value: false,
            error_msg: Some(error_msg.into()),
        }
    }

    /// `getRoot()`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:228 UnsuccessfulPackageLookupValue.getRoot
    pub fn get_root(&self) -> &Path {
        match &self.root {
            Some(root) => root,
            None => panic!("IllegalStateException"),
        }
    }

    /// `getErrorMsg()`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:202 SuccessfulPackageLookupValue.getErrorMsg
    pub fn get_error_msg(&self) -> &str {
        match &self.error_msg {
            Some(error_msg) => error_msg,
            None => panic!("IllegalStateException"),
        }
    }
}

/// The PACKAGE_LOOKUP seam of `ModuleFileFunction`, which needs it for the packages of
/// `include()`d files and of `single_version_override` patches
/// (ModuleFileFunction.java:466-490 and :896-915). PACKAGE_LOOKUP belongs to the loading phase
/// (`blaze_sky_loading`), which this crate cannot depend on, so the function is reached through
/// [`BzlmodConfig::package_lookup`], like [`RepositoryDirectoryLookup`].
///
/// An implementation backed by the real PACKAGE_LOOKUP honours `--package_path`,
/// `--deleted_packages`, `.bazelignore` and `REPO.bazel`'s `ignore_directories()`. The default,
/// [`BuiltinPackageLookup`], knows none of them and can only detect the last two (see there).
pub trait PackageLookup: Send + Sync {
    /// `env.getValue(PackageLookupValue.key(packageId))`, reduced to what `ModuleFileFunction`
    /// reads. `Ok(None)` when a dependency is unavailable (Java returns null); an error of
    /// PACKAGE_LOOKUP itself (`BuildFileNotFoundException`, `InconsistentFilesystemException`)
    /// is the `Err`.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:84 PackageLookupFunction.compute
    fn lookup(
        &self,
        package_id: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackageLookupResult>, ComputeError>;

    /// `PackageLookupFunction.explainNoBuildFileValue(packageId, env)`: the message for a
    /// package without a BUILD file, which lists the package path entries
    /// (`PrecomputedValue.PATH_PACKAGE_LOCATOR`). `Ok(None)` when a dependency is unavailable.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:130 PackageLookupFunction.explainNoBuildFileValue
    fn explain_no_build_file_value(
        &self,
        package_id: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<Option<String>, ComputeError>;
}

/// PORT: a stand-in for `PackageLookupFunction` (PACKAGE_LOOKUP), for commands and tests that do
/// not wire the real one: a package exists when its directory has a `BUILD.bazel` or `BUILD`
/// file, under the workspace for the main repository and under
/// [`RepositoryDirectoryLookup::repo_root`] for another repository (the module files of modules
/// with non-registry overrides may `include()` files of their own repository). That is
/// PACKAGE_LOOKUP's answer only with the default `--package_path` (`%workspace%`), no
/// `--deleted_packages`, and nothing ignored by `.bazelignore` or by `ignore_directories()` in
/// `REPO.bazel`.
///
/// The stand-in does not see the two flags (`BzlmodConfig` does not carry them): whoever passes
/// a non-default `--package_path` or any `--deleted_packages` must install a real
/// [`PackageLookup`]. It fails loudly (`unported!`) where it can tell that its answer may be
/// wrong: a `.bazelignore` file in the repository, a `REPO.bazel` that mentions
/// `ignore_directories`, and an I/O error on a BUILD file.
#[derive(Clone)]
pub struct BuiltinPackageLookup {
    /// The workspace directory, the only package path entry.
    pub workspace_root: PathBuf,
    /// Where the repositories other than the main one are (REPOSITORY_DIRECTORY seam).
    pub repository_directory_lookup: Arc<dyn RepositoryDirectoryLookup>,
}

/// `BazelSkyframeExecutorConstants.BUILD_FILES_BY_PRIORITY`.
// Java: com/google/devtools/build/lib/skyframe/BazelSkyframeExecutorConstants.java:32 BazelSkyframeExecutorConstants.BUILD_FILES_BY_PRIORITY
const BUILD_FILES_BY_PRIORITY: [&str; 2] = ["BUILD.bazel", "BUILD"];

impl BuiltinPackageLookup {
    /// The part of the stand-in's `PackageLookupFunction.compute` that follows the lookup of the
    /// repository's `root`: the ignored-subdirectories check and the search for a BUILD file. It
    /// requests no Skyframe dependency.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:84 PackageLookupFunction.compute
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:335 PackageLookupFunction.computeExternalPackageLookupValue
    pub fn find_package(root: &Path, package_id: &PackageIdentifier) -> PackageLookupResult {
        // packageKey.equals(LabelConstants.EXTERNAL_PACKAGE_IDENTIFIER)
        if package_id.get_repository().is_main()
            && package_id.get_package_fragment().get_path_string() == "external"
        {
            return PackageLookupResult::no_build_file();
        }

        // Check .bazelignore file under main repository. / Check .bazelignore file after
        // fetching the external repository.
        // PORT: IgnoredSubdirectoriesFunction is not ported here. A package under an ignored
        // directory is DELETED_PACKAGE_VALUE in Java, which this stand-in cannot tell.
        let bazelignore = root.join(".bazelignore");
        if std::fs::symlink_metadata(&bazelignore).is_ok() {
            blaze_util::unported!(
                "IgnoredSubdirectoriesFunction.compute",
                "{} exists; the PACKAGE_LOOKUP stand-in of ModuleFileFunction cannot tell \
                 whether package '{}' is ignored",
                path_string(&bazelignore),
                package_id
            );
        }
        let repo_file = root.join("REPO.bazel");
        // REPO.bazel is Starlark source: one char per byte (`ParserInput.fromLatin1`).
        if let Ok(content) = std::fs::read(&repo_file)
            && blaze_util::string_encoding::latin1_decode(&content).contains("ignore_directories")
        {
            blaze_util::unported!(
                "IgnoredSubdirectoriesFunction.compute",
                "{} uses ignore_directories(); the PACKAGE_LOOKUP stand-in of \
                 ModuleFileFunction cannot tell whether package '{}' is ignored",
                path_string(&repo_file),
                package_id
            );
        }

        // findPackageByBuildFile with a single package path entry (--package_path is not known
        // here; see the type's documentation).
        let package_dir = root.join(platform_path(
            package_id.get_package_fragment().get_path_string(),
        ));
        for build_file_name in BUILD_FILES_BY_PRIORITY {
            let build_file = package_dir.join(build_file_name);
            // fileValue.isFile(), following symbolic links.
            match std::fs::metadata(&build_file) {
                Ok(metadata) if metadata.is_file() => {
                    return PackageLookupResult::success(root.to_path_buf(), build_file_name);
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                // Java: "IO errors while looking for BUILD file reading ...", a symlink cycle, ...
                Err(e) => blaze_util::unported!(
                    "PackageLookupFunction.getFileValue",
                    "{}: {}",
                    path_string(&build_file),
                    e
                ),
            }
        }
        PackageLookupResult::no_build_file()
    }
}

impl PackageLookup for BuiltinPackageLookup {
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:84 PackageLookupFunction.compute
    fn lookup(
        &self,
        package_id: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackageLookupResult>, ComputeError> {
        // PORT: LabelValidator.validatePackageName is skipped: the identifiers come from parsed
        // labels. `deletedPackages` (--deleted_packages) is not known here; see the type's
        // documentation.
        let root: PathBuf = if package_id.get_repository().is_main() {
            self.workspace_root.clone()
        } else {
            // PORT: a repository that cannot be found is the lookup's error here; Java returns
            // NoRepositoryPackageLookupValue for RepositoryDirectoryValue.Failure.
            match self
                .repository_directory_lookup
                .repo_root(package_id.get_repository(), env)?
            {
                Some(r) => r,
                None => return Ok(None),
            }
        };
        Ok(Some(Self::find_package(&root, package_id)))
    }

    /// PORT: Java lists every package path entry of `PATH_PACKAGE_LOCATOR` for the main repo; the
    /// stand-in has a single entry (the workspace), for which Java prints the package fragment.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:130 PackageLookupFunction.explainNoBuildFileValue
    fn explain_no_build_file_value(
        &self,
        package_key: &PackageIdentifier,
        _env: &mut Environment<'_>,
    ) -> Result<Option<String>, ComputeError> {
        let educational_message = "Add a BUILD file to a directory to mark it as a package.";
        Ok(Some(if package_key.get_repository().is_main() {
            format!(
                "BUILD file not found in any of the following directories. {}\n - {}",
                educational_message,
                package_key.get_package_fragment().get_path_string()
            )
        } else {
            format!(
                "BUILD file not found in directory '{}' of external repository {}. {}",
                package_key.get_package_fragment(),
                package_key.get_repository(),
                educational_message
            )
        }))
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:555 ModuleFileFunction.getModuleFilePath
pub fn get_module_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(platform_path(MODULE_DOT_BAZEL_FILE_NAME.get_path_string()))
}

/// The Java record `toString` of a `ModuleOverride` (used in error messages).
fn module_override_to_java_string(override_: &ModuleOverride) -> String {
    use blaze_bzlmod_model::repo_spec::{AttributeValue, AttributeValues};

    // PORT: `Starlark.repr` of the attribute values, for the kinds AttributeValues holds. Control
    // characters other than \n, \r, \t are printed as is.
    fn repr(v: &AttributeValue, out: &mut String) {
        match v {
            AttributeValue::None => out.push_str("None"),
            AttributeValue::Bool(b) => out.push_str(if *b { "True" } else { "False" }),
            AttributeValue::Int(i) => out.push_str(&i.to_string()),
            AttributeValue::String(s) => repr_string(s, out),
            AttributeValue::Label(l) => {
                out.push_str("Label(");
                repr_string(&l.get_unambiguous_canonical_form(), out);
                out.push(')');
            }
            AttributeValue::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    repr(item, out);
                }
                out.push(']');
            }
            AttributeValue::Dict(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    repr(k, out);
                    out.push_str(": ");
                    repr(v, out);
                }
                out.push('}');
            }
        }
    }
    fn repr_string(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    fn attribute_values(values: &AttributeValues) -> String {
        let mut out = String::from("AttributeValues{attributes={");
        for (i, (k, v)) in values.attributes().iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_string(k, &mut out);
            out.push_str(": ");
            repr(v, &mut out);
        }
        out.push_str("}}");
        out
    }
    fn list<T: std::fmt::Display>(items: &[T]) -> String {
        let parts: Vec<String> = items.iter().map(|i| i.to_string()).collect();
        format!("[{}]", parts.join(", "))
    }
    match override_ {
        ModuleOverride::SingleVersion(o) => format!(
            "SingleVersionOverride[version={}, registry={}, patches={}, patchCmds={}, \
             patchStrip={}]",
            o.version,
            o.registry,
            list(
                &o.patches
                    .iter()
                    .map(|l| l.get_canonical_form())
                    .collect::<Vec<_>>()
            ),
            list(&o.patch_cmds),
            o.patch_strip
        ),
        ModuleOverride::MultipleVersion(o) => format!(
            "MultipleVersionOverride[versions={}, registry={}]",
            list(&o.versions),
            o.registry
        ),
        ModuleOverride::NonRegistry(o) => {
            let repo_spec = o.repo_spec();
            format!(
                "NonRegistryOverride[repoSpec=RepoSpec[repoRuleId={}, attributes={}]]",
                repo_spec
                    .repo_rule_id()
                    .map_or("null".to_string(), |id| id.to_string()),
                attribute_values(repo_spec.attributes())
            )
        }
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:560 ModuleFileFunction.buildRootModuleFileValue
pub fn build_root_module_file_value(
    module_thread_context: &mut ModuleThreadContext,
    include_label_to_compiled_module_file: &IndexMap<String, CompiledModuleFile>,
    command_overrides: &IndexMap<String, ModuleOverride>,
    event_handler: &dyn ExtendedEventHandler,
) -> Result<RootModuleFileValue, ComputeError> {
    let module = match module_thread_context.build_module(/* registry= */ None) {
        Ok(module) => module,
        Err(e) => {
            event_handler.handle(&Event::error_at(
                e.innermost_location.clone(),
                e.message_with_stack.clone(),
            ));
            return Err(errorf(
                ExternalDepsCode::BadModule,
                "error executing MODULE.bazel file for the root module".to_string(),
            ));
        }
    };
    for usage in &module.extension_usages {
        let first_proxy = &usage.proxies[0];
        if usage.isolation_key.is_some() && first_proxy.imports.is_empty() {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "the isolated usage at {} of extension {} defined in {} has no effect as no \
                     repositories are imported from it. Either import one or more repositories \
                     generated by the extension with use_repo or remove the usage.",
                    first_proxy.location, usage.extension_name, usage.extension_bzl_file
                ),
            ));
        }
    }

    let module_overrides = module_thread_context.build_overrides();
    // putAll(moduleOverrides).putAll(commandOverrides).buildKeepingLast()
    let mut overrides: IndexMap<String, ModuleOverride> = module_overrides;
    for (k, v) in command_overrides {
        overrides.insert(k.clone(), v.clone());
    }
    for (module_name, override_) in &overrides {
        let ModuleOverride::SingleVersion(svo) = override_ else {
            continue;
        };
        let Some(dep) = module.deps.get(module_name) else {
            continue;
        };
        let dep_version: &Version = &dep.version;
        if !dep_version.is_empty() && svo.version < *dep_version {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "module '{}' is overridden to use version '{}', which is lower than the \
                     version '{}' requested by the root module",
                    module_name, svo.version, dep_version
                ),
            ));
        }
    }

    // Check that overrides don't contain the root module itself.
    if let Some(root_override) = overrides.get(&module.name) {
        return Err(errorf(
            ExternalDepsCode::BadModule,
            format!(
                "invalid override for the root module found: {}",
                module_override_to_java_string(root_override)
            ),
        ));
    }
    let mut non_registry_override_module_to_repo_name: IndexMap<String, String> = IndexMap::new();
    for (repo_name, dep) in &module.deps {
        if matches!(
            overrides.get(&dep.name),
            Some(ModuleOverride::NonRegistry(_))
        ) {
            non_registry_override_module_to_repo_name.insert(dep.name.clone(), repo_name.clone());
        }
    }
    let mut non_registry_override_canonical_repo_to_module_name: IndexMap<RepositoryName, String> =
        IndexMap::new();
    for (name, override_) in &overrides {
        if matches!(override_, ModuleOverride::NonRegistry(_)) {
            // A module with a non-registry override always has a unique version across the
            // entire dep graph.
            non_registry_override_canonical_repo_to_module_name.insert(
                ModuleKey::new(name.clone(), Version::empty())
                    .get_canonical_repo_name_without_version(),
                name.clone(),
            );
        }
    }
    let mut module_file_paths: IndexSet<PathFragment> = IndexSet::new();
    module_file_paths.insert(MODULE_DOT_BAZEL_FILE_NAME.clone());
    for label in include_label_to_compiled_module_file.keys() {
        module_file_paths.insert(Label::parse_canonical_unchecked(label).to_path_fragment());
    }
    Ok(RootModuleFileValue {
        module,
        overrides,
        non_registry_override_canonical_repo_to_module_name,
        non_registry_override_module_to_repo_name,
        module_file_paths,
    })
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:747 ModuleFileFunction.getModuleFile
fn get_module_file(
    config: &BzlmodConfig,
    key: &ModuleKey,
    override_: Option<&ModuleOverride>,
    env: &mut Environment<'_>,
) -> Result<Option<GetModuleFileResult>, ComputeError> {
    // If there is a non-registry override for this module, we need to fetch the corresponding repo
    // first and read the module file from there.
    if let Some(ModuleOverride::NonRegistry(_)) = override_ {
        // A module with a non-registry override always has a unique version across the entire dep
        // graph.
        let canonical_repo_name = key.get_canonical_repo_name_without_version();
        let Some(repo_dir) = config
            .repository_directory_lookup
            .repo_root(&canonical_repo_name, env)?
        else {
            return Ok(None);
        };
        // This repo _definitely_ exists, since it has a non-registry override, which directly gets
        // "translated" into a repo spec.
        let module_file_path =
            repo_dir.join(platform_path(MODULE_DOT_BAZEL_FILE_NAME.get_path_string()));
        if config
            .file_value_lookup
            .file_value(&repo_dir, &MODULE_DOT_BAZEL_FILE_NAME, env)?
            .is_none()
        {
            return Ok(None);
        }
        let module_file_label = Label::create_unvalidated(
            PackageIdentifier::create(canonical_repo_name, PathFragment::create("")),
            MODULE_DOT_BAZEL_FILE_NAME.get_base_name(),
        );
        return Ok(Some(GetModuleFileResult {
            module_file: ModuleFile::create(
                read_module_file(&module_file_path)?,
                module_file_label.get_unambiguous_canonical_form(),
            ),
            registry: None,
            download_event_handler: StoredEventHandler::new(),
        }));
    }

    // Otherwise, we should get the module file from a registry.
    if key.version().is_empty() {
        // Print a friendlier error message if the user forgets to specify a version *and* doesn't
        // have a non-registry override.
        return Err(errorf(
            ExternalDepsCode::ModuleNotFound,
            format!(
                "bad bazel_dep on module '{}' with no version. Did you forget to specify a \
                 version, or a non-registry override?",
                key.name()
            ),
        ));
    }
    let mut registries: IndexSet<String> = (*REGISTRIES.get_required(env)?).clone();
    if let Some(override_) = override_ {
        // RegistryOverride (SingleVersionOverride, MultipleVersionOverride).
        let override_registry = override_
            .get_registry()
            .expect("non-registry overrides are handled above");
        if !override_registry.is_empty() {
            registries = IndexSet::from([override_registry.to_string()]);
        }
    }

    let registry_keys: Vec<RegistryKey> = registries.iter().map(RegistryKey::create).collect();
    let sky_keys: Vec<blaze_skyframe::SkyKey> = registry_keys
        .iter()
        .cloned()
        .map(blaze_skyframe::SkyKey::new)
        .collect();
    let registry_result = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    let mut registry_objects: Vec<Arc<dyn Registry>> = Vec::with_capacity(registry_keys.len());
    for registry_key in &registry_keys {
        let Some(registry) = registry_result.get_typed(registry_key) else {
            return Ok(None);
        };
        registry_objects.push((*registry).clone());
    }

    // Now go through the list of registries and use the first one that contains the requested
    // module.
    let download_event_handler = StoredEventHandler::new();
    let mut not_found_trace: Option<Vec<String>> = None;
    for registry in registry_objects {
        let original_module_file =
            match registry.get_module_file(key, &download_event_handler, &*config.registry_fetcher)
            {
                Ok(m) => m,
                Err(RegistryException::NotFound(message)) => {
                    not_found_trace.get_or_insert_with(Vec::new).push(message);
                    continue;
                }
                Err(RegistryException::Interrupted) => {
                    return Err(blaze_skyframe::Interrupted.into());
                }
                Err(e @ RegistryException::Io(_)) => {
                    return Err(registry_io_error(&registry, e));
                }
            };
        let Some(module_file) =
            maybe_patch_module_file(config, original_module_file, override_, env)?
        else {
            return Ok(None);
        };
        return Ok(Some(GetModuleFileResult {
            module_file,
            registry: Some(registry),
            download_event_handler,
        }));
    }

    Err(errorf(
        ExternalDepsCode::ModuleNotFound,
        format!(
            "module {} not found in registries:\n* {}",
            key,
            not_found_trace.unwrap_or_default().join("\n* ")
        ),
    ))
}

/// The `catch (MissingChecksumException e)` / `catch (IOException e)` clauses of
/// `getModuleFile`.
///
/// PORT: the model reports `MissingChecksumException` as `RegistryException::Io` with its
/// message ("Missing checksum for registry file ..."); it is recognized by that message.
fn registry_io_error(registry: &Arc<dyn Registry>, e: RegistryException) -> ComputeError {
    if e.get_message()
        .starts_with("Missing checksum for registry file ")
    {
        return throw(
            ExternalDepsException::with_cause(ExternalDepsCode::BadLockfile, e),
            Transience::Transient,
        );
    }
    throw(
        ExternalDepsException::with_cause_and_message(
            ExternalDepsCode::ErrorAccessingRegistry,
            e,
            &format!("Error accessing registry {}", registry.get_url()),
        ),
        Transience::Transient,
    )
}

/// Applies any patches specified in registry overrides.
///
/// This allows users to modify MODULE.bazel files and thus influence resolution and visibility
/// for modules via patches without having to replace the entire module via a non-registry
/// override.
///
/// Note: Only patch files from the main repo are applied, all other patches are ignored. This is
/// necessary as we can't load other repos during resolution (unless they are subject to a
/// non-registry override). Patch commands are also not supported as they cannot be selectively
/// applied to the module file only.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:872 ModuleFileFunction.maybePatchModuleFile
fn maybe_patch_module_file(
    config: &BzlmodConfig,
    module_file: ModuleFile,
    override_: Option<&ModuleOverride>,
    env: &mut Environment<'_>,
) -> Result<Option<ModuleFile>, ComputeError> {
    let Some(ModuleOverride::SingleVersion(single_version_override)) = override_ else {
        return Ok(Some(module_file));
    };
    let patches_in_main_repo: Vec<&Label> = single_version_override
        .patches
        .iter()
        .filter(|label| label.get_repository().is_main())
        .collect();
    if patches_in_main_repo.is_empty() {
        return Ok(Some(module_file));
    }

    // Get the patch paths.
    // Java: env.getValuesAndExceptions(patchPackageLookupKeys); if (env.valuesMissing()) return null
    let mut patch_package_lookup_result: Vec<Option<PackageLookupResult>> =
        Vec::with_capacity(patches_in_main_repo.len());
    for label in &patches_in_main_repo {
        patch_package_lookup_result.push(
            config
                .package_lookup
                .lookup(label.get_package_identifier(), env)?,
        );
    }
    if patch_package_lookup_result.iter().any(|v| v.is_none()) {
        return Ok(None);
    }
    let mut patch_paths: Vec<(PathBuf, PathFragment)> = Vec::new();
    for (i, label) in patches_in_main_repo.iter().enumerate() {
        let Some(pkg_lookup_value) = &patch_package_lookup_result[i] else {
            return Ok(None);
        };
        if !pkg_lookup_value.package_exists {
            let Some(explanation) = config
                .package_lookup
                .explain_no_build_file_value(label.get_package_identifier(), env)?
            else {
                return Ok(None);
            };
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "unable to load package for single_version_override patch {}: {}",
                    label.get_canonical_form(),
                    explanation
                ),
            ));
        }
        // RootedPath.toRootedPath(pkgLookupValue.getRoot(), label.toPathFragment())
        patch_paths.push((
            pkg_lookup_value.get_root().to_path_buf(),
            label.to_path_fragment(),
        ));
    }

    // Register dependencies on the patch files and ensure that they exist.
    // PORT: Java's batch request (getValuesAndExceptions) is one request per key here.
    let mut patch_file_values = Vec::with_capacity(patch_paths.len());
    for (root, relative) in &patch_paths {
        patch_file_values.push(config.file_value_lookup.file_value(root, relative, env)?);
    }
    if patch_file_values.iter().any(Option::is_none) {
        return Ok(None);
    }
    for ((root, relative), file_value) in patch_paths.iter().zip(&patch_file_values) {
        if !file_value.expect("checked above").is_file {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "error reading single_version_override patch [{}]/[{}]: is a directory or \
                     doesn't exist",
                    // RootedPath.toString
                    path_string(root),
                    relative
                ),
            ));
        }
    }

    // Apply the patches to the module file only.
    // PORT: Java patches a copy in an InMemoryFileSystem; PatchUtil (blaze_repo_fetch) works on
    // real paths, so the copy lives in a fresh directory under the output base's tmp/ that is
    // removed afterwards.
    let module_root = config
        .output_base
        .join("tmp")
        .join(format!("module-file-patch-{}", uuid::Uuid::new_v4()))
        .join("module");
    let result = apply_patches_to_module_file(
        &module_root,
        &module_file,
        &patch_paths,
        single_version_override.patch_strip,
    );
    if let Some(parent) = module_root.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
    result.map(Some)
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:935 ModuleFileFunction.maybePatchModuleFile
fn apply_patches_to_module_file(
    module_root: &Path,
    module_file: &ModuleFile,
    patch_paths: &[(PathBuf, PathFragment)],
    patch_strip: i32,
) -> Result<ModuleFile, ComputeError> {
    let io_error = |e: std::io::Error| {
        errorf(
            ExternalDepsCode::BadModule,
            format!("error applying single_version_override patches to module file: {e}"),
        )
    };
    // PORT: Java's copy is `/module/MODULE.bazel` in an InMemoryFileSystem, and PatchUtil's
    // messages name that path ("Renaming /module/MODULE.bazel while applying patches ..."). The
    // copy here is in a temporary directory, whose path is replaced by Java's in the messages.
    let module_root_string = path_string(module_root);
    let java_paths = |message: &str| message.replace(&module_root_string, "/module");
    std::fs::create_dir_all(module_root).map_err(io_error)?;
    let module_file_path = module_root.join("MODULE.bazel");
    std::fs::write(&module_file_path, &module_file.content).map_err(io_error)?;
    for (root, relative) in patch_paths {
        let patch_path = root.join(platform_path(relative.get_path_string()));
        use blaze_repo_fetch::decompressor::patch_util::{PatchError, apply_to_single_file};
        let fs: blaze_vfs::FileSystemRef = Arc::new(blaze_vfs::UnixFileSystem::new(
            blaze_vfs::DigestHashFunction::Sha256,
        ));
        let vfs_path = |p: &Path| blaze_vfs::Path::create_from_str(&path_string(p), &fs);
        match apply_to_single_file(
            &vfs_path(&patch_path),
            patch_strip,
            &vfs_path(module_root),
            &vfs_path(&module_file_path),
        ) {
            Ok(()) => {}
            Err(PatchError::Patch(e)) => {
                return Err(errorf(
                    ExternalDepsCode::BadModule,
                    format!(
                        "error applying single_version_override patch {} to module file: {}",
                        path_string(&patch_path),
                        java_paths(&e.message)
                    ),
                ));
            }
            Err(PatchError::Io(e)) => {
                return Err(errorf(
                    ExternalDepsCode::BadModule,
                    format!(
                        "error applying single_version_override patches to module file: {}",
                        java_paths(&e.to_string())
                    ),
                ));
            }
        }
    }
    let content = std::fs::read(&module_file_path).map_err(io_error)?;
    Ok(ModuleFile::create(content, module_file.location.clone()))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:964 ModuleFileFunction.readModuleFile
fn read_module_file(path: &Path) -> Result<Vec<u8>, ComputeError> {
    std::fs::read(path).map_err(|e| {
        errorf(
            ExternalDepsCode::ModuleNotFound,
            format!(
                "MODULE.bazel expected but not found at {}: {}",
                path_string(path),
                java_io_message(path, &e)
            ),
        )
    })
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:976 ModuleFileFunction.createModuleFile
fn create_module_file(path: &Path, bytes: &[u8]) -> Result<(), ComputeError> {
    std::fs::write(path, bytes).map_err(|e| {
        errorf(
            ExternalDepsCode::ExternalDepsUnknown,
            format!(
                "MODULE.bazel cannot be created at {}: {}",
                path_string(path),
                java_io_message(path, &e)
            ),
        )
    })
}
