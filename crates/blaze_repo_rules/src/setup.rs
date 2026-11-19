//! Assembling an evaluator that can fetch repositories and evaluate module extensions: the part
//! of `BazelRepositoryModule.beforeCommand` and `SkyframeExecutor` setup that the dev driver
//! (`blaze-fetch-dev`) and the tests share.
//!
//! PORT: not a Bazel class. The runtime's `fetch` / `sync` / `build` commands will do the same
//! from their `CommandEnvironment`.

use std::path::Path as StdPath;
use std::path::PathBuf;
use std::sync::Arc;

use blaze_bzlmod::registry_fetcher::DownloadManagerRegistryFetcher;
use blaze_bzlmod::skyframe::BzlmodConfig;
use blaze_bzlmod::skyframe::register_bzlmod_functions;
use blaze_bzlmod_model::LockfileMode;
use blaze_events::Event;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_repo_fetch::authandtls::credentialhelper::{
    CredentialHelperCredentials, CredentialHelperEnvironment, CredentialModule,
};
use blaze_repo_fetch::authandtls::google_auth_utils;
use blaze_repo_fetch::authandtls::{Credentials, StaticCredentials};
use blaze_repo_fetch::cache::local_repo_contents_cache::LocalRepoContentsCache;
use blaze_repo_fetch::cache::repository_cache::RepositoryCache;
use blaze_repo_fetch::downloader::download_manager::DownloadManager;
use blaze_repo_fetch::downloader::http_downloader::DelegatingDownloader;
use blaze_repo_fetch::downloader::http_downloader::HttpDownloader;
use blaze_repo_fetch::downloader::url_rewriter::UrlRewriter;
use blaze_repo_fetch::runtime::command_line_path_factory::CommandLinePathFactory;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::skyframe_executor::LoadingFunctionsConfig;
use blaze_sky_loading::skyframe_executor::inject_loading_precomputed;
use blaze_sky_loading::skyframe_executor::register_loading_functions;
use blaze_skyframe::Evaluator;
use blaze_skyframe::EvaluatorBuilder;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::DigestHashFunction;
use blaze_vfs::FileSystemRef;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::unix_file_system::UnixFileSystem;
use indexmap::IndexMap;

use crate::external_repos::BzlmodExternalRepos;
use crate::registration::RepoPrecomputedValues;
use crate::registration::bazel_starlark_environment;
use crate::registration::inject_repo_precomputed;
use crate::registration::register_repo_rules_functions;
use crate::registration::repository_fetch_function;
use crate::services::RepoServices;

/// Everything [`create_evaluator`] needs.
pub struct FetchSetup<'a> {
    pub workspace: &'a StdPath,
    pub working_directory: &'a StdPath,
    pub output_base: &'a StdPath,
    pub install_base: &'a StdPath,
    pub repo_options: &'a RepositoryOptions,
    pub starlark_semantics: StarlarkSemantics,
    pub client_env: IndexMap<String, String>,
    /// Java `CommandEnvironment.getRepoEnv()`.
    pub repo_env: IndexMap<String, String>,
    /// Java `CommandEnvironment.getNonstrictRepoEnv()`.
    pub nonstrict_repo_env: IndexMap<String, String>,
    pub precomputed: RepoPrecomputedValues,
    pub reporter: Arc<dyn ExtendedEventHandler>,
}

/// The command inputs of `BazelRepositoryModule.beforeCommand` and `CommandEnvironment.<init>`
/// that [`FetchSetup`] does not carry (it was written before they were wired): pass them to
/// [`prepare_repository_functions_with`] / [`create_evaluator_with`].
///
/// `Default` is a command without these inputs: no output user root (see
/// [`CommandInputs::output_user_root`]), the option defaults of `CommonCommandOptions` and
/// `AuthAndTLSOptions`, and no `LocalExecutionOptions`.
#[derive(Clone, Copy, Default)]
pub struct CommandInputs<'a> {
    /// Java `env.getDirectories().getServerDirectories().getOutputUserRoot()`
    /// (`--output_user_root`), under which the default repository cache lives.
    ///
    /// PORT: `None` is for callers of the older entry points, which have no output user root.
    /// The default cache location is then derived only when the output base is the client's
    /// default `<output_user_root>/<md5(workspace)>` (blaze.cc `GetOutputBase`), whose parent is
    /// the output user root; with any other `--output_base` and no `--repository_cache` the
    /// setup fails instead of guessing.
    pub output_user_root: Option<&'a StdPath>,
    /// Java `CommandEnvironment`'s `CommonCommandOptions` (the `--http_*` options of the
    /// `HttpDownloader`); `None` is the options' defaults.
    pub common_command_options: Option<&'a CommonCommandOptions>,
    /// Java `env.getOptions().getOptions(AuthAndTLSOptions.class)` (`--credential_helper`);
    /// `None` is the options' defaults.
    pub auth_and_tls_options: Option<&'a AuthAndTLSOptions>,
    /// Java `cmdEnv.getOptions().getOptions(LocalExecutionOptions.class)`, which is null for a
    /// command without these options (`ProcessWrapper.fromCommandEnvironment`).
    pub local_execution_options: Option<&'a LocalExecutionOptions>,
}

/// The evaluator and the lockfile mode (for `blaze_bzlmod::skyframe::after_command`).
pub struct FetchEvaluator {
    pub evaluator: Evaluator,
    pub lockfile_mode: LockfileMode,
}

fn to_path(fs: &FileSystemRef, p: &StdPath) -> Path {
    // PORT: byte-strings (a non-UTF-8 directory is converted lossily; Java keeps the bytes)
    Path::create(PathFragment::create(&p.display().to_string()), fs)
}

/// Java `BazelRepositoryModule.beforeCommand` (caches, download manager, repo env, process
/// wrapper, overrides) plus the Skyframe function registrations of the bzlmod, loading and
/// repository functions.
pub fn create_evaluator(setup: FetchSetup<'_>) -> Result<FetchEvaluator, String> {
    create_evaluator_with(setup, &CommandInputs::default())
}

/// [`create_evaluator`] with the command inputs [`FetchSetup`] does not carry.
pub fn create_evaluator_with(
    setup: FetchSetup<'_>,
    inputs: &CommandInputs<'_>,
) -> Result<FetchEvaluator, String> {
    let RepositoryFunctions {
        directories,
        services,
        bzlmod_config,
        repository_overrides,
        repo_contents_cache_path,
    } = prepare_repository_functions_with(
        &setup,
        inputs,
        UnixFileSystem::create(DigestHashFunction::Sha256),
    )?;
    let lockfile_mode = bzlmod_config.lockfile_mode;
    let rule_class_provider =
        Arc::new(blaze_packages::native_rules::RuleClassProvider::create_bazel());
    let starlark_environment = bazel_starlark_environment(&rule_class_provider);
    let fetch_function = repository_fetch_function(services.clone());
    let external_repos = Arc::new(BzlmodExternalRepos::new(fetch_function));
    let mut loading_config = LoadingFunctionsConfig::for_bazel(
        directories,
        external_repos,
        Arc::new(LoadingFunctionsConfig::default_native_registry()),
        starlark_environment,
        Some(rule_class_provider),
    );
    loading_config.repo_contents_cache_path = repo_contents_cache_path;

    let builder = register_bzlmod_functions(EvaluatorBuilder::new(), Arc::new(bzlmod_config));
    let builder = register_loading_functions(builder, &loading_config);
    let builder = inject_loading_precomputed(
        builder,
        &loading_config,
        setup.starlark_semantics,
        repository_overrides,
    );
    let builder = register_repo_rules_functions(builder, services);
    let builder = inject_repo_precomputed(builder, &setup.precomputed);
    Ok(FetchEvaluator {
        evaluator: builder.build(),
        lockfile_mode,
    })
}

/// What `BazelRepositoryModule.beforeCommand` sets up for a command's evaluator: the
/// repository services (caches, download manager, repo env, process wrapper), the Bzlmod
/// configuration and `--override_repository`.
pub struct RepositoryFunctions {
    pub directories: LoadingDirectories,
    pub services: Arc<RepoServices>,
    pub bzlmod_config: BzlmodConfig,
    pub repository_overrides: IndexMap<String, PathFragment>,
    pub repo_contents_cache_path: Option<Path>,
}

/// The `BazelRepositoryModule.beforeCommand` part of [`create_evaluator`], shared with the
/// runtime's `SkyframeExecutor`.
pub fn prepare_repository_functions(setup: &FetchSetup<'_>) -> Result<RepositoryFunctions, String> {
    prepare_repository_functions_in(setup, UnixFileSystem::create(DigestHashFunction::Sha256))
}

/// [`prepare_repository_functions`] with the paths on `fs` (the runtime's file system: paths of
/// different file system instances never compare equal or nest).
pub fn prepare_repository_functions_in(
    setup: &FetchSetup<'_>,
    fs: FileSystemRef,
) -> Result<RepositoryFunctions, String> {
    prepare_repository_functions_with(setup, &CommandInputs::default(), fs)
}

/// `BazelRepositoryModule.DEFAULT_CACHE_LOCATION`.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:138 BazelRepositoryModule.DEFAULT_CACHE_LOCATION
const DEFAULT_CACHE_LOCATION: &str = "cache/repos/v1";

/// An empty path fragment is turned into `None`; otherwise, it's treated as relative to the
/// workspace root.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:715 BazelRepositoryModule.toPath
fn to_workspace_path(path: &PathFragment, workspace: &Path) -> Option<Path> {
    if path.is_empty() {
        return None;
    }
    // It is important to use getWorkspace() here, not getWorkingDirectory(). (Blaze has one file
    // system for both.)
    Some(workspace.get_relative(path))
}

/// If the given path is absolute path, leave it as it is. If the given path is a relative path,
/// it is relative to the current working directory. If the given path starts with '%workspace%,
/// it is relative to the workspace root, which is the output of `bazel info workspace`.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:700 BazelRepositoryModule.getAbsolutePath
fn get_absolute_path(path: &str, workspace: &Path, working_directory: &Path) -> String {
    let path = path.replace("%workspace%", workspace.get_path_string());
    if !PathFragment::is_absolute_str(&path) {
        return working_directory
            .get_relative_str(&path)
            .get_path_string()
            .to_string();
    }
    path
}

/// The message of an `IOException` of the file system (Java `e.getMessage()`).
fn io_message(e: std::io::Error) -> String {
    blaze_repo_fetch::errors::IoException::from(e).get_message()
}

/// The output user root of a command whose caller did not pass one (see
/// [`CommandInputs::output_user_root`]): the parent of the client's default output base
/// `<output_user_root>/<md5(workspace)>`, and nothing for any other output base.
// Java: src/main/cpp/blaze.cc GetOutputBase / blaze_util_posix.cc:509 GetHashedBaseDir
fn output_user_root_of_default_output_base(
    workspace: &StdPath,
    output_base: &StdPath,
) -> Option<PathBuf> {
    // PORT: byte-strings (the C++ hashes the workspace path's bytes)
    let hashed = blaze_util::fingerprint::Fingerprint::with_digest_function(
        blaze_util::fingerprint::DigestHashFunction::Md5,
    )
    .add_bytes(workspace.display().to_string().as_bytes())
    .hex_digest_and_reset();
    if output_base.file_name().and_then(|n| n.to_str()) == Some(hashed.as_str()) {
        output_base.parent().map(StdPath::to_path_buf)
    } else {
        None
    }
}

/// The `HttpDownloader` of a command.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:302 CommandEnvironment.<init>
fn http_downloader_from_options(
    command_options: &CommonCommandOptions,
    reporter: &Arc<dyn ExtendedEventHandler>,
) -> Result<HttpDownloader, String> {
    let mut http_timeout_scaling = command_options.http_timeout_scaling as f32;
    if command_options.http_timeout_scaling <= 0.0 {
        reporter.handle(&Event::warn(
            "Ignoring request to scale http timeouts by a non-positive factor",
        ));
        http_timeout_scaling = 1.0;
    }
    if command_options.http_max_parallel_downloads <= 0 {
        // Java: blazeModuleEnvironment.exit(new AbruptExitException(... BAD_DOWNLOADER_CONFIG)).
        return Err(
            "The maximum number of parallel downloads needs to be a positive number".to_string(),
        );
    }
    Ok(HttpDownloader::new(
        command_options.http_connector_attempts,
        command_options.http_connector_retry_max_timeout,
        command_options.http_max_parallel_downloads,
        http_timeout_scaling,
    ))
}

/// [`prepare_repository_functions_in`] with the command inputs [`FetchSetup`] does not carry.
/// An `Err` is the message of Java's `AbruptExitException`.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:276 BazelRepositoryModule.beforeCommand
pub fn prepare_repository_functions_with(
    setup: &FetchSetup<'_>,
    inputs: &CommandInputs<'_>,
    fs: FileSystemRef,
) -> Result<RepositoryFunctions, String> {
    let repo_options = setup.repo_options;
    let reporter = &setup.reporter;
    let workspace = to_path(&fs, setup.workspace);
    let working_directory = to_path(&fs, setup.working_directory);
    let output_base = to_path(&fs, setup.output_base);

    // Java: new DownloadManager(repositoryCache.getDownloadCache(), env.getDownloaderDelegate(),
    // env.getHttpDownloader(), env.getReporter()). The download cache is taken once the
    // repository cache has its path (below); the setters keep Java's order.
    let default_command_options;
    let command_options = match inputs.common_command_options {
        Some(o) => o,
        None => {
            default_command_options =
                blaze_options::Options::<CommonCommandOptions>::get_defaults();
            &default_command_options
        }
    };
    let http_downloader = Arc::new(http_downloader_from_options(command_options, reporter)?);

    // Java: ProcessWrapper.fromCommandEnvironment(env).
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
    let process_wrapper_kill_delay = inputs
        .local_execution_options
        .map(LocalExecutionOptions::get_local_sigkill_grace_seconds);
    let process_wrapper_graceful_sigterm = inputs
        .local_execution_options
        .is_some_and(|o| o.process_wrapper_graceful_sigterm);

    // if (repoOptions.repositoryDownloaderRetries >= 0) downloadManager.setRetries(...)
    let retries = (repo_options.repository_downloader_retries >= 0)
        .then_some(repo_options.repository_downloader_retries);

    let timeout_scaling = if repo_options.experimental_scale_timeouts > 0.0 {
        repo_options.experimental_scale_timeouts
    } else {
        reporter.handle(&Event::warn(
            "Ignoring request to scale timeouts for repositories by a non-positive factor",
        ));
        1.0
    };

    // The repository cache (Java `repositoryCache.setPath(...)`): `--repository_cache` (relative
    // to the workspace), else `<output_user_root>/cache/repos/v1`.
    let mut repository_cache = RepositoryCache::new();
    match &repo_options.repository_cache {
        Some(p) => repository_cache.set_path(to_workspace_path(p.get(), &workspace)),
        None => {
            let output_user_root = match inputs.output_user_root {
                Some(root) => root.to_path_buf(),
                None => output_user_root_of_default_output_base(setup.workspace, setup.output_base)
                    .ok_or_else(|| {
                        format!(
                            "blaze: the default repository cache is \
                         <output_user_root>/{DEFAULT_CACHE_LOCATION}, but this command was set \
                         up without an output user root and its output base [{}] is not the \
                         default one. Pass --repository_cache=<path> (or --repository_cache= to \
                         disable the cache).",
                            setup.output_base.display()
                        )
                    })?,
            };
            repository_cache.set_path(Some(
                to_path(&fs, &output_user_root).get_relative_str(DEFAULT_CACHE_LOCATION),
            ));
        }
    }
    repository_cache
        .get_download_cache_mut()
        .set_hardlink(repo_options.use_hardlinks);
    // Note that the repo contents cache stuff has to happen _after_ the repo cache stuff, because
    // the specific settings about the repo contents cache might overwrite the repo cache
    // settings. In particular, if `--repo_contents_cache` is not set (it's null), we use whatever
    // default set by `repositoryCache.setPath(...)`.
    if let Some(p) = &repo_options.repo_contents_cache {
        repository_cache
            .get_repo_contents_cache_mut()
            .set_path(to_workspace_path(p.get(), &workspace));
    }
    let repo_contents_cache_path = repository_cache
        .get_repo_contents_cache()
        .get_path()
        .cloned();
    if let Some(repo_contents_cache_path) = &repo_contents_cache_path {
        // Check that the repo contents cache directory, which is managed by a garbage collecting
        // idle task, does not contain the output base. Since the specified output base path may be
        // a symlink, we resolve it fully. Intermediate symlinks do not have to be checked as the
        // garbage collector ignores symlinks. We also resolve the repo contents cache directory,
        // where intermediate symlinks also don't matter since deletion only occurs under the fully
        // resolved path.
        let resolved_output_base = match output_base.resolve_symbolic_links() {
            Ok(p) => p,
            // Will be created later.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => output_base.clone(),
            Err(e) => {
                return Err(format!("could not resolve output base: {}", io_message(e)));
            }
        };
        let resolved_repo_contents_cache = match repo_contents_cache_path.resolve_symbolic_links() {
            Ok(p) => p,
            // Will be created later.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => repo_contents_cache_path.clone(),
            Err(e) => {
                return Err(format!(
                    "could not resolve repo contents cache path: {}",
                    io_message(e)
                ));
            }
        };
        if resolved_output_base.starts_with(&resolved_repo_contents_cache) {
            // This is dangerous as the repo contents cache GC may delete files in the output base.
            return Err(format!(
                "The output base [{}] is inside the repo contents cache [{}]. This can cause \
                 spurious failures. Disable the repo contents cache with \
                 `--repo_contents_cache=`, or specify `--repo_contents_cache=<path that doesn't \
                 contain the output base>`.\n",
                resolved_output_base.get_path_string(),
                resolved_repo_contents_cache.get_path_string()
            ));
        }
        if repo_contents_cache_path.starts_with(&workspace) {
            // Having the repo contents cache inside the main repo is very dangerous. During the
            // lifetime of a Bazel invocation, we treat files inside the main repo as immutable.
            // This can cause mysterious failures if we write files inside the main repo during
            // the invocation, as is often the case with the repo contents cache.
            return Err(format!(
                "The repo contents cache [{}] is inside the main repo [{}]. This can cause \
                 spurious failures. Disable the repo contents cache with \
                 `--repo_contents_cache=`, or specify `--repo_contents_cache=<path outside the \
                 main repo>`.\n",
                repo_contents_cache_path.get_path_string(),
                workspace.get_path_string()
            ));
        }
    }
    let mut repo_contents_cache = LocalRepoContentsCache::new();
    repo_contents_cache.set_path(repo_contents_cache_path.clone());
    if repo_contents_cache.is_enabled() {
        // PORT: Java releases the shared lock in afterCommand; Blaze runs one command per
        // process, so the lock lives as long as the services (until the process exits). The GC
        // idle task (env.addIdleTask) needs an idle server, which Blaze does not have.
        repo_contents_cache
            .acquire_shared_lock()
            .map_err(|_| "could not acquire lock on repo contents cache".to_string())?;
    }

    let mut download_manager = DownloadManager::new(
        repository_cache.get_download_cache().clone(),
        Arc::new(DelegatingDownloader::new(http_downloader.clone())),
        http_downloader,
        reporter.clone(),
    );
    download_manager.set_disable_download(repo_options.disable_download);
    if let Some(retries) = retries {
        download_manager.set_retries(retries);
    }

    // PORT: byte-strings (the client environment is a String map here)
    let client_env: std::collections::HashMap<String, String> = setup
        .client_env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    match UrlRewriter::new_credentials_from_netrc(&client_env, Some(&working_directory)) {
        Ok(netrc_creds) => download_manager.set_netrc_creds(netrc_creds),
        Err(e) => {
            // If the credentials extraction failed, we're letting bazel try without credentials.
            reporter.handle(&Event::warn(format!(
                "Error parsing the .netrc file: {}.",
                e.message
            )));
        }
    }
    match UrlRewriter::get_downloader_url_rewriter(
        &workspace,
        repo_options.downloader_config.as_ref().map(|p| p.get()),
    ) {
        Ok(rewriter) => download_manager.set_url_rewriter(Some(rewriter)),
        Err(e) => {
            // It's important that the build stops ASAP, because this config file may be required
            // for security purposes, and the build must not proceed ignoring it.
            return Err(format!(
                "Failed to parse downloader config{}: {}",
                match &e.location {
                    Some(location) => format!(" at {location}"),
                    None => String::new(),
                },
                e.message
            ));
        }
    }

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:448 BazelRepositoryModule.beforeCommand
    let default_auth_and_tls_options;
    let auth_and_tls_options = match inputs.auth_and_tls_options {
        Some(o) => o,
        None => {
            default_auth_and_tls_options =
                blaze_options::Options::<AuthAndTLSOptions>::get_defaults();
            &default_auth_and_tls_options
        }
    };
    let credential_helper_environment = CredentialHelperEnvironment::new_builder()
        .set_event_reporter(reporter.clone())
        .set_workspace_path(Some(workspace.clone()))
        .set_client_environment(setup.client_env.clone())
        .set_helper_execution_timeout(auth_and_tls_options.credential_helper_timeout)
        .build();
    // env.getCommandLinePathFactory(): CommandLinePathFactory.create(fileSystem, directories),
    // with the roots `workspace` and `install_base`.
    let command_line_path_factory = CommandLinePathFactory::create_for_directories(
        fs.clone(),
        Some(workspace.clone()),
        Some(to_path(&fs, setup.install_base)),
    );
    match google_auth_utils::new_credential_helper_provider(
        &credential_helper_environment,
        &command_line_path_factory,
        &auth_and_tls_options.credential_helpers,
    ) {
        Ok(credential_helper_provider) => {
            let credential_helper_provider = Arc::new(credential_helper_provider);
            // credentialModule.getCredentialCache()
            let credential_cache = CredentialModule::instance().get_credential_cache().clone();
            download_manager.set_credential_factory(Arc::new(move |headers| {
                let fallback: Arc<dyn Credentials> =
                    Arc::new(StaticCredentials::new(headers.clone()));
                Arc::new(CredentialHelperCredentials::new(
                    credential_helper_provider.clone(),
                    credential_helper_environment.clone(),
                    credential_cache.clone(),
                    Some(fallback),
                ))
            }));
        }
        Err(e) => {
            reporter.handle(&Event::error(e.get_message()));
            return Err("Error initializing credential helper".to_string());
        }
    }

    download_manager.set_distdir(
        repo_options
            .experimental_distdir
            .iter()
            .map(|path| {
                let path = path.get();
                if path.is_absolute() {
                    Path::create(path.clone(), &fs)
                } else {
                    workspace.get_relative(path)
                }
            })
            .collect(),
    );
    let download_manager = Arc::new(download_manager);

    let install_base = to_path(&fs, setup.install_base);
    // Java ProcessWrapper.fromCommandEnvironment: `<install_base>/process-wrapper`.
    let process_wrapper = install_base.get_child("process-wrapper");
    let directories = LoadingDirectories {
        workspace: workspace.clone(),
        output_base,
        install_base,
    };
    let services = Arc::new(RepoServices {
        directories: directories.clone(),
        repo_env: Arc::new(setup.repo_env.clone()),
        nonstrict_repo_env: Arc::new(setup.nonstrict_repo_env.clone()),
        download_manager: Some(download_manager.clone()),
        timeout_scaling,
        process_wrapper: process_wrapper.exists().then_some(process_wrapper),
        process_wrapper_kill_delay,
        process_wrapper_graceful_sigterm,
        repo_contents_cache: Arc::new(repo_contents_cache),
    });

    // Java: moduleFileFunction / repoSpecFunction / yankedVersionsFunction
    // .setDownloadManager(downloadManager): the registries download through the same manager.
    let fetcher = Arc::new(DownloadManagerRegistryFetcher::new(download_manager));
    let bzlmod_config = BzlmodConfig::from_repository_options(
        repo_options,
        setup.workspace,
        setup.working_directory,
        setup.output_base,
        setup.install_base,
        setup.starlark_semantics.clone(),
        setup.client_env.clone(),
        setup.nonstrict_repo_env.clone(),
        fetcher,
    )
    .map_err(|e| e.message.clone())?;
    let mut bzlmod_config = bzlmod_config;
    bzlmod_config.repository_directory_lookup = Arc::new(RepositoryDirectoryFunctionLookup);
    bzlmod_config.package_lookup = Arc::new(PackageLookupFunctionLookup);

    // `--override_repository` (Java BazelRepositoryModule.beforeCommand overrideMap).
    // To get the usual latest-wins semantics, we need a mutable map, as the builder of an
    // immutable map does not allow redefining the values of existing keys.
    let mut repository_overrides: IndexMap<String, PathFragment> = IndexMap::new();
    for o in &repo_options.repository_overrides {
        if o.path.is_empty() {
            repository_overrides.shift_remove(&o.repository_name);
            continue;
        }
        let repo_path = get_absolute_path(&o.path, &workspace, &working_directory);
        repository_overrides.insert(o.repository_name.clone(), PathFragment::create(&repo_path));
    }

    Ok(RepositoryFunctions {
        directories,
        services,
        bzlmod_config,
        repository_overrides,
        repo_contents_cache_path,
    })
}

/// `ModuleFileFunction`'s lookup of a non-registry override's repository directory through
/// REPOSITORY_DIRECTORY (Java: `env.getValue(RepositoryDirectoryValue.key(canonicalRepoName))`,
/// then `((Success) repoDir).root()`), replacing bzlmod's builtin local-only lookup.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:756 ModuleFileFunction.getModuleFile
pub struct RepositoryDirectoryFunctionLookup;

impl blaze_bzlmod::skyframe::RepositoryDirectoryLookup for RepositoryDirectoryFunctionLookup {
    fn repo_root(
        &self,
        name: &blaze_cmdline::RepositoryName,
        env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<Option<PathBuf>, blaze_skyframe::ComputeError> {
        use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
        let Some(repo_dir) = env.get_value(RepositoryDirectoryKey::key(name.clone()))? else {
            return Ok(None);
        };
        // This repo _definitely_ exists, since it has a non-registry override, which directly
        // gets "translated" into a repo spec. So we can cast `repoDir` to `Success`.
        match &*repo_dir {
            RepositoryDirectoryValue::Success { root, .. } => Ok(Some(PathBuf::from(
                root.as_path()
                    .expect("repository root is a path")
                    .get_path_string(),
            ))),
            // PORT: Java's cast throws ClassCastException here.
            RepositoryDirectoryValue::Failure { error_msg } => panic!(
                "class RepositoryDirectoryValue$Failure cannot be cast to class \
                 RepositoryDirectoryValue$Success ({name}: {error_msg})"
            ),
        }
    }
}

/// `ModuleFileFunction`'s PACKAGE_LOOKUP requests (the packages of `include()`d files and of
/// `single_version_override` patches), answered by the loading phase's `PackageLookupFunction`
/// (which honours `--package_path`, `--deleted_packages`, `.bazelignore` and `REPO.bazel`),
/// replacing bzlmod's builtin stand-in.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:466 ModuleFileFunction.advanceHorizon
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:896 ModuleFileFunction.getModuleFile
pub struct PackageLookupFunctionLookup;

impl blaze_bzlmod::skyframe::PackageLookup for PackageLookupFunctionLookup {
    fn lookup(
        &self,
        package_id: &blaze_cmdline::PackageIdentifier,
        env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<Option<blaze_bzlmod::skyframe::PackageLookupResult>, blaze_skyframe::ComputeError>
    {
        use blaze_bzlmod::skyframe::PackageLookupResult;
        use blaze_sky_loading::package_lookup::{PackageLookupKey, PackageLookupValue};
        let Some(value) = env.get_value(PackageLookupKey::key(package_id.clone()))? else {
            return Ok(None);
        };
        Ok(Some(match &*value {
            PackageLookupValue::Success {
                root,
                build_file_name,
            } => PackageLookupResult::success(
                // PORT: byte-strings
                PathBuf::from(
                    root.as_path()
                        .expect("package roots are path roots")
                        .get_path_string(),
                ),
                build_file_name.get_filename_fragment().get_path_string(),
            ),
            PackageLookupValue::NoBuildFile => PackageLookupResult::no_build_file(),
            unsuccessful => PackageLookupResult::unsuccessful(unsuccessful.get_error_msg()),
        }))
    }

    fn explain_no_build_file_value(
        &self,
        package_id: &blaze_cmdline::PackageIdentifier,
        env: &mut blaze_skyframe::Environment<'_>,
    ) -> Result<Option<String>, blaze_skyframe::ComputeError> {
        Ok(Some(
            blaze_sky_loading::package_lookup::PackageLookupFunction::explain_no_build_file_value(
                package_id, env,
            )?,
        ))
    }
}
