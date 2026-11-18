//! Assembling an evaluator that can fetch repositories and evaluate module extensions: the part
//! of `BazelRepositoryModule.beforeCommand` and `SkyframeExecutor` setup that the dev driver
//! (`blaze-fetch-dev`) and the tests share.
//!
//! PORT: not a Bazel class. The runtime's `fetch` / `sync` / `build` commands will do the same
//! from their `CommandEnvironment`.

use std::path::Path as StdPath;
use std::path::PathBuf;
use std::sync::Arc;

use blaze_bzlmod::registry_fetcher::LocalDownloadManagerFetcher;
use blaze_bzlmod::skyframe::BzlmodConfig;
use blaze_bzlmod::skyframe::register_bzlmod_functions;
use blaze_bzlmod_model::LockfileMode;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_repo_fetch::cache::local_repo_contents_cache::LocalRepoContentsCache;
use blaze_repo_fetch::cache::repository_cache::RepositoryCache;
use blaze_repo_fetch::downloader::download_manager::DownloadManager;
use blaze_repo_fetch::downloader::http_downloader::DelegatingDownloader;
use blaze_repo_fetch::downloader::http_downloader::HttpDownloader;
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

/// The evaluator and the lockfile mode (for `blaze_bzlmod::skyframe::after_command`).
pub struct FetchEvaluator {
    pub evaluator: Evaluator,
    pub lockfile_mode: LockfileMode,
}

fn to_path(fs: &FileSystemRef, p: &StdPath) -> Path {
    Path::create(PathFragment::create(&p.display().to_string()), fs)
}

/// Java `BazelRepositoryModule.beforeCommand` (caches, download manager, repo env, process
/// wrapper, overrides) plus the Skyframe function registrations of the bzlmod, loading and
/// repository functions.
pub fn create_evaluator(setup: FetchSetup<'_>) -> Result<FetchEvaluator, String> {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let repo_options = setup.repo_options;

    // The repository cache (Java `repositoryCache.setPath(...)`): `--repository_cache`, else
    // `<output_user_root>/cache/repos/v1`.
    let repo_cache_path: Option<PathBuf> = match &repo_options.repository_cache {
        Some(p) if p.get_path_string().is_empty() => None,
        Some(p) => Some(PathBuf::from(p.get_path_string())),
        None => setup.output_base.parent().map(|p| p.join("cache/repos/v1")),
    };
    let mut repository_cache = RepositoryCache::new();
    repository_cache.set_path(repo_cache_path.as_deref().map(|p| to_path(&fs, p)));
    // The repo contents cache settings come after the repo cache's, which they may override.
    let mut repo_contents_cache_path = repository_cache
        .get_repo_contents_cache()
        .get_path()
        .cloned();
    if let Some(p) = &repo_options.repo_contents_cache {
        let p = p.get_path_string();
        repo_contents_cache_path = if p.is_empty() {
            None
        } else {
            Some(to_path(&fs, StdPath::new(&p)))
        };
    }
    repository_cache
        .get_download_cache_mut()
        .set_hardlink(repo_options.use_hardlinks);

    let http_downloader = Arc::new(HttpDownloader::new(
        repo_options.repository_downloader_retries,
        std::time::Duration::ZERO,
        8,
        repo_options.experimental_scale_timeouts as f32,
    ));
    let mut download_manager = DownloadManager::new(
        repository_cache.get_download_cache().clone(),
        Arc::new(DelegatingDownloader::new(http_downloader.clone())),
        http_downloader,
        setup.reporter.clone(),
    );
    download_manager.set_disable_download(repo_options.disable_download);
    download_manager.set_retries(repo_options.repository_downloader_retries);

    let install_base = to_path(&fs, setup.install_base);
    // Java ProcessWrapper.fromCommandEnvironment: `<install_base>/process-wrapper`.
    let process_wrapper = install_base.get_child("process-wrapper");
    let directories = LoadingDirectories {
        workspace: to_path(&fs, setup.workspace),
        output_base: to_path(&fs, setup.output_base),
        install_base,
    };
    let mut repo_contents_cache = LocalRepoContentsCache::new();
    repo_contents_cache.set_path(repo_contents_cache_path.clone());
    let services = Arc::new(RepoServices {
        directories: directories.clone(),
        repo_env: Arc::new(setup.repo_env.clone()),
        nonstrict_repo_env: Arc::new(setup.nonstrict_repo_env.clone()),
        download_manager: Some(Arc::new(download_manager)),
        timeout_scaling: repo_options.experimental_scale_timeouts,
        process_wrapper: process_wrapper.exists().then_some(process_wrapper),
        repo_contents_cache: Arc::new(repo_contents_cache),
    });

    let fetcher = Arc::new(LocalDownloadManagerFetcher::new(
        repo_cache_path,
        setup.reporter.clone(),
        repo_options.repository_downloader_retries,
    ));
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
    let lockfile_mode = bzlmod_config.lockfile_mode;

    // `--override_repository` (Java BazelRepositoryModule.beforeCommand overrideMap).
    let mut repository_overrides: IndexMap<String, PathFragment> = IndexMap::new();
    for o in &repo_options.repository_overrides {
        if o.path.is_empty() {
            repository_overrides.shift_remove(&o.repository_name);
            continue;
        }
        let p = StdPath::new(&o.path);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            setup.working_directory.join(p)
        };
        repository_overrides.insert(
            o.repository_name.clone(),
            PathFragment::create(&abs.display().to_string()),
        );
    }

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
