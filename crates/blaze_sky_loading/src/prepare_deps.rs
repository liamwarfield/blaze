//! The universe of SkyQuery: PREPARE_DEPS_OF_PATTERNS, PREPARE_DEPS_OF_PATTERN,
//! PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY, COLLECT_TARGETS_IN_PACKAGE and
//! COLLECT_PACKAGES_UNDER_DIRECTORY. After a successful evaluation of a
//! PREPARE_DEPS_OF_PATTERNS key the graph holds a TRANSITIVE_TRAVERSAL node for every target
//! matching the patterns and for their transitive dependencies.
//!
//! Java: com/google/devtools/build/lib/skyframe/{PrepareDepsOfPatternsFunction,
//! PrepareDepsOfPatternsValue,PrepareDepsOfPatternFunction,PrepareDepsOfPatternValue,
//! PrepareDepsOfTargetsUnderDirectoryFunction,PrepareDepsOfTargetsUnderDirectoryValue,
//! CollectTargetsInPackageFunction,CollectTargetsInPackageValue,
//! CollectPackagesUnderDirectoryFunction,CollectPackagesUnderDirectoryValue,
//! RecursiveDirectoryTraversalFunction}.java and
//! com/google/devtools/build/skyframe/GraphTraversingHelper.java.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryName, SignedTargetPattern, TargetParsingException,
    TargetPattern, TargetPatternParser, TargetPatternType, TargetPatternsCode,
};
use blaze_events::{Event, ExtendedEventHandler, NullEventHandler};
use blaze_packages::exceptions::{
    DetailedExitCode, NoSuchPackageException, NoSuchTargetException, PackageLoadingCode,
};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyError, SkyFunctionException,
    SkyFunctionName, SkyKey,
};
use blaze_vfs::{PathFragment, Root, RootedPath};
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::directories::LoadingDirectories;
use crate::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use crate::io::IOException;
use crate::package_function::PackageKey;
use crate::pkgcache::PathPackageLocator;
use crate::recursive_pkg::{
    PackageErrorMessageKey, PackageErrorMessageValue, ProcessPackageDirectory,
    ProcessPackageDirectoryException, RecursivePkgKey,
};
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue, RepositoryMappingKey};
use crate::target_pattern_eval::{self, TargetPatternEvalError, TargetPatternResolver};
use crate::target_pattern_function::{
    EnvironmentBackedRecursivePackageProvider, FilteringPolicy, ProviderError,
    RecursivePackageProvider, TargetPatternKey,
    combine_targets_below_directory_with_negative_patterns, get_parsing_error_message,
    get_path_fragment, resolve_package_targets, target_parsing_exception_of,
};
use crate::transitive_traversal::TransitiveTraversalKey;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:72 SkyFunctions.PREPARE_DEPS_OF_PATTERNS
pub const PREPARE_DEPS_OF_PATTERNS: SkyFunctionName = SkyFunctionName("PREPARE_DEPS_OF_PATTERNS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:75 SkyFunctions.PREPARE_DEPS_OF_PATTERN
pub const PREPARE_DEPS_OF_PATTERN: SkyFunctionName = SkyFunctionName("PREPARE_DEPS_OF_PATTERN");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:77 SkyFunctions.PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY
pub const PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY: SkyFunctionName =
    SkyFunctionName("PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:79 SkyFunctions.COLLECT_TARGETS_IN_PACKAGE
pub const COLLECT_TARGETS_IN_PACKAGE: SkyFunctionName =
    SkyFunctionName("COLLECT_TARGETS_IN_PACKAGE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:82 SkyFunctions.COLLECT_PACKAGES_UNDER_DIRECTORY
pub const COLLECT_PACKAGES_UNDER_DIRECTORY: SkyFunctionName =
    SkyFunctionName("COLLECT_PACKAGES_UNDER_DIRECTORY");

/// Java `GraphTraversingHelper.declareDependenciesAndCheckIfValuesMissing`: requests `keys` and
/// returns whether a value is missing; a dependency whose exception `allowed` accepts counts as
/// present.
// Java: com/google/devtools/build/skyframe/GraphTraversingHelper.java:37 GraphTraversingHelper.declareDependenciesAndCheckIfValuesMissing
fn declare_dependencies_and_check_if_values_missing(
    env: &mut Environment<'_>,
    keys: &[SkyKey],
    allowed: &dyn Fn(&SkyError) -> bool,
) -> Result<bool, blaze_skyframe::Interrupted> {
    let result = env.get_values_and_exceptions(keys)?;
    if env.values_missing() {
        return Ok(true);
    }
    for key in keys {
        match result.get_or_throw_dyn_with(key, |e| allowed(e).then_some(())) {
            Err(()) => continue,
            Ok(Some(_)) => {}
            Ok(None) => {
                // Java: bugReporter.logUnexpected("Value for: '%s' was missing, this should
                // never happen", key).
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn is_no_such_package_or_target(e: &SkyError) -> bool {
    e.downcast_ref::<NoSuchPackageException>().is_some()
        || e.downcast_ref::<NoSuchTargetException>().is_some()
}

fn is_no_such_package_or_process_package_directory(e: &SkyError) -> bool {
    e.downcast_ref::<NoSuchPackageException>().is_some()
        || e.downcast_ref::<ProcessPackageDirectoryException>()
            .is_some()
}

// ---------------------------------------------------------------------------------------------
// COLLECT_PACKAGES_UNDER_DIRECTORY

/// The key of COLLECT_PACKAGES_UNDER_DIRECTORY: a `RecursivePkgSkyKey` with that function name.
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:236 CollectPackagesUnderDirectoryValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CollectPackagesUnderDirectoryKey(pub RecursivePkgKey);

impl CollectPackagesUnderDirectoryKey {
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:229 CollectPackagesUnderDirectoryValue.key
    pub fn key(
        repository: RepositoryName,
        rooted_path: RootedPath,
        excluded_paths: IgnoredSubdirectories,
    ) -> Self {
        CollectPackagesUnderDirectoryKey(RecursivePkgKey::key(
            repository,
            rooted_path,
            excluded_paths,
        ))
    }
}

// Java: com/google/devtools/build/lib/skyframe/RecursivePkgKey.java:63 RecursivePkgKey.toString
// Java: com/google/devtools/build/lib/skyframe/RecursivePkgSkyKey.java:29 RecursivePkgSkyKey.toString
impl fmt::Debug for CollectPackagesUnderDirectoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}",
            COLLECT_PACKAGES_UNDER_DIRECTORY,
            self.0.to_key_string()
        )
    }
}

impl Key for CollectPackagesUnderDirectoryKey {
    type Value = CollectPackagesUnderDirectoryValue;
    const FUNCTION: SkyFunctionName = COLLECT_PACKAGES_UNDER_DIRECTORY;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:82 SkyFunctions.COLLECT_PACKAGES_UNDER_DIRECTORY (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:100 CollectPackagesUnderDirectoryValue.NoErrorCollectPackagesUnderDirectoryValue.equals
    fn values_equal(
        old: &CollectPackagesUnderDirectoryValue,
        new: &CollectPackagesUnderDirectoryValue,
    ) -> bool {
        old == new
    }
}

/// Whether a directory is a package (or failed to be one), and whether each of its
/// subdirectories transitively contains a package or an error.
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:54 CollectPackagesUnderDirectoryValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectPackagesUnderDirectoryValue {
    is_directory_package: bool,
    /// `Some` for `ErrorCollectPackagesUnderDirectoryValue` (which is never a package).
    error_message: Option<String>,
    /// Java's `ImmutableMap<RootedPath, Boolean>`, in insertion order.
    subdirectory_transitively_contains_packages_or_errors: IndexMap<RootedPath, bool>,
}

impl CollectPackagesUnderDirectoryValue {
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:181 CollectPackagesUnderDirectoryValue.ofError
    pub fn of_error(error_message: String, subdirectories: IndexMap<RootedPath, bool>) -> Self {
        CollectPackagesUnderDirectoryValue {
            is_directory_package: false,
            error_message: Some(error_message),
            subdirectory_transitively_contains_packages_or_errors: subdirectories,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:193 CollectPackagesUnderDirectoryValue.ofNoError
    pub fn of_no_error(
        is_directory_package: bool,
        subdirectories: IndexMap<RootedPath, bool>,
    ) -> Self {
        CollectPackagesUnderDirectoryValue {
            is_directory_package,
            error_message: None,
            subdirectory_transitively_contains_packages_or_errors: subdirectories,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:83 CollectPackagesUnderDirectoryValue.isDirectoryPackage
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:137 CollectPackagesUnderDirectoryValue.ErrorCollectPackagesUnderDirectoryValue.isDirectoryPackage
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:82 CollectPackagesUnderDirectoryValue.NoErrorCollectPackagesUnderDirectoryValue.isDirectoryPackage
    pub fn is_directory_package(&self) -> bool {
        self.is_directory_package
    }

    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:89 CollectPackagesUnderDirectoryValue.getErrorMessage
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:142 CollectPackagesUnderDirectoryValue.ErrorCollectPackagesUnderDirectoryValue.getErrorMessage
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:87 CollectPackagesUnderDirectoryValue.NoErrorCollectPackagesUnderDirectoryValue.getErrorMessage
    pub fn get_error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryValue.java:222 CollectPackagesUnderDirectoryValue.getSubdirectoryTransitivelyContainsPackagesOrErrors
    pub fn get_subdirectory_transitively_contains_packages_or_errors(
        &self,
    ) -> &IndexMap<RootedPath, bool> {
        &self.subdirectory_transitively_contains_packages_or_errors
    }
}

/// What a directory's own package contributes (Java `PackageDirectoryConsumer`).
// Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:105 RecursiveDirectoryTraversalFunction.PackageDirectoryConsumer
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:110 CollectPackagesUnderDirectoryFunction.MyPackageDirectoryConsumer.<init>
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:112 CollectPackagesUnderDirectoryFunction.MyPackageDirectoryConsumer.notePackage
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:117 CollectPackagesUnderDirectoryFunction.MyPackageDirectoryConsumer.notePackageError
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:122 CollectPackagesUnderDirectoryFunction.MyPackageDirectoryConsumer.isDirectoryPackage
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:126 CollectPackagesUnderDirectoryFunction.MyPackageDirectoryConsumer.getErrorMessage
enum OwnPackage {
    None,
    /// `notePackage`.
    Package,
    /// `notePackageError`.
    Error(String),
}

/// Java `RecursiveDirectoryTraversalFunction.visitDirectory` up to the aggregation: the
/// directory's own package and the values of the subdirectory keys `to_key` makes from the
/// child `RecursivePkgKey`s. `Ok(None)` when a value is missing.
// Java: com/google/devtools/build/lib/skyframe/RecursiveDirectoryTraversalFunction.java:137 RecursiveDirectoryTraversalFunction.visitDirectory
#[allow(clippy::type_complexity)]
fn visit_directory<K: Key + Clone>(
    directories: &LoadingDirectories,
    recursive_pkg_key: &RecursivePkgKey,
    to_key: impl Fn(&RecursivePkgKey) -> K,
    env: &mut Environment<'_>,
) -> Result<Option<(OwnPackage, Vec<(RecursivePkgKey, Arc<K::Value>)>)>, ComputeError> {
    let process_package_directory_result = ProcessPackageDirectory { directories }
        .get_package_existence_and_subdir_deps(
            recursive_pkg_key.get_rooted_path(),
            recursive_pkg_key.get_repository_name(),
            recursive_pkg_key.get_excluded_paths(),
            env,
        )?;
    if env.values_missing() {
        return Ok(None);
    }
    let Some(process_package_directory_result) = process_package_directory_result else {
        return Ok(None);
    };
    let child_deps: Vec<(RecursivePkgKey, K)> = process_package_directory_result
        .child_deps
        .iter()
        .map(|k| (k.clone(), to_key(k)))
        .collect();
    let child_sky_keys: Vec<SkyKey> = child_deps
        .iter()
        .map(|(_, k)| SkyKey::new(k.clone()))
        .collect();
    let mut consumer = OwnPackage::None;
    let dependent_sky_values;
    if process_package_directory_result.package_exists {
        let root_relative_path = recursive_pkg_key
            .get_rooted_path()
            .get_root_relative_path()
            .clone();
        let package_error_message_key = PackageErrorMessageKey(PackageIdentifier::create(
            recursive_pkg_key.get_repository_name().clone(),
            root_relative_path.clone(),
        ));
        // In a no-keep-going build during error bubbling, PackageErrorMessageFunction may throw a
        // NoSuchPackageException. Since we don't catch such an exception here, this SkyFunction
        // will return immediately with a missing value, and the NoSuchPackageException will
        // propagate up.
        let mut keys = vec![SkyKey::new(package_error_message_key.clone())];
        keys.extend(child_sky_keys.iter().cloned());
        dependent_sky_values = env.get_values_and_exceptions(&keys)?;
        if env.values_missing() {
            return Ok(None);
        }
        let Some(pkg_error_message_value) =
            dependent_sky_values.get_typed(&package_error_message_key)
        else {
            return Ok(None);
        };
        match &*pkg_error_message_value {
            PackageErrorMessageValue::NoError => consumer = OwnPackage::Package,
            PackageErrorMessageValue::Error => {
                env.listener().handle(&Event::error(format!(
                    "package contains errors: {}",
                    root_relative_path.get_path_string()
                )));
                consumer = OwnPackage::Package;
            }
            PackageErrorMessageValue::NoSuchPackageException(msg) => {
                // The package had errors, but don't fail-fast as there might be subpackages
                // below the current directory.
                env.listener().handle(&Event::error(msg.clone()));
                consumer = OwnPackage::Error(msg.clone());
            }
        }
    } else {
        dependent_sky_values = env.get_values_and_exceptions(&child_sky_keys)?;
        if env.values_missing() {
            return Ok(None);
        }
    }
    let mut subdirectory_sky_values_from_deps = Vec::with_capacity(child_deps.len());
    for (recursive_key, key) in child_deps {
        let Some(sky_value) = dependent_sky_values.get_typed(&key) else {
            return Ok(None);
        };
        subdirectory_sky_values_from_deps.push((recursive_key, sky_value));
    }
    // PORT: ProcessPackageDirectoryResult.getAdditionalValuesToAggregate is always empty in
    // Bazel's ProcessPackageDirectory.
    Ok(Some((consumer, subdirectory_sky_values_from_deps)))
}

/// COLLECT_PACKAGES_UNDER_DIRECTORY (Java `CollectPackagesUnderDirectoryFunction`).
// Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:37 CollectPackagesUnderDirectoryFunction
pub struct CollectPackagesUnderDirectoryFunction {
    directories: LoadingDirectories,
}

impl CollectPackagesUnderDirectoryFunction {
    pub fn new(directories: LoadingDirectories) -> Self {
        CollectPackagesUnderDirectoryFunction { directories }
    }

    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:45 CollectPackagesUnderDirectoryFunction.compute
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:55 CollectPackagesUnderDirectoryFunction.MyTraversalFunction.<init>
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:59 CollectPackagesUnderDirectoryFunction.MyTraversalFunction.getInitialConsumer
    // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:64 CollectPackagesUnderDirectoryFunction.MyTraversalFunction.getSkyKeyForSubdirectory
    pub fn compute(
        &self,
        key: &CollectPackagesUnderDirectoryKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<CollectPackagesUnderDirectoryValue>, ComputeError> {
        let Some((consumer, subdirectory_sky_values)) = visit_directory(
            &self.directories,
            &key.0,
            |k| CollectPackagesUnderDirectoryKey(k.clone()),
            env,
        )?
        else {
            return Ok(None);
        };
        // Java MyTraversalFunction.aggregateWithSubdirectorySkyValues.
        // Java: com/google/devtools/build/lib/skyframe/CollectPackagesUnderDirectoryFunction.java:73 CollectPackagesUnderDirectoryFunction.MyTraversalFunction.aggregateWithSubdirectorySkyValues
        // Aggregate the child subdirectory package state.
        let mut subdirectories: IndexMap<RootedPath, bool> = IndexMap::new();
        for (recursive_pkg_key, collect_packages_value) in subdirectory_sky_values {
            let packages_or_errors_in_subdirectory = collect_packages_value.is_directory_package()
                || collect_packages_value.get_error_message().is_some()
                || collect_packages_value
                    .get_subdirectory_transitively_contains_packages_or_errors()
                    .values()
                    .any(|b| *b);
            subdirectories.insert(
                recursive_pkg_key.get_rooted_path().clone(),
                packages_or_errors_in_subdirectory,
            );
        }
        Ok(Some(match consumer {
            OwnPackage::Error(error_message) => {
                CollectPackagesUnderDirectoryValue::of_error(error_message, subdirectories)
            }
            OwnPackage::Package => {
                CollectPackagesUnderDirectoryValue::of_no_error(true, subdirectories)
            }
            OwnPackage::None => {
                CollectPackagesUnderDirectoryValue::of_no_error(false, subdirectories)
            }
        }))
    }
}

// ---------------------------------------------------------------------------------------------
// COLLECT_TARGETS_IN_PACKAGE

/// The key of COLLECT_TARGETS_IN_PACKAGE.
// Java: com/google/devtools/build/lib/skyframe/CollectTargetsInPackageValue.java:45 CollectTargetsInPackageValue.CollectTargetsInPackageKey
// PORT: skipped, the key is a plain struct (derived equality and hashing, no interner) and its function name is `Key::FUNCTION`: CollectTargetsInPackageValue.CollectTargetsInPackageKey.<init>, CollectTargetsInPackageValue.CollectTargetsInPackageKey.create, CollectTargetsInPackageValue.CollectTargetsInPackageKey.functionName, CollectTargetsInPackageValue.CollectTargetsInPackageKey.getSkyKeyInterner
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CollectTargetsInPackageKey {
    package_id: PackageIdentifier,
    filtering_policy: FilteringPolicy,
}

impl CollectTargetsInPackageKey {
    // Java: com/google/devtools/build/lib/skyframe/CollectTargetsInPackageValue.java:38 CollectTargetsInPackageValue.key
    pub fn key(package_id: PackageIdentifier, filtering_policy: FilteringPolicy) -> Self {
        CollectTargetsInPackageKey {
            package_id,
            filtering_policy,
        }
    }
}

// Java: AutoValue toString of CollectTargetsInPackageKey.
impl fmt::Debug for CollectTargetsInPackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CollectTargetsInPackageKey{{packageId={}, filteringPolicy={:?}}}",
            self.package_id, self.filtering_policy
        )
    }
}

impl Key for CollectTargetsInPackageKey {
    type Value = CollectTargetsInPackageValue;
    const FUNCTION: SkyFunctionName = COLLECT_TARGETS_IN_PACKAGE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:79 SkyFunctions.COLLECT_TARGETS_IN_PACKAGE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: the INSTANCE singleton (no `equals`; every value is the same object).
    fn values_equal(
        _old: &CollectTargetsInPackageValue,
        _new: &CollectTargetsInPackageValue,
    ) -> bool {
        true
    }
}

/// A marker value: the package's targets (and their transitive dependencies) are in the graph.
// Java: com/google/devtools/build/lib/skyframe/CollectTargetsInPackageValue.java:28 CollectTargetsInPackageValue
// PORT: skipped, a unit struct stands for the INSTANCE singleton: CollectTargetsInPackageValue.<init>
#[derive(Debug)]
pub struct CollectTargetsInPackageValue;

/// COLLECT_TARGETS_IN_PACKAGE (Java `CollectTargetsInPackageFunction`).
// Java: com/google/devtools/build/lib/skyframe/CollectTargetsInPackageFunction.java:39 CollectTargetsInPackageFunction.compute
pub fn collect_targets_in_package_function(
    argument: &CollectTargetsInPackageKey,
    env: &mut Environment<'_>,
) -> Result<Option<CollectTargetsInPackageValue>, ComputeError> {
    let package_id = &argument.package_id;
    let package_value = match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(
        PackageKey::key(package_id.clone()),
    )? {
        // If the argument is a package that doesn't exist, the aggregator function can return a
        // success value immediately.
        Err(_) => return Ok(Some(CollectTargetsInPackageValue)),
        Ok(v) => v,
    };
    if env.values_missing() {
        return Ok(None);
    }
    let Some(package_value) = package_value else {
        return Ok(None);
    };
    let pkg = package_value.get_package();
    if pkg.contains_errors() {
        env.listener().handle(&Event::error(format!(
            "package contains errors: {}",
            package_id.get_package_fragment().get_path_string()
        )));
    }
    let keys: Vec<SkyKey> = resolve_package_targets(pkg, &argument.filtering_policy)
        .iter()
        .map(|target| SkyKey::new(TransitiveTraversalKey::key(target.get_label().clone())))
        .collect();
    Ok(
        if declare_dependencies_and_check_if_values_missing(env, &keys, &|_| false)? {
            None
        } else {
            Some(CollectTargetsInPackageValue)
        },
    )
}

// ---------------------------------------------------------------------------------------------
// PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY

/// The key of PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY: a directory and a filtering policy.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryValue.java:76 PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey
// PORT: skipped, the key is a plain struct (derived equality and hashing, no interner; the fields are read directly) and its function name is `Key::FUNCTION`: PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.<init>, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.create, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.intern, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.getRecursivePkgKey, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.getFilteringPolicy, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.functionName, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.equals, PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.hashCode
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrepareDepsOfTargetsUnderDirectoryKey {
    recursive_pkg_key: RecursivePkgKey,
    filtering_policy: FilteringPolicy,
}

impl PrepareDepsOfTargetsUnderDirectoryKey {
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryValue.java:63 PrepareDepsOfTargetsUnderDirectoryValue.key
    pub fn key(
        repository: RepositoryName,
        rooted_path: RootedPath,
        excluded_paths: IgnoredSubdirectories,
        filtering_policy: FilteringPolicy,
    ) -> Self {
        PrepareDepsOfTargetsUnderDirectoryKey {
            recursive_pkg_key: RecursivePkgKey::key(repository, rooted_path, excluded_paths),
            filtering_policy,
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryValue.java:133 PrepareDepsOfTargetsUnderDirectoryValue.PrepareDepsOfTargetsUnderDirectoryKey.toString
impl fmt::Debug for PrepareDepsOfTargetsUnderDirectoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PrepareDepsOfTargetsUnderDirectoryKey{{pkg-key={}, filtering policy={:?}}}",
            self.recursive_pkg_key.to_key_string(),
            self.filtering_policy
        )
    }
}

impl Key for PrepareDepsOfTargetsUnderDirectoryKey {
    type Value = PrepareDepsOfTargetsUnderDirectoryValue;
    const FUNCTION: SkyFunctionName = PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:77 SkyFunctions.PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: the INSTANCE singleton.
    fn values_equal(
        _old: &PrepareDepsOfTargetsUnderDirectoryValue,
        _new: &PrepareDepsOfTargetsUnderDirectoryValue,
    ) -> bool {
        true
    }
}

/// A marker value: the targets in the directory's package and in the non-excluded packages
/// beneath it, and their transitive dependencies, are in the graph.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryValue.java:44 PrepareDepsOfTargetsUnderDirectoryValue
// PORT: skipped, a unit struct stands for the INSTANCE singleton: PrepareDepsOfTargetsUnderDirectoryValue.<init>
#[derive(Debug)]
pub struct PrepareDepsOfTargetsUnderDirectoryValue;

/// PREPARE_DEPS_OF_TARGETS_UNDER_DIRECTORY (Java `PrepareDepsOfTargetsUnderDirectoryFunction`).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryFunction.java:36 PrepareDepsOfTargetsUnderDirectoryFunction
pub struct PrepareDepsOfTargetsUnderDirectoryFunction {
    directories: LoadingDirectories,
}

impl PrepareDepsOfTargetsUnderDirectoryFunction {
    pub fn new(directories: LoadingDirectories) -> Self {
        PrepareDepsOfTargetsUnderDirectoryFunction { directories }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfTargetsUnderDirectoryFunction.java:45 PrepareDepsOfTargetsUnderDirectoryFunction.compute
    pub fn compute(
        &self,
        argument: &PrepareDepsOfTargetsUnderDirectoryKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PrepareDepsOfTargetsUnderDirectoryValue>, ComputeError> {
        let filtering_policy = &argument.filtering_policy;
        let recursive_pkg_key = &argument.recursive_pkg_key;
        let package_existence_and_subdir_deps = ProcessPackageDirectory {
            directories: &self.directories,
        }
        .get_package_existence_and_subdir_deps(
            recursive_pkg_key.get_rooted_path(),
            recursive_pkg_key.get_repository_name(),
            recursive_pkg_key.get_excluded_paths(),
            env,
        )?;
        if env.values_missing() {
            return Ok(None);
        }
        let Some(package_existence_and_subdir_deps) = package_existence_and_subdir_deps else {
            return Ok(None);
        };
        let mut keys_to_request: Vec<SkyKey> = Vec::new();
        if package_existence_and_subdir_deps.package_exists {
            keys_to_request.push(SkyKey::new(CollectTargetsInPackageKey::key(
                PackageIdentifier::create(
                    recursive_pkg_key.get_repository_name().clone(),
                    recursive_pkg_key
                        .get_rooted_path()
                        .get_root_relative_path()
                        .clone(),
                ),
                filtering_policy.clone(),
            )));
        }
        // Java's key transformer: PrepareDepsOfTargetsUnderDirectoryValue.key(repository,
        // subdirectory, excludedSubdirectoriesBeneathSubdirectory, filteringPolicy).
        keys_to_request.extend(
            package_existence_and_subdir_deps
                .child_deps
                .iter()
                .map(|k| {
                    SkyKey::new(PrepareDepsOfTargetsUnderDirectoryKey {
                        recursive_pkg_key: k.clone(),
                        filtering_policy: filtering_policy.clone(),
                    })
                }),
        );
        Ok(
            if declare_dependencies_and_check_if_values_missing(
                env,
                &keys_to_request,
                &is_no_such_package_or_process_package_directory,
            )? {
                None
            } else {
                Some(PrepareDepsOfTargetsUnderDirectoryValue)
            },
        )
    }
}

// ---------------------------------------------------------------------------------------------
// PREPARE_DEPS_OF_PATTERN

/// The key of PREPARE_DEPS_OF_PATTERN: a [`TargetPatternKey`] (always with the `NO_FILTER`
/// policy).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:194 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.Key
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:185 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.<init>
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:189 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.getSkyKey
// PORT: skipped, the key is a newtype key (Key::FUNCTION; no interning): PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.Key.create, PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.Key.functionName, PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyValue.Key.getSkyKeyInterner
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrepareDepsOfPatternKey(pub TargetPatternKey);

// Java: AbstractSkyKey.toString: "<function>:<argument>".
impl fmt::Debug for PrepareDepsOfPatternKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:?}", PREPARE_DEPS_OF_PATTERN, self.0)
    }
}

impl Key for PrepareDepsOfPatternKey {
    type Value = PrepareDepsOfPatternValue;
    const FUNCTION: SkyFunctionName = PREPARE_DEPS_OF_PATTERN;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:75 SkyFunctions.PREPARE_DEPS_OF_PATTERN (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:55 PrepareDepsOfPatternValue.equals
    // PORT: skipped, a marker value is never hashed (all values are equal): PrepareDepsOfPatternValue.hashCode
    fn values_equal(_old: &PrepareDepsOfPatternValue, _new: &PrepareDepsOfPatternValue) -> bool {
        true
    }
}

/// A marker value: the targets matching the pattern and their transitive dependencies are in the
/// graph.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:46 PrepareDepsOfPatternValue
#[derive(Debug)]
pub struct PrepareDepsOfPatternValue;

/// A pattern that could not be turned into a key, with the pattern as written (Java
/// `PrepareDepsOfPatternSkyKeyException`).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:158 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyException
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:169 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyException.getException
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:173 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeyException.getOriginalPattern
pub struct PrepareDepsOfPatternSkyKeyException {
    pub exception: TargetParsingException,
    pub original_pattern: String,
}

/// The keys of a pattern sequence and the patterns that failed (Java
/// `PrepareDepsOfPatternSkyKeysAndExceptions`).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:137 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeysAndExceptions
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:148 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeysAndExceptions.getValues
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:152 PrepareDepsOfPatternValue.PrepareDepsOfPatternSkyKeysAndExceptions.getExceptions
pub struct PrepareDepsOfPatternSkyKeysAndExceptions {
    pub values: Vec<PrepareDepsOfPatternKey>,
    pub exceptions: Vec<PrepareDepsOfPatternSkyKeyException>,
}

/// Returns the keys of `patterns`, each excluding the directories of the later negative
/// targets-below-directory patterns it contains, and the patterns that are not allowed.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternValue.java:88 PrepareDepsOfPatternValue.keys
pub fn prepare_deps_of_pattern_keys(
    patterns: &[String],
    main_repo_target_parser: &TargetPatternParser,
) -> PrepareDepsOfPatternSkyKeysAndExceptions {
    let mut result_values = Vec::new();
    let mut result_exceptions = Vec::new();
    let mut target_pattern_keys = Vec::new();
    for pattern in patterns {
        match SignedTargetPattern::parse(pattern, main_repo_target_parser) {
            Ok(p) => target_pattern_keys.push(TargetPatternKey::key(p, FilteringPolicy::NoFilter)),
            Err(e) => result_exceptions.push(PrepareDepsOfPatternSkyKeyException {
                exception: e,
                original_pattern: pattern.clone(),
            }),
        }
    }
    // This code path is evaluated only for query universe preloading, and the quadratic cost of
    // the code below (i.e. for each pattern, consider each later pattern as a candidate for
    // subdirectory exclusion) is only acceptable because all the use cases for query universe
    // preloading involve short (<10 items) pattern sequences.
    let combined_target_pattern_keys = combine_targets_below_directory_with_negative_patterns(
        &target_pattern_keys,
        /* exclude_single_targets= */ false,
    );
    for target_pattern_key in combined_target_pattern_keys {
        if target_pattern_key.is_negative()
            && target_pattern_key.get_parsed_pattern().get_type()
                != TargetPatternType::TargetsBelowDirectory
        {
            result_exceptions.push(PrepareDepsOfPatternSkyKeyException {
                exception: TargetParsingException::new(
                    "Negative target patterns of types other than \"targets below directory\" are \
                     not permitted.",
                    TargetPatternsCode::NegativeTargetPatternNotAllowed,
                ),
                original_pattern: format!("{:?}", target_pattern_key),
            });
        } else {
            result_values.push(PrepareDepsOfPatternKey(target_pattern_key));
        }
    }
    PrepareDepsOfPatternSkyKeysAndExceptions {
        values: result_values,
        exceptions: result_exceptions,
    }
}

/// What PREPARE_DEPS_OF_PATTERN throws for a filesystem problem: the pattern and the cause.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:159 PrepareDepsOfPatternFunction.PrepareDepsOfPatternException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct PrepareDepsOfPatternException {
    message: String,
}

impl PrepareDepsOfPatternException {
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:163 PrepareDepsOfPatternFunction.PrepareDepsOfPatternException.<init>
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:173 PrepareDepsOfPatternFunction.PrepareDepsOfPatternException.<init>
    fn new(pattern: &TargetPattern, cause_message: &str) -> Self {
        PrepareDepsOfPatternException {
            message: format!(
                "Preparing deps of pattern '{}' failed: {}",
                pattern.get_original_pattern(),
                cause_message
            ),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java: the cause's code for a `ProcessPackageDirectoryException` and
    /// `TRANSIENT_INCONSISTENT_FILESYSTEM_ERROR` for an `InconsistentFilesystemException`; both
    /// are that code. PORT: Java's `ProcessPackageDirectoryException` detail carries the cause's
    /// own message; this one carries the wrapping message.
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:191 PrepareDepsOfPatternFunction.PrepareDepsOfPatternException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        DetailedExitCode::package_loading(
            self.message.clone(),
            PackageLoadingCode::TransientInconsistentFilesystemError,
        )
    }
}

/// PREPARE_DEPS_OF_PATTERN (Java `PrepareDepsOfPatternFunction`).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:70 PrepareDepsOfPatternFunction
pub struct PrepareDepsOfPatternFunction {
    pkg_path: Arc<RwLock<PathPackageLocator>>,
}

impl PrepareDepsOfPatternFunction {
    pub fn new(pkg_path: Arc<RwLock<PathPackageLocator>>) -> Self {
        PrepareDepsOfPatternFunction { pkg_path }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:79 PrepareDepsOfPatternFunction.compute
    pub fn compute(
        &self,
        key: &PrepareDepsOfPatternKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PrepareDepsOfPatternValue>, ComputeError> {
        let pattern_key = &key.0;
        // DepsOfPatternPreparer below expects to be able to ignore the filtering policy from the
        // TargetPatternKey, which should be valid because PrepareDepsOfPatternValue.keys
        // unconditionally creates TargetPatternKeys with the NO_FILTER filtering policy.
        blaze_util::check_state!(
            *pattern_key.get_policy() == FilteringPolicy::NoFilter,
            "{:?}",
            pattern_key.get_policy()
        );
        let parsed_pattern = pattern_key.get_parsed_pattern();

        let Some(repository_ignored_prefixes) = env.get_value(IgnoredSubdirectoriesKey::key(
            parsed_pattern.get_repository(),
        ))?
        else {
            return Ok(None);
        };
        // This SkyFunction is used to load the universe, so we want both the ignored directories
        // from the global list of exclusions (set with .bazelignore in Bazel) and the excluded
        // directories from the TargetPatternKey itself to be embedded in the SkyKeys created and
        // used by the DepsOfPatternPreparer. The DepsOfPatternPreparer ignores
        // excludedSubdirectories and embeds repositoryIgnoredPatterns in SkyKeys it creates and
        // uses.
        let excluded_patterns: indexmap::IndexSet<PathFragment> = pattern_key
            .get_excluded_subdirectories()
            .iter()
            .cloned()
            .collect();
        let repository_ignored_patterns = repository_ignored_prefixes
            .as_ignored_subdirectories()
            .union(&IgnoredSubdirectories::of(excluded_patterns, Vec::new()));

        let pkg_roots: Vec<Root> = self.pkg_path.read().get_path_entries().to_vec();
        let preparer = DepsOfPatternPreparer {
            package_provider: EnvironmentBackedRecursivePackageProvider::new(env),
            pkg_roots,
        };
        let result = target_pattern_eval::eval(
            parsed_pattern,
            &preparer,
            &|| Ok(repository_ignored_patterns.clone()),
            &[],
            &mut |_| {},
        );
        match result {
            Ok(()) => Ok(Some(PrepareDepsOfPatternValue)),
            Err(TargetPatternEvalError::TargetParsing(e)) => {
                Err(SkyFunctionException::persistent(e).into())
            }
            // The DepsOfPatternPreparer constructed above might throw MissingDepException to
            // signal when it has a dependency on a missing Environment value.
            Err(TargetPatternEvalError::MissingDep) => Ok(None),
            Err(TargetPatternEvalError::Interrupted) => Err(blaze_skyframe::Interrupted.into()),
            Err(TargetPatternEvalError::ProcessPackageDirectory(message)) => Err(
                SkyFunctionException::persistent(PrepareDepsOfPatternException::new(
                    parsed_pattern,
                    &message,
                ))
                .into(),
            ),
            Err(TargetPatternEvalError::InconsistentFilesystem(e)) => Err(
                SkyFunctionException::persistent(PrepareDepsOfPatternException::new(
                    parsed_pattern,
                    e.get_message(),
                ))
                .into(),
            ),
        }
    }
}

/// A [`TargetPatternResolver`] whose only job is to request the TRANSITIVE_TRAVERSAL and
/// directory keys of a pattern (Java `DepsOfPatternPreparer`); it returns no targets.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:202 PrepareDepsOfPatternFunction.DepsOfPatternPreparer
struct DepsOfPatternPreparer<'e, 'x> {
    // Because PrepareDepsOfPatternFunction's only goal is to ensure the proper Skyframe nodes and
    // edges are in the graph, we don't need to worry about
    // EnvironmentBackedRecursivePackageProvider#encounteredPackageErrors.
    package_provider: EnvironmentBackedRecursivePackageProvider<'e, 'x>,
    pkg_roots: Vec<Root>,
}

impl DepsOfPatternPreparer<'_, '_> {
    fn provider_error(
        e: ProviderError,
        message: impl FnOnce(&str) -> String,
    ) -> TargetPatternEvalError {
        match e {
            ProviderError::Eval(e) => e,
            ProviderError::NoSuchThing(e) => TargetPatternEvalError::TargetParsing(
                target_parsing_exception_of(message(&e.get_message()), &e.get_detailed_exit_code()),
            ),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:251 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getTargetsInPackage
    fn get_targets_in_package_with_policy(
        &self,
        original_pattern: &str,
        package_identifier: &PackageIdentifier,
        policy: &FilteringPolicy,
    ) -> Result<Vec<()>, TargetPatternEvalError> {
        // EnvironmentBackedRecursivePackageProvider ignores the handler (Java reads the
        // environment's listener).
        let pkg = self
            .package_provider
            .get_package(NullEventHandler::INSTANCE, package_identifier)
            .map_err(|e| {
                Self::provider_error(e, |_| {
                    get_parsing_error_message("package contains errors", Some(original_pattern))
                })
            })?;
        let sky_keys: Vec<SkyKey> = resolve_package_targets(&pkg, policy)
            .iter()
            .map(|target| SkyKey::new(TransitiveTraversalKey::key(target.get_label().clone())))
            .collect();
        if self.package_provider.with_env(|env| {
            declare_dependencies_and_check_if_values_missing(
                env,
                &sky_keys,
                &is_no_such_package_or_target,
            )
        })? {
            return Err(TargetPatternEvalError::MissingDep);
        }
        Ok(Vec::new())
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:342 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getDeps
    fn get_deps(
        repository: &RepositoryName,
        repository_ignored_subdirectories: &IgnoredSubdirectories,
        policy: &FilteringPolicy,
        rooted_path: RootedPath,
    ) -> Vec<SkyKey> {
        vec![
            SkyKey::new(PrepareDepsOfTargetsUnderDirectoryKey::key(
                repository.clone(),
                rooted_path.clone(),
                repository_ignored_subdirectories.clone(),
                policy.clone(),
            )),
            SkyKey::new(CollectPackagesUnderDirectoryKey::key(
                repository.clone(),
                rooted_path,
                repository_ignored_subdirectories.clone(),
            )),
        ]
    }
}

impl TargetPatternResolver<()> for DepsOfPatternPreparer<'_, '_> {
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:218 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.warn
    fn warn(&self, msg: &str) {
        self.package_provider
            .with_env(|env| env.listener().handle(&Event::warn(msg)));
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:223 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getTargetOrNull
    fn get_target_or_null(&self, _label: &Label) -> Result<Option<()>, TargetPatternEvalError> {
        // This method is used in just one place, TargetPattern.TargetsInPackage
        // #getWildcardConflict. Returning null tells #getWildcardConflict that there is not a
        // target with a name like "all" or "all-targets", which means that
        // TargetPattern.TargetsInPackage will end up calling DepsOfTargetPreparer
        // #getTargetsInPackage.
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:234 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getExplicitTarget
    fn get_explicit_target(&self, label: &Label) -> Result<Vec<()>, TargetPatternEvalError> {
        let target = self
            .package_provider
            .get_target(NullEventHandler::INSTANCE, label)
            .map_err(|e| Self::provider_error(e, |m| m.to_owned()))?;
        let key = TransitiveTraversalKey::key(target.get_label().clone());
        enum Caught {
            NoSuchThing(blaze_packages::exceptions::NoSuchThingException),
        }
        let token = self.package_provider.with_env(|env| {
            env.get_value_or_throw_with(key, |e| {
                if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                    Some(Caught::NoSuchThing(
                        blaze_packages::exceptions::NoSuchThingException::Package(e.clone()),
                    ))
                } else {
                    e.downcast_ref::<NoSuchTargetException>().map(|e| {
                        Caught::NoSuchThing(
                            blaze_packages::exceptions::NoSuchThingException::Target(e.clone()),
                        )
                    })
                }
            })
        })?;
        match token {
            Err(Caught::NoSuchThing(e)) => Err(TargetPatternEvalError::TargetParsing(
                target_parsing_exception_of(e.get_message(), &e.get_detailed_exit_code()),
            )),
            Ok(None) => Err(TargetPatternEvalError::MissingDep),
            Ok(Some(_)) => Ok(Vec::new()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:251 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getTargetsInPackage
    fn get_targets_in_package(
        &self,
        original_pattern: &str,
        package_identifier: &PackageIdentifier,
        rules_only: bool,
    ) -> Result<Vec<()>, TargetPatternEvalError> {
        let policy = if rules_only {
            FilteringPolicy::RulesOnly
        } else {
            FilteringPolicy::NoFilter
        };
        self.get_targets_in_package_with_policy(original_pattern, package_identifier, &policy)
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:285 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.isPackage
    fn is_package(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Result<bool, TargetPatternEvalError> {
        self.package_provider
            .is_package(NullEventHandler::INSTANCE, package_identifier)
            .map_err(|e| Self::provider_error(e, |m| m.to_owned()))
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:291 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.getTargetKind
    fn get_target_kind(&self, _target: &()) -> String {
        // Because DepsOfPatternPreparer#getTargetOrNull always returns null, this method is never
        // called.
        panic!("UnsupportedOperationException")
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternFunction.java:300 PrepareDepsOfPatternFunction.DepsOfPatternPreparer.findTargetsBeneathDirectory
    fn find_targets_beneath_directory(
        &self,
        repository: &RepositoryName,
        _original_pattern: &str,
        directory: &str,
        rules_only: bool,
        repository_ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
        _callback: &mut dyn FnMut(Vec<()>),
    ) -> Result<(), TargetPatternEvalError> {
        let directory_path_fragment = get_path_fragment(directory)?;
        blaze_util::check_argument!(
            excluded_subdirectories.is_empty(),
            "{:?}",
            excluded_subdirectories
        );
        let policy = if rules_only {
            FilteringPolicy::RulesOnly
        } else {
            FilteringPolicy::NoFilter
        };
        let mut roots: Vec<Root> = Vec::new();
        if repository.is_main() {
            roots.extend(self.pkg_roots.iter().cloned());
        } else {
            let repository_value = self
                .package_provider
                .with_env(|env| env.get_value(RepositoryDirectoryKey::key(repository.clone())))?;
            let Some(repository_value) = repository_value else {
                return Err(TargetPatternEvalError::MissingDep);
            };
            match &*repository_value {
                RepositoryDirectoryValue::Failure { error_msg } => {
                    // This shouldn't be possible; we're given a repository, so we assume that
                    // the caller has already checked for its existence.
                    panic!(
                        "IllegalStateException: No such repository '{}': {}",
                        repository, error_msg
                    );
                }
                RepositoryDirectoryValue::Success { root, .. } => roots.push(root.clone()),
            }
        }
        for root in roots {
            let rooted_path = RootedPath::to_rooted_path(root, &directory_path_fragment);
            let keys = Self::get_deps(
                repository,
                repository_ignored_subdirectories,
                &policy,
                rooted_path,
            );
            if self.package_provider.with_env(|env| {
                declare_dependencies_and_check_if_values_missing(env, &keys, &|_| false)
            })? {
                return Err(TargetPatternEvalError::MissingDep);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// PREPARE_DEPS_OF_PATTERNS

/// The key of PREPARE_DEPS_OF_PATTERNS: a sequence of target patterns and the offset they are
/// relative to (Java `TargetPatternSequence`, a `UniverseSkyKey`).
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:68 PrepareDepsOfPatternsValue.TargetPatternSequence
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternSequence {
    patterns: Vec<String>,
    offset: PathFragment,
}

impl TargetPatternSequence {
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:60 PrepareDepsOfPatternsValue.key
    pub fn key(patterns: Vec<String>, offset: PathFragment) -> Self {
        TargetPatternSequence { patterns, offset }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:91 PrepareDepsOfPatternsValue.TargetPatternSequence.getPatterns
    pub fn get_patterns(&self) -> &[String] {
        &self.patterns
    }

    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:95 PrepareDepsOfPatternsValue.TargetPatternSequence.getOffset
    pub fn get_offset(&self) -> &PathFragment {
        &self.offset
    }
}

// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:121 PrepareDepsOfPatternsValue.TargetPatternSequence.toString
impl fmt::Debug for TargetPatternSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TargetPatternSequence{{patterns=[{}], offset={}}}",
            self.patterns.join(", "),
            self.offset
        )
    }
}

impl Key for TargetPatternSequence {
    type Value = PrepareDepsOfPatternsValue;
    const FUNCTION: SkyFunctionName = PREPARE_DEPS_OF_PATTERNS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:72 SkyFunctions.PREPARE_DEPS_OF_PATTERNS (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:105 PrepareDepsOfPatternsValue.equals
    fn values_equal(old: &PrepareDepsOfPatternsValue, new: &PrepareDepsOfPatternsValue) -> bool {
        old.target_pattern_keys == new.target_pattern_keys
    }
}

/// The target pattern keys of a universe whose targets and their transitive dependencies are in
/// the graph.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:48 PrepareDepsOfPatternsValue
#[derive(Debug)]
pub struct PrepareDepsOfPatternsValue {
    target_pattern_keys: Vec<TargetPatternKey>,
}

impl PrepareDepsOfPatternsValue {
    // Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsValue.java:55 PrepareDepsOfPatternsValue.getTargetPatternKeys
    pub fn get_target_pattern_keys(&self) -> &[TargetPatternKey] {
        &self.target_pattern_keys
    }
}

/// Java `ParsingFailedEvent(originalPattern, message)`, posted for a universe pattern that
/// cannot be parsed.
// Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:25 ParsingFailedEvent
#[derive(Debug, Clone)]
pub struct ParsingFailedEvent {
    pub pattern: String,
    pub message: String,
}

impl blaze_events::Postable for ParsingFailedEvent {}

/// The PREPARE_DEPS_OF_PATTERN keys of a universe key; the patterns that cannot be parsed are
/// reported.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunction.java:46 PrepareDepsOfPatternsFunction.getSkyKeys
pub fn get_sky_keys(
    target_pattern_sequence: &TargetPatternSequence,
    event_handler: &dyn ExtendedEventHandler,
    main_repo_mapping: &blaze_cmdline::RepositoryMapping,
) -> Vec<PrepareDepsOfPatternKey> {
    let main_repo_target_parser = TargetPatternParser::new(
        target_pattern_sequence.get_offset().clone(),
        blaze_cmdline::repository_name::MAIN.clone(),
        main_repo_mapping.clone(),
    );
    let sky_keys_and_exceptions = prepare_deps_of_pattern_keys(
        target_pattern_sequence.get_patterns(),
        &main_repo_target_parser,
    );
    for sky_key_exception in &sky_keys_and_exceptions.exceptions {
        let e = &sky_key_exception.exception;
        // We post an event here rather than in handleTargetParsingException because the
        // TargetPatternFunction already posts an event unless the pattern cannot be parsed, in
        // which case the caller (i.e., us) needs to post an event.
        event_handler.post(Arc::new(ParsingFailedEvent {
            pattern: sky_key_exception.original_pattern.clone(),
            message: e.get_message().to_owned(),
        }));
        handle_target_parsing_exception(event_handler, &sky_key_exception.original_pattern, e);
    }
    sky_keys_and_exceptions.values
}

/// Java `getTargetPatternKeys(skyKeys)`.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunction.java:83 PrepareDepsOfPatternsFunction.getTargetPatternKeys
pub fn get_target_pattern_keys(sky_keys: &[PrepareDepsOfPatternKey]) -> Vec<TargetPatternKey> {
    sky_keys.iter().map(|k| k.0.clone()).collect()
}

// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunction.java:138 PrepareDepsOfPatternsFunction.handleTargetParsingException
fn handle_target_parsing_exception(
    event_handler: &dyn ExtendedEventHandler,
    raw_pattern: &str,
    e: &TargetParsingException,
) {
    let error_message = e.get_message();
    event_handler.handle(&Event::error(format!(
        "Skipping '{}': {}",
        raw_pattern, error_message
    )));
}

/// PREPARE_DEPS_OF_PATTERNS (Java `PrepareDepsOfPatternsFunction.compute`): when it returns,
/// all targets matching the sequence, and those targets' transitive dependencies, have been
/// loaded.
// Java: com/google/devtools/build/lib/skyframe/PrepareDepsOfPatternsFunction.java:95 PrepareDepsOfPatternsFunction.compute
pub fn prepare_deps_of_patterns_function(
    sky_key: &TargetPatternSequence,
    env: &mut Environment<'_>,
) -> Result<Option<PrepareDepsOfPatternsValue>, ComputeError> {
    let Some(repository_mapping_value) = env.get_value(RepositoryMappingKey::key(
        blaze_cmdline::repository_name::MAIN.clone(),
    ))?
    else {
        return Ok(None);
    };
    let main_repo_mapping = repository_mapping_value
        .repository_mapping()
        .expect("the main repository's mapping")
        .clone();
    let sky_keys = get_sky_keys(sky_key, env.listener(), &main_repo_mapping);

    let dyn_keys: Vec<SkyKey> = sky_keys.iter().cloned().map(SkyKey::new).collect();
    let tokens_by_key = env.get_values_and_exceptions(&dyn_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    enum Caught {
        TargetParsing(TargetParsingException),
        /// `ProcessPackageDirectoryException` or `InconsistentFilesystemException`.
        Catastrophic,
    }
    for key in &sky_keys {
        match tokens_by_key.get_or_throw_with(key, |e| {
            if let Some(e) = e.downcast_ref::<TargetParsingException>() {
                Some(Caught::TargetParsing(e.clone()))
            } else if e
                .downcast_ref::<ProcessPackageDirectoryException>()
                .is_some()
                || e.downcast_ref::<IOException>()
                    .is_some_and(|e| e.is_inconsistent_filesystem())
            {
                Some(Caught::Catastrophic)
            } else {
                None
            }
        }) {
            Ok(Some(_)) => {}
            Ok(None) => {
                // Java: BugReport.sendNonFatalBugReport("SkyValue ... was missing, this should
                // never happen").
                return Ok(None);
            }
            Err(Caught::TargetParsing(e)) => {
                // If a target pattern can't be evaluated, notify the user of the problem and keep
                // going.
                handle_target_parsing_exception(env.listener(), key.0.get_pattern(), &e);
            }
            Err(Caught::Catastrophic) => {
                // ProcessPackageDirectoryException indicates a catastrophic
                // InconsistentFilesystemException, which will be handled later by a caller.
                return Ok(None);
            }
        }
    }

    Ok(Some(PrepareDepsOfPatternsValue {
        target_pattern_keys: get_target_pattern_keys(&sky_keys),
    }))
}
