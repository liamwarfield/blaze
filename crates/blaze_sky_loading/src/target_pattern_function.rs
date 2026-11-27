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
use std::collections::{HashMap, HashSet};
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
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, Transience,
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
// PORT: skipped, derived PartialEq/Hash: a variant equals itself (AbstractFilteringPolicy, by class) and And compares both parts: FilteringPolicies.hashCode, FilteringPolicies.equals
// PORT: skipped, the variant's fields (And(x, y)): FilteringPolicies.getFirstPolicy, FilteringPolicies.getSecondPolicy
// PORT: skipped, RuleTypeFilter and ruleTypeExplicit are the Custom policies of their users (toolchain and execution platform registration, by name): FilteringPolicies.ruleTypeExplicit, FilteringPolicies.create
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum FilteringPolicy {
    NoFilter,
    FilterManual,
    FilterTests,
    RulesOnly,
    And(Box<FilteringPolicy>, Box<FilteringPolicy>),
    /// Excludes multiple single targets (Java `TargetExcludingFilteringPolicy`, package-private
    /// in `skyframe`); the labels in insertion order without duplicates (`ImmutableSet`).
    // Java: com/google/devtools/build/lib/skyframe/TargetExcludingFilteringPolicy.java:27 TargetExcludingFilteringPolicy
    // Java: com/google/devtools/build/lib/skyframe/TargetExcludingFilteringPolicy.java:30 TargetExcludingFilteringPolicy.<init>
    // PORT: skipped, derived PartialEq/Hash compare the label lists (ImmutableSet.equals is order-insensitive; the only producer builds them in pattern order, so equal sets are equal lists): TargetExcludingFilteringPolicy.equals, TargetExcludingFilteringPolicy.hashCode
    ExcludedTargets(Vec<Label>),
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
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:74 FilteringPolicies.NoFilter.shouldRetain
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:86 FilteringPolicies.FilterManual.shouldRetain
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:93 FilteringPolicies.FilterTests.shouldRetain
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:101 FilteringPolicies.RulesOnly.shouldRetain
    // Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:142 FilteringPolicies.AndFilteringPolicy.shouldRetain
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
            // Java: com/google/devtools/build/lib/skyframe/TargetExcludingFilteringPolicy.java:35 TargetExcludingFilteringPolicy.shouldRetain
            FilteringPolicy::ExcludedTargets(excluded_single_targets) => {
                !excluded_single_targets.contains(target.get_label())
            }
            FilteringPolicy::Custom(_, predicate) => (predicate.0)(target, explicit),
        }
    }
}

// Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:80 FilteringPolicies.NoFilter.toString
// Java: com/google/devtools/build/lib/pkgcache/FilteringPolicies.java:169 FilteringPolicies.AndFilteringPolicy.toString
impl fmt::Debug for FilteringPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilteringPolicy::NoFilter => f.write_str("[]"),
            FilteringPolicy::And(x, y) => write!(f, "and_filter({:?}, {:?})", x, y),
            // PORT: Java prints Object.toString() (class name and identity hash) for these.
            FilteringPolicy::FilterManual => f.write_str("FilterManual"),
            FilteringPolicy::FilterTests => f.write_str("FilterTests"),
            FilteringPolicy::RulesOnly => f.write_str("RulesOnly"),
            // Java: com/google/devtools/build/lib/skyframe/TargetExcludingFilteringPolicy.java:40 TargetExcludingFilteringPolicy.toString
            FilteringPolicy::ExcludedTargets(excluded_single_targets) => write!(
                f,
                "excludedTargets[{}]",
                excluded_single_targets
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
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

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:240 TargetPatternValue.TargetPatternKey.getSignedParsedPattern
    pub fn get_signed_parsed_pattern(&self) -> &SignedTargetPattern {
        &self.signed_parsed_pattern
    }
}

/// Takes a list of [`TargetPatternKey`]s and, for each one, adds the directories of the later
/// negative targets-below-directory patterns it contains as excluded subdirectories (and, with
/// `exclude_single_targets`, the later negative single targets). A negative pattern so combined
/// is dropped; the others are kept in order.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:82 TargetPatternValue.combineTargetsBelowDirectoryWithNegativePatterns
pub fn combine_targets_below_directory_with_negative_patterns(
    keys: &[TargetPatternKey],
    exclude_single_targets: bool,
) -> Vec<TargetPatternKey> {
    let mut builder = Vec::new();
    // We use indicesOfNegativePatternsThatNeedToBeIncluded to avoid adding negative TBD or single
    // target patterns that have already been combined with previous patterns as an excluded
    // directory or excluded single target.
    let mut indices_of_negative_patterns_that_need_to_be_included: HashSet<usize> = HashSet::new();
    let mut positive_pattern_seen = false;
    for (i, target_pattern_key) in keys.iter().enumerate() {
        if target_pattern_key.is_negative() {
            if indices_of_negative_patterns_that_need_to_be_included.contains(&i)
                || !positive_pattern_seen
            {
                builder.push(target_pattern_key.clone());
            }
        } else {
            positive_pattern_seen = true;
            let (target_pattern_key_maybe, indices) = compute_target_pattern_key_with_exclusions(
                target_pattern_key,
                i,
                keys,
                exclude_single_targets,
            );
            if let Some(k) = target_pattern_key_maybe {
                builder.push(k);
            }
            indices_of_negative_patterns_that_need_to_be_included.extend(indices);
        }
    }
    builder
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:108 TargetPatternValue.setExcludedDirectoriesAndTargets
fn set_excluded_directories_and_targets(
    original: &TargetPatternKey,
    excluded_subdirectories: Vec<PathFragment>,
    excluded_single_targets: Vec<Label>,
) -> TargetPatternKey {
    let mut policy = original.policy.clone();
    if !excluded_single_targets.is_empty() {
        policy = FilteringPolicy::and(
            policy,
            FilteringPolicy::ExcludedTargets(excluded_single_targets),
        );
    }
    TargetPatternKey {
        signed_parsed_pattern: original.signed_parsed_pattern.clone(),
        policy,
        excluded_subdirectories,
    }
}

/// Java `TargetPatternKeyWithExclusionsResult`: the key (absent when a later negative pattern
/// excludes all of it) and the indices of the later negative patterns still to be applied.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:132 TargetPatternValue.computeTargetPatternKeyWithExclusions
fn compute_target_pattern_key_with_exclusions(
    target_pattern_key: &TargetPatternKey,
    position: usize,
    keys: &[TargetPatternKey],
    exclude_single_targets: bool,
) -> (Option<TargetPatternKey>, Vec<usize>) {
    use blaze_cmdline::{
        TargetPatternTargetsBelowDirectoryContainsResult as Contains, TargetPatternType,
    };
    let target_pattern = target_pattern_key.get_parsed_pattern();
    // ImmutableSet builders: insertion order, no duplicates.
    let mut excluded_directories: Vec<PathFragment> = Vec::new();
    let mut excluded_single_targets: Vec<Label> = Vec::new();
    let mut indices_of_negative_patterns_that_need_to_be_included: Vec<usize> = Vec::new();
    for (j, later_target_pattern_key) in keys.iter().enumerate().skip(position + 1) {
        let later_parsed_pattern = later_target_pattern_key.get_parsed_pattern();
        if !later_target_pattern_key.is_negative() {
            continue;
        }
        if later_parsed_pattern.get_type() == TargetPatternType::TargetsBelowDirectory {
            let later_parsed_targets_below_directory = later_parsed_pattern
                .as_targets_below_directory()
                .expect("TARGETS_BELOW_DIRECTORY");
            if target_pattern.get_type() == TargetPatternType::TargetsBelowDirectory {
                let targets_below_directory = target_pattern
                    .as_targets_below_directory()
                    .expect("TARGETS_BELOW_DIRECTORY");
                if later_parsed_targets_below_directory.contains(targets_below_directory)
                    == Contains::DirectoryExclusionWouldBeExact
                {
                    return (None, Vec::new());
                } else {
                    match targets_below_directory.contains(later_parsed_targets_below_directory) {
                        Contains::DirectoryExclusionWouldBeExact => {
                            let dir = later_parsed_targets_below_directory
                                .get_directory()
                                .get_package_fragment()
                                .clone();
                            if !excluded_directories.contains(&dir) {
                                excluded_directories.push(dir);
                            }
                        }
                        Contains::DirectoryExclusionWouldBeTooBroad => {
                            indices_of_negative_patterns_that_need_to_be_included.push(j);
                        }
                        Contains::NotContained => {
                            // Nothing to do with this pattern.
                        }
                    }
                }
            }
        } else if exclude_single_targets
            && later_parsed_pattern.get_type() == TargetPatternType::SingleTarget
        {
            let label = later_parsed_pattern.get_single_target_label().clone();
            if !excluded_single_targets.contains(&label) {
                excluded_single_targets.push(label);
            }
        } else {
            indices_of_negative_patterns_that_need_to_be_included.push(j);
        }
    }
    (
        Some(set_excluded_directories_and_targets(
            target_pattern_key,
            excluded_directories,
            excluded_single_targets,
        )),
        indices_of_negative_patterns_that_need_to_be_included,
    )
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
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:69 SkyFunctions.TARGET_PATTERN (createSemiHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::SemiHermetic;
    // Default `values_equal` (never equal): Java's `TargetPatternValue` (and its
    // `ResolvedTargets`) has no `equals`.
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternValue.java:48 TargetPatternValue
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

/// `new TargetParsingException(message, e, e.getDetailedExitCode())` for a
/// `NoSuchThingException` `e`: the exception keeps the thing's `DetailedExitCode`.
// W4-analysis: was TARGET_PATTERNS_UNKNOWN (exit code 37) for every such exception.
// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:140 RecursivePackageProviderBackedTargetPatternResolver.getTargetOrNull
pub(crate) fn target_parsing_exception_of(
    message: String,
    detailed_exit_code: &blaze_packages::exceptions::DetailedExitCode,
) -> TargetParsingException {
    TargetParsingException::with_detailed_exit_code(
        message,
        util_detailed_exit_code(detailed_exit_code),
    )
}

/// The lib/util `DetailedExitCode` of blaze_packages' reduction of it (message and code).
fn util_detailed_exit_code(
    detailed_exit_code: &blaze_packages::exceptions::DetailedExitCode,
) -> blaze_util::DetailedExitCode {
    detailed_exit_code.to_util()
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

    /// Runs `f` on the environment (Java code that holds both the provider and its `env`).
    pub(crate) fn with_env<R>(&self, f: impl FnOnce(&mut Environment<'x>) -> R) -> R {
        f(&mut self.env.borrow_mut())
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
        let Some(package_locator) = PATH_PACKAGE_LOCATOR.get(&mut env)? else {
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
pub(crate) fn get_parsing_error_message(message: &str, original_pattern: Option<&str>) -> String {
    match original_pattern {
        None => message.to_owned(),
        Some(p) => format!("while parsing '{}': {}", p, message),
    }
}

// Java: com/google/devtools/build/lib/pkgcache/TargetPatternResolverUtil.java:42 TargetPatternResolverUtil.resolvePackageTargets
pub(crate) fn resolve_package_targets(pkg: &Package, policy: &FilteringPolicy) -> Vec<Target> {
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
pub(crate) fn get_path_fragment(path_prefix: &str) -> Result<PathFragment, TargetParsingException> {
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
// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:70 RecursivePackageProviderBackedTargetPatternResolver
pub struct RecursivePackageProviderBackedTargetPatternResolver<'a> {
    pub policy: FilteringPolicy,
    recursive_package_provider: &'a dyn RecursivePackageProvider,
    event_handler: &'a dyn ExtendedEventHandler,
}

impl<'a> RecursivePackageProviderBackedTargetPatternResolver<'a> {
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:85 RecursivePackageProviderBackedTargetPatternResolver.<init>
    // PORT: packageSemaphore and maxConcurrentGetTargetsTasks bound the tasks of a parallel
    // executor, and the batching callback factory is always
    // SimplePackageIdentifierBatchingCallback::new at the pin (TargetPatternFunction,
    // SkyframeTargetPatternEvaluator, SkyQueryEnvironment, PostAnalysisQueryEnvironment); the
    // tasks run one at a time here, so none of the three is a parameter.
    //
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:112 RecursivePackageProviderBackedTargetPatternResolver.getPackage
    // (private; its callers call `recursive_package_provider.get_package` directly.)
    //
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:244 RecursivePackageProviderBackedTargetPatternResolver.findTargetsBeneathDirectoryAsync
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:408 RecursivePackageProviderBackedTargetPatternResolver.isSuccessful
    // PORT: skipped, the futures. Java's only asynchronous caller is SkyQueryEnvironment's
    // parallel pattern evaluation (TargetPattern.evalAsync), which this port runs synchronously
    // through `find_targets_beneath_directory`. The two differ only for exceptions that cannot
    // reach a SkyQuery (ProcessPackageDirectoryException and NoSuchPackageException from a
    // graph-backed provider, where Java throws IllegalStateException) and for an interrupt
    // (a cancelled future against InterruptedException; both end the query interrupted).
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
            ProviderError::NoSuchThing(e) => TargetPatternEvalError::TargetParsing(
                target_parsing_exception_of(e.get_message(), &e.get_detailed_exit_code()),
            ),
        }
    }
}

impl TargetPatternResolver<Target> for RecursivePackageProviderBackedTargetPatternResolver<'_> {
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:103 RecursivePackageProviderBackedTargetPatternResolver.warn
    fn warn(&self, msg: &str) {
        self.event_handler.handle(&Event::warn(msg));
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:117 RecursivePackageProviderBackedTargetPatternResolver.getTargetOrNull
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

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:131 RecursivePackageProviderBackedTargetPatternResolver.getExplicitTarget
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

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:144 RecursivePackageProviderBackedTargetPatternResolver.getTargetsInPackage
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
                    target_parsing_exception_of(message, &e.get_detailed_exit_code()),
                ))
            }
            Err(ProviderError::Eval(e)) => Err(e),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:187 RecursivePackageProviderBackedTargetPatternResolver.isPackage
    fn is_package(
        &self,
        package_identifier: &PackageIdentifier,
    ) -> Result<bool, TargetPatternEvalError> {
        self.recursive_package_provider
            .is_package(self.event_handler, package_identifier)
            .map_err(Self::provider_error)
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:193 RecursivePackageProviderBackedTargetPatternResolver.getTargetKind
    fn get_target_kind(&self, target: &Target) -> String {
        target.get_target_kind()
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:198 RecursivePackageProviderBackedTargetPatternResolver.findTargetsBeneathDirectory
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:289 RecursivePackageProviderBackedTargetPatternResolver.findTargetsBeneathDirectoryAsyncImpl
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
        // getPackageTargetsCallback: each batch of package ids becomes a GetTargetsInPackagesTask.
        // PORT: Java submits the task to a direct executor from inside the batching callback, so
        // it runs while the stream is paused; here the batches are collected and their tasks run
        // in the same order once the stream is done (the providers' streams borrow the
        // provider). The targets and the callback's batches are the same; only events of the
        // stream after the first 1000 packages come before those of the tasks.
        let mut package_id_batches: Vec<Vec<PackageIdentifier>> = Vec::new();
        let path_fragment = get_path_fragment(directory)?;
        let streamed = {
            let mut batching_callback = SimplePackageIdentifierBatchingCallback::new(
                |batch| package_id_batches.push(batch),
                MAX_PACKAGES_BULK_GET,
            );
            let streamed = self
                .recursive_package_provider
                .stream_packages_under_directory(
                    &mut |batch| batching_callback.process(batch),
                    self.event_handler,
                    repository,
                    &path_fragment,
                    forbidden_subdirectories,
                    excluded_subdirectories,
                );
            // try-with-resources: closed (flushed) also when the stream throws.
            batching_callback.close();
            streamed
        };
        match streamed {
            Ok(()) => {}
            Err(ProviderError::NoSuchThing(e)) => {
                // Can happen during a Skyframe no-keep-going evaluation.
                return Err(target_parsing_exception_of(
                    format!(
                        "error loading package under directory '{}': {}",
                        directory,
                        e.get_message()
                    ),
                    &e.get_detailed_exit_code(),
                )
                .into());
            }
            Err(ProviderError::Eval(e)) => return Err(e),
        }
        if package_id_batches.is_empty() {
            return Err(TargetParsingException::new(
                format!("no targets found beneath '{}'", path_fragment),
                TargetPatternsCode::TargetsMissing,
            )
            .into());
        }
        // Futures.whenAllSucceed over tasks that a direct executor has already run: every task
        // runs, and the first failure in submission order is thrown.
        let mut first_failure = None;
        for package_identifiers in &package_id_batches {
            if let Err(e) = self.get_targets_in_packages_task(
                package_identifiers,
                original_pattern,
                &actual_policy,
                callback,
            ) {
                first_failure.get_or_insert(e);
            }
        }
        match first_failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl RecursivePackageProviderBackedTargetPatternResolver<'_> {
    /// Gets all matching targets in the given packages, filters them, and passes them to the
    /// target batch callback.
    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:368 RecursivePackageProviderBackedTargetPatternResolver.GetTargetsInPackagesTask.call
    // PORT: acquireTaskLock/releaseTaskLock and the package semaphore bound concurrent tasks;
    // the tasks run one at a time here.
    fn get_targets_in_packages_task(
        &self,
        package_identifiers: &[PackageIdentifier],
        original_pattern: &str,
        actual_policy: &FilteringPolicy,
        callback: &mut dyn FnMut(Vec<Target>),
    ) -> Result<(), TargetPatternEvalError> {
        let resolved_targets =
            self.bulk_get_targets_in_package(original_pattern, package_identifiers, actual_policy)?;
        let mut filtered_targets = Vec::with_capacity(calculate_size(&resolved_targets));
        for (_, targets) in resolved_targets {
            filtered_targets.extend(targets);
        }
        callback(filtered_targets);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:161 RecursivePackageProviderBackedTargetPatternResolver.bulkGetTargetsInPackage
    fn bulk_get_targets_in_package(
        &self,
        original_pattern: &str,
        pkg_ids: &[PackageIdentifier],
        policy: &FilteringPolicy,
    ) -> Result<Vec<(PackageIdentifier, Vec<Target>)>, TargetPatternEvalError> {
        let pkgs = match self
            .recursive_package_provider
            .bulk_get_packages(self.event_handler, pkg_ids)
        {
            Ok(pkgs) => pkgs,
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
        let pkgs: HashMap<&PackageIdentifier, &Arc<Package>> =
            pkgs.iter().map(|(id, pkg)| (id, pkg)).collect();
        if pkgs.len() != pkg_ids.len() {
            let missing: Vec<String> = pkg_ids
                .iter()
                .filter(|id| !pkgs.contains_key(id))
                .map(|id| id.to_string())
                .collect();
            panic!(
                "IllegalStateException: Bulk package retrieval missing results: [{}]",
                missing.join(", ")
            );
        }
        let mut result = Vec::with_capacity(pkg_ids.len());
        for pkg_id in pkg_ids {
            let pkg = pkgs[pkg_id];
            Package::maybe_add_package_contains_errors_event_to_handler(pkg, self.event_handler);
            result.push((pkg_id.clone(), resolve_package_targets(pkg, policy)));
        }
        Ok(result)
    }
}

// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:399 RecursivePackageProviderBackedTargetPatternResolver.calculateSize
fn calculate_size<K, T>(resolved_targets: &[(K, Vec<T>)]) -> usize {
    resolved_targets
        .iter()
        .map(|(_, targets)| targets.len())
        .sum()
}

// Java: com/google/devtools/build/lib/skyframe/RecursivePackageProviderBackedTargetPatternResolver.java:75 RecursivePackageProviderBackedTargetPatternResolver.MAX_PACKAGES_BULK_GET
const MAX_PACKAGES_BULK_GET: usize = 1000;

/// Shards a stream of package identifiers, in order, into fixed-size batches; the final batch
/// may be smaller than the others.
// Java: com/google/devtools/build/lib/skyframe/SimplePackageIdentifierBatchingCallback.java:25 SimplePackageIdentifierBatchingCallback
pub struct SimplePackageIdentifierBatchingCallback<F: FnMut(Vec<PackageIdentifier>)> {
    batch_results: F,
    batch_size: usize,
    package_identifiers: Vec<PackageIdentifier>,
}

impl<F: FnMut(Vec<PackageIdentifier>)> SimplePackageIdentifierBatchingCallback<F> {
    // Java: com/google/devtools/build/lib/skyframe/SimplePackageIdentifierBatchingCallback.java:35 SimplePackageIdentifierBatchingCallback.<init>
    pub fn new(batch_results: F, batch_size: usize) -> Self {
        SimplePackageIdentifierBatchingCallback {
            batch_results,
            batch_size,
            package_identifiers: Vec::with_capacity(batch_size),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SimplePackageIdentifierBatchingCallback.java:43 SimplePackageIdentifierBatchingCallback.process
    pub fn process(&mut self, partial_result: Vec<PackageIdentifier>) {
        for path in partial_result {
            self.package_identifiers.push(path);
            if self.package_identifiers.len() >= self.batch_size {
                self.flush();
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SimplePackageIdentifierBatchingCallback.java:55 SimplePackageIdentifierBatchingCallback.close
    pub fn close(&mut self) {
        self.flush();
    }

    // Java: com/google/devtools/build/lib/skyframe/SimplePackageIdentifierBatchingCallback.java:60 SimplePackageIdentifierBatchingCallback.flush
    fn flush(&mut self) {
        if !self.package_identifiers.is_empty() {
            // reset(): a new builder with the expected size.
            let batch = std::mem::replace(
                &mut self.package_identifiers,
                Vec::with_capacity(self.batch_size),
            );
            (self.batch_results)(batch);
        }
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
                if let Target::OutputFile(output_file) = &target
                    && output_file_generating_rule_class(output_file) == "cc_library"
                {
                    continue;
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
