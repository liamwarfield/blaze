//! Ports of `skyframe/TargetPatternPhaseFunction.java` (with `TargetPatternPhaseValue` and its
//! key), `skyframe/TestsForTargetPatternFunction.java` (+ value and key),
//! `skyframe/TestExpansionFunction.java` (+ value and key), `skyframe/TargetPatternErrorFunction.java`,
//! `pkgcache/TestFilter.java` and `packages/TestTargetUtils.filterTests`.
//!
//! PORT: Java's `ResolvedTargets<Target>` compares targets by identity; Rust `Target`s are not
//! hashable, so these functions keep `ResolvedTargets<Label>` and a label → target map (a label
//! names exactly one target of a loaded package).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::resolved_targets::ResolvedTargets;
use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryMapping, SignedTargetPattern, TargetParsingException,
    TargetPatternParser,
};
use blaze_events::Event;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_options::OptionScalar;
use blaze_packages::attribute::nonconfigurable_attribute_mapper::NonconfigurableAttributeMapper;
use blaze_packages::attribute::{AggregatingAttributeMapper, AttributeMap};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::target::target_utils;
use blaze_packages::target::{RuleOrMacroInstance, Target};
use blaze_packages::types::{AttrValue, Type};
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_sky_loading::recursive_pkg::PackageErrorKey;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::target_pattern_function::{FilteringPolicy, TargetPatternKey};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Key, SkyFunctionException, SkyFunctionName,
    SkyKey, Transience,
};
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::pattern_expanding_error::{ParsingFailedEvent, PatternExpandingError};

/// `SkyFunctions.TARGET_PATTERN_PHASE`.
pub const TARGET_PATTERN_PHASE: SkyFunctionName = SkyFunctionName("TARGET_PATTERN_PHASE");
/// `SkyFunctions.TEST_SUITE_EXPANSION` (the function of `TestsForTargetPatternValue` keys).
pub const TEST_SUITE_EXPANSION: SkyFunctionName = SkyFunctionName("TEST_SUITE_EXPANSION");
/// `SkyFunctions.TESTS_IN_SUITE`.
pub const TESTS_IN_SUITE: SkyFunctionName = SkyFunctionName("TESTS_IN_SUITE");
/// `SkyFunctions.TARGET_PATTERN_ERROR`.
pub const TARGET_PATTERN_ERROR: SkyFunctionName = SkyFunctionName("TARGET_PATTERN_ERROR");

// -------------------------------------------------------------------------------------------
// TestFilter.

/// Predicate that implements test filtering using the command-line options in `LoadingOptions`.
/// Implements `Hash` and `Eq` so it can be used as a Skyframe key.
///
/// PORT: the size and timeout filter sets are also kept in their option string form, which is
/// what `toString` prints (`ImmutableSet.copyOf` of an `EnumSet`: enum order).
// Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:37 TestFilter
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TestFilter {
    test_size_filter_set: String,
    test_timeout_filter_set: String,
    test_tag_filter_list: Vec<String>,
    test_lang_filter_list: Vec<String>,
    test_sizes: BTreeSet<TestSize>,
    test_timeouts: BTreeSet<TestTimeout>,
}

impl TestFilter {
    /// Convert the options into a test filter.
    // Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:44 TestFilter.forOptions
    pub fn for_options(options: &LoadingOptions) -> TestFilter {
        TestFilter {
            test_size_filter_set: if options.test_size_filter_set.is_empty() {
                String::new()
            } else {
                options.test_size_filter_set.java_to_string()
            },
            test_timeout_filter_set: if options.test_timeout_filter_set.is_empty() {
                String::new()
            } else {
                options.test_timeout_filter_set.java_to_string()
            },
            test_tag_filter_list: options.test_tag_filter_list.clone(),
            test_lang_filter_list: options.test_lang_filter_list.clone(),
            test_sizes: options.test_size_filter_set.0.clone(),
            test_timeouts: options.test_timeout_filter_set.0.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:85 TestFilter.apply
    pub fn apply(&self, input: &Target) -> bool {
        // The predicates are and-ed in the order the constructor composes them.
        // Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:58 TestFilter.<init>
        if !self.test_sizes.is_empty() && !test_size_filter(&self.test_sizes)(input) {
            return false;
        }
        if !self.test_timeouts.is_empty() && !test_timeout_filter(&self.test_timeouts)(input) {
            return false;
        }
        if !self.test_tag_filter_list.is_empty()
            && !target_utils::tag_filter(&self.test_tag_filter_list)(input)
        {
            return false;
        }
        if !self.test_lang_filter_list.is_empty()
            && !test_lang_filter(&self.test_lang_filter_list)(input)
        {
            return false;
        }
        true
    }
}

/// Returns a predicate to be used for test size filtering, i.e., that only accepts tests of the
/// given size.
// Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:122 TestFilter.testSizeFilter
pub fn test_size_filter(allowed_sizes: &BTreeSet<TestSize>) -> impl Fn(&Target) -> bool + '_ {
    move |target| match target.as_rule() {
        Some(rule) => test_size(rule).is_some_and(|size| allowed_sizes.contains(&size)),
        None => false,
    }
}

/// `TestSize.getTestSize(Rule)`.
// Java: com/google/devtools/build/lib/packages/TestSize.java:70 TestSize.getTestSize
fn test_size(test_target: &blaze_packages::target::rule::Rule) -> Option<TestSize> {
    match NonconfigurableAttributeMapper::of(test_target).get("size", Type::String) {
        Some(AttrValue::String(attr)) => TestSize::get_test_size(&attr),
        _ => panic!("NullPointerException: size of {}", test_target.get_label()),
    }
}

/// Returns a predicate to be used for test timeout filtering, i.e., that only accepts tests of
/// the given timeout.
// Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:131 TestFilter.testTimeoutFilter
pub fn test_timeout_filter(
    allowed_timeouts: &BTreeSet<TestTimeout>,
) -> impl Fn(&Target) -> bool + '_ {
    move |target| match target.as_rule() {
        Some(rule) => rule
            .get_test_timeout()
            .is_some_and(|timeout| allowed_timeouts.contains(&timeout)),
        None => false,
    }
}

/// Returns a predicate to be used for test language filtering, i.e., that only accepts tests of
/// the specified languages.
// Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:140 TestFilter.testLangFilter
fn test_lang_filter(lang_filter_list: &[String]) -> impl Fn(&Target) -> bool + use<> {
    let mut required_langs: HashSet<String> = HashSet::new();
    let mut excluded_langs: HashSet<String> = HashSet::new();

    for lang in lang_filter_list {
        if let Some(lang) = lang.strip_prefix('-') {
            excluded_langs.insert(lang.to_owned());
        } else {
            required_langs.insert(lang.clone());
        }
    }

    move |rule| {
        // TargetUtils.getRuleLanguage(Target): `((Rule) target).getRuleClass()`.
        let rule = rule
            .as_rule()
            .unwrap_or_else(|| panic!("ClassCastException: {} is not a Rule", rule.get_label()));
        let rule_lang = target_utils::get_rule_language(rule.get_rule_class());
        (required_langs.is_empty() || required_langs.contains(rule_lang))
            && !excluded_langs.contains(rule_lang)
    }
}

impl fmt::Debug for TestFilter {
    // Java: com/google/devtools/build/lib/pkgcache/TestFilter.java:110 TestFilter.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = |s: &str| {
            if s.is_empty() {
                "[]".to_owned()
            } else {
                s.to_owned()
            }
        };
        write!(
            f,
            "TestFilter{{testSizeFilterSet={}, testTimeoutFilterSet={}, testTagFilterList=[{}], testLangFilterList=[{}]}}",
            set(&self.test_size_filter_set),
            set(&self.test_timeout_filter_set),
            self.test_tag_filter_list.join(", "),
            self.test_lang_filter_list.join(", ")
        )
    }
}

/// Filters `tests` (by label) by the tags of the test suite.
// Java: com/google/devtools/build/lib/packages/TestTargetUtils.java:101 TestTargetUtils.filterTests
fn filter_tests(
    test_suite: &blaze_packages::target::rule::Rule,
    tests: &mut IndexMap<Label, Target>,
) {
    let tags_attribute =
        match NonconfigurableAttributeMapper::of(test_suite).get("tags", Type::StringList) {
            Some(AttrValue::StringList(tags)) => tags,
            _ => Vec::new(),
        };
    // Split the tags list into positive and negative tags
    let (positive_tags, negative_tags) = target_utils::sort_tags_by_sense(&tags_attribute);
    tests.retain(|_, t| {
        let rule = t.as_rule().expect("tests are rules");
        test_matches_filters_rule(rule, &positive_tags, &negative_tags)
    });
}

// Java: com/google/devtools/build/lib/packages/TestTargetUtils.java:84 TestTargetUtils.testMatchesFilters
fn test_matches_filters_rule(
    test_target: &blaze_packages::target::rule::Rule,
    required_tags: &HashSet<String>,
    excluded_tags: &HashSet<String>,
) -> bool {
    let non_configurable_attrs = NonconfigurableAttributeMapper::of(test_target);
    let mut test_tags: HashSet<String> = match non_configurable_attrs.get("tags", Type::StringList)
    {
        Some(AttrValue::StringList(tags)) => tags.into_iter().collect(),
        _ => HashSet::new(),
    };
    if let Some(AttrValue::String(size)) = non_configurable_attrs.get("size", Type::String) {
        test_tags.insert(size);
    }
    target_utils::test_matches_filters(&test_tags, required_tags, excluded_tags, true)
}

// -------------------------------------------------------------------------------------------
// TARGET_PATTERN_ERROR.

/// The key of `TargetPatternErrorFunction`.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternErrorFunction.java:44 TargetPatternErrorFunction.Key
#[derive(Clone, PartialEq, Eq)]
pub struct TargetPatternErrorKey {
    exception: TargetParsingException,
}

impl Hash for TargetPatternErrorKey {
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternErrorFunction.java:67 TargetPatternErrorFunction.Key.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.exception.get_message().hash(state);
        self.exception.get_code().hash(state);
    }
}

impl fmt::Debug for TargetPatternErrorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Java: Object.toString of the key class.
        write!(
            f,
            "TargetPatternErrorFunction.Key({})",
            self.exception.get_message()
        )
    }
}

impl TargetPatternErrorKey {
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternErrorFunction.java:39 TargetPatternErrorFunction.key
    pub fn key(e: &TargetParsingException) -> TargetPatternErrorKey {
        TargetPatternErrorKey {
            exception: e.clone(),
        }
    }
}

/// Never computed successfully.
#[derive(Debug)]
pub struct TargetPatternErrorValue;

impl Key for TargetPatternErrorKey {
    type Value = TargetPatternErrorValue;
    const FUNCTION: SkyFunctionName = TARGET_PATTERN_ERROR;
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternErrorFunction.java:91 TargetPatternErrorFunction.compute
fn target_pattern_error_function(
    key: &TargetPatternErrorKey,
    _env: &mut Environment<'_>,
) -> Result<Option<TargetPatternErrorValue>, ComputeError> {
    Err(SkyFunctionException::new(key.exception.clone(), Transience::Persistent).into())
}

// -------------------------------------------------------------------------------------------
// TESTS_IN_SUITE.

/// Key of `TestExpansionValue`: a test_suite label and whether expansion is strict.
// Java: com/google/devtools/build/lib/skyframe/TestExpansionValue.java:60 TestExpansionValue.TestExpansionKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TestExpansionKey {
    label: Label,
    strict: bool,
}

impl TestExpansionKey {
    // Java: com/google/devtools/build/lib/skyframe/TestExpansionValue.java:53 TestExpansionValue.key
    pub fn key(target: &Target, strict: bool) -> TestExpansionKey {
        TestExpansionKey {
            label: target.get_label().clone(),
            strict,
        }
    }
}

impl fmt::Debug for TestExpansionKey {
    // Java: com/google/devtools/build/lib/skyframe/TestExpansionValue.java:81 TestExpansionKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TestsInSuite({}, strict={})", self.label, self.strict)
    }
}

/// The tests a test_suite expands to.
// Java: com/google/devtools/build/lib/skyframe/TestExpansionValue.java:33 TestExpansionValue
#[derive(Clone, Debug)]
pub struct TestExpansionValue {
    labels: ResolvedTargets<Label>,
}

impl TestExpansionValue {
    pub fn get_labels(&self) -> &ResolvedTargets<Label> {
        &self.labels
    }
}

impl Key for TestExpansionKey {
    type Value = TestExpansionValue;
    const FUNCTION: SkyFunctionName = TESTS_IN_SUITE;
}

// Java: com/google/devtools/build/lib/skyframe/TestExpansionFunction.java:52 TestExpansionFunction.compute
fn test_expansion_function(
    expansion: &TestExpansionKey,
    env: &mut Environment<'_>,
) -> Result<Option<TestExpansionValue>, ComputeError> {
    let package_key = PackageKey::key(expansion.label.get_package_identifier().clone());
    let pkg = env.get_value(package_key)?;
    if env.values_missing() {
        return Ok(None);
    }
    let pkg = pkg.expect("not missing");
    let rule = pkg
        .get_package()
        .get_rule(expansion.label.get_name())
        .cloned()
        .unwrap_or_else(|| panic!("NullPointerException: no rule {}", expansion.label));
    let result = compute_expanded_tests(env, &rule, expansion.strict)?;
    if env.values_missing() {
        return Ok(None);
    }
    Ok(Some(TestExpansionValue { labels: result }))
}

/// Populates the result with all the tests associated with the specified `rule`.
///
/// CAUTION! Keep this logic consistent with `TestSuite`!
// Java: com/google/devtools/build/lib/skyframe/TestExpansionFunction.java:75 TestExpansionFunction.computeExpandedTests
fn compute_expanded_tests(
    env: &mut Environment<'_>,
    rule: &Arc<blaze_packages::target::rule::Rule>,
    strict: bool,
) -> Result<ResolvedTargets<Label>, ComputeError> {
    // PORT: java-hash-order. Java collects into a HashSet<Target>; insertion order here.
    let mut result: IndexMap<Label, Target> = IndexMap::new();
    let mut has_error = false;

    let mut prerequisites: Vec<Target> = Vec::new();
    // Note that prerequisites can contain input file targets; the test_suite rule does not
    // restrict the set of targets that can appear in tests or suites.
    has_error |= get_prerequisites(env, rule, "tests", &mut prerequisites)?;

    // 1. Add all tests
    for test in &prerequisites {
        if target_utils::is_test_rule(test) {
            result.insert(test.get_label().clone(), test.clone());
        } else if strict && !target_utils::is_test_suite_rule(test) {
            // If strict mode is enabled, then give an error for any non-test, non-test-suite
            // targets.
            env.listener().handle(&Event::error_at(
                Some(blaze_sky_loading::starlark_util::event_location(
                rule.get_location(),
            )),
                format!(
                    "in test_suite rule '{}': expecting a test or a test_suite rule but '{}' is not one.",
                    rule.get_label(),
                    test.get_label()
                ),
            ));
            has_error = true;
        }
    }

    // 2. Add implicit dependencies on tests in same package, if any.
    let mut implicit_tests: Vec<Target> = Vec::new();
    has_error |= get_prerequisites(env, rule, "$implicit_tests", &mut implicit_tests)?;
    for target in implicit_tests {
        // The Package construction of $implicit_tests ensures that this check never fails, but
        // we add it here anyway for compatibility with future code.
        if target_utils::is_test_rule(&target) {
            result.insert(target.get_label().clone(), target);
        }
    }

    // 3. Filter based on tags, size, env.
    filter_tests(rule, &mut result);

    // 4. Expand all rules recursively, collecting labels.
    let mut labels_builder = ResolvedTargets::<Label>::builder();
    // Don't set filtered targets; they would be removed from the containing test suite.
    labels_builder.merge(&ResolvedTargets::new(
        result.keys().cloned().collect(),
        has_error,
    ));

    for suite in &prerequisites {
        if target_utils::is_test_suite_rule(suite) {
            let Some(value) = env.get_value(TestExpansionKey::key(suite, strict))? else {
                continue;
            };
            labels_builder.merge(value.get_labels());
        }
    }

    Ok(labels_builder.build())
}

/// Adds the set of targets found in the attribute named `attr_name`. Returns true if the method
/// found a problem during the lookup process; the actual error message is reported to the
/// environment.
// Java: com/google/devtools/build/lib/skyframe/TestExpansionFunction.java:154 TestExpansionFunction.getPrerequisites
fn get_prerequisites(
    env: &mut Environment<'_>,
    rule: &Arc<blaze_packages::target::rule::Rule>,
    attr_name: &str,
    targets: &mut Vec<Target>,
) -> Result<bool, ComputeError> {
    let mapper = AggregatingAttributeMapper::of(&**rule);
    let mut labels: Vec<Label> = Vec::new();
    // PORT: java-hash-order (a HashSet of package identifiers; only membership is observable).
    let mut pkg_identifiers: IndexSet<PackageIdentifier> = IndexSet::new();
    mapper.visit_labels(attr_name, &mut |label: &Label| {
        labels.push(label.clone());
        pkg_identifiers.insert(label.get_package_identifier().clone());
    });
    let keys: Vec<SkyKey> = pkg_identifiers
        .iter()
        .map(|p| SkyKey::new(PackageKey::key(p.clone())))
        .collect();
    let packages = env.get_values_and_exceptions(&keys)?;
    if env.values_missing() {
        return Ok(false);
    }
    let mut has_error = false;
    let mut package_map: HashMap<PackageIdentifier, Arc<Package>> = HashMap::new();
    for key in &pkg_identifiers {
        match packages
            .get_or_throw_typed::<PackageKey, NoSuchPackageException>(&PackageKey::key(key.clone()))
        {
            Ok(None) => return Ok(false),
            Ok(Some(package_value)) => {
                package_map.insert(key.clone(), package_value.get_package().clone());
            }
            Err(e) => {
                env.listener().handle(&Event::error(e.get_message()));
                has_error = true;
            }
        }
    }

    for label in labels {
        let Some(pkg) = package_map.get(label.get_package_identifier()) else {
            continue;
        };
        if pkg.contains_errors() {
            has_error = true;
            // Abort the build if --nokeep_going.
            match env.get_value_or_throw::<PackageErrorKey, NoSuchPackageException>(
                PackageErrorKey(label.get_package_identifier().clone()),
            )? {
                Ok(_) => return Ok(false),
                Err(_) => {
                    // PackageErrorFunction always throws this exception, and this fact is used by
                    // Skyframe to abort the build. If we get here, it's either because of error
                    // bubbling or because we're in --keep_going mode. In either case, we *should*
                    // ignore the exception.
                }
            }
        }
        match pkg.get_target(label.get_name()) {
            Ok(t) => targets.push(t.clone()),
            Err(e) => {
                env.listener().handle(&Event::error(e.get_message()));
                has_error = true;
            }
        }
    }
    Ok(has_error)
}

// -------------------------------------------------------------------------------------------
// TEST_SUITE_EXPANSION.

/// Key of `TestsForTargetPatternValue`: the targets of a pattern whose test suites to expand.
// Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternValue.java:52 TestsForTargetPatternValue.TestsForTargetPatternKey
#[derive(Clone, PartialEq, Eq)]
pub struct TestsForTargetPatternKey {
    targets: IndexSet<Label>,
}

impl Hash for TestsForTargetPatternKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Java ImmutableSet.hashCode: order-insensitive.
        let mut sum: u64 = 0;
        for t in &self.targets {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            t.hash(&mut h);
            sum = sum.wrapping_add(h.finish());
        }
        sum.hash(state);
    }
}

impl TestsForTargetPatternKey {
    // Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternValue.java:47 TestsForTargetPatternValue.key
    pub fn key(targets: IndexSet<Label>) -> TestsForTargetPatternKey {
        TestsForTargetPatternKey { targets }
    }
}

impl fmt::Debug for TestsForTargetPatternKey {
    // Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternValue.java:76 TestsForTargetPatternKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ExpandTestSuites([{}])",
            self.targets
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// The tests of the targets of a pattern (test suites expanded).
// Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternValue.java:32 TestsForTargetPatternValue
#[derive(Clone, Debug)]
pub struct TestsForTargetPatternValue {
    labels: ResolvedTargets<Label>,
}

impl TestsForTargetPatternValue {
    pub fn get_labels(&self) -> &ResolvedTargets<Label> {
        &self.labels
    }
}

impl Key for TestsForTargetPatternKey {
    type Value = TestsForTargetPatternValue;
    const FUNCTION: SkyFunctionName = TEST_SUITE_EXPANSION;
}

// Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternFunction.java:45 TestsForTargetPatternFunction.compute
fn tests_for_target_pattern_function(
    expansion: &TestsForTargetPatternKey,
    env: &mut Environment<'_>,
) -> Result<Option<TestsForTargetPatternValue>, ComputeError> {
    let Some(targets) = labels_to_targets(env, &expansion.targets, false)? else {
        return Ok(None);
    };
    let mut tests_in_suites_keys: Vec<SkyKey> = Vec::new();
    for target in targets.targets.values() {
        if target_utils::is_test_suite_rule(target) {
            tests_in_suites_keys.push(SkyKey::new(TestExpansionKey::key(target, true)));
        }
    }
    let tests_in_suites = env.get_values_and_exceptions(&tests_in_suites_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut result: IndexSet<Label> = IndexSet::new();
    let mut has_error = targets.has_error;
    let mut key_index = 0;
    for target in targets.targets.values() {
        if target_utils::is_test_rule(target) {
            result.insert(target.get_label().clone());
        } else if target_utils::is_test_suite_rule(target) {
            let key = &tests_in_suites_keys[key_index];
            key_index += 1;
            let Some(value) = tests_in_suites.get(key) else {
                return Ok(None);
            };
            let value = value
                .downcast::<TestExpansionValue>()
                .unwrap_or_else(|_| panic!("TESTS_IN_SUITE value"));
            result.extend(value.get_labels().get_targets().iter().cloned());
            has_error |= value.get_labels().has_error();
        } else {
            result.insert(target.get_label().clone());
        }
    }
    // We use ResolvedTargets in order to associate an error flag; the result should never
    // contain any filtered targets.
    Ok(Some(TestsForTargetPatternValue {
        labels: ResolvedTargets::new(result, has_error),
    }))
}

/// Targets keyed by label in `labels` order, with an error flag (Java `ResolvedTargets<Target>`
/// without filtered targets).
#[derive(Clone, Debug, Default)]
pub struct ResolvedTargetSet {
    pub targets: IndexMap<Label, Target>,
    pub has_error: bool,
}

/// Returns `None` where Java returns null (values missing).
// Java: com/google/devtools/build/lib/skyframe/TestsForTargetPatternFunction.java:88 TestsForTargetPatternFunction.labelsToTargets
pub fn labels_to_targets(
    env: &mut Environment<'_>,
    labels: &IndexSet<Label>,
    has_error: bool,
) -> Result<Option<ResolvedTargetSet>, ComputeError> {
    let mut pkg_identifiers: IndexSet<PackageIdentifier> = IndexSet::new();
    for label in labels {
        pkg_identifiers.insert(label.get_package_identifier().clone());
    }
    let keys: Vec<SkyKey> = pkg_identifiers
        .iter()
        .map(|p| SkyKey::new(PackageKey::key(p.clone())))
        .collect();
    let packages = env.get_values_and_exceptions(&keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut builder = ResolvedTargetSet {
        targets: IndexMap::new(),
        has_error,
    };
    let mut package_map: HashMap<PackageIdentifier, Arc<Package>> = HashMap::new();
    for (pkg_id, key) in pkg_identifiers.iter().zip(&keys) {
        // Don't bother to check for exceptions - the incoming list should only contain valid
        // targets.
        let Some(packages_value) = packages.get(key) else {
            // PORT: BugReport.sendBugReport skipped (bug reports).
            return Ok(None);
        };
        let packages_value = packages_value
            .downcast::<PackageValue>()
            .unwrap_or_else(|_| panic!("PACKAGE value"));
        package_map.insert(pkg_id.clone(), packages_value.get_package().clone());
    }

    for label in labels {
        let Some(pkg) = package_map.get(label.get_package_identifier()) else {
            continue;
        };
        match pkg.get_target(label.get_name()) {
            Ok(t) => {
                builder.targets.insert(label.clone(), t.clone());
                if pkg.contains_errors() {
                    builder.has_error = true;
                }
            }
            Err(_) => builder.has_error = true,
        }
    }
    Ok(Some(builder))
}

// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:66 TargetPatternPhaseValue.getTargetsFromLabels
fn get_targets_from_labels(
    labels: &IndexSet<Label>,
    event_handler: Arc<dyn blaze_events::ExtendedEventHandler>,
    package_manager: &blaze_sky_loading::package_manager::SkyframePackageManager,
) -> Result<Vec<Target>, blaze_skyframe::Interrupted> {
    use blaze_sky_loading::package_manager::NoSuchPackageOrInterrupted;
    let mut result: Vec<Target> = Vec::with_capacity(labels.len());
    for label in labels {
        let package = match package_manager
            .get_package(event_handler.clone(), label.get_package_identifier())
        {
            Ok(package) => package,
            Err(NoSuchPackageOrInterrupted::Interrupted) => {
                return Err(blaze_skyframe::Interrupted);
            }
            Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => panic!(
                "IllegalStateException: Failed to get preloaded package from \
                     TargetPatternPhaseValue for {label}: {e}"
            ),
        };
        match package.get_target(label.get_name()) {
            // ImmutableSet.Builder: a label appears once in targetLabels.
            Ok(target) => result.push(target.clone()),
            Err(e) => panic!(
                "IllegalStateException: Failed to get preloaded package from \
                 TargetPatternPhaseValue for {label}: {e}"
            ),
        }
    }
    Ok(result)
}

// -------------------------------------------------------------------------------------------
// TARGET_PATTERN_PHASE.

/// Create a target pattern phase value key.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:186 TargetPatternPhaseValue.TargetPatternPhaseKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TargetPatternPhaseKey {
    target_patterns: Vec<String>,
    offset: PathFragment,
    compile_one_dependency: bool,
    build_tests_only: bool,
    determine_tests: bool,
    build_target_filter: Vec<String>,
    build_manual_tests: bool,
    expand_test_suites: bool,
    test_filter: Option<TestFilter>,
}

impl TargetPatternPhaseKey {
    /// Create a target pattern phase value key.
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:142 TargetPatternPhaseValue.key
    pub fn key(
        target_patterns: Vec<String>,
        offset: PathFragment,
        compile_one_dependency: bool,
        build_tests_only: bool,
        determine_tests: bool,
        build_target_filter: Vec<String>,
        build_manual_tests: bool,
        expand_test_suites: bool,
        test_filter: Option<TestFilter>,
    ) -> TargetPatternPhaseKey {
        TargetPatternPhaseKey {
            target_patterns,
            offset,
            compile_one_dependency,
            build_tests_only,
            determine_tests,
            build_target_filter,
            build_manual_tests,
            expand_test_suites,
            test_filter,
        }
    }

    pub fn get_target_patterns(&self) -> &[String] {
        &self.target_patterns
    }

    pub fn get_offset(&self) -> &PathFragment {
        &self.offset
    }

    pub fn get_determine_tests(&self) -> bool {
        self.determine_tests
    }

    pub fn get_build_tests_only(&self) -> bool {
        self.build_tests_only
    }

    pub fn is_expand_test_suites(&self) -> bool {
        self.expand_test_suites
    }
}

impl fmt::Debug for TargetPatternPhaseKey {
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:258 TargetPatternPhaseKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.target_patterns.join(", "))?;
        if !self.offset.is_empty() {
            write!(f, " OFFSET={}", self.offset.get_path_string())?;
        }
        if self.compile_one_dependency {
            f.write_str(" COMPILE_ONE_DEPENDENCY")?;
        }
        if self.build_tests_only {
            f.write_str(" BUILD_TESTS_ONLY")?;
        }
        if self.determine_tests {
            f.write_str(" DETERMINE_TESTS")?;
        }
        if self.expand_test_suites {
            f.write_str(" EXPAND_TEST_SUITES")?;
        }
        if let Some(test_filter) = &self.test_filter {
            write!(f, " {:?}", test_filter)?;
        }
        Ok(())
    }
}

/// A value referring to a computed set of resolved targets. This is used for the results of
/// target pattern parsing.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:44 TargetPatternPhaseValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetPatternPhaseValue {
    target_labels: IndexSet<Label>,
    tests_to_run_labels: Option<IndexSet<Label>>,
    non_expanded_labels: IndexSet<Label>,
    has_error: bool,
    has_post_expansion_error: bool,
}

impl TargetPatternPhaseValue {
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:110 TargetPatternPhaseValue.getTargetLabels
    pub fn get_target_labels(&self) -> &IndexSet<Label> {
        &self.target_labels
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:115 TargetPatternPhaseValue.getTestsToRunLabels
    pub fn get_tests_to_run_labels(&self) -> Option<&IndexSet<Label>> {
        self.tests_to_run_labels.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:96 TargetPatternPhaseValue.getNonExpandedLabels
    pub fn get_non_expanded_labels(&self) -> &IndexSet<Label> {
        &self.non_expanded_labels
    }

    /// The targets of [`Self::get_target_labels`], from the (already loaded) packages.
    /// `Err(Interrupted)` where Java throws `InterruptedException`.
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:84 TargetPatternPhaseValue.getTargets
    pub fn get_targets(
        &self,
        event_handler: Arc<dyn blaze_events::ExtendedEventHandler>,
        package_manager: &blaze_sky_loading::package_manager::SkyframePackageManager,
    ) -> Result<Vec<Target>, blaze_skyframe::Interrupted> {
        get_targets_from_labels(&self.target_labels, event_handler, package_manager)
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:119 TargetPatternPhaseValue.hasError
    pub fn has_error(&self) -> bool {
        self.has_error
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseValue.java:123 TargetPatternPhaseValue.hasPostExpansionError
    pub fn has_post_expansion_error(&self) -> bool {
        self.has_post_expansion_error
    }
}

impl Key for TargetPatternPhaseKey {
    type Value = TargetPatternPhaseValue;
    const FUNCTION: SkyFunctionName = TARGET_PATTERN_PHASE;
}

/// Represents the expansion of a single target pattern.
struct ExpandedPattern {
    pattern: TargetPatternKey,
    resolved_targets: ResolvedTargetSet,
}

/// Takes a list of target patterns corresponding to a command line and turns it into a set of
/// resolved Targets.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:76 TargetPatternPhaseFunction.compute
fn target_pattern_phase_function(
    options: &TargetPatternPhaseKey,
    env: &mut Environment<'_>,
) -> Result<Option<TargetPatternPhaseValue>, ComputeError> {
    let Some(repository_mapping_value) = env.get_value(RepositoryMappingKey::key(MAIN.clone()))?
    else {
        return Ok(None);
    };
    let repository_mapping = repository_mapping_value
        .repository_mapping()
        .cloned()
        .expect("the main repository has a mapping");

    // Determine targets to build:
    let mut failed_patterns: Vec<String> = Vec::new();
    let expanded_patterns =
        get_targets_to_build(env, options, &repository_mapping, &mut failed_patterns)?;
    let targets: Option<MergedTargets> = if env.values_missing() {
        None
    } else {
        Some(merge_all(
            &expanded_patterns,
            !failed_patterns.is_empty(),
            options,
        ))
    };

    // Record labels before they're expanded. For example, if the build requests a test_suite
    // //foo, record //foo here instead of the tests the suite expands to.
    let non_expanded_labels: IndexSet<Label> = match &targets {
        None => IndexSet::new(),
        Some(t) => t.targets.keys().cloned().collect(),
    };

    // If the --build_tests_only option was specified or we want to run tests, we need to
    // determine the list of targets to test. For that, we remove manual tests and apply the
    // command-line filters. Also, if --build_tests_only is specified, then the list of filtered
    // targets will be set as build list as well.
    let mut test_targets: Option<MergedTargets> = None;
    if options.determine_tests || options.build_tests_only {
        test_targets = determine_tests(
            env,
            &options.target_patterns,
            &options.offset,
            &repository_mapping,
            options.test_filter.as_ref(),
        )?;
        blaze_util::check_state!(env.values_missing() || test_targets.is_some());
    }

    let mut test_expansion_keys: IndexMap<Label, SkyKey> = IndexMap::new();
    if let Some(targets) = &targets {
        for target in targets.targets.values() {
            if target_utils::is_test_suite_rule(target) && options.expand_test_suites {
                let label = target.get_label().clone();
                let test_expansion_key =
                    SkyKey::new(TestsForTargetPatternKey::key(IndexSet::from([
                        label.clone()
                    ])));
                test_expansion_keys.insert(label, test_expansion_key);
            }
        }
    }
    let expansion_keys: Vec<SkyKey> = test_expansion_keys.values().cloned().collect();
    let expanded_tests = env.get_values_and_exceptions(&expansion_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    let mut targets = targets.expect("not missing");

    let mut tests_to_run: Option<IndexMap<Label, Target>> = None;

    if let Some(test_targets) = test_targets {
        // Parse the targets to get the tests.
        if test_targets.targets.is_empty() && !test_targets.filtered_targets.is_empty() {
            env.listener().handle(&Event::warn(
                "All specified test targets were excluded by filters",
            ));
        }

        if options.build_tests_only {
            // Replace original targets to build with test targets, so that only targets that are
            // actually going to be built are loaded in the loading phase. Note that this has a
            // side effect that any test_suite target requested to be built is replaced by the set
            // of *_test targets it represents; for example, this affects the status and the
            // summary reports.
            // PORT: the filtered targets only feed TargetParsingCompleteEvent, which the command
            // builds for the build event stream (blaze_runtime commands/bep.rs).
            let has_error = targets.has_error;
            targets = MergedTargets {
                targets: test_targets.targets.clone(),
                filtered_targets: test_targets.filtered_targets.clone(),
                has_error: test_targets.has_error || has_error,
            };
            if options.determine_tests {
                tests_to_run = Some(test_targets.targets.clone());
            }
        } else
        /*if (determineTests)*/
        {
            let tests = test_targets.targets.clone();
            // Merging in all testsToRun guarantees that targets that will be built (because they
            // are tests) are not considered to be "filtered out", even if they were initially
            // filtered out. We can't merge in testTargets because its set of filteredTargets could
            // include targets that we're building but not testing.
            for (label, target) in &tests {
                targets
                    .targets
                    .entry(label.clone())
                    .or_insert(target.clone());
                targets.filtered_targets.shift_remove(label);
            }
            targets.has_error |= test_targets.has_error;
            tests_to_run = Some(tests);
        }
        if let Some(tests_to_run) = &tests_to_run {
            // Note that testsToRun can still be null here, if buildTestsOnly && !shouldRunTests.
            let missing: Vec<String> = tests_to_run
                .keys()
                .filter(|l| !targets.targets.contains_key(*l))
                .map(|l| l.to_string())
                .collect();
            if !missing.is_empty() {
                panic!(
                    "IllegalStateException: Internal consistency check failed; some targets are \
                     scheduled for test execution but not for building ([{}])",
                    missing.join(", ")
                );
            }
        }
    }

    if targets.has_error {
        env.listener()
            .handle(&Event::warn("Target pattern parsing failed."));
    }

    maybe_report_deprecation(env, &targets.targets);

    let mut expanded_labels_builder = ResolvedTargets::<Label>::builder();
    for target in targets.targets.values() {
        let label = target.get_label().clone();
        if target_utils::is_test_suite_rule(target) && options.expand_test_suites {
            let expansion_key = test_expansion_keys
                .get(&label)
                .unwrap_or_else(|| panic!("NullPointerException: {}", label));
            let Some(value) = expanded_tests.get(expansion_key) else {
                return Ok(None);
            };
            let value = value
                .downcast::<TestsForTargetPatternValue>()
                .unwrap_or_else(|_| panic!("TEST_SUITE_EXPANSION value"));
            expanded_labels_builder.merge(value.get_labels());
            // PORT: testSuiteExpansions only feed TargetParsingCompleteEvent (see above).
        } else {
            expanded_labels_builder.add(label);
        }
    }
    let target_labels = expanded_labels_builder.build();
    let Some(expanded_targets) =
        labels_to_targets(env, target_labels.get_targets(), target_labels.has_error())?
    else {
        // PORT: Java dereferences the (null) result here only when values are missing.
        return Ok(None);
    };
    let tests_to_run_labels: Option<IndexSet<Label>> =
        tests_to_run.map(|t| t.keys().cloned().collect());
    // PORT: TargetParsingCompleteEvent is built by the command for the build event stream
    // (blaze_runtime commands/bep.rs); the removed (test suite) labels only feed
    // LoadingPhaseCompleteEvent's filtered set, which no ported subscriber reads.
    let result = TargetPatternPhaseValue {
        target_labels: target_labels.get_targets().clone(),
        tests_to_run_labels,
        non_expanded_labels: if set_equals(&non_expanded_labels, target_labels.get_targets()) {
            target_labels.get_targets().clone()
        } else {
            non_expanded_labels
        },
        has_error: targets.has_error,
        has_post_expansion_error: expanded_targets.has_error,
    };
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:246 TargetPatternPhaseFunction.compute
    env.listener().post(Arc::new(
        blaze_buildtool::buildtool::buildevent::LoadingPhaseCompleteEvent::new(
            result.target_labels.iter().cloned().collect(),
            repository_mapping.clone(),
        ),
    ));
    Ok(Some(result))
}

fn set_equals(a: &IndexSet<Label>, b: &IndexSet<Label>) -> bool {
    a.len() == b.len() && a.iter().all(|l| b.contains(l))
}

/// Emit a warning when a deprecated target is mentioned on the command line.
///
/// Note that this does not stop us from emitting "target X depends on deprecated target Y" style
/// warnings for the same target and it is a good thing; *depending* on a target and *wanting* to
/// build it are different things.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:262 TargetPatternPhaseFunction.maybeReportDeprecation
fn maybe_report_deprecation(env: &mut Environment<'_>, targets: &IndexMap<Label, Target>) {
    for target in targets.values() {
        let Some(rule) = target.as_rule() else {
            continue;
        };
        if rule.is_attribute_value_explicitly_specified("deprecation") {
            let deprecation = match NonconfigurableAttributeMapper::of(&**rule)
                .get("deprecation", Type::String)
            {
                Some(AttrValue::String(s)) => s,
                _ => "null".to_owned(),
            };
            env.listener().handle(&Event::warn_at(
                Some(blaze_sky_loading::starlark_util::event_location(
                    rule.get_location(),
                )),
                format!(
                    "target '{}' is deprecated: {}",
                    rule.get_label(),
                    deprecation
                ),
            ));
        }
    }
}

/// `ResolvedTargets<Target>` with filtered targets.
#[derive(Clone, Debug, Default)]
struct MergedTargets {
    targets: IndexMap<Label, Target>,
    filtered_targets: IndexMap<Label, Target>,
    has_error: bool,
}

/// Interprets the command-line arguments by expanding each pattern to targets and populating the
/// list of `failed_patterns`.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:285 TargetPatternPhaseFunction.getTargetsToBuild
fn get_targets_to_build(
    env: &mut Environment<'_>,
    options: &TargetPatternPhaseKey,
    repo_mapping: &RepositoryMapping,
    failed_patterns: &mut Vec<String>,
) -> Result<Vec<ExpandedPattern>, ComputeError> {
    let parser =
        TargetPatternParser::new(options.offset.clone(), MAIN.clone(), repo_mapping.clone());
    let policy = if options.build_manual_tests {
        FilteringPolicy::NoFilter
    } else {
        FilteringPolicy::FilterManual
    };
    let mut pattern_sky_keys: Vec<TargetPatternKey> =
        Vec::with_capacity(options.target_patterns.len());
    for pattern in &options.target_patterns {
        match SignedTargetPattern::parse(pattern, &parser) {
            Ok(signed) => pattern_sky_keys.push(TargetPatternKey::key(signed, policy.clone())),
            Err(e) => {
                failed_patterns.push(pattern.clone());
                // We post a PatternExpandingError here - the pattern could not be parsed, so we
                // don't even get to run TargetPatternFunction.
                env.listener()
                    .post(Arc::new(PatternExpandingError::failed_term(
                        pattern.clone(),
                        e.get_message(),
                    )));
                // We generally skip patterns that don't parse. We report a parsing failed
                // exception to the event bus here, but not in determineTests below, which goes
                // through the same list. Note that the TargetPatternFunction otherwise reports
                // these events (but only if the target pattern could be parsed successfully).
                env.listener().post(Arc::new(ParsingFailedEvent::new(
                    pattern.clone(),
                    e.get_message(),
                )));
                let _ = env.get_value_or_throw::<TargetPatternErrorKey, TargetParsingException>(
                    TargetPatternErrorKey::key(&e),
                )?;
                // We ignore this. Keep going is active.
                env.listener().handle(&Event::error(format!(
                    "Skipping '{}': {}",
                    pattern,
                    e.get_message()
                )));
            }
        }
    }

    let keys: Vec<SkyKey> = pattern_sky_keys.iter().cloned().map(SkyKey::new).collect();
    let resolved_patterns = env.get_values_and_exceptions(&keys)?;
    let mut expanded_patterns: Vec<ExpandedPattern> = Vec::with_capacity(pattern_sky_keys.len());

    for pattern in &pattern_sky_keys {
        let value = match resolved_patterns
            .get_or_throw_typed::<TargetPatternKey, TargetParsingException>(pattern)
        {
            Ok(v) => v,
            Err(e) => {
                let raw_pattern = pattern.get_pattern().to_owned();
                let error_message = e.get_message().to_owned();
                failed_patterns.push(raw_pattern.clone());
                env.listener()
                    .post(Arc::new(PatternExpandingError::failed_term(
                        raw_pattern.clone(),
                        error_message.clone(),
                    )));
                env.listener().handle(&Event::error(format!(
                    "Skipping '{}': {}",
                    raw_pattern, error_message
                )));
                continue;
            }
        };
        let Some(value) = value else {
            continue;
        };
        // TODO(ulfjack): This is terribly inefficient.
        let Some(as_targets) = labels_to_targets(env, value.get_targets(), value.has_error())?
        else {
            continue;
        };
        expanded_patterns.push(ExpandedPattern {
            pattern: pattern.clone(),
            resolved_targets: as_targets,
        });
    }

    Ok(expanded_patterns)
}

/// Merges expansions from all patterns into a single `ResolvedTargets` instance.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:355 TargetPatternPhaseFunction.mergeAll
fn merge_all(
    expanded_patterns: &[ExpandedPattern],
    has_error: bool,
    options: &TargetPatternPhaseKey,
) -> MergedTargets {
    let mut builder = MergedTargets {
        has_error,
        ..MergedTargets::default()
    };

    for expansion in expanded_patterns {
        if expansion.pattern.is_negative() {
            // builder.filter(Predicates.not(Predicates.in(...)))
            let negative = &expansion.resolved_targets.targets;
            let removed: Vec<Label> = builder
                .targets
                .keys()
                .filter(|l| negative.contains_key(*l))
                .cloned()
                .collect();
            for label in removed {
                if let Some(t) = builder.targets.shift_remove(&label) {
                    builder.filtered_targets.insert(label, t);
                }
            }
        } else {
            // ResolvedTargets.Builder.merge: add the targets, set the error flag.
            for (label, target) in &expansion.resolved_targets.targets {
                builder.targets.insert(label.clone(), target.clone());
                builder.filtered_targets.shift_remove(label);
            }
            builder.has_error |= expansion.resolved_targets.has_error;
        }
    }

    // builder.filter(TargetUtils.tagFilter(options.getBuildTargetFilter()))
    let tag_filter = target_utils::tag_filter(&options.build_target_filter);
    let removed: Vec<Label> = builder
        .targets
        .iter()
        .filter(|(_, t)| !tag_filter(t))
        .map(|(l, _)| l.clone())
        .collect();
    for label in removed {
        if let Some(t) = builder.targets.shift_remove(&label) {
            builder.filtered_targets.insert(label, t);
        }
    }
    if options.compile_one_dependency {
        blaze_util::unported!("CompileOneDependencyTransformer.transformCompileOneDependency");
    }
    builder
}

/// Interpret test target labels from the command-line arguments and return the corresponding set
/// of targets, handling the filter flags, and expanding test suites.
// Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:408 TargetPatternPhaseFunction.determineTests
fn determine_tests(
    env: &mut Environment<'_>,
    target_patterns: &[String],
    offset: &PathFragment,
    repo_mapping: &RepositoryMapping,
    test_filter: Option<&TestFilter>,
) -> Result<Option<MergedTargets>, ComputeError> {
    let parser = TargetPatternParser::new(offset.clone(), MAIN.clone(), repo_mapping.clone());
    let mut pattern_sky_keys: Vec<TargetPatternKey> = Vec::new();
    for pattern in target_patterns {
        if let Ok(signed) = SignedTargetPattern::parse(pattern, &parser) {
            pattern_sky_keys.push(TargetPatternKey::key(signed, FilteringPolicy::FilterTests));
        }
        // Skip.
    }
    let keys: Vec<SkyKey> = pattern_sky_keys.iter().cloned().map(SkyKey::new).collect();
    let resolved_patterns = env.get_values_and_exceptions(&keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut expanded_suite_keys: Vec<SkyKey> = Vec::new();
    for key in &pattern_sky_keys {
        match resolved_patterns.get_or_throw_typed::<TargetPatternKey, TargetParsingException>(key)
        {
            Ok(Some(value)) => {
                expanded_suite_keys.push(SkyKey::new(TestsForTargetPatternKey::key(
                    value.get_targets().clone(),
                )));
            }
            // PORT: BugReport.sendBugReport skipped (bug reports).
            Ok(None) => return Ok(None),
            // Skip.
            Err(_) => continue,
        }
    }
    let expanded_suites = env.get_values_and_exceptions(&expanded_suite_keys)?;
    if env.values_missing() {
        return Ok(None);
    }

    let mut test_targets_builder = MergedTargets::default();
    let mut suite_key_index = 0;
    for pattern in &pattern_sky_keys {
        match resolved_patterns
            .get_or_throw_typed::<TargetPatternKey, TargetParsingException>(pattern)
        {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(None),
            // This was already reported in getTargetsToBuild (maybe merge the two code paths?).
            Err(_) => continue,
        }

        let key = &expanded_suite_keys[suite_key_index];
        suite_key_index += 1;
        let Some(expanded_suites_value) = expanded_suites.get(key) else {
            // PORT: BugReport.logUnexpected skipped (bug reports).
            return Ok(None);
        };
        let expanded_suites_value = expanded_suites_value
            .downcast::<TestsForTargetPatternValue>()
            .unwrap_or_else(|_| panic!("TEST_SUITE_EXPANSION value"));
        let Some(resolved) = labels_to_targets(
            env,
            expanded_suites_value.get_labels().get_targets(),
            expanded_suites_value.get_labels().has_error(),
        )?
        else {
            return Ok(None);
        };
        if pattern.is_negative() {
            let removed: Vec<Label> = test_targets_builder
                .targets
                .keys()
                .filter(|l| resolved.targets.contains_key(*l))
                .cloned()
                .collect();
            for label in removed {
                if let Some(t) = test_targets_builder.targets.shift_remove(&label) {
                    test_targets_builder.filtered_targets.insert(label, t);
                }
            }
            test_targets_builder.has_error |= resolved.has_error;
        } else {
            for (label, target) in resolved.targets {
                test_targets_builder.targets.insert(label, target);
            }
            test_targets_builder.has_error |= resolved.has_error;
        }
    }

    if let Some(test_filter) = test_filter {
        let removed: Vec<Label> = test_targets_builder
            .targets
            .iter()
            .filter(|(_, t)| !test_filter.apply(t))
            .map(|(l, _)| l.clone())
            .collect();
        for label in removed {
            if let Some(t) = test_targets_builder.targets.shift_remove(&label) {
                test_targets_builder.filtered_targets.insert(label, t);
            }
        }
    }
    Ok(Some(test_targets_builder))
}

/// Registers TARGET_PATTERN_PHASE, TEST_SUITE_EXPANSION, TESTS_IN_SUITE and
/// TARGET_PATTERN_ERROR (Java `SkyframeExecutor.skyFunctions()` entries).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
pub fn register_target_pattern_phase_functions(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .typed_function::<TargetPatternPhaseKey, _>(target_pattern_phase_function)
        .typed_function::<TestsForTargetPatternKey, _>(tests_for_target_pattern_function)
        .typed_function::<TestExpansionKey, _>(test_expansion_function)
        .typed_function::<TargetPatternErrorKey, _>(target_pattern_error_function)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_pattern_phase_key_to_string() {
        let key = TargetPatternPhaseKey::key(
            vec!["//...".to_owned()],
            PathFragment::empty_fragment(),
            false,
            false,
            false,
            Vec::new(),
            true,
            false,
            None,
        );
        assert_eq!(format!("{:?}", key), "[//...]");
    }

    #[test]
    fn target_pattern_phase_key_to_string_with_flags() {
        let key = TargetPatternPhaseKey::key(
            vec!["//:all".to_owned(), "//python:all".to_owned()],
            PathFragment::create("foo"),
            false,
            true,
            true,
            Vec::new(),
            false,
            true,
            None,
        );
        assert_eq!(
            format!("{:?}", key),
            "[//:all, //python:all] OFFSET=foo BUILD_TESTS_ONLY DETERMINE_TESTS EXPAND_TEST_SUITES"
        );
    }
}
