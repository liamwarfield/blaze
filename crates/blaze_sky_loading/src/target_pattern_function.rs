//! TARGET_PATTERN: resolving a target pattern to its targets.
//!
//! Java: com/google/devtools/build/lib/skyframe/{TargetPatternFunction,TargetPatternValue,
//! EnvironmentBackedRecursivePackageProvider,
//! RecursivePackageProviderBackedTargetPatternResolver}.java,
//! com/google/devtools/build/lib/pkgcache/{RecursivePackageProvider,FilteringPolicy,
//! FilteringPolicies,TargetPatternResolverUtil,AbstractRecursivePackageProvider}.java.
//!
//! PORT: the resolver runs on the calling thread (Java's direct executor in Skyframe); the
//! package semaphore, task semaphore and batching of 1000 packages per task do not change the
//! result and are not ported.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryName, SignedTargetPattern, SignedTargetPatternSign,
    TargetParsingException, TargetPattern, TargetPatternsCode,
};
use blaze_events::{Event, ExtendedEventHandler, StoredEventHandler};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_packages::package::Package;
use blaze_packages::target::target_utils;
use blaze_packages::target::{OutputFile, Target};
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, SkyKey, Transience,
};
use blaze_vfs::{PathFragment, Root, RootedPath};

use crate::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use crate::io::IOException;
use crate::package_function::PackageKey;
use crate::package_lookup::PackageLookupKey;
use crate::precomputed::PATH_PACKAGE_LOCATOR;
use crate::recursive_pkg::{PackageErrorKey, ProcessPackageDirectoryException, RecursivePkgKey};
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use crate::sky_functions;
use crate::target_pattern_eval::{self, TargetPatternEvalError, TargetPatternResolver};

/// A filtering policy for targets (Java `FilteringPolicy` and the `FilteringPolicies`
/// constants).
// Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:27 FilteringPolicies
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum FilteringPolicy {
    NoFilter,
    FilterManual,
    FilterTests,
    RulesOnly,
    And(Box<FilteringPolicy>, Box<FilteringPolicy>),
    /// A filtering policy defined elsewhere (Java: other `FilteringPolicy` implementations,
    /// e.g. `PlatformLookupUtil.HAS_PLATFORM_INFO`, `RegisteredToolchainsFunction`'s
    /// `ruleTypeExplicit("toolchain")`), by name and predicate.
    // W4-analysis: used by toolchain and execution platform registration.
    Custom(&'static str, FilterFn),
}

/// The predicate of [`FilteringPolicy::Custom`]; policies are identified by their name, so the
/// function takes no part in equality and hashing.
// W4-analysis: see FilteringPolicy::Custom.
#[derive(Clone, Copy)]
pub struct FilterFn(pub fn(&Target, bool) -> bool);

impl PartialEq for FilterFn {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for FilterFn {}

impl std::hash::Hash for FilterFn {
    fn hash<H: std::hash::Hasher>(&self, _state: &mut H) {}
}

impl FilteringPolicy {
    /// Returns the result of applying y, if target passes x.
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:35 FilteringPolicies.and
    pub fn and(x: FilteringPolicy, y: FilteringPolicy) -> FilteringPolicy {
        if x == FilteringPolicy::NoFilter {
            return y;
        }
        if y == FilteringPolicy::NoFilter {
            return x;
        }
        FilteringPolicy::And(Box::new(x), Box::new(y))
    }

    /// Returns true if this target should be retained.
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicy.java:33 FilteringPolicy.shouldRetain
    pub fn should_retain(&self, target: &Target, explicit: bool) -> bool {
        match self {
            FilteringPolicy::NoFilter => true,
            FilteringPolicy::FilterManual => explicit || !target_utils::has_manual_tag(target),
            FilteringPolicy::FilterTests => {
                target_utils::is_test_or_test_suite_rule(target)
                    && FilteringPolicy::FilterManual.should_retain(target, explicit)
            }
            FilteringPolicy::RulesOnly => matches!(target, Target::Rule(_)),
            FilteringPolicy::And(x, y) => {
                x.should_retain(target, explicit) && y.should_retain(target, explicit)
            }
            FilteringPolicy::Custom(_, predicate) => (predicate.0)(target, explicit),
        }
    }
}

// Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:80 FilteringPolicies.NoFilter.toString
impl fmt::Debug for FilteringPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilteringPolicy::NoFilter => f.write_str("[]"),
            FilteringPolicy::And(x, y) => write!(f, "and_filter({:?}, {:?})", x, y),
            // PORT: Java prints Object.toString() (class name and identity hash) for these.
            FilteringPolicy::FilterManual => f.write_str("FilterManual"),
            FilteringPolicy::FilterTests => f.write_str("FilterTests"),
            FilteringPolicy::RulesOnly => f.write_str("RulesOnly"),
            FilteringPolicy::Custom(name, _) => f.write_str(name),
        }
    }
}

/// The key of TARGET_PATTERN.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:195 TargetPatternValue.TargetPatternKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternKey {
    signed_parsed_pattern: SignedTargetPattern,
    policy: FilteringPolicy,
    excluded_subdirectories: Vec<PathFragment>,
}

impl TargetPatternKey {
    /// Create a target pattern key (excluded subdirectories empty).
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:63 TargetPatternValue.key
    pub fn key(pattern: SignedTargetPattern, policy: FilteringPolicy) -> TargetPatternKey {
        let policy = if pattern.sign() == SignedTargetPatternSign::Positive {
            policy
        } else {
            FilteringPolicy::NoFilter
        };
        TargetPatternKey {
            signed_parsed_pattern: pattern,
            policy,
            excluded_subdirectories: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:224 TargetPatternValue.TargetPatternKey.getPattern
    pub fn get_pattern(&self) -> &str {
        self.signed_parsed_pattern.pattern().get_original_pattern()
    }

    pub fn get_parsed_pattern(&self) -> &TargetPattern {
        self.signed_parsed_pattern.pattern()
    }

    pub fn get_policy(&self) -> &FilteringPolicy {
        &self.policy
    }

    pub fn is_negative(&self) -> bool {
        self.signed_parsed_pattern.sign() == SignedTargetPatternSign::Negative
    }

    pub fn get_excluded_subdirectories(&self) -> &[PathFragment] {
        &self.excluded_subdirectories
    }
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:249 TargetPatternValue.TargetPatternKey.toString
impl fmt::Debug for TargetPatternKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let excluded: Vec<&str> = self
            .excluded_subdirectories
            .iter()
            .map(|p| p.get_path_string())
            .collect();
        write!(
            f,
            "{}{}, excludedSubdirs=[{}], filteringPolicy={:?}",
            if self.is_negative() { "-" } else { "" },
            self.get_pattern(),
            excluded.join(", "),
            self.policy
        )
    }
}

impl Key for TargetPatternKey {
    type Value = TargetPatternValue;
    const FUNCTION: SkyFunctionName = sky_functions::TARGET_PATTERN;
}

/// The labels a pattern resolves to, in resolution order (Java `TargetPatternValue` holding a
/// `ResolvedTargets<Label>`).
// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:44 TargetPatternValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetPatternValue {
    targets: indexmap::IndexSet<Label>,
    has_error: bool,
}

impl TargetPatternValue {
    pub fn get_targets(&self) -> &indexmap::IndexSet<Label> {
        &self.targets
    }

    pub fn has_error(&self) -> bool {
        self.has_error
    }
}

/// Supplies packages to the resolver (Java `RecursivePackageProvider`).
// Java: com/google/devtools/build/lib/pkgcache/RecursivePackageProvider.java:35 RecursivePackageProvider
pub trait RecursivePackageProvider {
    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:52 PackageProvider.getPackage
    fn get_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError>;

    // Java: com/google/devtools/build/lib/pkgcache/PackageProvider.java:96 PackageProvider.isPackage
    fn is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError>;

    /// Calls `results` with the packages beneath `directory`.
    // Java: com/google/devtools/build/lib/pkgcache/RecursivePackageProvider.java:65 RecursivePackageProvider.streamPackagesUnderDirectory
    fn stream_packages_under_directory(
        &self,
        results: &mut dyn FnMut(Vec<PackageIdentifier>),
        event_handler: &dyn ExtendedEventHandler,
        repository: &RepositoryName,
        directory: &PathFragment,
        ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError>;

    // Java: com/google/devtools/build/lib/pkgcache/AbstractRecursivePackageProvider.java:39 AbstractRecursivePackageProvider.getTarget
    fn get_target(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        label: &Label,
    ) -> Result<Target, ProviderError> {
        // TODO(https://github.com/bazelbuild/bazel/issues/23852): don't expand the full package
        // if lazy macro expansion is enabled.
        let pkg = self.get_package(event_handler, label.get_package_identifier())?;
        match pkg.get_target(label.get_name()) {
            Ok(t) => Ok(t.clone()),
            Err(e) => Err(ProviderError::NoSuchThing(NoSuchThingException::Target(e))),
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/RecursivePackageProvider.java:83 RecursivePackageProvider.bulkGetPackages
    fn bulk_get_packages(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        pkg_ids: &[PackageIdentifier],
    ) -> Result<Vec<(PackageIdentifier, Arc<Package>)>, ProviderError> {
        let mut result = Vec::with_capacity(pkg_ids.len());
        for pkg_id in pkg_ids {
            result.push((pkg_id.clone(), self.get_package(event_handler, pkg_id)?));
        }
        Ok(result)
    }
}

/// What a [`RecursivePackageProvider`] can throw.
#[derive(Debug, Clone)]
pub enum ProviderError {
    /// `NoSuchPackageException` / `NoSuchTargetException`.
    NoSuchThing(NoSuchThingException),
    Eval(TargetPatternEvalError),
}

impl From<NoSuchPackageException> for ProviderError {
    fn from(e: NoSuchPackageException) -> Self {
        ProviderError::NoSuchThing(NoSuchThingException::Package(e))
    }
}

impl From<TargetPatternEvalError> for ProviderError {
    fn from(e: TargetPatternEvalError) -> Self {
        ProviderError::Eval(e)
    }
}

impl From<blaze_skyframe::Interrupted> for ProviderError {
    fn from(_: blaze_skyframe::Interrupted) -> Self {
        ProviderError::Eval(TargetPatternEvalError::Interrupted)
    }
}

/// A [`TargetParsingException`] from a `NoSuchThingException`.
///
/// PORT: Java keeps the thing's DetailedExitCode (a PackageLoading code); the ported
/// `TargetParsingException` holds only a TargetPatterns code.
fn target_parsing_exception_of(message: String) -> TargetParsingException {
    TargetParsingException::new(message, TargetPatternsCode::TargetPatternsUnknown)
}

/// A [`RecursivePackageProvider`] over a Skyframe environment.
// Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:58 EnvironmentBackedRecursivePackageProvider
pub struct EnvironmentBackedRecursivePackageProvider<'e, 'x> {
    env: RefCell<&'e mut Environment<'x>>,
    encountered_package_errors: Cell<bool>,
    no_such_package_exception: RefCell<Option<NoSuchPackageException>>,
}

impl<'e, 'x> EnvironmentBackedRecursivePackageProvider<'e, 'x> {
    pub fn new(env: &'e mut Environment<'x>) -> Self {
        EnvironmentBackedRecursivePackageProvider {
            env: RefCell::new(env),
            encountered_package_errors: Cell::new(false),
            no_such_package_exception: RefCell::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:71 EnvironmentBackedRecursivePackageProvider.encounteredPackageErrors
    pub fn encountered_package_errors(&self) -> bool {
        self.encountered_package_errors.get()
    }

    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:76 EnvironmentBackedRecursivePackageProvider.maybeGetNoSuchPackageException
    pub fn maybe_get_no_such_package_exception(&self) -> Option<NoSuchPackageException> {
        self.no_such_package_exception.borrow().clone()
    }

    fn set_no_such_package_exception_if_unset(&self, e: &NoSuchPackageException) {
        let mut slot = self.no_such_package_exception.borrow_mut();
        if slot.is_none() {
            *slot = Some(e.clone());
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:111 EnvironmentBackedRecursivePackageProvider.handlePackageoidErrors
    fn handle_packageoid_errors(&self, pkg: &Package) -> Result<(), ProviderError> {
        if pkg.contains_errors() {
            // If this is a nokeep_going build, we must shut the build down by throwing an
            // exception. To do that, we request a node that will throw an exception, and then
            // try to catch it and continue. This gives the framework notification to shut down
            // the build if it should.
            let mut env = self.env.borrow_mut();
            match env.get_value_or_throw::<PackageErrorKey, NoSuchPackageException>(
                PackageErrorKey(pkg.get_package_identifier().clone()),
            )? {
                Err(e) => {
                    // If this is a keep_going build, then the user of this
                    // RecursivePackageProvider has two options for handling the "package in
                    // error" case: inspect the package, or check encounteredPackageErrors.
                    drop(env);
                    self.encountered_package_errors.set(true);
                    self.set_no_such_package_exception_if_unset(&e);
                }
                Ok(_) => {
                    return Err(TargetPatternEvalError::MissingDep.into());
                }
            }
        }
        Ok(())
    }
}

impl RecursivePackageProvider for EnvironmentBackedRecursivePackageProvider<'_, '_> {
    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:82 EnvironmentBackedRecursivePackageProvider.getPackage
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        let pkg_value = self
            .env
            .borrow_mut()
            .get_value_or_throw::<PackageKey, NoSuchPackageException>(PackageKey::key(
                package_name.clone(),
            ))?;
        let pkg_value = match pkg_value {
            Err(e) => {
                self.encountered_package_errors.set(true);
                return Err(e.into());
            }
            Ok(None) => return Err(TargetPatternEvalError::MissingDep.into()),
            Ok(Some(v)) => v,
        };
        let pkg = pkg_value.get_package().clone();
        self.handle_packageoid_errors(&pkg)?;
        Ok(pkg)
    }

    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:137 EnvironmentBackedRecursivePackageProvider.isPackage
    fn is_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        let key = PackageLookupKey::key(package_id.clone());
        let mut env = self.env.borrow_mut();
        enum Caught {
            NoSuchPackage(NoSuchPackageException),
            InconsistentFilesystem(IOException),
        }
        match env.get_value_or_throw_with(key, |e| {
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                Some(Caught::NoSuchPackage(e.clone()))
            } else {
                e.downcast_ref::<IOException>()
                    .filter(|e| e.is_inconsistent_filesystem())
                    .map(|e| Caught::InconsistentFilesystem(e.clone()))
            }
        })? {
            Err(Caught::NoSuchPackage(e)) => {
                drop(env);
                self.set_no_such_package_exception_if_unset(&e);
                self.encountered_package_errors.set(true);
                Ok(false)
            }
            Err(Caught::InconsistentFilesystem(e)) => {
                Err(TargetPatternEvalError::InconsistentFilesystem(e).into())
            }
            Ok(Some(v)) => Ok(v.package_exists()),
            Ok(None) => Err(TargetPatternEvalError::MissingDep.into()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/EnvironmentBackedRecursivePackageProvider.java:160 EnvironmentBackedRecursivePackageProvider.streamPackagesUnderDirectory
    fn stream_packages_under_directory(
        &self,
        results: &mut dyn FnMut(Vec<PackageIdentifier>),
        event_handler: &dyn ExtendedEventHandler,
        repository: &RepositoryName,
        directory: &PathFragment,
        ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        let mut env = self.env.borrow_mut();
        let Some(package_locator) = PATH_PACKAGE_LOCATOR.get(&mut **env)? else {
            return Err(TargetPatternEvalError::MissingDep.into());
        };
        let mut roots: Vec<Root> = Vec::new();
        if repository.is_main() {
            roots.extend(package_locator.get_path_entries().iter().cloned());
        } else {
            let Some(repository_value) =
                env.get_value(RepositoryDirectoryKey::key(repository.clone()))?
            else {
                return Err(TargetPatternEvalError::MissingDep.into());
            };
            match &*repository_value {
                RepositoryDirectoryValue::Failure { error_msg } => {
                    event_handler.handle(&Event::error(format!(
                        "No such repository '{}': {}",
                        repository, error_msg
                    )));
                    return Ok(());
                }
                RepositoryDirectoryValue::Success { root, .. } => roots.push(root.clone()),
            }
        }
        let filtered_ignored_subdirectories =
            ignored_subdirectories.filter_for_directory(directory);
        let recursive_package_keys: Vec<RecursivePkgKey> = roots
            .iter()
            .map(|r| {
                RecursivePkgKey::key(
                    repository.clone(),
                    RootedPath::to_rooted_path(r.clone(), directory),
                    filtered_ignored_subdirectories.clone(),
                )
            })
            .collect();
        let sky_keys: Vec<SkyKey> = recursive_package_keys
            .iter()
            .cloned()
            .map(SkyKey::new)
            .collect();
        let recursive_package_values = env.get_values_and_exceptions(&sky_keys)?;
        let mut first_nspe: Option<NoSuchPackageException> = None;
        let mut first_ppde: Option<ProcessPackageDirectoryException> = None;
        let mut errors_seen = false;
        let mut lookups = Vec::new();
        for key in &recursive_package_keys {
            enum Caught {
                NoSuchPackage(NoSuchPackageException),
                ProcessPackageDirectory(ProcessPackageDirectoryException),
            }
            let lookup = match recursive_package_values.get_or_throw_with(key, |e| {
                if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                    Some(Caught::NoSuchPackage(e.clone()))
                } else {
                    e.downcast_ref::<ProcessPackageDirectoryException>()
                        .map(|e| Caught::ProcessPackageDirectory(e.clone()))
                }
            }) {
                Err(Caught::NoSuchPackage(e)) => {
                    // NoSuchPackageException can happen during error bubbling in a
                    // no-keep-going build.
                    if first_nspe.is_none() {
                        first_nspe = Some(e.clone());
                    }
                    errors_seen = true;
                    self.set_no_such_package_exception_if_unset(&e);
                    continue;
                }
                Err(Caught::ProcessPackageDirectory(e)) => {
                    first_ppde.get_or_insert(e);
                    continue;
                }
                Ok(Some(v)) => v,
                Ok(None) => continue,
            };
            lookups.push(lookup);
        }
        if let Some(e) = first_ppde {
            return Err(TargetPatternEvalError::ProcessPackageDirectory(
                e.get_message().to_owned(),
            )
            .into());
        }
        let values_missing = env.values_missing();
        drop(env);
        if errors_seen {
            self.encountered_package_errors.set(true);
        }
        for lookup in lookups {
            if lookup.has_errors() {
                self.encountered_package_errors.set(true);
            }
            if values_missing {
                // If values are missing, we're only checking for errors, not constructing a
                // result.
                continue;
            }
            for package_name in lookup.get_packages() {
                let package_name_path_fragment = PathFragment::create(package_name);
                if !excluded_subdirectories
                    .iter()
                    .any(|e| package_name_path_fragment.starts_with(e))
                {
                    results(vec![PackageIdentifier::create(
                        repository.clone(),
                        package_name_path_fragment,
                    )]);
                }
            }
        }
        if let Some(e) = first_nspe {
            return Err(e.into());
        }
        if values_missing {
            return Err(TargetPatternEvalError::MissingDep.into());
        }
        Ok(())
    }
}

/// A [`RecursivePackageProvider`] over already loaded packages.
// Java: com/google/devtools/build/lib/pkgcache/RecursivePackageProvider.java:110 RecursivePackageProvider.PackageBackedRecursivePackageProvider
pub struct PackageBackedRecursivePackageProvider {
    packages: HashMap<PackageIdentifier, Arc<Package>>,
}

impl PackageBackedRecursivePackageProvider {
    pub fn new(packages: HashMap<PackageIdentifier, Arc<Package>>) -> Self {
        PackageBackedRecursivePackageProvider { packages }
    }
}

impl RecursivePackageProvider for PackageBackedRecursivePackageProvider {
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        match self.packages.get(package_name) {
            Some(p) => Ok(p.clone()),
            None => Err(NoSuchPackageException::new(package_name.clone(), "").into()),
        }
    }

    fn is_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        Ok(self.packages.contains_key(package_name))
    }

    fn stream_packages_under_directory(
        &self,
        _results: &mut dyn FnMut(Vec<PackageIdentifier>),
        _event_handler: &dyn ExtendedEventHandler,
        _repository: &RepositoryName,
        _directory: &PathFragment,
        _ignored_subdirectories: &IgnoredSubdirectories,
        _excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        panic!("UnsupportedOperationException")
    }
}

// Java: com/google/devtools/build/lib/pkgcache/TargetPatternResolverUtil.java:34 TargetPatternResolverUtil.getParsingErrorMessage
fn get_parsing_error_message(message: &str, original_pattern: Option<&str>) -> String {
    match original_pattern {
        None => message.to_owned(),
        Some(p) => format!("while parsing '{}': {}", p, message),
    }
}

// Java: com/google/devtools/build/lib/pkgcache/TargetPatternResolverUtil.java:42 TargetPatternResolverUtil.resolvePackageTargets
fn resolve_package_targets(pkg: &Package, policy: &FilteringPolicy) -> Vec<Target> {
    if *policy == FilteringPolicy::NoFilter {
        return pkg.get_targets().values().cloned().collect();
    }
    pkg.get_targets()
        .values()
        .filter(|t| policy.should_retain(t, false))
        .cloned()
        .collect()
}

// Java: com/google/devtools/build/lib/pkgcache/TargetPatternResolverUtil.java:54 TargetPatternResolverUtil.getPathFragment
fn get_path_fragment(path_prefix: &str) -> Result<PathFragment, TargetParsingException> {
    let directory = PathFragment::create(path_prefix);
    if directory.contains_uplevel_references() {
        return Err(TargetParsingException::new(
            format!(
                "up-level references are not permitted: '{}'",
                directory.get_path_string()
            ),
            TargetPatternsCode::UpLevelReferencesNotAllowed,
        ));
    }
    if !path_prefix.is_empty()
        && blaze_cmdline::label_validator::validate_package_name(path_prefix).is_some()
    {
        return Err(TargetParsingException::new(
            format!("'{}' is not a valid package name", path_prefix),
            TargetPatternsCode::PackageNameInvalid,
        ));
    }
    Ok(directory)
}

/// A [`TargetPatternResolver`] over a [`RecursivePackageProvider`].
// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:62 RecursivePackageProviderBackedTargetPatternResolver
pub struct RecursivePackageProviderBackedTargetPatternResolver<'a> {
    pub policy: FilteringPolicy,
    recursive_package_provider: &'a dyn RecursivePackageProvider,
    event_handler: &'a dyn ExtendedEventHandler,
}

impl<'a> RecursivePackageProviderBackedTargetPatternResolver<'a> {
    pub fn new(
        recursive_package_provider: &'a dyn RecursivePackageProvider,
        event_handler: &'a dyn ExtendedEventHandler,
        policy: FilteringPolicy,
    ) -> Self {
        RecursivePackageProviderBackedTargetPatternResolver {
            policy,
            recursive_package_provider,
            event_handler,
        }
    }

    fn provider_error(e: ProviderError) -> TargetPatternEvalError {
        match e {
            ProviderError::Eval(e) => e,
            ProviderError::NoSuchThing(e) => {
                TargetPatternEvalError::TargetParsing(target_parsing_exception_of(e.get_message()))
            }
        }
    }
}

impl TargetPatternResolver<Target> for RecursivePackageProviderBackedTargetPatternResolver<'_> {
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:100 RecursivePackageProviderBackedTargetPatternResolver.warn
    fn warn(&self, msg: &str) {
        self.event_handler.handle(&Event::warn(msg));
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:115 RecursivePackageProviderBackedTargetPatternResolver.getTargetOrNull
    fn get_target_or_null(&self, label: &Label) -> Result<Option<Target>, TargetPatternEvalError> {
        if !self.is_package(label.get_package_identifier())? {
            return Ok(None);
        }
        match self
            .recursive_package_provider
            .get_target(self.event_handler, label)
        {
            Ok(t) => Ok(Some(t)),
            Err(ProviderError::NoSuchThing(_)) => Ok(None),
            Err(ProviderError::Eval(e)) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:129 RecursivePackageProviderBackedTargetPatternResolver.getExplicitTarget
    fn get_explicit_target(&self, label: &Label) -> Result<Vec<Target>, TargetPatternEvalError> {
        match self
            .recursive_package_provider
            .get_target(self.event_handler, label)
        {
            Ok(target) => Ok(if self.policy.should_retain(&target, true) {
                vec![target]
            } else {
                Vec::new()
            }),
            Err(e) => Err(Self::provider_error(e)),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:142 RecursivePackageProviderBackedTargetPatternResolver.getTargetsInPackage
    fn get_targets_in_package(
        &self,
        original_pattern: &str,
        package_identifier: &PackageIdentifier,
        rules_only: bool,
    ) -> Result<Vec<Target>, TargetPatternEvalError> {
        let actual_policy = if rules_only {
            FilteringPolicy::and(FilteringPolicy::RulesOnly, self.policy.clone())
        } else {
            self.policy.clone()
        };
        match self
            .recursive_package_provider
            .get_package(self.event_handler, package_identifier)
        {
            Ok(pkg) => {
                Package::maybe_add_package_contains_errors_event_to_handler(
                    &pkg,
                    self.event_handler,
                );
                Ok(resolve_package_targets(&pkg, &actual_policy))
            }
            Err(ProviderError::NoSuchThing(e)) => {
                let message = get_parsing_error_message(&e.get_message(), Some(original_pattern));
                Err(TargetPatternEvalError::TargetParsing(
                    target_parsing_exception_of(message),
                ))
            }
            Err(ProviderError::Eval(e)) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:208 RecursivePackageProviderBackedTargetPatternResolver.isPackage
    fn is_package(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Result<bool, TargetPatternEvalError> {
        self.recursive_package_provider
            .is_package(self.event_handler, package_identifier)
            .map_err(Self::provider_error)
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:213 RecursivePackageProviderBackedTargetPatternResolver.getTargetKind
    fn get_target_kind(&self, target: &Target) -> String {
        target.get_target_kind()
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:218 RecursivePackageProviderBackedTargetPatternResolver.findTargetsBeneathDirectory
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:300 RecursivePackageProviderBackedTargetPatternResolver.findTargetsBeneathDirectoryAsyncImpl
    fn find_targets_beneath_directory(
        &self,
        repository: &RepositoryName,
        original_pattern: &str,
        directory: &str,
        rules_only: bool,
        forbidden_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
        callback: &mut dyn FnMut(Vec<Target>),
    ) -> Result<(), TargetPatternEvalError> {
        let actual_policy = if rules_only {
            FilteringPolicy::and(FilteringPolicy::RulesOnly, self.policy.clone())
        } else {
            self.policy.clone()
        };
        let path_fragment = get_path_fragment(directory)?;
        let mut package_ids: Vec<PackageIdentifier> = Vec::new();
        let streamed = self
            .recursive_package_provider
            .stream_packages_under_directory(
                &mut |batch| package_ids.extend(batch),
                self.event_handler,
                repository,
                &path_fragment,
                forbidden_subdirectories,
                excluded_subdirectories,
            );
        match streamed {
            Ok(()) => {}
            Err(ProviderError::NoSuchThing(e)) => {
                // Can happen during a Skyframe no-keep-going evaluation.
                return Err(TargetParsingException::new(
                    format!(
                        "error loading package under directory '{}': {}",
                        directory,
                        e.get_message()
                    ),
                    TargetPatternsCode::TargetPatternsUnknown,
                )
                .into());
            }
            Err(ProviderError::Eval(e)) => return Err(e),
        }
        if package_ids.is_empty() {
            return Err(TargetParsingException::new(
                format!("no targets found beneath '{}'", path_fragment),
                TargetPatternsCode::TargetsMissing,
            )
            .into());
        }
        // Java GetTargetsInPackagesTask: bulkGetTargetsInPackage over each batch (here: all
        // packages; the order of the targets is the same).
        let pkgs = match self
            .recursive_package_provider
            .bulk_get_packages(self.event_handler, &package_ids)
        {
            Ok(p) => p,
            Err(ProviderError::NoSuchThing(e)) => {
                let message = get_parsing_error_message(&e.get_message(), Some(original_pattern));
                panic!(
                    "IllegalStateException: Mismatch: Expected given pkgIds to correspond to \
                     valid Packages. {}",
                    message
                );
            }
            Err(ProviderError::Eval(e)) => return Err(e),
        };
        let mut filtered_targets = Vec::new();
        for (_, pkg) in &pkgs {
            Package::maybe_add_package_contains_errors_event_to_handler(pkg, self.event_handler);
            filtered_targets.extend(resolve_package_targets(pkg, &actual_policy));
        }
        callback(filtered_targets);
        Ok(())
    }
}

/// The error of TARGET_PATTERN (Java `TargetPatternFunction.TargetPatternFunctionException`):
/// a [`TargetParsingException`], catastrophic for filesystem inconsistencies.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternFunction.java:152 TargetPatternFunction.TargetPatternFunctionException
fn target_pattern_function_exception(
    e: TargetParsingException,
    is_catastrophic: bool,
) -> ComputeError {
    if is_catastrophic {
        SkyFunctionException::catastrophic(e, Transience::Persistent).into()
    } else {
        SkyFunctionException::persistent(e).into()
    }
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternFunction.java:139 TargetPatternFunction.maybeThrowEncounteredException
fn maybe_throw_encountered_exception(
    pattern: &TargetPattern,
    provider: &EnvironmentBackedRecursivePackageProvider<'_, '_>,
) -> Result<(), ComputeError> {
    if let Some(e) = provider.maybe_get_no_such_package_exception() {
        return Err(target_pattern_function_exception(
            TargetParsingException::new(
                format!(
                    "Error evaluating '{}': {}",
                    pattern.get_original_pattern(),
                    e.get_message()
                ),
                TargetPatternsCode::PackageNotFound,
            ),
            false,
        ));
    }
    Ok(())
}

/// TARGET_PATTERN (Java `TargetPatternFunction.compute`).
// Java: com/google/devtools/build/lib/skyframe/TargetPatternFunction.java:55 TargetPatternFunction.compute
pub fn target_pattern_function(
    pattern_key: &TargetPatternKey,
    env: &mut Environment<'_>,
) -> Result<Option<TargetPatternValue>, ComputeError> {
    let parsed_pattern = pattern_key.get_parsed_pattern();

    let Some(ignored_subdirectories) = env.get_value(IgnoredSubdirectoriesKey::key(
        parsed_pattern.get_repository(),
    ))?
    else {
        return Ok(None);
    };
    // Java gives the resolver env.getListener(); the environment is borrowed by the provider
    // here, so the resolver's events are stored and replayed on the listener, in order.
    let events = StoredEventHandler::new();
    let provider = EnvironmentBackedRecursivePackageProvider::new(env);
    let result =
        target_pattern_function_internal(pattern_key, &ignored_subdirectories, &provider, &events);
    events.replay_on(provider.env.borrow().listener());
    result
}

fn target_pattern_function_internal(
    pattern_key: &TargetPatternKey,
    ignored_subdirectories: &crate::ignored_subdirectories::IgnoredSubdirectoriesValue,
    provider: &EnvironmentBackedRecursivePackageProvider<'_, '_>,
    events: &StoredEventHandler,
) -> Result<Option<TargetPatternValue>, ComputeError> {
    let parsed_pattern = pattern_key.get_parsed_pattern();
    let resolver = RecursivePackageProviderBackedTargetPatternResolver::new(
        provider,
        events,
        pattern_key.get_policy().clone(),
    );
    // Java ResolvedTargets.Builder<Target>: an insertion-ordered set (targets are unique by
    // label within a build).
    let mut resolved_targets: indexmap::IndexSet<Label> = indexmap::IndexSet::new();
    let result = target_pattern_eval::eval(
        parsed_pattern,
        &resolver,
        &|| Ok(ignored_subdirectories.as_ignored_subdirectories().clone()),
        pattern_key.get_excluded_subdirectories(),
        &mut |partial_result| {
            for target in partial_result {
                // TODO(b/156899726): This will go away as soon as we remove implicit outputs
                // from cc_library completely.
                if let Target::OutputFile(output_file) = &target {
                    if output_file_generating_rule_class(output_file) == "cc_library" {
                        continue;
                    }
                }
                resolved_targets.insert(target.get_label().clone());
            }
        },
    );
    let mut has_error = false;
    match result {
        Ok(()) => {
            if provider.encountered_package_errors() {
                has_error = true;
            }
        }
        Err(TargetPatternEvalError::TargetParsing(e)) => {
            // PORT: Java posts a ParsingFailedEvent here (event bus not ported).
            return Err(target_pattern_function_exception(e, false));
        }
        Err(TargetPatternEvalError::ProcessPackageDirectory(message)) => {
            return Err(target_pattern_function_exception(
                TargetParsingException::new(message, TargetPatternsCode::TargetPatternsUnknown),
                true,
            ));
        }
        Err(TargetPatternEvalError::InconsistentFilesystem(e)) => {
            return Err(target_pattern_function_exception(
                TargetParsingException::new(
                    e.get_message(),
                    TargetPatternsCode::TargetPatternsUnknown,
                ),
                true,
            ));
        }
        Err(TargetPatternEvalError::MissingDep) => {
            // If there is at least one missing dependency, we should eagerly throw any exception
            // we might have encountered: if we are in error bubbling, this could be our last
            // chance.
            maybe_throw_encountered_exception(parsed_pattern, provider)?;
            return Ok(None);
        }
        Err(TargetPatternEvalError::Interrupted) => {
            if provider.env.borrow().in_error_bubbling() {
                maybe_throw_encountered_exception(parsed_pattern, provider)?;
            }
            return Err(blaze_skyframe::Interrupted.into());
        }
    }
    let in_error_bubbling = provider.env.borrow().in_error_bubbling();
    if in_error_bubbling {
        maybe_throw_encountered_exception(parsed_pattern, provider)?;
    }
    // Java: resolvedTargets.getFilteredTargets() is empty here (no filtered targets are added).
    Ok(Some(TargetPatternValue {
        targets: resolved_targets,
        has_error,
    }))
}

/// Java `outputFile.getGeneratingRule().getRuleClass()`.
fn output_file_generating_rule_class(output_file: &OutputFile) -> String {
    output_file
        .get_generating_rule()
        .get_rule_class()
        .to_owned()
}
