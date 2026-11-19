//! The `query` command's view of the executor: blaze_query's `pkgcache` interfaces over
//! blaze_sky_loading's `SkyframePackageManager` (Java: `SkyframeExecutor.getPackageManager()`
//! implements `TargetProvider`, `CachingPackageLocator`; `SkyframeTargetPatternEvaluator`).
//!
//! PORT: not a Bazel class; the same adapters `blaze-query-dev` used, shared by
//! `blaze_runtime`'s `QueryCommand`.

use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier, RepositoryMapping, TargetPatternParser};
use blaze_events::ExtendedEventHandler;
use blaze_packages::target::{InputFile, Target};
use blaze_query::commands::query_command::QueryCommandEnvironment;
use blaze_query::common::query_transitive_package_preloader::SkyframeQueryTransitivePackagePreloader;
use blaze_query::pkgcache::{
    CachingPackageLocator, GetSiblingsError, GetTargetError, PreloadTargetPatternsError,
    TargetPatternPreloader, TargetProvider,
};
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::InterruptedException;
use blaze_sky_loading::package_manager::{
    self, NoSuchPackageOrInterrupted, NoSuchThingOrInterrupted, SkyframePackageManager,
};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

/// `SkyframePackageManager` as query's `pkgcache::TargetProvider`.
pub struct PackageManagerTargetProvider(pub Arc<SkyframePackageManager>);

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

/// `SkyframePackageManager` as query's `pkgcache::CachingPackageLocator`.
pub struct PackageManagerCachingPackageLocator(pub Arc<SkyframePackageManager>);

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

/// `SkyframePackageManager.preload_target_patterns` as query's
/// `pkgcache::TargetPatternPreloader` (Java `SkyframeTargetPatternEvaluator`).
pub struct PackageManagerTargetPatternPreloader(pub Arc<SkyframePackageManager>);

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

/// What `QueryEnvironmentBasedCommand.execInternal` reads from the `CommandEnvironment`, over the
/// command's executor.
// Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:96 QueryEnvironmentBasedCommand.execInternal
pub fn query_command_environment(
    skyframe_executor: &SkyframeExecutor,
    reporter: Arc<dyn ExtendedEventHandler>,
    output: OutputStream,
    relative_working_directory: PathFragment,
    working_directory: blaze_vfs::Path,
    main_repo_mapping: RepositoryMapping,
) -> QueryCommandEnvironment {
    let package_manager = skyframe_executor.get_package_manager().clone();
    QueryCommandEnvironment {
        reporter,
        output,
        hash_function: skyframe_executor.get_digest_function(),
        starlark_semantics: skyframe_executor.get_effective_starlark_semantics().clone(),
        relative_working_directory,
        working_directory,
        main_repo_mapping,
        // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
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
            SkyframeQueryTransitivePackagePreloader::new(package_manager),
        )),
    }
}
