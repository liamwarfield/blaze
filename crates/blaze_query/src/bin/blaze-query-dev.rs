//! A temporary entry point that runs `query` against a workspace whose external repositories
//! were recorded from the oracle (W3), until `query` is wired into Blaze's command dispatcher.
//!
//! usage: blaze-query-dev --workspace <dir> --output_base <dir> --repos <repos.json>
//!                        [--install_base <dir>] '<expr>' [query flags...]
//!        blaze-query-dev help
//!
//! Prints the query result to stdout and errors/warnings to stderr like Bazel's UI, and exits
//! with Bazel's exit code. `scripts/w3/query-compare` depends on this interface.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier, TargetPatternParser};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_options::{OptionsClass, OptionsParser};
use blaze_packages::target::{InputFile, Target};
use blaze_query::commands::query_command::{QueryCommandEnvironment, exec_internal};
use blaze_query::common::query_transitive_package_preloader::SkyframeQueryTransitivePackagePreloader;
use blaze_query::compat::failure_details;
use blaze_query::pkgcache::{
    CachingPackageLocator, GetSiblingsError, GetTargetError, PreloadTargetPatternsError,
    TargetPatternPreloader, TargetProvider,
};
use blaze_query_engine::InterruptedException;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::package_function::SkyframePackageLocator;
use blaze_sky_loading::package_manager::{
    self, NoSuchPackageOrInterrupted, NoSuchThingOrInterrupted,
    RepositoryMappingResolutionOrInterrupted, SkyframePackageManager,
};
use blaze_sky_loading::repos::OracleRepos;
use blaze_sky_loading::skyframe_executor::{
    LoadingFunctionsConfig, inject_loading_precomputed_with_package_options,
    register_loading_functions,
};
use blaze_skyframe::EvaluatorBuilder;
use blaze_starlark::StarlarkSemantics;
use blaze_util::string_encoding::{
    internal_to_platform, latin1_encode_lossy, platform_to_internal,
};
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

const USAGE: &str = "usage: blaze-query-dev --workspace <dir> --output_base <dir> --repos <repos.json> \
[--install_base <dir>] '<expr>' [query flags...]";

/// The oracle's install base (embedded `@bazel_tools`), per AGENTS.md.
const DEFAULT_INSTALL_BASE: &str =
    ".cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9";

/// Prints events like Bazel's terminal UI with `--color=no --curses=no`: `ERROR: `, `WARNING: `
/// and `INFO: ` prefixes, the location (if any) and the message.
///
/// PORT: the UI (`UiEventHandler`) is not ported; this covers the event kinds query reports.
struct StderrEventHandler;

impl EventHandler for StderrEventHandler {
    fn handle(&self, event: &Event) {
        let prefix = match event.get_kind() {
            EventKind::Error => "ERROR: ",
            EventKind::Warning => "WARNING: ",
            EventKind::Info => "INFO: ",
            EventKind::Debug => "DEBUG: ",
            _ => return,
        };
        let mut line = String::from(prefix);
        if let Some(location) = event.get_location() {
            line.push_str(&location.to_string());
            line.push_str(": ");
        }
        line.push_str(&event.get_message());
        line.push('\n');
        // The console is a Latin-1 stream on Bazel's server.
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(&latin1_encode_lossy(&line));
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for StderrEventHandler {
    fn post(&self, _obj: Arc<dyn Postable>) {}
}

/// Adapts blaze_sky_loading's `SkyframePackageManager` (Java `TargetProvider`) to query's
/// `pkgcache::TargetProvider`.
struct PackageManagerTargetProvider(Arc<SkyframePackageManager>);

fn no_such_thing_or_interrupted(e: NoSuchThingOrInterrupted) -> GetTargetError {
    match e {
        NoSuchThingOrInterrupted::NoSuchThing(e) => GetTargetError::NoSuchThing(e),
        NoSuchThingOrInterrupted::Interrupted => {
            GetTargetError::Interrupted(InterruptedException::new())
        }
    }
}

impl TargetProvider for PackageManagerTargetProvider {
    fn get_target(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Result<Target, GetTargetError> {
        package_manager::TargetProvider::get_target(&*self.0, event_handler.clone(), label)
            .map_err(no_such_thing_or_interrupted)
    }

    fn get_build_file(&self, target: &Target) -> Result<Arc<InputFile>, InterruptedException> {
        package_manager::TargetProvider::get_build_file(&*self.0, target)
            .map_err(|_| InterruptedException::new())
    }

    fn get_sibling_targets_in_package(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        target: &Target,
    ) -> Result<Vec<Target>, GetSiblingsError> {
        package_manager::TargetProvider::get_sibling_targets_in_package(
            &*self.0,
            event_handler.clone(),
            target,
        )
        .map_err(|e| match e {
            NoSuchPackageOrInterrupted::NoSuchPackage(e) => GetSiblingsError::NoSuchPackage(e),
            NoSuchPackageOrInterrupted::Interrupted => {
                GetSiblingsError::Interrupted(InterruptedException::new())
            }
        })
    }
}

/// Adapts `SkyframePackageManager` (Java `CachingPackageLocator`).
struct PackageManagerCachingPackageLocator(Arc<SkyframePackageManager>);

impl blaze_query::pkgcache::PackageProvider for PackageManagerTargetProvider {
    fn get_package(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<blaze_packages::package::Package>, blaze_query::pkgcache::GetPackageError> {
        self.0
            .get_package(event_handler.clone(), package_name)
            .map_err(|e| match e {
                NoSuchPackageOrInterrupted::NoSuchPackage(e) => {
                    blaze_query::pkgcache::GetPackageError::NoSuchPackage(e)
                }
                NoSuchPackageOrInterrupted::Interrupted => {
                    blaze_query::pkgcache::GetPackageError::Interrupted(InterruptedException::new())
                }
            })
    }
}

impl CachingPackageLocator for PackageManagerCachingPackageLocator {
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path> {
        package_manager::CachingPackageLocator::get_build_file_for_package(&*self.0, package_name)
    }

    fn get_base_name_for_loaded_package(&self, package_name: &PackageIdentifier) -> Option<String> {
        Some(
            package_manager::CachingPackageLocator::get_base_name_for_loaded_package(
                &*self.0,
                package_name,
            ),
        )
    }
}

/// Adapts `SkyframePackageManager.preload_target_patterns` (Java
/// `SkyframeTargetPatternEvaluator`) to query's `pkgcache::TargetPatternPreloader`.
struct PackageManagerTargetPatternPreloader(Arc<SkyframePackageManager>);

impl TargetPatternPreloader for PackageManagerTargetPatternPreloader {
    fn preload_target_patterns(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        main_repo_target_parser: &TargetPatternParser,
        patterns: &[String],
        keep_going: bool,
    ) -> Result<IndexMap<String, Vec<Target>>, PreloadTargetPatternsError> {
        self.0
            .preload_target_patterns(
                event_handler.clone(),
                main_repo_target_parser,
                patterns,
                keep_going,
            )
            .map_err(|e| match e {
                package_manager::PreloadTargetPatternsError::TargetParsing(e) => {
                    PreloadTargetPatternsError::TargetParsing(e)
                }
                package_manager::PreloadTargetPatternsError::Interrupted => {
                    PreloadTargetPatternsError::Interrupted(InterruptedException::new())
                }
            })
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    // argv is a platform string; Java's server receives it as an internal string.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| platform_to_internal(&a).into_owned())
        .collect();
    if args.first().map(|a| a.as_str()) == Some("help") {
        println!("{USAGE}");
        return;
    }

    let mut workspace: Option<String> = None;
    let mut output_base: Option<String> = None;
    let mut repos: Option<String> = None;
    let mut install_base: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n, Some(v.to_owned())),
            None => (a.as_str(), None),
        };
        let slot = match name {
            "--workspace" => Some(&mut workspace),
            "--output_base" => Some(&mut output_base),
            "--repos" => Some(&mut repos),
            "--install_base" => Some(&mut install_base),
            _ => None,
        };
        match slot {
            Some(slot) => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i)
                            .cloned()
                            .unwrap_or_else(|| usage_error(&format!("{name} needs a value")))
                    }
                };
                *slot = Some(value);
            }
            None => rest.push(a.clone()),
        }
        i += 1;
    }
    let workspace = workspace.unwrap_or_else(|| usage_error("missing --workspace"));
    let output_base = output_base.unwrap_or_else(|| usage_error("missing --output_base"));
    let repos = repos.unwrap_or_else(|| usage_error("missing --repos"));
    let install_base = install_base.unwrap_or_else(|| {
        let home = std::env::var_os("HOME")
            .map(|h| platform_to_internal(&h).into_owned())
            .unwrap_or_default();
        format!("{home}/{DEFAULT_INSTALL_BASE}")
    });

    // Java: BlazeCommandDispatcher parses the command's options; query allows residue.
    let mut parser = OptionsParser::builder()
        .options_classes(&[
            QueryOptions::class(),
            PackageOptions::class(),
            KeepGoingOption::class(),
            LoadingPhaseThreadsOption::class(),
        ])
        .allow_residue(true)
        .build()
        .unwrap_or_else(|e| panic!("ConstructionException: {e:?}"));
    let rest_refs: Vec<&str> = rest.iter().map(|s| s.as_str()).collect();
    if let Err(e) = parser.parse(&rest_refs) {
        eprintln!("ERROR: {e}");
        std::process::exit(2);
    }
    let query_options: QueryOptions = parser.get_options().expect("QueryOptions");
    let keep_going = parser
        .get_options::<KeepGoingOption>()
        .expect("KeepGoingOption")
        .keep_going;
    let loading_phase_threads = parser
        .get_options::<LoadingPhaseThreadsOption>()
        .expect("LoadingPhaseThreadsOption")
        .threads;
    let residue = parser.get_residue();

    let digest_function = DigestHashFunction::Sha256;
    let fs = UnixFileSystem::create(digest_function);
    let directories = LoadingDirectories {
        workspace: Path::create(PathFragment::create(&workspace), &fs),
        output_base: Path::create(PathFragment::create(&output_base), &fs),
        install_base: Path::create(PathFragment::create(&install_base), &fs),
    };
    let external_repos = match OracleRepos::load(
        std::path::Path::new(&*internal_to_platform(&repos)),
        directories.clone(),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: {}", e.0);
            std::process::exit(2);
        }
    };
    let mut config =
        LoadingFunctionsConfig::for_bazel_default(directories, Arc::new(external_repos));
    // Java: SkyframeExecutor.syncPackageLoading (--package_path, --deleted_packages,
    // --default_visibility, the config_setting visibility flags).
    let package_options: PackageOptions = parser.get_options().expect("PackageOptions");
    config.sync_package_loading(&package_options);
    let starlark_semantics = StarlarkSemantics::default();
    let evaluator = Arc::new(
        inject_loading_precomputed_with_package_options(
            register_loading_functions(EvaluatorBuilder::new(), &config),
            &config,
            starlark_semantics.clone(),
            IndexMap::new(),
            &package_options,
        )
        .build(),
    );

    let reporter: Arc<dyn ExtendedEventHandler> = Arc::new(StderrEventHandler);
    let package_manager = Arc::new(SkyframePackageManager::new(
        evaluator,
        SkyframePackageLocator {
            pkg_locator: config.pkg_locator.clone(),
            deleted_packages: config.deleted_packages.clone(),
        },
    ));

    // env.getSkyframeExecutor().getMainRepoMapping(keepGoing, threads, reporter)
    let main_repo_mapping = match package_manager.get_main_repo_mapping(
        keep_going,
        loading_phase_threads as usize,
        reporter.clone(),
    ) {
        Ok(mapping) => mapping,
        Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
            reporter.handle(&Event::error(e.to_string()));
            let detailed_exit_code = failure_details::from_packages(&e.detailed_exit_code);
            std::process::exit(detailed_exit_code.get_exit_code());
        }
        Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
            reporter.handle(&Event::error("query interrupted"));
            std::process::exit(8);
        }
    };

    let stdout: Rc<RefCell<dyn Write>> = Rc::new(RefCell::new(std::io::stdout()));
    let env = QueryCommandEnvironment {
        reporter,
        output: stdout,
        hash_function: digest_function,
        starlark_semantics,
        relative_working_directory: PathFragment::empty_fragment(),
        working_directory: Path::create(PathFragment::create(&workspace), &fs),
        main_repo_mapping,
        query_functions: Vec::new(),
        target_provider: Arc::new(PackageManagerTargetProvider(package_manager.clone())),
        package_provider: Arc::new(PackageManagerTargetProvider(package_manager.clone())),
        caching_package_locator: Arc::new(PackageManagerCachingPackageLocator(
            package_manager.clone(),
        )),
        target_pattern_preloader: Arc::new(PackageManagerTargetPatternPreloader(
            package_manager.clone(),
        )),
        query_transitive_package_preloader: Some(Arc::new(
            SkyframeQueryTransitivePackagePreloader::new(package_manager.clone()),
        )),
        // PORT: the dev binary has no SkyframeExecutor: no SkyQuery.
        graph_factory: None,
        package_path: None,
        query_evaluation_parallelism_level: 1,
    };

    let detailed_exit_code = exec_internal(
        &env,
        &query_options,
        keep_going,
        loading_phase_threads,
        &residue,
    );
    let _ = std::io::stdout().flush();
    std::process::exit(detailed_exit_code.get_exit_code());
}
