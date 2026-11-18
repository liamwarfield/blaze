//! Loading packages and targets from outside Skyframe: the query-facing package manager.
//!
//! Java: com/google/devtools/build/lib/pkgcache/{TargetProvider,PackageProvider}.java,
//! com/google/devtools/build/lib/skyframe/SkyframePackageManager.java,
//! `SkyframeExecutor.SkyframePackageLoader.getPackage` and `SkyframeExecutor.getMainRepoMapping`.

use std::sync::Arc;

use blaze_cmdline::repository_name::MAIN;
use std::collections::HashMap;

use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryMapping, SignedTargetPattern, TargetParsingException,
    TargetPattern, TargetPatternParser, TargetPatternType, TargetPatternsCode,
};
use blaze_events::Event;
use indexmap::{IndexMap, IndexSet};

use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::target_pattern_eval::TargetPatternEvalError;
use crate::target_pattern_function::{
    FilteringPolicy, PackageBackedRecursivePackageProvider,
    RecursivePackageProviderBackedTargetPatternResolver, TargetPatternKey, TargetPatternValue,
};
use blaze_events::ExtendedEventHandler;
use blaze_packages::build::glob_cache::CachingPackageLocator as GlobCachingPackageLocator;
use blaze_packages::exceptions::{
    DetailedExitCode, FailureCode, NoSuchPackageException, NoSuchThingException,
};
use blaze_packages::package::Package;
use blaze_packages::target::{InputFile, Target};
use blaze_skyframe::{EvaluationContext, EvaluationResult, Evaluator, Interrupted, SkyKey};
use blaze_vfs::Path;

use crate::bzl_load::BzlLoadFailedException;
use crate::package_function::{PackageKey, PackageValue, SkyframePackageLocator};
use crate::package_lookup::PackageLookupKey;
use crate::repos::RepositoryMappingKey;

/// Java `SkyframeExecutor.DEFAULT_THREAD_COUNT` outside tests.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:381 SkyframeExecutor.DEFAULT_THREAD_COUNT
pub const DEFAULT_THREAD_COUNT: usize = 200;

/// A `NoSuchThingException` or an interruption (Java `throws NoSuchPackageException,
/// NoSuchTargetException, InterruptedException`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum NoSuchThingOrInterrupted {
    #[error("{0}")]
    NoSuchThing(#[from] NoSuchThingException),
    #[error("interrupted")]
    Interrupted,
}

impl From<Interrupted> for NoSuchThingOrInterrupted {
    fn from(_: Interrupted) -> Self {
        NoSuchThingOrInterrupted::Interrupted
    }
}

impl From<NoSuchPackageException> for NoSuchThingOrInterrupted {
    fn from(e: NoSuchPackageException) -> Self {
        NoSuchThingOrInterrupted::NoSuchThing(NoSuchThingException::Package(e))
    }
}

/// A `NoSuchPackageException` or an interruption.
#[derive(Debug, Clone, thiserror::Error)]
pub enum NoSuchPackageOrInterrupted {
    #[error("{0}")]
    NoSuchPackage(#[from] NoSuchPackageException),
    #[error("interrupted")]
    Interrupted,
}

impl From<Interrupted> for NoSuchPackageOrInterrupted {
    fn from(_: Interrupted) -> Self {
        NoSuchPackageOrInterrupted::Interrupted
    }
}

impl From<NoSuchPackageOrInterrupted> for NoSuchThingOrInterrupted {
    fn from(e: NoSuchPackageOrInterrupted) -> Self {
        match e {
            NoSuchPackageOrInterrupted::NoSuchPackage(e) => e.into(),
            NoSuchPackageOrInterrupted::Interrupted => NoSuchThingOrInterrupted::Interrupted,
        }
    }
}

/// API for retrieving targets.
// Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:29 TargetProvider
pub trait TargetProvider: Send + Sync {
    /// Returns the Target identified by `label`, loading, parsing and evaluating the package if
    /// it is not already loaded.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:38 TargetProvider.getTarget
    fn get_target(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Result<Target, NoSuchThingOrInterrupted>;

    /// Returns the BUILD file target of the package which contains the given target.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:44 TargetProvider.getBuildFile
    fn get_build_file(&self, target: &Target) -> Result<Arc<InputFile>, Interrupted>;

    /// Returns all targets in the package which contains the given target.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:53 TargetProvider.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target: &Target,
    ) -> Result<Vec<Target>, NoSuchPackageOrInterrupted>;
}

/// Finds BUILD files (Java `CachingPackageLocator`, with `SkyframePackageManager`'s
/// `getBaseNameForLoadedPackage`).
// Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:25 CachingPackageLocator
pub trait CachingPackageLocator: Send + Sync {
    /// Returns path of BUILD file for specified package iff the specified package exists, `None`
    /// otherwise.
    // Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:36 CachingPackageLocator.getBuildFileForPackage
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path>;

    /// The base name of the BUILD file of an already visited, existing package.
    // Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:44 CachingPackageLocator.getBaseNameForLoadedPackage
    fn get_base_name_for_loaded_package(&self, package_name: &PackageIdentifier) -> String;
}

/// The error of [`SkyframePackageManager::get_main_repo_mapping`].
// Java: com/google/devtools/build/lib/skyframe/RepositoryMappingValue.java:153 RepositoryMappingValue.RepositoryMappingResolutionException
#[derive(Debug, Clone, thiserror::Error)]
#[error("Error computing the main repository mapping: {}", detailed_exit_code.message)]
pub struct RepositoryMappingResolutionException {
    pub detailed_exit_code: DetailedExitCode,
}

/// The error of the main repo mapping computation, or an interruption.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RepositoryMappingResolutionOrInterrupted {
    #[error("{0}")]
    Resolution(#[from] RepositoryMappingResolutionException),
    #[error("interrupted")]
    Interrupted,
}

impl From<Interrupted> for RepositoryMappingResolutionOrInterrupted {
    fn from(_: Interrupted) -> Self {
        RepositoryMappingResolutionOrInterrupted::Interrupted
    }
}

/// Loads packages through the evaluator (Java `SkyframePackageManager` over
/// `SkyframeExecutor.SkyframePackageLoader`).
// Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:42 SkyframePackageManager
pub struct SkyframePackageManager {
    evaluator: Arc<Evaluator>,
    locator: SkyframePackageLocator,
    /// Java's `valueLookupLock`: evaluations are not re-entrant.
    value_lookup_lock: parking_lot::Mutex<()>,
}

impl SkyframePackageManager {
    pub fn new(evaluator: Arc<Evaluator>, locator: SkyframePackageLocator) -> Self {
        SkyframePackageManager {
            evaluator,
            locator,
            value_lookup_lock: parking_lot::Mutex::new(()),
        }
    }

    pub fn get_evaluator(&self) -> &Arc<Evaluator> {
        &self.evaluator
    }

    /// Evaluates `keys` (Java `SkyframeExecutor.evaluate(roots, keepGoing, numThreads,
    /// eventHandler)`).
    pub fn evaluate(
        &self,
        keys: &[SkyKey],
        keep_going: bool,
        num_threads: usize,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<EvaluationResult, Interrupted> {
        let _lock = self.value_lookup_lock.lock();
        let ctx = EvaluationContext {
            keep_going,
            parallelism: num_threads,
            event_handler,
            ..EvaluationContext::default()
        };
        self.evaluator.evaluate(keys, &ctx)
    }

    /// Returns the package (possibly in error), loading it in keep-going mode.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2836 SkyframeExecutor.SkyframePackageLoader.getPackage
    pub fn get_package(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        pkg_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, NoSuchPackageOrInterrupted> {
        let key = PackageKey::key(pkg_name.clone());
        // Loading a single package shouldn't be too bad to do in keep_going mode even if the
        // build overall is in nokeep_going mode: the worst that happens is we parse some
        // unnecessary .bzl files.
        let result = self.evaluate(
            &[SkyKey::new(key.clone())],
            /* keep_going= */ true,
            DEFAULT_THREAD_COUNT,
            event_handler,
        )?;
        if let Some(error) = result.get_error(&SkyKey::new(key.clone())) {
            // Java checkCycles.
            if !error.get_cycle_info().is_empty() {
                // PORT: Java reports the cycles with its CyclesReporter (not ported).
                return Err(
                    NoSuchPackageException::build_file_contains_errors_with_message(
                        pkg_name.clone(),
                        format!("Cycle encountered while loading package {}", pkg_name),
                        None,
                    )
                    .into(),
                );
            }
            let e = error
                .get_exception()
                .unwrap_or_else(|| panic!("{} {:?}", pkg_name, error));
            // PackageFunction should be catching, swallowing, and rethrowing all transitive
            // errors as NoSuchPackageExceptions or constructing packages with errors, since
            // we're in keep_going mode.
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                return Err(e.clone().into());
            }
            panic!(
                "IllegalStateException: Unexpected Exception type from PackageValue for '{}'' \
                 with error: {:?}",
                pkg_name, error
            );
        }
        let value: Arc<PackageValue> = result.get(&key).expect("PACKAGE value without error");
        Ok(value.get_package().clone())
    }

    /// The package of an already loaded package, else loads it.
    fn get_loaded_package(
        &self,
        pkg_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, Interrupted> {
        if let Some(v) = self
            .evaluator
            .get_existing_value(&SkyKey::new(PackageKey::key(pkg_name.clone())))
        {
            let v = v
                .downcast::<PackageValue>()
                .unwrap_or_else(|_| panic!("PACKAGE value of {}", pkg_name));
            return Ok(v.get_package().clone());
        }
        match self.get_package(Arc::new(blaze_events::StoredEventHandler::new()), pkg_name) {
            Ok(p) => Ok(p),
            Err(NoSuchPackageOrInterrupted::Interrupted) => Err(Interrupted),
            Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => {
                panic!("package of a loaded target is not loadable: {}", e)
            }
        }
    }

    /// Java `SkyframeExecutor.getMainRepoMapping(keepGoing, loadingPhaseThreads, eventHandler)`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3420 SkyframeExecutor.getMainRepoMapping
    pub fn get_main_repo_mapping(
        &self,
        keep_going: bool,
        loading_phase_threads: usize,
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> Result<RepositoryMapping, RepositoryMappingResolutionOrInterrupted> {
        let main_repo_mapping_key = RepositoryMappingKey::key(MAIN.clone());
        let eval_result = self.evaluate(
            &[SkyKey::new(main_repo_mapping_key.clone())],
            keep_going,
            loading_phase_threads,
            event_handler,
        )?;
        if eval_result.has_error() {
            let error_info = eval_result
                .get_error(&SkyKey::new(main_repo_mapping_key.clone()))
                .or_else(|| eval_result.get_first_error())
                .expect("an error");
            let Some(e) = error_info.get_exception() else {
                // PORT: Java reports the cycles with its CyclesReporter (not ported).
                return Err(RepositoryMappingResolutionException {
                    detailed_exit_code: DetailedExitCode {
                        message: "cycles detected during computation of main repo mapping"
                            .to_owned(),
                        code: FailureCode::Other(
                            "ExternalRepository.REPOSITORY_MAPPING_RESOLUTION_FAILED".to_owned(),
                        ),
                    },
                }
                .into());
            };
            // Java: e instanceof DetailedException.
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                return Err(RepositoryMappingResolutionException {
                    detailed_exit_code: e.get_detailed_exit_code(),
                }
                .into());
            }
            if let Some(e) = e.downcast_ref::<BzlLoadFailedException>() {
                return Err(RepositoryMappingResolutionException {
                    detailed_exit_code: e.get_detailed_exit_code().clone(),
                }
                .into());
            }
            return Err(RepositoryMappingResolutionException {
                detailed_exit_code: DetailedExitCode {
                    message: format!("error during computation of main repo mapping: {}", e),
                    code: FailureCode::Other("ExternalRepository".to_owned()),
                },
            }
            .into());
        }
        let value = eval_result
            .get(&main_repo_mapping_key)
            .expect("REPOSITORY_MAPPING value");
        Ok(value
            .repository_mapping()
            .cloned()
            .expect("the main repository has a mapping"))
    }
}

impl TargetProvider for SkyframePackageManager {
    // Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:75 SkyframePackageManager.getTarget
    fn get_target(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Result<Target, NoSuchThingOrInterrupted> {
        // TODO(https://github.com/bazelbuild/bazel/issues/23852): don't expand the full package
        // if lazy macro expansion is enabled.
        let pkg = self.get_package(event_handler, label.get_package_identifier())?;
        match pkg.get_target(label.get_name()) {
            Ok(t) => Ok(t.clone()),
            Err(e) => Err(NoSuchThingOrInterrupted::NoSuchThing(
                NoSuchThingException::Target(e),
            )),
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:108 PackageProvider.getBuildFile
    fn get_build_file(&self, target: &Target) -> Result<Arc<InputFile>, Interrupted> {
        // Monolithic package.
        let pkg = self.get_loaded_package(target.get_label().get_package_identifier())?;
        Ok(pkg.get_build_file().clone())
    }

    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:147 PackageProvider.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        target: &Target,
    ) -> Result<Vec<Target>, NoSuchPackageOrInterrupted> {
        let pkg = self.get_package(event_handler, target.get_label().get_package_identifier())?;
        Ok(pkg.get_targets().values().cloned().collect())
    }
}

impl CachingPackageLocator for SkyframePackageManager {
    // Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:103 SkyframePackageManager.getBuildFileForPackage
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path> {
        self.locator.get_build_file_for_package(package_name)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:116 SkyframePackageManager.getBaseNameForLoadedPackage
    fn get_base_name_for_loaded_package(&self, package_name: &PackageIdentifier) -> String {
        let pkg_lookup_value = self
            .evaluator
            .get_existing_value(&SkyKey::new(PackageLookupKey::key(package_name.clone())))
            .unwrap_or_else(|| {
                panic!("Package should already have been visited: {}", package_name)
            });
        let pkg_lookup_value = pkg_lookup_value
            .downcast::<crate::package_lookup::PackageLookupValue>()
            .unwrap_or_else(|_| panic!("PACKAGE_LOOKUP value of {}", package_name));
        assert!(
            pkg_lookup_value.package_exists(),
            "Package must exist: {} {:?}",
            package_name,
            pkg_lookup_value
        );
        pkg_lookup_value
            .get_build_file_name()
            .get_filename_fragment()
            .get_base_name()
            .to_owned()
    }
}

/// What [`SkyframePackageManager::preload_target_patterns`] throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PreloadTargetPatternsError {
    #[error("{}", .0.get_message())]
    TargetParsing(TargetParsingException),
    #[error("interrupted")]
    Interrupted,
}

impl From<Interrupted> for PreloadTargetPatternsError {
    fn from(_: Interrupted) -> Self {
        PreloadTargetPatternsError::Interrupted
    }
}

/// A pattern and the key that resolves it (Java `SkyframeTargetPatternEvaluator.PatternLookup`):
/// simple patterns (a single target, or the targets of one package) are resolved from their
/// PACKAGE value, the others by TARGET_PATTERN.
// Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:239 SkyframeTargetPatternEvaluator.PatternLookup
enum PatternLookup {
    Simple {
        pattern: String,
        key: PackageKey,
        target_pattern: TargetPattern,
    },
    Normal {
        pattern: String,
        key: TargetPatternKey,
    },
}

impl PatternLookup {
    fn pattern(&self) -> &str {
        match self {
            PatternLookup::Simple { pattern, .. } | PatternLookup::Normal { pattern, .. } => {
                pattern
            }
        }
    }

    fn sky_key(&self) -> SkyKey {
        match self {
            PatternLookup::Simple { key, .. } => SkyKey::new(key.clone()),
            PatternLookup::Normal { key, .. } => SkyKey::new(key.clone()),
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:221 SkyframeTargetPatternEvaluator.createPatternParsingError
fn create_pattern_parsing_error(e: &TargetParsingException, pattern: &str) -> Event {
    // PORT: Java attaches the DetailedExitCode as an event property.
    Event::error(format!("Skipping '{}': {}", pattern, e.get_message()))
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:209 SkyframeTargetPatternEvaluator.isSimple
fn is_simple(target_pattern: &TargetPattern) -> bool {
    match target_pattern.get_type() {
        TargetPatternType::SingleTarget | TargetPatternType::TargetsInPackage => true,
        // Both of these require multiple package lookups. PATH_AS_TARGET needs to find the
        // enclosing package, and TARGETS_BELOW_DIRECTORY recursively looks for all packages
        // under a specified directory.
        TargetPatternType::PathAsTarget | TargetPatternType::TargetsBelowDirectory => false,
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:155 SkyframeTargetPatternEvaluator.wrapException
fn wrap_exception(
    exception: &blaze_skyframe::SkyError,
    key_is_package: bool,
) -> TargetParsingException {
    if key_is_package && let Some(e) = exception.downcast_ref::<NoSuchPackageException>() {
        // A "simple" target pattern (like "//pkg:t") doesn't have a TargetPatternKey, just a
        // Package key, so it results in NoSuchPackageException that we transform here.
        // PORT: Java keeps the exception's DetailedExitCode.
        return TargetParsingException::new(
            e.get_message(),
            TargetPatternsCode::TargetPatternsUnknown,
        );
    }
    TargetParsingException::new(
        format!(
            "Target parsing failed due to unexpected exception: {}",
            exception
        ),
        TargetPatternsCode::CannotPreloadTarget,
    )
}

impl SkyframePackageManager {
    /// Attempts to parse and load the given patterns; the returned map contains the results for
    /// each pattern successfully parsed (Java `SkyframeTargetPatternEvaluator`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:62 SkyframeTargetPatternEvaluator.preloadTargetPatterns
    pub fn preload_target_patterns(
        &self,
        event_handler: Arc<dyn ExtendedEventHandler>,
        main_repo_target_parser: &TargetPatternParser,
        patterns: &[String],
        keep_going: bool,
    ) -> Result<IndexMap<String, Vec<Target>>, PreloadTargetPatternsError> {
        let mut result_builder: IndexMap<String, Vec<Target>> = IndexMap::new();
        let mut pattern_lookups: Vec<PatternLookup> = Vec::new();
        let mut all_keys: Vec<SkyKey> = Vec::new();
        for pattern in patterns {
            assert!(!pattern.starts_with('-'));
            // Java createPatternLookup.
            match SignedTargetPattern::parse(pattern, main_repo_target_parser) {
                Ok(signed) => {
                    let key = TargetPatternKey::key(signed, FilteringPolicy::NoFilter);
                    let lookup = if is_simple(key.get_parsed_pattern()) {
                        PatternLookup::Simple {
                            pattern: pattern.clone(),
                            key: PackageKey::key(key.get_parsed_pattern().get_directory().clone()),
                            target_pattern: key.get_parsed_pattern().clone(),
                        }
                    } else {
                        PatternLookup::Normal {
                            pattern: pattern.clone(),
                            key,
                        }
                    };
                    all_keys.push(lookup.sky_key());
                    pattern_lookups.push(lookup);
                }
                Err(e) => {
                    // PORT: Java posts a ParsingFailedEvent (event bus not ported).
                    if !keep_going {
                        return Err(PreloadTargetPatternsError::TargetParsing(e));
                    }
                    event_handler.handle(&create_pattern_parsing_error(&e, pattern));
                    result_builder.insert(pattern.clone(), Vec::new());
                }
            }
        }

        let result = self.evaluate(
            &all_keys,
            keep_going,
            DEFAULT_THREAD_COUNT,
            event_handler.clone(),
        )?;
        if let Some(catastrophe) = result.get_catastrophe() {
            if let Some(e) = catastrophe.downcast_ref::<TargetParsingException>() {
                return Err(PreloadTargetPatternsError::TargetParsing(e.clone()));
            }
            return Err(PreloadTargetPatternsError::TargetParsing(wrap_exception(
                catastrophe,
                false,
            )));
        }
        for pattern_lookup in &pattern_lookups {
            let key = pattern_lookup.sky_key();
            if let Some(result_value) = result.get_dyn(&key) {
                let processed = match pattern_lookup {
                    PatternLookup::Simple { target_pattern, .. } => {
                        let pkg = result_value
                            .clone()
                            .downcast::<PackageValue>()
                            .unwrap_or_else(|_| panic!("PACKAGE value"))
                            .get_package()
                            .clone();
                        Self::process_simple_lookup(&*event_handler, target_pattern, pkg)
                    }
                    PatternLookup::Normal { .. } => {
                        let value = result_value
                            .clone()
                            .downcast::<TargetPatternValue>()
                            .unwrap_or_else(|_| panic!("TARGET_PATTERN value"));
                        Ok(self.process_normal_lookup(&value))
                    }
                };
                match processed {
                    Ok(targets) => {
                        result_builder.insert(pattern_lookup.pattern().to_owned(), targets);
                    }
                    Err(e) => {
                        if !keep_going {
                            return Err(PreloadTargetPatternsError::TargetParsing(e));
                        }
                        event_handler
                            .handle(&create_pattern_parsing_error(&e, pattern_lookup.pattern()));
                        result_builder.insert(pattern_lookup.pattern().to_owned(), Vec::new());
                    }
                }
            } else {
                let raw_pattern = pattern_lookup.pattern();
                let Some(error) = result.get_error(&key) else {
                    // Java: BugReport for a keep-going build without an error for the key.
                    continue;
                };
                let target_parsing_exception = match error.get_exception() {
                    // This exception could be a TargetParsingException for a target pattern, a
                    // NoSuchPackageException for a label (or package wildcard), or potentially a
                    // lower-level exception if there is a bug in error handling.
                    Some(exception) => match exception.downcast_ref::<TargetParsingException>() {
                        Some(tpe) => tpe.clone(),
                        None => wrap_exception(
                            exception,
                            matches!(pattern_lookup, PatternLookup::Simple { .. }),
                        ),
                    },
                    None => {
                        assert!(
                            !error.get_cycle_info().is_empty(),
                            "No exception or cycle {:?} {:?}",
                            key,
                            error
                        );
                        // PORT: Java reports the cycles with its CyclesReporter (not ported).
                        TargetParsingException::new(
                            "cycles detected during target parsing",
                            TargetPatternsCode::Cycle,
                        )
                    }
                };
                if keep_going {
                    event_handler.handle(&create_pattern_parsing_error(
                        &target_parsing_exception,
                        raw_pattern,
                    ));
                } else {
                    return Err(PreloadTargetPatternsError::TargetParsing(
                        target_parsing_exception,
                    ));
                }
                result_builder.insert(pattern_lookup.pattern().to_owned(), Vec::new());
            }
        }
        Ok(result_builder)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:281 SkyframeTargetPatternEvaluator.SimpleLookup.process
    fn process_simple_lookup(
        event_handler: &dyn ExtendedEventHandler,
        target_pattern: &TargetPattern,
        pkg: Arc<Package>,
    ) -> Result<Vec<Target>, TargetParsingException> {
        let mut packages = HashMap::new();
        packages.insert(pkg.get_package_identifier().clone(), pkg);
        let provider = PackageBackedRecursivePackageProvider::new(packages);
        let resolver = RecursivePackageProviderBackedTargetPatternResolver::new(
            &provider,
            event_handler,
            FilteringPolicy::NoFilter,
        );
        let mut result: Vec<Target> = Vec::new();
        match crate::target_pattern_eval::eval(
            target_pattern,
            &resolver,
            &|| Ok(IgnoredSubdirectories::empty()),
            &[],
            &mut |partial_result| result = partial_result,
        ) {
            Ok(()) => Ok(result),
            Err(TargetPatternEvalError::TargetParsing(e)) => Err(e),
            Err(e) => panic!(
                "IllegalStateException: PackageBackedRecursivePackageProvider doesn't throw for \
                 {}: {:?}",
                target_pattern, e
            ),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeTargetPatternEvaluator.java:262 SkyframeTargetPatternEvaluator.NormalLookup.process
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternsResultBuilder.java:40 TargetPatternsResultBuilder.build
    fn process_normal_lookup(&self, value: &TargetPatternValue) -> Vec<Target> {
        let mut packages: HashMap<PackageIdentifier, Arc<Package>> = HashMap::new();
        let mut targets: IndexSet<Label> = IndexSet::new();
        let mut result = Vec::new();
        for label in value.get_targets() {
            if !targets.insert(label.clone()) {
                continue;
            }
            let pkg_id = label.get_package_identifier();
            let pkg = packages.entry(pkg_id.clone()).or_insert_with(|| {
                self.evaluator
                    .get_existing_value(&SkyKey::new(PackageKey::key(pkg_id.clone())))
                    .unwrap_or_else(|| panic!("{} not in the graph", pkg_id))
                    .downcast::<PackageValue>()
                    .unwrap_or_else(|_| panic!("PACKAGE value"))
                    .get_package()
                    .clone()
            });
            // This target exists: TARGET_PATTERN found it in this package.
            result.push(
                pkg.get_target(label.get_name())
                    .unwrap_or_else(|e| panic!("IllegalStateException: {}", e.get_message()))
                    .clone(),
            );
        }
        result
    }
}
