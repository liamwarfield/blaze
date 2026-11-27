//! Test helpers: a scratch directory on the real file system (Java `Scratch` /
//! `FoundationTestCase`) and an evaluator with the loading functions registered.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use blaze_cmdline::RepositoryName;
use blaze_events::StoredEventHandler;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_skyframe::{
    ComputeError, Environment, EvaluationContext, EvaluationResult, Evaluator, EvaluatorBuilder,
    Key, SkyKey,
};
use blaze_starlark::StarlarkSemantics;
use blaze_util::string_encoding::{internal_to_platform, platform_to_internal};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Root, UnixFileSystem,
};
use indexmap::IndexMap;

use blaze_packages::builtins::{
    BazelStarlarkEnvironment, RuleClassProviderRegistrations, StarlarkEnv, StarlarkGlobals,
    StarlarkGlobalsFns,
};

use crate::directories::LoadingDirectories;
use crate::pkgcache::PathPackageLocator;
use crate::repos::{ExternalRepos, RepositoryDirectoryValue, RepositoryMappingValue};
use crate::skyframe_executor::{
    LoadingFunctionsConfig, inject_loading_precomputed,
    inject_loading_precomputed_with_package_options, register_loading_functions,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh directory under the system temp directory, removed on drop.
pub struct Scratch {
    pub fs: FileSystemRef,
    pub root: Path,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch::with_file_system(|unix| unix)
    }

    /// A scratch directory whose paths are on `wrap(unix_file_system)`, e.g. a
    /// `blaze_vfs::testing::FaultInjectingFileSystem` (Java tests' `FileSystem` subclasses).
    pub fn with_file_system(wrap: impl FnOnce(FileSystemRef) -> FileSystemRef) -> Scratch {
        let fs = wrap(UnixFileSystem::create(DigestHashFunction::Sha256));
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!(
            "blaze_sky_loading_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // Use the real path so that paths compare equal to what the file system reports.
        let base = std::fs::canonicalize(&base).unwrap();
        let root = fs.get_path(&platform_to_internal(base.as_os_str()));
        Scratch { fs, root }
    }

    pub fn path(&self, rel: &str) -> Path {
        if rel.starts_with('/') {
            return self.fs.get_path(rel);
        }
        self.root.get_relative(&PathFragment::create(rel))
    }

    /// Writes `contents` (the bytes of its UTF-8 text) to the file `rel`, an internal string.
    pub fn file(&self, rel: &str, contents: &str) -> Path {
        self.file_bytes(rel, contents.as_bytes())
    }

    pub fn file_bytes(&self, rel: &str, contents: &[u8]) -> Path {
        let p = self.path(rel);
        std::fs::create_dir_all(platform(p.get_parent_directory().as_ref().unwrap())).unwrap();
        std::fs::write(platform(&p), contents).unwrap();
        p
    }

    pub fn dir(&self, rel: &str) -> Path {
        let p = self.path(rel);
        std::fs::create_dir_all(platform(&p)).unwrap();
        p
    }

    pub fn symlink(&self, link: &str, target: &str) -> Path {
        let p = self.path(link);
        std::fs::create_dir_all(platform(p.get_parent_directory().as_ref().unwrap())).unwrap();
        std::os::unix::fs::symlink(&*internal_to_platform(target), platform(&p)).unwrap();
        p
    }
}

/// The platform form of `path` (an internal string), for the `std::fs` calls of the scratch
/// helpers.
pub fn platform(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(internal_to_platform(path.get_path_string()).into_owned())
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(platform(&self.root));
    }
}

/// [`ExternalRepos`] as the Java tests configure it: REPOSITORY_MAPPING always answers
/// `VALUE_FOR_EMPTY_ROOT_MODULE`, and REPOSITORY_DIRECTORY knows the repos in `dirs` (as if
/// fetched to that root) and fails like `RepositoryFetchFunction` for the others.
#[derive(Default)]
pub struct FakeRepos {
    pub dirs: HashMap<String, Path>,
}

impl ExternalRepos for FakeRepos {
    fn compute_repository_mapping(
        &self,
        _repository_name: &RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMappingValue>, ComputeError> {
        Ok(Some(RepositoryMappingValue::value_for_empty_root_module()))
    }

    fn compute_repository_directory(
        &self,
        repository_name: &RepositoryName,
        _env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryDirectoryValue>, ComputeError> {
        if !repository_name.is_visible() {
            return Ok(Some(RepositoryDirectoryValue::Failure {
                error_msg: format!(
                    "No repository visible as '@{}' from {}",
                    repository_name.get_name(),
                    repository_name.get_context_repo_display_string()
                ),
            }));
        }
        match self.dirs.get(repository_name.get_name()) {
            Some(dir) => Ok(Some(RepositoryDirectoryValue::Success {
                root: Root::from_path(dir.clone()),
                exclude_from_vendoring: true,
            })),
            None => Ok(Some(RepositoryDirectoryValue::Failure {
                error_msg: format!("Repository '{}' is not defined", repository_name),
            })),
        }
    }
}

/// A Starlark environment with blaze_starlark's builtins only (no rules, no B/C natives), for
/// tests of the Skyframe functions themselves.
pub fn minimal_starlark_environment() -> Arc<BazelStarlarkEnvironment> {
    let fns = StarlarkGlobalsFns {
        util_toplevels: vec![
            blaze_starlark::depset::register_depset,
            blaze_starlark::json::register_json,
            blaze_packages::proto::register_proto,
        ],
        bzl_globals: vec![
            blaze_starlark::structs::register_struct,
            blaze_starlark::label::register_label,
        ],
        scl_toplevels: vec![blaze_starlark::structs::register_struct],
        ..StarlarkGlobalsFns::default()
    };
    Arc::new(BazelStarlarkEnvironment::new(
        StarlarkGlobals::new(&fns),
        "9.3.2",
        StarlarkEnv::new(),
        &RuleClassProviderRegistrations::bazel(),
    ))
}

/// A scratch workspace (`<scratch>/workspace`), output base and install base, and the Bazel
/// loading configuration over them.
pub struct TestSetup {
    pub scratch: Scratch,
    pub config: LoadingFunctionsConfig,
    pub semantics: StarlarkSemantics,
    /// The command's `PackageOptions`, when a test sets them
    /// ([`TestSetup::set_package_options`]); `None` uses the defaults' precomputed values.
    pub package_options: Option<PackageOptions>,
}

impl TestSetup {
    pub fn new(repos: FakeRepos) -> TestSetup {
        TestSetup::with_scratch(repos, Scratch::new())
    }

    /// Like [`TestSetup::new`] on the given scratch directory (and its file system).
    pub fn with_scratch(repos: FakeRepos, scratch: Scratch) -> TestSetup {
        let directories = LoadingDirectories {
            workspace: scratch.dir("workspace"),
            output_base: scratch.dir("output_base"),
            install_base: scratch.dir("install_base"),
        };
        let config = LoadingFunctionsConfig::for_bazel(
            directories,
            Arc::new(repos),
            Arc::new(LoadingFunctionsConfig::default_native_registry()),
            minimal_starlark_environment(),
            None,
        );
        TestSetup {
            scratch,
            config,
            semantics: StarlarkSemantics::from_flags(&BTreeMap::new()).unwrap(),
            package_options: None,
        }
    }

    /// Like [`TestSetup::new`] with Bazel's rule class provider and Starlark environment (for
    /// PACKAGE), and builtins injection disabled (`--experimental_builtins_bzl_path=`).
    pub fn with_bazel_rules(repos: FakeRepos) -> TestSetup {
        let scratch = Scratch::new();
        let directories = LoadingDirectories {
            workspace: scratch.dir("workspace"),
            output_base: scratch.dir("output_base"),
            install_base: scratch.dir("install_base"),
        };
        let config = LoadingFunctionsConfig::for_bazel_default(directories, Arc::new(repos));
        let mut semantics = StarlarkSemantics::from_flags(&BTreeMap::new()).unwrap();
        semantics.experimental_builtins_bzl_path = String::new();
        TestSetup {
            scratch,
            config,
            semantics,
            package_options: None,
        }
    }

    pub fn workspace(&self) -> &Path {
        self.config.directories.get_workspace()
    }

    /// A path relative to the workspace.
    pub fn ws(&self, rel: &str) -> String {
        format!("workspace/{}", rel)
    }

    pub fn set_package_path(&self, entries: Vec<Root>) {
        let locator = PathPackageLocator::new(
            Some(self.config.directories.get_output_base().clone()),
            entries,
            self.config.build_files_by_priority.clone(),
        );
        *self.config.pkg_locator.write() = locator;
    }

    /// Java `SkyframeExecutor.syncPackageLoading` with the command's `PackageOptions`
    /// (`--package_path`, `--deleted_packages`, `--default_visibility`, ...). Call it after the
    /// scratch files of the package path entries exist.
    pub fn set_package_options(&mut self, package_options: PackageOptions) {
        self.config.sync_package_loading(&package_options);
        self.package_options = Some(package_options);
    }

    pub fn evaluator(&self) -> Evaluator {
        let builder = register_loading_functions(EvaluatorBuilder::new(), &self.config);
        match &self.package_options {
            None => inject_loading_precomputed(
                builder,
                &self.config,
                self.semantics.clone(),
                IndexMap::new(),
            ),
            Some(package_options) => inject_loading_precomputed_with_package_options(
                builder,
                &self.config,
                self.semantics.clone(),
                IndexMap::new(),
                package_options,
            ),
        }
        .build()
    }
}

/// Evaluates `keys` with a fresh context.
pub fn evaluate(
    evaluator: &Evaluator,
    keys: Vec<SkyKey>,
    keep_going: bool,
    events: Arc<StoredEventHandler>,
) -> EvaluationResult {
    let ctx = EvaluationContext {
        keep_going,
        event_handler: events,
        ..EvaluationContext::default()
    };
    evaluator.evaluate(&keys, &ctx).expect("not interrupted")
}

/// Evaluates one key and returns its value; panics on error.
pub fn value_of<K: Key + Clone>(evaluator: &Evaluator, key: K) -> Arc<K::Value> {
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(evaluator, vec![SkyKey::new(key.clone())], false, events);
    assert!(
        !result.has_error(),
        "unexpected error evaluating {:?}: {:?}",
        key,
        result
    );
    result.get(&key).expect("value")
}
