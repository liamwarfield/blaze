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
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use super::config::BzlmodConfig;
use super::keys::{ModuleFileKey, ModuleFileValue, RegistryKey};
use super::precomputed::{
    ClientEnvironmentKey, IGNORE_DEV_DEPS, INJECTED_REPOSITORIES, MODULE_OVERRIDES, REGISTRIES,
    STARLARK_SEMANTICS,
};
use super::{java_io_message, throw};
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
    // PORT: profiling ("fetch module file: <key>") skipped.
    let Some(get_module_file_result) = get_module_file(
        config,
        module_key,
        root_module_file_value.overrides.get(module_key.name()),
        env,
    )?
    else {
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
    // PORT: Java first requests FileValue.key(moduleFilePath) to depend on the file; FILE is not
    // ported here and a single build per process needs no invalidation.
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
        ModuleFile::create(
            module_file_contents,
            module_file_path.to_string_lossy().into_owned(),
        ),
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
    let mut rooted_paths: Vec<PathBuf> = Vec::with_capacity(pending.len());
    for (i, include_label) in include_labels.iter().enumerate() {
        let Some(pkg_lookup_value) =
            package_lookup(config, include_label.get_package_identifier(), env)?
        else {
            return Ok(None);
        };
        let Some(root) = pkg_lookup_value else {
            let message = explain_no_build_file_value(include_label.get_package_identifier());
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "unable to load package for '{}' included at {}: {}",
                    pending[i].include_label, pending[i].location, message
                ),
            ));
        };
        rooted_paths.push(root.join(include_label.to_path_fragment().get_path_string()));
    }
    // PORT: Java requests FileValue keys for the include files here (a dependency for
    // invalidation); the stand-in checks the file system directly below.
    let mut new_horizon: Vec<IncludeStatement> = Vec::new();
    for (i, rooted_path) in rooted_paths.iter().enumerate() {
        if !rooted_path.is_file() {
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
            ModuleFile::create(bytes, rooted_path.to_string_lossy().into_owned()),
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

/// PORT: a stand-in for `PackageLookupFunction` (PACKAGE_LOOKUP), which is not ported in this
/// crate. The outer `Option` is "values missing"; the inner one is `None` when the package has
/// no BUILD file (`PackageLookupValue.NO_BUILD_FILE_VALUE`), otherwise the package root.
/// Main-repo packages are looked up under the workspace (a single package path entry); other
/// repos under [`super::config::RepositoryDirectoryLookup::repo_root`]. Deleted packages,
/// invalid package names and local-repository boundaries are not checked.
fn package_lookup(
    config: &BzlmodConfig,
    package_id: &PackageIdentifier,
    env: &mut Environment<'_>,
) -> Result<Option<Option<PathBuf>>, ComputeError> {
    let root: PathBuf = if package_id.get_repository().is_main() {
        config.workspace_root.clone()
    } else {
        match config
            .repository_directory_lookup
            .repo_root(package_id.get_repository(), env)?
        {
            Some(r) => r,
            None => return Ok(None),
        }
    };
    let package_dir = root.join(package_id.get_package_fragment().get_path_string());
    // BazelSkyframeExecutorConstants.BUILD_FILES_BY_PRIORITY
    for build_file_name in ["BUILD.bazel", "BUILD"] {
        if package_dir.join(build_file_name).is_file() {
            return Ok(Some(Some(root)));
        }
    }
    Ok(Some(None))
}

/// The message for a package without a BUILD file.
///
/// PORT: Java lists every package path entry of `PATH_PACKAGE_LOCATOR` for the main repo; Blaze
/// has a single entry (the workspace), for which Java prints the package fragment.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:130 PackageLookupFunction.explainNoBuildFileValue
fn explain_no_build_file_value(package_key: &PackageIdentifier) -> String {
    let educational_message = "Add a BUILD file to a directory to mark it as a package.";
    if package_key.get_repository().is_main() {
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
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:555 ModuleFileFunction.getModuleFilePath
pub fn get_module_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(MODULE_DOT_BAZEL_FILE_NAME.get_path_string())
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
        let module_file_path = repo_dir.join(MODULE_DOT_BAZEL_FILE_NAME.get_path_string());
        // PORT: FileValue.key(moduleFilePath) dependency skipped (FILE is not ported here).
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
    let mut patch_paths: Vec<(PathBuf, PathFragment)> = Vec::new();
    for label in &patches_in_main_repo {
        let Some(pkg_lookup_value) = package_lookup(config, label.get_package_identifier(), env)?
        else {
            return Ok(None);
        };
        let Some(root) = pkg_lookup_value else {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "unable to load package for single_version_override patch {}: {}",
                    label.get_canonical_form(),
                    explain_no_build_file_value(label.get_package_identifier())
                ),
            ));
        };
        patch_paths.push((root, label.to_path_fragment()));
    }

    // Register dependencies on the patch files and ensure that they exist.
    for (root, relative) in &patch_paths {
        if !root.join(relative.get_path_string()).is_file() {
            return Err(errorf(
                ExternalDepsCode::BadModule,
                format!(
                    "error reading single_version_override patch [{}]/[{}]: is a directory or \
                     doesn't exist",
                    root.display(),
                    relative
                ),
            ));
        }
    }

    // Apply the patches to the module file only.
    blaze_util::unported!(
        "PatchUtil.applyToSingleFile",
        "single_version_override patches for {}",
        module_file.location
    )
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:964 ModuleFileFunction.readModuleFile
fn read_module_file(path: &Path) -> Result<Vec<u8>, ComputeError> {
    std::fs::read(path).map_err(|e| {
        errorf(
            ExternalDepsCode::ModuleNotFound,
            format!(
                "MODULE.bazel expected but not found at {}: {}",
                path.display(),
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
                path.display(),
                java_io_message(path, &e)
            ),
        )
    })
}
