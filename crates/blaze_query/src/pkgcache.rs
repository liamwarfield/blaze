//! The `lib/pkgcache` interfaces query uses: `TargetProvider`, `TargetPatternPreloader`,
//! `TargetEdgeObserver`, `CachingPackageLocator`, and the transitive-closure preloading seam of
//! `query2/common/QueryTransitivePackagePreloader`.
//!
//! PORT: these interfaces are implemented on top of Skyframe by blaze_sky_loading (owner D).
//! blaze_sky_loading cannot depend on blaze_query, so the interfaces query consumes are declared
//! here, in Java's shape, and the dev binary / command adapts blaze_sky_loading's
//! implementations to them. Move them next to their implementations once the crate layout
//! allows.

use blaze_cmdline::{Label, PackageIdentifier, TargetParsingException, TargetPatternParser};
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::Attribute;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_packages::target::{InputFile, Target};
use blaze_query_engine::{InterruptedException, QueryError, QueryExpression};
use blaze_vfs::Path;
use indexmap::IndexMap;
use std::sync::Arc;

/// What `TargetProvider.getTarget` throws: `NoSuchPackageException`, `NoSuchTargetException`
/// (both [`NoSuchThingException`]) or `InterruptedException`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GetTargetError {
    #[error(transparent)]
    NoSuchThing(#[from] NoSuchThingException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

/// What `TargetProvider.getSiblingTargetsInPackage` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GetSiblingsError {
    #[error(transparent)]
    NoSuchPackage(#[from] NoSuchPackageException),
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

/// What `TargetPatternPreloader.preloadTargetPatterns` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PreloadTargetPatternsError {
    #[error(transparent)]
    TargetParsing(#[from] TargetParsingException),
    /// A `TargetParsingException` built from another exception's `DetailedExitCode` (Java
    /// `new TargetParsingException(message, cause, detailedExitCode)`), which blaze_cmdline's
    /// `TargetParsingException` (a `TargetPatterns` code only) cannot hold.
    #[error("{message}")]
    TargetParsingWithFailureDetail {
        message: String,
        failure_detail: blaze_protos::failure_details::FailureDetail,
    },
    #[error(transparent)]
    Interrupted(#[from] InterruptedException),
}

/// API for retrieving targets.
///
/// PORT: Java's `getTarget` may return null when Skyframe needs a restart (only under
/// SkyQuery-style walkable graphs); Blaze's blocking evaluator never does, so the result is
/// non-null.
// Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:27 TargetProvider
pub trait TargetProvider {
    /// Returns the Target identified by "label", loading, parsing and evaluating the package if
    /// it is not already loaded.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:38 TargetProvider.getTarget
    fn get_target(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        label: &Label,
    ) -> Result<Target, GetTargetError>;

    /// Returns the BUILD file of the package of the given target.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:42 TargetProvider.getBuildFile
    fn get_build_file(&self, target: &Target) -> Result<Arc<InputFile>, InterruptedException>;

    /// Returns all of the targets in `target`'s package, in package order.
    // Java: com/google/devtools/build/lib/pkgcache/TargetProvider.java:53 TargetProvider.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        target: &Target,
    ) -> Result<Vec<Target>, GetSiblingsError>;
}

/// A preloader for target patterns.
// Java: com/google/devtools/build/lib/pkgcache/TargetPatternPreloader.java:27 TargetPatternPreloader
pub trait TargetPatternPreloader {
    /// Attempts to parse and load the given collection of patterns; the returned map contains
    /// the results for each pattern successfully parsed. As a side effect, calling this method
    /// populates the Skyframe graph, so subsequent calls are faster.
    ///
    /// The map preserves the order of `patterns` (Java `ImmutableMap`); each pattern's targets
    /// keep the evaluator's order.
    // Java: com/google/devtools/build/lib/pkgcache/TargetPatternPreloader.java:36 TargetPatternPreloader.preloadTargetPatterns
    fn preload_target_patterns(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        main_repo_target_parser: &TargetPatternParser,
        patterns: &[String],
        keep_going: bool,
    ) -> Result<IndexMap<String, Vec<Target>>, PreloadTargetPatternsError>;
}

/// Java `CachingPackageLocator`.
// Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:25 CachingPackageLocator
pub trait CachingPackageLocator {
    /// Returns the path of the BUILD file of the given package, or `None` if the package does
    /// not exist.
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path>;

    /// Returns the base name of the BUILD file of the given (loaded) package, or `None`.
    fn get_base_name_for_loaded_package(&self, package_name: &PackageIdentifier) -> Option<String>;
}

/// The transitive-target preloading of `QueryTransitivePackagePreloader.preloadTransitiveTargets`
/// (TRANSITIVE_TARGET evaluation of `labels_to_visit`, then Java's error handling).
// Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:43 QueryTransitivePackagePreloader
pub trait QueryTransitivePackagePreloader {
    /// Unconditionally evaluates the transitive closure of the given labels; on errors throws a
    /// `QueryException` attributed to `caller_for_error` when it is set.
    // Java: com/google/devtools/build/lib/query2/common/QueryTransitivePackagePreloader.java:137 QueryTransitivePackagePreloader.preloadTransitiveTargets
    fn preload_transitive_targets(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        labels_to_visit: &[Label],
        keep_going: bool,
        parallel_threads: i32,
        caller_for_error: Option<&QueryExpression>,
    ) -> Result<(), QueryError>;
}

/// An observer of the visitation over a target graph.
// Java: com/google/devtools/build/lib/pkgcache/TargetEdgeObserver.java:24 TargetEdgeObserver
pub trait TargetEdgeObserver {
    /// Called when an edge is discovered. May be called more than once for the same (from, to)
    /// pair. `attribute` is set iff `from` is a rule.
    fn edge(&self, from: &Target, attribute: Option<&Attribute>, to: &Target);

    /// Called when a Target has a reference to a non-existent target. `target` may be `None`
    /// (e.g. in the case of an implicit dependency on a subincluded file).
    fn missing_edge(&self, target: Option<&Target>, to: &Label, e: &NoSuchThingException);

    /// Called when a node is discovered. May be called more than once for the same node.
    fn node(&self, node: &Target);
}
