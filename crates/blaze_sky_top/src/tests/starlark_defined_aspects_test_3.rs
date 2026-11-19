//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java`
//! (an `AnalysisTestCase`), part 3 of 3: the `@Test` methods 123 to 182 in file order
//! (`testTopLevelAspectRequiresAspect_aspectRequiredByMultipleAspects` through
//! `starlarkDefinedAspectCodec`). Parts 1 and 2 are `starlark_defined_aspects_test_1.rs` and
//! `starlark_defined_aspects_test_2.rs`; each file keeps its own copy of the helpers it needs.
//!
//! Not ported here:
//!
//! - `exposeNativeAspectToStarlark` (registers `TestAspects` native aspects and rules on a
//!   `TestRuleClassProvider`): no test of this part uses it, and the harness has no
//!   `TestRuleClassProvider` (see `crate::testing::build_view_test_case`).
//! - The nested `WithKeepGoing` subclass reruns every test with `Flag.KEEP_GOING` and
//!   `keepGoing() == true`. The tests are not duplicated; [`StarlarkDefinedAspectsTest::keep_going`]
//!   is Java's `keepGoing()` of the base class (`false`), and every test keeps Java's
//!   `if (keepGoing())` branch.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use blaze_analysis::aspect_key_creator::{AspectKey, TopLevelAspectsKey};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::provider_map::{ProviderInstance, TransitiveInfoProviderMap};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_events::EventKind;
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_options::Options;
use blaze_packages::aspect::AspectClass;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::aspect::StarlarkDefinedAspect;
use blaze_sky_loading::bzl_load::{BzlLoadKey, BzlLoadValue};
use blaze_skyframe::WalkableGraph;
use blaze_skyframe::{SkyKey, SkyValue};
use blaze_starlark::structs::StarlarkInfo;
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use starlark::values::OwnedFrozenValue;
use starlark::values::list::ListRef;

use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::TargetPatternPhaseError;
use crate::testing::build_view_test_case::{LOADING_PHASE_THREADS, parse_label};
use crate::testing::{
    AnalysisTestCase, BuildViewTestCase, assert_contains_exactly, assert_contains_exactly_in_order,
    has_error,
};

/// What Java's `AnalysisTestCase.update` throws.
#[derive(Debug)]
enum UpdateError {
    /// `TargetParsingException` (loading of the target patterns failed).
    // Read through `Debug` in failure messages.
    TargetParsing(#[allow(dead_code)] String),
    /// What `BuildView.update` throws (`ViewCreationFailedException`, ...).
    BuildView(BuildViewError),
}

/// Java `StarlarkDefinedAspectsTest` (an `AnalysisTestCase`).
///
/// PORT: the harness's fail-fast check is private to it and its `update` takes no aspect
/// parameters, so this file updates through its own [`StarlarkDefinedAspectsTest::update_with`]
/// (Java `AnalysisTestCase.update(eventBus, config, explicitTargetPatterns, aspects,
/// aspectsParameters, labels...)`) and tracks whether the test removed the fail-fast handler
/// (`fail_fast`), failing on the first error event as Java's `failFastHandler` does.
struct StarlarkDefinedAspectsTest {
    t: AnalysisTestCase,
    fail_fast: bool,
}

impl Deref for StarlarkDefinedAspectsTest {
    type Target = AnalysisTestCase;
    fn deref(&self) -> &AnalysisTestCase {
        &self.t
    }
}

impl DerefMut for StarlarkDefinedAspectsTest {
    fn deref_mut(&mut self) -> &mut AnalysisTestCase {
        &mut self.t
    }
}

impl StarlarkDefinedAspectsTest {
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:167 AnalysisTestCase.createMocks
    fn new() -> StarlarkDefinedAspectsTest {
        StarlarkDefinedAspectsTest {
            t: BuildViewTestCase::analysis_test_case(),
            fail_fast: true,
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:88 StarlarkDefinedAspectsTest.keepGoing
    fn keep_going(&self) -> bool {
        false
    }

    /// `reporter.removeHandler(failFastHandler)`.
    fn remove_fail_fast_handler(&mut self) {
        self.t.remove_fail_fast_handler();
        self.fail_fast = false;
    }

    /// `update(labels...)`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:470 AnalysisTestCase.update
    fn update(&mut self, labels: &[&str]) -> Result<AnalysisResult, UpdateError> {
        self.update_aspects_with_params(&[], &[], labels)
    }

    /// `update(aspects, labels...)`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:474 AnalysisTestCase.update
    fn update_aspects(
        &mut self,
        aspects: &[&str],
        labels: &[&str],
    ) -> Result<AnalysisResult, UpdateError> {
        self.update_aspects_with_params(aspects, &[], labels)
    }

    /// `update(aspects, aspectsParameters, labels...)`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:487 AnalysisTestCase.update
    fn update_aspects_with_params(
        &mut self,
        aspects: &[&str],
        aspects_parameters: &[(&str, &str)],
        labels: &[&str],
    ) -> Result<AnalysisResult, UpdateError> {
        let keep_going = self
            .t
            .default_flags()
            .contains(crate::testing::Flag::KeepGoing);
        let result = self.update_with(keep_going, aspects, aspects_parameters, labels);
        self.check_fail_fast();
        result
    }

    /// `update(eventBus, config, explicitTargetPatterns, aspects, aspectsParameters, labels...)`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:364 AnalysisTestCase.update
    fn update_with(
        &mut self,
        keep_going: bool,
        aspects: &[&str],
        aspects_parameters: &[(&str, &str)],
        labels: &[&str],
    ) -> Result<AnalysisResult, UpdateError> {
        // Java: update(String...) posts to a new EventBus.
        self.t.event_bus.clear();
        // Java: invalidateFilesUnderPathForTesting(EVERYTHING_MODIFIED).
        self.t.invalidate_packages();
        let loading_options: LoadingOptions = Options::<LoadingOptions>::get_defaults();
        let analysis_options: AnalysisOptions = Options::<AnalysisOptions>::get_defaults();
        let build_options = self.t.get_target_configuration().get_options().clone();
        let handler = self.t.event_handler();
        let executor = self.t.skyframe_executor();
        let loading_result = match executor.load_target_patterns_with_filters(
            handler.clone(),
            &labels.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &PathFragment::create(""),
            &loading_options,
            LOADING_PHASE_THREADS,
            keep_going,
            /* determine_tests= */ false,
        ) {
            Ok(v) => v,
            Err(TargetPatternPhaseError::TargetParsing(e)) => {
                return Err(UpdateError::TargetParsing(e.get_message().to_string()));
            }
            Err(TargetPatternPhaseError::Interrupted) => {
                return Err(UpdateError::BuildView(BuildViewError::Interrupted));
            }
        };
        let options = BuildViewUpdateOptions {
            keep_going,
            check_for_action_conflicts: true,
            aspects: aspects.iter().map(|s| s.to_string()).collect(),
            explicit_target_patterns: IndexSet::new(),
            skip_incompatible_explicit_targets: analysis_options.skip_incompatible_explicit_targets,
            report_incompatible_targets: true,
            include_execution_phase: false,
        };
        let aspects_parameters: IndexMap<String, String> = aspects_parameters
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        build_view::update_with_aspects_parameters(
            executor,
            &loading_result,
            &build_options,
            &options,
            &aspects_parameters,
            handler,
        )
        .map_err(UpdateError::BuildView)
    }

    /// Java's `failFastHandler`: the first error event fails the test.
    fn check_fail_fast(&self) {
        if !self.fail_fast {
            return;
        }
        for event in self.t.event_collector.iter() {
            if EventKind::ERRORS.contains(event.get_kind()) {
                panic!("{event}");
            }
        }
    }

    /// `getStarlarkProvider(target, providerSymbol)`: the provider is defined in the .bzl file
    /// of the target's rule.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:702 AnalysisTestCase.getStarlarkProvider
    fn get_starlark_provider_of_target(
        &mut self,
        target: &ConfiguredTarget,
        provider_symbol: &str,
    ) -> OwnedFrozenValue {
        let rule_target = self
            .t
            .get_target(&target.get_label().to_string())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let bzl = rule_target
            .get_associated_rule()
            .expect("a rule")
            .get_rule_class_object()
            .get_rule_definition_environment_label()
            .expect("a Starlark rule")
            .to_string();
        target
            .get(&starlark_provider_key(&bzl, provider_symbol))
            .unwrap_or_else(|| panic!("NullPointerException: no {provider_symbol} in {target:?}"))
    }

    /// The keys of the done `ASPECT` nodes of aspect `aspect_label` on target `target_label`.
    // Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10238 StarlarkDefinedAspectsTest.getAspectKeys
    fn get_aspect_keys(&mut self, target_label: &str, aspect_label: &str) -> Vec<AspectKey> {
        let evaluator = self.t.get_evaluator();
        evaluator
            .get_all_keys_for_testing()
            .iter()
            .filter(|key| evaluator.get_existing_value(key).is_some())
            .filter_map(|key| key.downcast::<AspectKey>())
            .filter(|aspect_key| {
                aspect_key.get_aspect_class().get_name() == aspect_label
                    && aspect_key.get_label().to_string() == target_label
            })
            .cloned()
            .collect()
    }

    /// `skyframeExecutor.getDoneSkyValueForIntrospection(key)`.
    // Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10322 StarlarkDefinedAspectsTest.getDoneValue
    fn get_done_value(&mut self, key: &SkyKey) -> SkyValue {
        self.t
            .get_evaluator()
            .get_existing_value(key)
            .unwrap_or_else(|| panic!("AssertionError: no done value for {key:?}"))
    }

    /// Returns the only node whose key matches the given predicate, or `None` if there is none.
    ///
    /// PORT: Java returns the `InMemoryNodeEntry`; this returns its key, and
    /// [`Self::get_direct_deps`] / [`Self::get_aspect_value`] read the entry.
    // Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10337 StarlarkDefinedAspectsTest.findOnlyNodeEntry
    fn find_only_node_entry(&mut self, predicate: impl Fn(&SkyKey) -> bool) -> Option<SkyKey> {
        let matching_entries: Vec<SkyKey> = self
            .t
            .get_evaluator()
            .get_all_keys_for_testing()
            .into_iter()
            .filter(|key| predicate(key))
            .collect();
        assert!(
            matching_entries.len() <= 1,
            "Found multiple entries: {:?}",
            matching_entries
                .iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
        );
        matching_entries.into_iter().next()
    }

    /// `InMemoryNodeEntry.getDirectDeps()`.
    fn get_direct_deps(&mut self, key: &SkyKey) -> Vec<SkyKey> {
        let evaluator = self.t.get_evaluator();
        WalkableGraph::get_direct_deps(&*evaluator, key)
    }

    /// `(AspectValue) InMemoryNodeEntry.getValue()`.
    fn get_aspect_value(&mut self, key: &SkyKey) -> Arc<AspectValue> {
        self.get_done_value(key)
            .downcast::<AspectValue>()
            .unwrap_or_else(|_| panic!("ClassCastException: {key:?} is not an AspectValue"))
    }
}

/// `assertThrows(ViewCreationFailedException.class, () -> update(...))`.
fn assert_throws_view_creation_failed(result: Result<AnalysisResult, UpdateError>) {
    match result {
        Err(UpdateError::BuildView(BuildViewError::ViewCreationFailed { .. })) => {}
        Err(e) => panic!("expected ViewCreationFailedException, got {e:?}"),
        Ok(_) => panic!("expected ViewCreationFailedException, but update succeeded"),
    }
}

/// `assertThrows(TargetParsingException.class, () -> update(...))`.
fn assert_throws_target_parsing(result: Result<AnalysisResult, UpdateError>) {
    match result {
        Err(UpdateError::TargetParsing(_)) => {}
        Err(e) => panic!("expected TargetParsingException, got {e:?}"),
        Ok(_) => panic!("expected TargetParsingException, but update succeeded"),
    }
}

/// `new StarlarkProvider.Key(keyForBuild(Label.parseCanonical(bzl)), symbol)`.
fn starlark_provider_key(bzl: &str, provider_symbol: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: parse_label(bzl),
        exported_name: provider_symbol.to_string(),
    }
}

/// `getStarlarkProvider(ProviderCollection, label, providerSymbol)` on a configured aspect.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:717 AnalysisTestCase.getStarlarkProvider
fn get_starlark_provider(
    target: &ConfiguredAspect,
    label: &str,
    provider_symbol: &str,
) -> OwnedFrozenValue {
    target
        .get(&starlark_provider_key(label, provider_symbol))
        .unwrap_or_else(|| panic!("NullPointerException: no {provider_symbol}"))
}

/// `((StructImpl) info).getValue(name)` as a `String` (`getValue(name, String.class)`).
fn get_value_string(info: &OwnedFrozenValue, name: &str) -> String {
    let v = info.value();
    let s = StarlarkInfo::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a StructImpl"));
    let field = s
        .get_field(name)
        .unwrap_or_else(|| panic!("no field '{name}' in {v}"));
    field
        .unpack_str()
        .unwrap_or_else(|| panic!("ClassCastException: {field} is not a String"))
        .to_string()
}

/// `(Sequence<?>) ((StructImpl) info).getValue(name)` (`Starlark.toIterable`) as strings.
fn get_value_strings(info: &OwnedFrozenValue, name: &str) -> Vec<String> {
    let v = info.value();
    let s = StarlarkInfo::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a StructImpl"));
    let field = s
        .get_field(name)
        .unwrap_or_else(|| panic!("no field '{name}' in {v}"));
    let list = ListRef::from_value(field)
        .unwrap_or_else(|| panic!("ClassCastException: {field} is not a StarlarkList"));
    list.iter()
        .map(|e| {
            e.unpack_str()
                .unwrap_or_else(|| panic!("{e} is not a String"))
                .to_string()
        })
        .collect()
}

/// The configured aspect of the Starlark aspect exported as `aspect_name`.
// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10249 StarlarkDefinedAspectsTest.getConfiguredAspect
fn get_configured_aspect(
    aspects_map: &IndexMap<AspectKey, Arc<AspectValue>>,
    aspect_name: &str,
) -> Option<ConfiguredAspect> {
    for (key, value) in aspects_map {
        if let AspectClass::Starlark(starlark_aspect_class) = key.get_aspect_class() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            if aspect_exported_name == aspect_name {
                return Some(value.get_configured_object());
            }
        }
    }
    None
}

/// `Iterables.getOnlyElement`.
fn get_only_element<T>(items: &[T]) -> &T {
    assert_eq!(items.len(), 1, "expected one element");
    &items[0]
}

/// `assertThat(actual).containsAtLeast(expected...)`.
fn assert_contains_at_least(actual: &[String], expected: &[&str]) {
    for e in expected {
        assert!(
            actual.iter().any(|a| a == e),
            "containsAtLeast: {e} missing from {actual:?}"
        );
    }
}

/// `assertThat(a).isEqualTo(b)` for `TransitiveInfoProviderMap`s (Java
/// `ImmutableSharedKeyMap.equals`: the same keys in the same order and equal values; the values
/// compared here are the same objects).
fn assert_provider_maps_equal(a: &TransitiveInfoProviderMap, b: &TransitiveInfoProviderMap) {
    let a: Vec<_> = a.entries().collect();
    let b: Vec<_> = b.entries().collect();
    assert_eq!(a.len(), b.len(), "{a:?} != {b:?}");
    for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
        assert_eq!(ka, kb);
        let same = match (va, vb) {
            (ProviderInstance::Info(x), ProviderInstance::Info(y))
            | (ProviderInstance::Legacy(x), ProviderInstance::Legacy(y)) => {
                x.value().ptr_eq(y.value())
            }
            (ProviderInstance::Native(x), ProviderInstance::Native(y)) => Arc::ptr_eq(x, y),
            _ => false,
        };
        assert!(same, "{ka:?}: {va:?} != {vb:?}");
    }
}

/// `scratch.file(...).setExecutable(true)`.
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let p = path.get_path_string();
    let mut perms = std::fs::metadata(p).expect("stat").permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(p, perms).expect("chmod");
}

/// `RoundTripping.roundTripWithSkyframe(this::getDoneValue, value)`.
fn round_trip_with_skyframe(_value: &OwnedFrozenValue) -> OwnedFrozenValue {
    blaze_util::unported!("RoundTripping.roundTripWithSkyframe")
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10066 StarlarkDefinedAspectsTest.writeAspectOnAliasTestFiles
fn write_aspect_on_alias_test_files(f: &StarlarkDefinedAspectsTest) {
    f.scratch.file(
        "test/config_setting/BUILD",
        &["config_setting(name='defines', values={'define': 'foo=1'})"],
    );
    f.scratch.file(
        "test/defs.bzl",
        &[r#"p1 = provider()
AspectInfo = provider()

def _a_impl(target, ctx):
  return [AspectInfo(aspect_a_result = 'target {}, p1.val = {}'.format(target.label,
      target[p1].val))]

a = aspect(
  implementation = _a_impl,
  attr_aspects = ['dep'],
  required_providers = [p1],
)

def _rule_impl(ctx):
  return [p1(val = 'v1')]

rule_with_p1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
  },
  provides = [p1],
)

rule_without_p1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            r#"load('//test:defs.bzl', 'rule_with_p1', 'rule_without_p1')
alias(
    name = 'alias_target',
    actual = select({
                      "//test/config_setting:defines": ":t1",
                      "//conditions:default": ":t2",
                      })
        )
rule_with_p1(name = 't1')
rule_without_p1(name = 't2')
"#,
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6784 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_aspectRequiredByMultipleAspects
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_binary from the mock rules_cc)"]
fn test_top_level_aspect_requires_aspect_aspect_required_by_multiple_aspects() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   return []
aspect_c = aspect(implementation = _impl)
aspect_b = aspect(implementation = _impl, requires = [aspect_c])
aspect_a = aspect(implementation = _impl, requires = [aspect_c])
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            "cc_binary(name = 'main_target')",
        ],
    );
    let analysis_result = f
        .update_aspects(
            &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    assert_eq!(configured_aspects.len(), 3);
    assert!(get_configured_aspect(configured_aspects, "aspect_a").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_b").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_c").is_some());
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6812 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_aspectRequiredByMultipleAspects2
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_binary from the mock rules_cc)"]
fn test_top_level_aspect_requires_aspect_aspect_required_by_multiple_aspects2() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   return []
aspect_d = aspect(implementation = _impl)
aspect_c = aspect(implementation = _impl, requires = [aspect_d])
aspect_b = aspect(implementation = _impl, requires = [aspect_d])
aspect_a = aspect(implementation = _impl, requires = [aspect_b, aspect_c])
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            "cc_binary(name = 'main_target')",
        ],
    );
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%aspect_a"], &["//test:main_target"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    assert_eq!(configured_aspects.len(), 4);
    assert!(get_configured_aspect(configured_aspects, "aspect_a").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_b").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_c").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_d").is_some());
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6840 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_requireExistingAspect_passed
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_binary from the mock rules_cc)"]
fn test_top_level_aspect_requires_aspect_require_existing_aspect_passed() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   return []
aspect_b = aspect(implementation = _impl)
aspect_a = aspect(implementation = _impl, requires = [aspect_b])
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            "cc_binary(name = 'main_target')",
        ],
    );
    let analysis_result = f
        .update_aspects(
            &["test/defs.bzl%aspect_b", "test/defs.bzl%aspect_a"],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    assert_eq!(configured_aspects.len(), 2);
    assert!(get_configured_aspect(configured_aspects, "aspect_a").is_some());
    assert!(get_configured_aspect(configured_aspects, "aspect_b").is_some());
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6866 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_requireExistingAspect_failed
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_binary from the mock rules_cc)"]
fn test_top_level_aspect_requires_aspect_require_existing_aspect_failed() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   return []
aspect_b = aspect(implementation = _impl)
aspect_a = aspect(implementation = _impl, requires = [aspect_b])
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            "cc_binary(name = 'main_target')",
        ],
    );
    f.remove_fail_fast_handler();

    // The call to `update` does not throw an exception when "--keep_going" is passed in the
    // WithKeepGoing test suite. Otherwise, it throws ViewCreationFailedException.
    if f.keep_going() {
        let result = f
            .update_aspects(
                &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects(
            &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "aspect //test:defs.bzl%aspect_b was added before as a required aspect of aspect \
         //test:defs.bzl%aspect_a",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6903 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_ruleAttributes
#[test]
fn test_top_level_aspect_requires_aspect_rule_attributes() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"RequiredAspectProv = provider()
BaseAspectProv = provider()

def _required_aspect_impl(target, ctx):
  p_val = ['In required_aspect, p = {} on target {}'
              .format(ctx.rule.attr.p, target.label)]
  if ctx.rule.attr.dep and RequiredAspectProv in ctx.rule.attr.dep:
    p_val += ctx.rule.attr.dep[RequiredAspectProv].p_val
  return [RequiredAspectProv(p_val = p_val)]
required_aspect = aspect(
  implementation = _required_aspect_impl,
)

def _base_aspect_impl(target, ctx):
  p_val = []
  p_val += target[RequiredAspectProv].p_val
  p_val += ['In base_aspect, p = {} on target {}'.format(ctx.rule.attr.p, target.label)]
  if ctx.rule.attr.dep:
    p_val += ctx.rule.attr.dep[BaseAspectProv].p_val
  return [BaseAspectProv(p_val = p_val)]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  requires = [required_aspect],
)

def _rule_impl(ctx):
  pass

my_rule = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
    'p' : attr.string(values = ['main_val', 'dep_val']),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target',
  p = 'main_val',
)
my_rule(
  name = 'dep_target',
  p = 'dep_val',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%base_aspect"], &["//test:main_target"])
        .expect("update");

    // required_aspect can only run on main_target when propagated alone since its attr_aspects is
    // empty.
    let configured_aspects = analysis_result.get_aspects_map();
    let required_aspect = get_configured_aspect(configured_aspects, "required_aspect");
    assert!(required_aspect.is_some());
    let required_aspect_prov = starlark_provider_key("//test:defs.bzl", "RequiredAspectProv");
    let required_aspect_provider = required_aspect
        .unwrap()
        .get(&required_aspect_prov)
        .expect("NullPointerException: RequiredAspectProv");
    assert_contains_exactly(
        &get_value_strings(&required_aspect_provider, "p_val"),
        &["In required_aspect, p = main_val on target @@//test:main_target"],
    );

    // base_aspect can run on main_target and dep_target and it can also see the providers created
    // by running required_target on them.
    let base_aspect = get_configured_aspect(configured_aspects, "base_aspect");
    assert!(base_aspect.is_some());
    let base_aspect_prov = starlark_provider_key("//test:defs.bzl", "BaseAspectProv");
    let base_aspect_provider = base_aspect
        .unwrap()
        .get(&base_aspect_prov)
        .expect("NullPointerException: BaseAspectProv");
    assert_contains_exactly(
        &get_value_strings(&base_aspect_provider, "p_val"),
        &[
            "In base_aspect, p = dep_val on target @@//test:dep_target",
            "In base_aspect, p = main_val on target @@//test:main_target",
            "In required_aspect, p = dep_val on target @@//test:dep_target",
            "In required_aspect, p = main_val on target @@//test:main_target",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:6991 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_inheritPropagationAttributes
#[test]
fn test_top_level_aspect_requires_aspect_inherit_propagation_attributes() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"BaseAspectProv = provider()
FirstRequiredAspectProv = provider()
SecondRequiredAspectProv = provider()

def _second_required_aspect_impl(target, ctx):
  result = []
  if getattr(ctx.rule.attr, 'second_dep'):
    result += getattr(ctx.rule.attr, 'second_dep')[SecondRequiredAspectProv].result
  result += ['second_required_aspect run on target {}'.format(target.label)]
  return [SecondRequiredAspectProv(result = result)]
second_required_aspect = aspect(
  implementation = _second_required_aspect_impl,
  attr_aspects = ['second_dep'],
)

def _first_required_aspect_impl(target, ctx):
  result = []
  result += target[SecondRequiredAspectProv].result
  if getattr(ctx.rule.attr, 'first_dep'):
    result += getattr(ctx.rule.attr, 'first_dep')[FirstRequiredAspectProv].result
  result += ['first_required_aspect run on target {}'.format(target.label)]
  return [FirstRequiredAspectProv(result = result)]
first_required_aspect = aspect(
  implementation = _first_required_aspect_impl,
  attr_aspects = ['first_dep'],
  requires = [second_required_aspect],
)

def _base_aspect_impl(target, ctx):
  result = []
  result += target[FirstRequiredAspectProv].result
  if getattr(ctx.rule.attr, 'base_dep'):
    result += getattr(ctx.rule.attr, 'base_dep')[BaseAspectProv].result
  result += ['base_aspect run on target {}'.format(target.label)]
  return [BaseAspectProv(result = result)]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['base_dep'],
  requires = [first_required_aspect],
)

def _my_rule_impl(ctx):
  pass

my_rule = rule(
  implementation = _my_rule_impl,
  attrs = {
    'base_dep': attr.label(),
    'first_dep': attr.label(),
    'second_dep': attr.label()
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  base_dep = ':base_dep_target',
  first_dep = ':first_dep_target',
  second_dep = ':second_dep_target',
)
my_rule(
  name = 'base_dep_target',
)
my_rule(
  name = 'first_dep_target',
)
my_rule(
  name = 'second_dep_target',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%base_aspect"], &["//test:main_target"])
        .expect("update");

    // base_aspect should propagate only along its attr_aspects: 'base_dep'
    // first_required_aspect should propagate along 'base_dep' and 'first_dep'
    // second_required_aspect should propagate along 'base_dep', 'first_dep' and 'second_dep'
    let configured_aspects = analysis_result.get_aspects_map();
    let base_aspect = get_configured_aspect(configured_aspects, "base_aspect");
    assert!(base_aspect.is_some());
    let base_aspect_prov = starlark_provider_key("//test:defs.bzl", "BaseAspectProv");
    let base_aspect_provider = base_aspect
        .unwrap()
        .get(&base_aspect_prov)
        .expect("NullPointerException: BaseAspectProv");
    assert_contains_exactly(
        &get_value_strings(&base_aspect_provider, "result"),
        &[
            "second_required_aspect run on target @@//test:second_dep_target",
            "second_required_aspect run on target @@//test:main_target",
            "second_required_aspect run on target @@//test:first_dep_target",
            "second_required_aspect run on target @@//test:base_dep_target",
            "first_required_aspect run on target @@//test:first_dep_target",
            "first_required_aspect run on target @@//test:main_target",
            "first_required_aspect run on target @@//test:base_dep_target",
            "base_aspect run on target @@//test:base_dep_target",
            "base_aspect run on target @@//test:main_target",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7099 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_inheritRequiredProviders
#[test]
fn test_top_level_aspect_requires_aspect_inherit_required_providers() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"Prov_A = provider()
Prov_B = provider()
Prov_C = provider()

CollectorProv = provider()

def _aspect_c_impl(target, ctx):
  collector_result = ['aspect_c run on target {} and value of Prov_C = {}'
                                .format(target.label, target[Prov_C].val)]
  return [CollectorProv(result = collector_result)]
aspect_c = aspect(
  implementation = _aspect_c_impl,
  required_providers = [Prov_C],
  attr_aspects = ['dep'],
)

def _aspect_b_impl(target, ctx):
  collector_result = []
  collector_result += ctx.rule.attr.dep[CollectorProv].result
  collector_result += ['aspect_b run on target {} and value of Prov_B = {}'
                                 .format(target.label, target[Prov_B].val)]
  return [CollectorProv(result = collector_result)]
aspect_b = aspect(
  implementation = _aspect_b_impl,
  required_providers = [Prov_B],
  requires = [aspect_c],
  attr_aspects = ['dep'],
)

def _aspect_a_impl(target, ctx):
  collector_result = []
  collector_result += ctx.rule.attr.dep[CollectorProv].result
  collector_result += ['aspect_a run on target {} and value of Prov_A = {}'
                                 .format(target.label, target[Prov_A].val)]
  return [CollectorProv(result = collector_result)]
aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  required_providers = [Prov_A],
  requires = [aspect_b],
)

def _my_rule_impl(ctx):
  return [Prov_A(val='main_val_a')]
my_rule = rule(
  implementation = _my_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
  provides = [Prov_A]
)

def _rule_with_prov_a_impl(ctx):
  return [Prov_A(val='val_a')]
rule_with_prov_a = rule(
  implementation = _rule_with_prov_a_impl,
  attrs = {
    'dep': attr.label(),
  },
  provides = [Prov_A]
)

def _rule_with_prov_b_impl(ctx):
  return [Prov_B(val = 'val_b')]
rule_with_prov_b = rule(
  implementation = _rule_with_prov_b_impl,
  attrs = {
    'dep': attr.label(),
  },
  provides = [Prov_B]
)

def _rule_with_prov_c_impl(ctx):
  return [Prov_C(val = 'val_c')]
rule_with_prov_c = rule(
  implementation = _rule_with_prov_c_impl,
  provides = [Prov_C]
)
"#],
    );
    f.scratch.file("test/BUILD", &["load('//test:defs.bzl', 'my_rule', 'rule_with_prov_a', 'rule_with_prov_b', 'rule_with_prov_c')", "my_rule(", "  name = 'main_target',", "  dep = ':target_with_prov_a',", ")", "rule_with_prov_a(", "  name = 'target_with_prov_a',", "  dep = ':target_with_prov_b'", ")", "rule_with_prov_b(", "  name = 'target_with_prov_b',", "  dep = ':target_with_prov_c'", ")", "rule_with_prov_c(", "  name = 'target_with_prov_c'", ")"]);
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%aspect_a"], &["//test:main_target"])
        .expect("update");

    // aspect_a should run on main_target and target_with_prov_a
    // aspect_b can reach target_with_prov_b because it inherits the required_providers of aspect_a
    // aspect_c can reach target_with_prov_c because it inherits the required_providers of aspect_a
    // and aspect_b
    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let collector_prov = starlark_provider_key("//test:defs.bzl", "CollectorProv");
    let collector_provider = aspect_a
        .unwrap()
        .get(&collector_prov)
        .expect("NullPointerException: CollectorProv");
    assert_contains_exactly_in_order(
        &get_value_strings(&collector_provider, "result"),
        &[
            "aspect_c run on target @@//test:target_with_prov_c and value of Prov_C = val_c",
            "aspect_b run on target @@//test:target_with_prov_b and value of Prov_B = val_b",
            "aspect_a run on target @@//test:target_with_prov_a and value of Prov_A = val_a",
            "aspect_a run on target @@//test:main_target and value of Prov_A = main_val_a",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7228 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_inspectRequiredAspectActions
#[test]
fn test_top_level_aspect_requires_aspect_inspect_required_aspect_actions() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"BaseAspectProvider = provider()
def _required_aspect_impl(target, ctx):
  f = ctx.actions.declare_file('dummy.txt')
  ctx.actions.run_shell(outputs = [f], command='echo xxx > $(location f)',
                        mnemonic='RequiredAspectAction')
  return []
required_aspect = aspect(
  implementation = _required_aspect_impl,
)

def _base_aspect_impl(target, ctx):
  required_aspect_action = None
  for action in target.actions:
    if action.mnemonic == 'RequiredAspectAction':
      required_aspect_action = action
  if required_aspect_action:
    return [BaseAspectProvider(result = 'base_aspect can see required_aspect action')]
  else:
    return [BaseAspectProvider(result = 'base_aspect cannot see required_aspect action')]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  requires = [required_aspect]
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%base_aspect"], &["//test:main_target"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let base_aspect = get_configured_aspect(configured_aspects, "base_aspect");
    assert!(base_aspect.is_some());
    let base_aspect_prov = starlark_provider_key("//test:defs.bzl", "BaseAspectProvider");
    let base_aspect_provider = base_aspect
        .unwrap()
        .get(&base_aspect_prov)
        .expect("NullPointerException: BaseAspectProvider");
    assert_eq!(
        get_value_string(&base_aspect_provider, "result"),
        "base_aspect can see required_aspect action"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7287 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_inspectRequiredAspectGeneratedFiles
#[test]
fn test_top_level_aspect_requires_aspect_inspect_required_aspect_generated_files() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"BaseAspectProvider = provider()
def _required_aspect_impl(target, ctx):
  file = ctx.actions.declare_file('required_aspect_file')
  ctx.actions.write(file, 'data')
  return [OutputGroupInfo(out = [file])]
required_aspect = aspect(
  implementation = _required_aspect_impl,
)

def _base_aspect_impl(target, ctx):
  files = ['base_aspect can see file ' + f.path.split('/')[-1]
               for f in target[OutputGroupInfo].out.to_list()]
  return [BaseAspectProvider(my_files = files)]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  requires = [required_aspect]
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["test/defs.bzl%base_aspect"], &["//test:main_target"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let base_aspect = get_configured_aspect(configured_aspects, "base_aspect");
    assert!(base_aspect.is_some());
    let base_aspect_prov = starlark_provider_key("//test:defs.bzl", "BaseAspectProvider");
    let base_aspect_provider = base_aspect
        .unwrap()
        .get(&base_aspect_prov)
        .expect("NullPointerException: BaseAspectProvider");
    assert_contains_exactly(
        &get_value_strings(&base_aspect_provider, "my_files"),
        &["base_aspect can see file required_aspect_file"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7341 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_withRequiredAspectProvidersSatisfied
#[test]
fn test_top_level_aspect_requires_aspect_with_required_aspect_providers_satisfied() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"prov_a = provider()
prov_b = provider()
prov_b_forwarded = provider()
AspectInfo = provider()

def _aspect_b_impl(target, ctx):
  result = 'aspect_b on target {} '.format(target.label)
  if prov_b in target:
    result += 'found prov_b = {}'.format(target[prov_b].val)
    return [
      AspectInfo(aspect_b_result = result),
      prov_b_forwarded(val = target[prov_b].val)
     ]
  else:
    result += 'cannot find prov_b'
    return AspectInfo(aspect_b_result = result)
aspect_b = aspect(
  implementation = _aspect_b_impl,
  required_aspect_providers = [prov_b]
)

def _aspect_a_impl(target, ctx):
  result = 'aspect_a on target {} '.format(target.label)
  if prov_a in target:
    result += 'found prov_a = {}'.format(target[prov_a].val)
  else:
    result += 'cannot find prov_a'
  if prov_b_forwarded in target:
    result += ' and found prov_b = {}'.format(target[prov_b_forwarded].val)
  else:
    result += ' but cannot find prov_b'
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  required_aspect_providers = [prov_a],
  attr_aspects = ['dep'],
  requires = [aspect_b]
)

def _aspect_with_prov_a_impl(target, ctx):
  return [prov_a(val = 'a1')]
aspect_with_prov_a = aspect(
  implementation = _aspect_with_prov_a_impl,
  provides = [prov_a],
  attr_aspects = ['dep'],
)

def _aspect_with_prov_b_impl(target, ctx):
  return [prov_b(val = 'b1')]
aspect_with_prov_b = aspect(
  implementation = _aspect_with_prov_b_impl,
  provides = [prov_b],
  attr_aspects = ['dep'],
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
)
"#],
    );
    let analysis_result = f
        .update_aspects(
            &[
                "test/defs.bzl%aspect_with_prov_a",
                "test/defs.bzl%aspect_with_prov_b",
                "test/defs.bzl%aspect_a",
            ],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_string(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_eq!(
        aspect_a_result,
        "aspect_a on target @@//test:main_target found prov_a = a1 and found prov_b = b1"
    );

    let aspect_b = get_configured_aspect(configured_aspects, "aspect_b");
    assert!(aspect_b.is_some());
    let aspect_b_result = get_value_string(
        &get_starlark_provider(&aspect_b.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_b_result",
    );
    assert_eq!(
        aspect_b_result,
        "aspect_b on target @@//test:main_target found prov_b = b1"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7444 StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_withRequiredAspectProvidersNotFound
#[test]
fn test_top_level_aspect_requires_aspect_with_required_aspect_providers_not_found() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"prov_a = provider()
prov_b = provider()
AspectInfo = provider()

def _aspect_b_impl(target, ctx):
  result = 'aspect_b on target {} '.format(target.label)
  if prov_b in target:
    result += 'found prov_b = {}'.format(target[prov_b].val)
  else:
    result += 'cannot find prov_b'
  return AspectInfo(aspect_b_result = result)
aspect_b = aspect(
  implementation = _aspect_b_impl,
  required_aspect_providers = [prov_b]
)

def _aspect_a_impl(target, ctx):
  result = 'aspect_a on target {} '.format(target.label)
  if prov_a in target:
    result += 'found prov_a = {}'.format(target[prov_a].val)
  else:
    result += 'cannot find prov_a'
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  required_aspect_providers = [prov_a],
  attr_aspects = ['dep'],
  requires = [aspect_b]
)

def _aspect_with_prov_a_impl(target, ctx):
  return [prov_a(val = 'a1')]
aspect_with_prov_a = aspect(
  implementation = _aspect_with_prov_a_impl,
  provides = [prov_a],
  attr_aspects = ['dep'],
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
)
"#],
    );
    let analysis_result = f
        .update_aspects(
            &["test/defs.bzl%aspect_with_prov_a", "test/defs.bzl%aspect_a"],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_string(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_eq!(
        aspect_a_result,
        "aspect_a on target @@//test:main_target found prov_a = a1"
    );

    let aspect_b = get_configured_aspect(configured_aspects, "aspect_b");
    assert!(aspect_b.is_some());
    let aspect_b_result = get_value_string(
        &get_starlark_provider(&aspect_b.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_b_result",
    );
    assert_eq!(
        aspect_b_result,
        "aspect_b on target @@//test:main_target cannot find prov_b"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7527 StarlarkDefinedAspectsTest.testDependentAspectWithNonExecutableTool_doesNotCrash
#[test]
fn test_dependent_aspect_with_non_executable_tool_does_not_crash() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test_defs:foo_binary.bzl', 'foo_binary')
load('//test_defs:foo_library.bzl', 'foo_library')
foo_binary(name='bin', srcs=['bin.sh'])
foo_library(name='lib')
"#],
    );
    f.scratch.file(
        "test/defs.bzl",
        &[
            "AInfo = provider()",
            "BInfo = provider()",
            "def _aspect_a(target, ctx):",
            "  return [AInfo(value=str(ctx.attr._attr.label))]",
            "aspect_a = aspect(",
            "  implementation = _aspect_a,",
            "  provides=[AInfo],",
            "  attrs = {'_attr': attr.label(default=':lib')},",
            ")",
            "def _aspect_b(target, ctx):",
            "  return [BInfo(value=str(ctx.executable._attr.path.split('/')[-1]))]",
            "aspect_b = aspect(",
            "  implementation = _aspect_b,",
            "  required_aspect_providers = [AInfo],",
            "  attrs = {'_attr': attr.label(default=':bin', executable=True, cfg='exec')},",
            ")",
        ],
    );
    f.scratch.file("test/bin.sh", &[]);
    set_executable(&f.scratch.resolve("test/bin.sh"));

    let result = f
        .update_aspects(
            &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
            &["//test:bin"],
        )
        .expect("update");

    let aspect_b = result
        .get_aspects_map()
        .iter()
        .filter(|(k, _)| k.get_aspect_name().ends_with("aspect_b"))
        .map(|(_, v)| v.get_configured_object())
        .next();
    assert!(aspect_b.is_some());

    let prov_b = starlark_provider_key("//test:defs.bzl", "BInfo");
    assert_eq!(
        get_value_string(
            &aspect_b
                .unwrap()
                .get(&prov_b)
                .expect("NullPointerException: BInfo"),
            "value"
        ),
        "bin"
    );

    let aspect_a = result
        .get_aspects_map()
        .iter()
        .filter(|(k, _)| k.get_aspect_name().ends_with("aspect_a"))
        .map(|(_, v)| v.get_configured_object())
        .next();
    assert!(aspect_a.is_some());

    let prov_a = starlark_provider_key("//test:defs.bzl", "AInfo");
    assert_eq!(
        get_value_string(
            &aspect_a
                .unwrap()
                .get(&prov_a)
                .expect("NullPointerException: AInfo"),
            "value"
        ),
        "@@//test:lib"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7585 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters
#[test]
fn test_top_level_aspects_with_parameters() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {} and a_p = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.a_p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'a_p' : attr.string(values = ['a_p_v1', 'a_p_v2'])},
)

def _aspect_b_impl(target, ctx):
  result = ['aspect_b on target {}, p1 = {} and b_p = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.b_p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_b_result
  return AspectInfo(aspect_b_result = result)

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'b_p' : attr.string(values = ['b_p_v1', 'b_p_v2'])},
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a", "//test:defs.bzl%aspect_b"],
            &[("p1", "p1_v1"), ("a_p", "a_p_v1"), ("b_p", "b_p_v1")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p1 = p1_v1 and a_p = a_p_v1",
            "aspect_a on target @@//test:dep_target_1, p1 = p1_v1 and a_p = a_p_v1",
            "aspect_a on target @@//test:dep_target_2, p1 = p1_v1 and a_p = a_p_v1",
        ],
    );

    let aspect_b = get_configured_aspect(configured_aspects, "aspect_b");
    assert!(aspect_b.is_some());
    let aspect_b_result = get_value_strings(
        &get_starlark_provider(&aspect_b.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_b_result",
    );
    assert_contains_exactly(
        &aspect_b_result,
        &[
            "aspect_b on target @@//test:main_target, p1 = p1_v1 and b_p = b_p_v1",
            "aspect_b on target @@//test:dep_target_1, p1 = p1_v1 and b_p = b_p_v1",
            "aspect_b on target @@//test:dep_target_2, p1 = p1_v1 and b_p = b_p_v1",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7673 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_differentAllowedValues
#[test]
fn test_top_level_aspects_with_parameters_different_allowed_values() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {} and p2 = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.p2)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']) },
)

def _aspect_b_impl(target, ctx):
  result = ['aspect_b on target {}, p1 = {} and p2 = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.p2)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_b_result
  return struct(aspect_b_result = result)

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v2', 'p1_v3']) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a", "//test:defs.bzl%aspect_b"],
                &[("p1", "p1_v1")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a", "//test:defs.bzl%aspect_b"],
            &[("p1", "p1_v1")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "//test:defs.bzl%aspect_b: invalid value in 'p1' attribute: has to be one of 'p1_v2' or \
         'p1_v3' instead of 'p1_v1'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7752 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_useDefaultValue
#[test]
fn test_top_level_aspects_with_parameters_use_default_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {} and p2 = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.p2)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2'], default = 'p1_v1'),
            'p2' : attr.string(values = ['p2_v1', 'p2_v2'], default = 'p2_v1')},
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p1", "p1_v2")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p1 = p1_v2 and p2 = p2_v1",
            "aspect_a on target @@//test:dep_target_1, p1 = p1_v2 and p2 = p2_v1",
            "aspect_a on target @@//test:dep_target_2, p1 = p1_v2 and p2 = p2_v1",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7815 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_passParametersToRequiredAspect
#[test]
fn test_top_level_aspects_with_parameters_pass_parameters_to_required_aspect() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectAInfo = provider()
AspectBInfo = provider()
def _aspect_b_impl(target, ctx):
  result = ['aspect_b on target {}, p1 = {} and p3 = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.p3)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectBInfo].aspect_b_result
  return AspectBInfo(aspect_b_result = result)

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'p3' : attr.string(values = ['p3_v1', 'p3_v2', 'p3_v3'])},
)

def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {} and p2 = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.p2)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectAInfo].aspect_a_result
  return AspectAInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'p2' : attr.string(values = ['p2_v1', 'p2_v2'])},
  requires = [aspect_b],
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p1", "p1_v1"), ("p2", "p2_v2"), ("p3", "p3_v3")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectAInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p1 = p1_v1 and p2 = p2_v2",
            "aspect_a on target @@//test:dep_target_1, p1 = p1_v1 and p2 = p2_v2",
            "aspect_a on target @@//test:dep_target_2, p1 = p1_v1 and p2 = p2_v2",
        ],
    );

    let aspect_b = get_configured_aspect(configured_aspects, "aspect_b");
    assert!(aspect_b.is_some());
    let aspect_b_result = get_value_strings(
        &get_starlark_provider(&aspect_b.unwrap(), "//test:defs.bzl", "AspectBInfo"),
        "aspect_b_result",
    );
    assert_contains_exactly(
        &aspect_b_result,
        &[
            "aspect_b on target @@//test:main_target, p1 = p1_v1 and p3 = p3_v3",
            "aspect_b on target @@//test:dep_target_1, p1 = p1_v1 and p3 = p3_v3",
            "aspect_b on target @@//test:dep_target_2, p1 = p1_v1 and p3 = p3_v3",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7905 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_invalidParameterValue
#[test]
fn test_top_level_aspects_with_parameters_invalid_parameter_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(values = ['p_v1', 'p_v2']) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a"],
                &[("p", "p_v")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "p_v")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "//test:defs.bzl%aspect_a: invalid value in 'p' attribute: has to be one of 'p_v1' or \
         'p_v2' instead of 'p_v'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:7971 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_missingMandatoryParameter
#[test]
fn test_top_level_aspects_with_parameters_missing_mandatory_parameter() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {}'.
                                    format(target.label, ctx.attr.p1)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(mandatory = True, default = 'p1_v1',
                               values = ['p1_v1', 'p1_v2']),
            'p2' : attr.string(values = ['p2_v1', 'p2_v2'])},
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a"],
                &[("p2", "p2_v1")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p2", "p2_v1")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "Missing mandatory attribute 'p1' for aspect '//test:defs.bzl%aspect_a'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8038 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_unusedParameter
#[test]
fn test_top_level_aspects_with_parameters_unused_parameter() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p1 = {} and a_p = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.a_p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'a_p' : attr.string(values = ['a_p_v1', 'a_p_v2'])},
)

def _aspect_b_impl(target, ctx):
  result = ['aspect_b on target {}, p1 = {} and b_p = {}'.
                                    format(target.label, ctx.attr.p1, ctx.attr.b_p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_b_result
  return struct(aspect_b_result = result)

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep'],
  attrs = { 'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
            'b_p' : attr.string(values = ['b_p_v1', 'b_p_v2'])},
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a", "//test:defs.bzl%aspect_b"],
                &[("p2", "p2_v1"), ("b_p", "b_p_v1")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a", "//test:defs.bzl%aspect_b"],
            &[("p2", "p2_v1"), ("b_p", "b_p_v1")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "Parameters '[p2]' are not parameters of any of the top-level aspects but they are \
         specified in --aspects_parameters.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8120 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_invalidDefaultParameterValue
#[test]
fn test_top_level_aspects_with_parameters_invalid_default_parameter_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(values = ['p_v1', 'p_v2']) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(&["//test:defs.bzl%aspect_a"], &[], &["//test:main_target"])
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "//test:defs.bzl%aspect_a: invalid value in 'p' attribute: has to be one of 'p_v1' or \
         'p_v2' instead of ''",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8179 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_noNeedForAllowedValues
#[test]
fn test_top_level_aspects_with_parameters_no_need_for_allowed_values() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(default='val') },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "p_v")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p = p_v",
            "aspect_a on target @@//test:dep_target_1, p = p_v",
            "aspect_a on target @@//test:dep_target_2, p = p_v",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8244 StarlarkDefinedAspectsTest.testAttrAspectParameterMissingRequiredValues
#[test]
fn test_attr_aspect_parameter_missing_required_values() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   pass
my_aspect = aspect(_impl,
   attrs = { 'param' : attr.string(default = 'c') }
)
def _rule_impl(ctx):
   pass
r1 = rule(_rule_impl, attrs={'dep': attr.label(aspects = [my_aspect])})
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 'main_target')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update(&["//test:main_target"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update(&["//test:main_target"]));
    }
    f.assert_contains_event(
        "Aspect //test:defs.bzl%my_aspect: Aspect parameter attribute 'param' must use the \
         'values' restriction.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8282 StarlarkDefinedAspectsTest.testAttrRequiredAspectParameterMissingRequiredValues
#[test]
fn test_attr_required_aspect_parameter_missing_required_values() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _impl(target, ctx):
   pass
required_aspect = aspect(_impl,
   attrs = { 'p1' : attr.string(default = 'b') }
)
my_aspect = aspect(_impl,
   attrs = { 'p2' : attr.string(default = 'c', values = ['c']) },
   requires = [required_aspect],
)
def _rule_impl(ctx):
   pass
r1 = rule(_rule_impl, attrs={'dep': attr.label(aspects = [my_aspect])})
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 'main_target')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update(&["//test:main_target"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update(&["//test:main_target"]));
    }
    f.assert_contains_event(
        "Aspect //test:defs.bzl%required_aspect: Aspect parameter attribute 'p1' must use the \
         'values' restriction.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8321 StarlarkDefinedAspectsTest.integerAspectParameter_mandatoryAttrNotCoveredByRule
#[test]
fn integer_aspect_parameter_mandatory_attr_not_covered_by_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2], mandatory = True) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type int.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8361 StarlarkDefinedAspectsTest.integerAspectParameter_mandatoryAttrWithWrongTypeInRule
#[test]
fn integer_aspect_parameter_mandatory_attr_with_wrong_type_in_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2], mandatory = True) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),
              'my_attr': attr.string() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type int.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8402 StarlarkDefinedAspectsTest.integerAspectParameter_attrWithoutDefaultNotCoveredByRule
#[test]
fn integer_aspect_parameter_attr_without_default_not_covered_by_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int(values = [1, 2]) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type int.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8442 StarlarkDefinedAspectsTest.integerAspectParameter_attrWithoutDefaultWrongTypeInRule
#[test]
fn integer_aspect_parameter_attr_without_default_wrong_type_in_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int(values = [1, 2]) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),
              'my_attr': attr.string() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type int.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8483 StarlarkDefinedAspectsTest.integerAspectParameter_missingValuesRestriction
#[test]
fn integer_aspect_parameter_missing_values_restriction() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int() },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),
              'my_attr' : attr.int() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered: Aspect parameter attribute 'my_attr' must use \
         the 'values' restriction.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8524 StarlarkDefinedAspectsTest.integerAspectParameter_invalidDefault
#[test]
fn integer_aspect_parameter_invalid_default() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.int(default = 2, values = [0, 1]) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect parameter attribute 'my_attr' has a bad default value: has to be one of '0' or '1' \
         instead of '2'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8564 StarlarkDefinedAspectsTest.aspectIntegerParameter_withDefaultValue
#[test]
fn aspect_integer_parameter_with_default_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].my_aspect_result
  return AspectInfo(my_aspect_result = result)

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2, 3]) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = 1"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8613 StarlarkDefinedAspectsTest.aspectIntegerParameter_valueOverwrittenByRuleDefault
#[test]
fn aspect_integer_parameter_value_overwritten_by_rule_default() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.my_aspect_result
  return [AspectInfo(my_aspect_result = result)]

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2, 3]) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]),
              'my_attr': attr.int(default = 2) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = 2"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8663 StarlarkDefinedAspectsTest.aspectIntegerParameter_valueOverwrittenByTargetValue
#[test]
fn aspect_integer_parameter_value_overwritten_by_target_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].my_aspect_result
  return AspectInfo(my_aspect_result = result)

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.int(default = 1, values = [1, 2, 3]) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]),
              'my_attr': attr.int(default = 2) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
        my_attr = 3,
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = 3"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8714 StarlarkDefinedAspectsTest.testTopLevelAspectsWithParameters_invalidIntegerParameterValue
#[test]
fn test_top_level_aspects_with_parameters_invalid_integer_parameter_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.int(values = [1, 2]) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a"],
                &[("p", "3")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "3")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "//test:defs.bzl%aspect_a: invalid value in 'p' attribute: has to be one of '1' or '2' \
         instead of '3'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8780 StarlarkDefinedAspectsTest.testTopLevelAspectsWithIntegerParameter
#[test]
fn test_top_level_aspects_with_integer_parameter() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.int(values = [1, 2, 3]) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "2")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p = 2",
            "aspect_a on target @@//test:dep_target_1, p = 2",
            "aspect_a on target @@//test:dep_target_2, p = 2",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8842 StarlarkDefinedAspectsTest.testTopLevelAspectsWithIntegerParameter_useDefaultValue
#[test]
fn test_top_level_aspects_with_integer_parameter_use_default_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.int(default = 1, values = [1, 2, 3]) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["//test:defs.bzl%aspect_a"], &["//test:main_target"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p = 1",
            "aspect_a on target @@//test:dep_target_1, p = 1",
            "aspect_a on target @@//test:dep_target_2, p = 1",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8901 StarlarkDefinedAspectsTest.booleanAspectParameter_mandatoryAttrNotCoveredByRule
#[test]
fn boolean_aspect_parameter_mandatory_attr_not_covered_by_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.bool(default = True, mandatory = True) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type boolean.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8941 StarlarkDefinedAspectsTest.booleanAspectParameter_mandatoryAttrWithWrongTypeInRule
#[test]
fn boolean_aspect_parameter_mandatory_attr_with_wrong_type_in_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.bool(default = True, mandatory = True) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),
              'my_attr': attr.string() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type boolean.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:8982 StarlarkDefinedAspectsTest.booleanAspectParameter_attrWithoutDefaultNotCoveredByRule
#[test]
fn boolean_aspect_parameter_attr_without_default_not_covered_by_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.bool() },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type boolean.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9022 StarlarkDefinedAspectsTest.booleanAspectParameter_attrWithoutDefaultWrongTypeInRule
#[test]
fn boolean_aspect_parameter_attr_without_default_wrong_type_in_rule() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.bool() },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]),
              'my_attr': attr.string() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name ='main')
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update_aspects(&[], &["//test:main"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update_aspects(&[], &["//test:main"]));
    }

    f.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type boolean.",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9063 StarlarkDefinedAspectsTest.aspectBooleanParameter_withDefaultValue
#[test]
fn aspect_boolean_parameter_with_default_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.my_aspect_result
  return AspectInfo(my_aspect_result = result)

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.bool(default = True) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = True"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9113 StarlarkDefinedAspectsTest.aspectBooleanParameter_valueOverwrittenByRuleDefault
#[test]
fn aspect_boolean_parameter_value_overwritten_by_rule_default() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].my_aspect_result
  return AspectInfo(my_aspect_result = result)

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.bool(default = True) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]),
              'my_attr': attr.bool(default = False) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = False"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9163 StarlarkDefinedAspectsTest.aspectBooleanParameter_valueOverwrittenByTargetValue
#[test]
fn aspect_boolean_parameter_value_overwritten_by_target_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
  result = ['my_aspect on target {}, my_attr = {}'.
                                    format(target.label, ctx.attr.my_attr)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].my_aspect_result
  return AspectInfo(my_aspect_result = result)

RuleInfo = provider()
def _rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(my_rule_result = ctx.attr.dep[AspectInfo].my_aspect_result)
  pass

MyAspect = aspect(
    implementation = _aspect_impl,
    attrs = { 'my_attr' : attr.bool(default = True) },
)

my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'dep' : attr.label(aspects=[MyAspect]),
              'my_attr': attr.bool(default = True) }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'main_target',
        dep = ':dep_target',
        my_attr = False,
)
my_rule(name = 'dep_target')
"#],
    );
    let analysis_result = f
        .update_aspects(&[], &["//test:main_target"])
        .expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let rule_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "my_rule_result",
    );
    assert_contains_exactly(
        &rule_result,
        &["my_aspect on target @@//test:dep_target, my_attr = False"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9214 StarlarkDefinedAspectsTest.testTopLevelAspectsWithBooleanParameter
#[test]
fn test_top_level_aspects_with_boolean_parameter() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.bool() },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "y")],
            &["//test:main_target"],
        )
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p = True",
            "aspect_a on target @@//test:dep_target_1, p = True",
            "aspect_a on target @@//test:dep_target_2, p = True",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9276 StarlarkDefinedAspectsTest.testTopLevelAspectsWithBooleanParameter_useDefaultValue
#[test]
fn test_top_level_aspects_with_boolean_parameter_use_default_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep[AspectInfo].aspect_a_result
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.bool(default = False) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    let analysis_result = f
        .update_aspects(&["//test:defs.bzl%aspect_a"], &["//test:main_target"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "aspect_a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_strings(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on target @@//test:main_target, p = False",
            "aspect_a on target @@//test:dep_target_1, p = False",
            "aspect_a on target @@//test:dep_target_2, p = False",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9335 StarlarkDefinedAspectsTest.testTopLevelAspectsWithBooleanParameter_invalidValue
#[test]
fn test_top_level_aspects_with_boolean_parameter_invalid_value() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.bool() },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label() },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a ViewCreationFailedException.
    if f.keep_going() {
        let analysis_result = f
            .update_aspects_with_params(
                &["//test:defs.bzl%aspect_a"],
                &[("p", "x")],
                &["//test:main_target"],
            )
            .expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_view_creation_failed(f.update_aspects_with_params(
            &["//test:defs.bzl%aspect_a"],
            &[("p", "x")],
            &["//test:main_target"],
        ));
    }
    f.assert_contains_event(
        "//test:defs.bzl%aspect_a: expected value of type 'bool' for attribute 'p' but got 'x'",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9400 StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameterNotProvided
#[test]
fn test_rule_aspect_with_mandatory_parameter_not_provided() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(default = 'p_v', values = ['p_v'], mandatory = True) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label(aspects = [aspect_a]) },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update(&["//test:main_target"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update(&["//test:main_target"]));
    }
    f.assert_contains_event(
        "Aspect //test:defs.bzl%aspect_a requires rule my_rule to specify attribute 'p' with type \
         string",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9456 StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameterProvidedWrongType
#[test]
fn test_rule_aspect_with_mandatory_parameter_provided_wrong_type() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _aspect_a_impl(target, ctx):
  result = ['aspect_a on target {}, p = {}'.
                                    format(target.label, ctx.attr.p)]
  if ctx.rule.attr.dep:
    result += ctx.rule.attr.dep.aspect_a_result
  return struct(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(default = 'p_v', values = ['p_v'], mandatory = True) },
)

def _my_rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _my_rule_impl,
   attrs = { 'dep' : attr.label(aspects = [aspect_a]),
             'p': attr.int() }
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_target_1',
)
my_rule(
  name = 'dep_target_1',
  dep = ':dep_target_2',
)
my_rule(
  name = 'dep_target_2',
)
"#],
    );
    f.remove_fail_fast_handler();

    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    if f.keep_going() {
        let analysis_result = f.update(&["//test:main_target"]).expect("update");
        assert!(has_error(&analysis_result));
    } else {
        assert_throws_target_parsing(f.update(&["//test:main_target"]));
    }
    f.assert_contains_event(
        "Aspect //test:defs.bzl%aspect_a requires rule my_rule to specify attribute 'p' with type \
         string",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9513 StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameter_useRuleDefault
#[test]
fn test_rule_aspect_with_mandatory_parameter_use_rule_default() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = 'aspect_a on target {}, p = {}'.format(target.label, ctx.attr.p)
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(default = 'p_v1', values = ['p_v1', 'p_v2'],
                              mandatory = True) },
)

RuleInfo = provider()
def _main_rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(aspect_a_result = ctx.attr.dep[AspectInfo].aspect_a_result)
  pass
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_a]),
    'p' : attr.string(default = 'p_v2'),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'main_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
main_rule(
  name = 'dep_target',
)
"#],
    );
    let analysis_result = f.update(&["//test:main"]).expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let aspect_a_result = get_value_string(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "aspect_a_result",
    );
    assert_eq!(
        aspect_a_result,
        "aspect_a on target @@//test:dep_target, p = p_v2"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9565 StarlarkDefinedAspectsTest.testRuleAspectWithMandatoryParameterProvided
#[test]
fn test_rule_aspect_with_mandatory_parameter_provided() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()
def _aspect_a_impl(target, ctx):
  result = 'aspect_a on target {}, p = {}'.format(target.label, ctx.attr.p)
  return AspectInfo(aspect_a_result = result)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  attrs = { 'p' : attr.string(default = 'p_v2', values = ['p_v1', 'p_v2'],
                              mandatory = True) },
)

RuleInfo = provider()
def _main_rule_impl(ctx):
  if ctx.attr.dep:
    return RuleInfo(aspect_a_result = ctx.attr.dep[AspectInfo].aspect_a_result)
  pass
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_a]),
    'p' : attr.string(mandatory = True),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'main_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
  p = 'p_v1',
)
main_rule(
  name = 'dep_target',
  p = 'p_v2',
)
"#],
    );
    let analysis_result = f.update(&["//test:main"]).expect("update");

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let aspect_a_result = get_value_string(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "aspect_a_result",
    );
    assert_eq!(
        aspect_a_result,
        "aspect_a on target @@//test:dep_target, p = p_v1"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9619 StarlarkDefinedAspectsTest.testAspectLabelIsRepoMapped
#[test]
fn test_aspect_label_is_repo_mapped() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch
        .overwrite_file("MODULE.bazel", &["module(name = 'my_repo')"]);
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"load(':rule.bzl', 'MyInfo')
def _impl(target, ctx):
   if MyInfo not in target:
       fail('Provider identity mismatch')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    f.scratch.file(
        "test/rule.bzl",
        &[r#"MyInfo = provider()
def _impl(ctx):
    return [MyInfo()]
my_rule = rule(implementation=_impl)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load(':rule.bzl', 'my_rule')
my_rule(name = 'target')
"#],
    );
    let result = f
        .update_aspects(&["@my_repo//test:aspect.bzl%MyAspect"], &["//test:target"])
        .expect("update");
    assert!(!has_error(&result));
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9652 StarlarkDefinedAspectsTest.testAspectKeyCreatedOnlyOnceForSameBaseKeysInDiffOrder
#[test]
fn test_aspect_key_created_only_once_for_same_base_keys_in_diff_order() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"a_provider = provider()
b_provider = provider()
c_provider = provider()
RuleInfo = provider()

def _a_impl(target, ctx):
  result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      result.extend(dep[a_provider].value)
  result.append('aspect a on target {} aspect_ids {}'.format(target.label,
                                                                ctx.aspect_ids))
  return [a_provider(value = result)]
a = aspect(
  implementation = _a_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [[b_provider], [c_provider]],
)

def _b_impl(target, ctx):
  return [b_provider(value = ['aspect b on target {}'.format(target.label)])]
b = aspect(
  implementation = _b_impl,
  attr_aspects = ['deps'],
  provides = [b_provider],
)

def _c_impl(target, ctx):
  return [c_provider(value = ['aspect c on target {}'.format(target.label)])]
c = aspect(
  implementation = _c_impl,
  attr_aspects = ['deps'],
  provides = [c_provider]
)

def _r1_impl(ctx):
  result = []
  if ctx.attr.deps:
    for dep in ctx.attr.deps:
      result.extend(dep[a_provider].value)
  return RuleInfo(aspect_a_collected_result = result)
r1 = rule(
  implementation = _r1_impl,
  attrs = {
    'deps': attr.label_list(aspects = [c, b, a]),
  },
)

def _r2_impl(ctx):
  pass
r2 = rule(
  implementation = _r2_impl,
  attrs = {
    'deps': attr.label_list(aspects = [b]),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1', 'r2')
r1(
  name = 't1',
# base_keys of aspect a on t3 are [c, b]
  deps = [':t2', ':t3'],
)
r2(
  name = 't2',
  # aspects reaching t3 will be [b, c, b, a], after deduplicating aspects path, it will be
  # [b, c, a] and as a result the base_keys of aspect a will be [b, c]
  deps = [':t3'],
)
r2(
  name = 't3',
)
"#],
    );
    f.update(&["//test:t1"]).expect("update");

    // Aspect a should have a single AspectKey for its application on t3 and the baseKeys in it will
    // be sorted as [b, c]
    let keys_for_aspect_a_on_t3 = f.get_aspect_keys("//test:t3", "//test:defs.bzl%a");
    assert_eq!(keys_for_aspect_a_on_t3.len(), 1);

    let base_keys = keys_for_aspect_a_on_t3[0].get_base_keys();
    let base_key_names: Vec<String> = base_keys
        .iter()
        .map(|k| k.get_aspect_class().get_name().to_string())
        .collect();
    assert_contains_exactly_in_order(&base_key_names, &["//test:defs.bzl%b", "//test:defs.bzl%c"]);
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9747 StarlarkDefinedAspectsTest.testAspectRunsTwiceWithDiffBaseAspectsDeps
#[test]
fn test_aspect_runs_twice_with_diff_base_aspects_deps() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"a_provider = provider()
b_provider = provider()
c_provider = provider()

def _a_impl(target, ctx):
  result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      result.extend(dep[a_provider].value)
  if b_provider in target:
    result.append('aspect a on {} sees b_provider = {}'.format(target.label,
                                                              target[b_provider].value))
  return [a_provider(value = result)]
a = aspect(
  implementation = _a_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [[b_provider], [c_provider]],
)

def _b_impl(target, ctx):
  result = 'aspect b cannot see c_provider'
  if c_provider in target:
    result = 'aspect b can see c_provider'
  return [b_provider(value = result)]
b = aspect(
  implementation = _b_impl,
  attr_aspects = ['deps'],
  provides = [b_provider],
  required_aspect_providers = [[c_provider]]
)

def _c_impl(target, ctx):
  return [c_provider(value = ['aspect c on target {}'.format(target.label)])]
c = aspect(
  implementation = _c_impl,
  attr_aspects = ['deps'],
  provides = [c_provider]
)

RuleInfo = provider()
def _r1_impl(ctx):
  result = []
  if ctx.attr.deps:
    for dep in ctx.attr.deps:
      result.extend(dep[a_provider].value)
  return RuleInfo(aspect_a_collected_result = result)
r1 = rule(
  implementation = _r1_impl,
  attrs = {
    'deps': attr.label_list(aspects = [a]),
  },
)

def _r2_impl(ctx):
  pass
r2 = rule(
  implementation = _r2_impl,
  attrs = {
    'deps': attr.label_list(aspects = [c, b]),
  },
)

def _r3_impl(ctx):
  pass
r3 = rule(
  implementation = _r3_impl,
  attrs = {
    'deps': attr.label_list(aspects = [b, c]),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1', 'r2', 'r3')
r1(
  name = 't1',
  # t1 propagate aspect (a) to targets (t2 and t3)
  deps = [':t2', ':t3'],
)
r2(
  name = 't2',
  # t2 propagates aspects (c, b) to target t4 and aspect a is propagated from the prev
  # level aspects path on t4 is [c, b, a], this means a can see b and b can see c
  deps = [':t4'],
)
r3(
  name = 't3',
# t3 propagates aspects (b, c) to target t4 and aspect a is propagated from the prev level
# aspects path on t4 is [b, c, a], this means a can see b but b cannot see c
 deps = [':t4'],
)
r1(
  name = 't4',
)
"#],
    );
    let analysis_result = f.update(&["//test:t1"]).expect("update");

    // Aspect a should have 2 AspectKeys for its application on t4, one where in the basekeys b can
    // see c and the other is where b cannot see c
    let keys_for_aspect_a_on_t4 = f.get_aspect_keys("//test:t4", "//test:defs.bzl%a");
    assert_eq!(keys_for_aspect_a_on_t4.len(), 2);

    let configured_target = get_only_element(analysis_result.get_targets_to_build());
    let aspect_a_result = get_value_strings(
        &f.get_starlark_provider_of_target(configured_target, "RuleInfo"),
        "aspect_a_collected_result",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect a on @@//test:t4 sees b_provider = aspect b can see c_provider",
            "aspect a on @@//test:t4 sees b_provider = aspect b cannot see c_provider",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9867 StarlarkDefinedAspectsTest.testAspectWithSameExplicitAttributeNameAsUnderlyingTarget
#[test]
fn test_aspect_with_same_explicit_attribute_name_as_underlying_target() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"MyInfo = provider()
def _a_impl(target, ctx):
  value = 'x from aspect = {}, x from target = {}'.format(ctx.attr.x, ctx.rule.attr.x)
  return MyInfo(aspect_result = value)
a = aspect(
  implementation = _a_impl,
  attrs = {
    'x': attr.string(default = 'xyz')
  },
)

def _rule_impl(ctx):
  pass
r1 = rule(
  implementation = _rule_impl,
  attrs = {
    'x': attr.int(default = 4)
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 't1')
"#],
    );
    let analysis_result = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:t1"])
        .expect("update");

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_string(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "MyInfo"),
        "aspect_result",
    );
    assert_eq!(aspect_a_result, "x from aspect = xyz, x from target = 4");
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9910 StarlarkDefinedAspectsTest.testAspectNotDependOnTargetDeps
#[test]
fn test_aspect_not_depend_on_target_deps() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _a_impl(target, ctx):
  return []
a = aspect(
  implementation = _a_impl,
  attr_aspects = ['dep'],
  attrs = {
    '_tool': attr.label(default = '//test:tool'),
  },
)

def _rule_impl(ctx):
  pass
r1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
    'another_dep': attr.label(),
    '_tool': attr.label(default = '//test:tool'),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 't1', dep = ':t2', another_dep = 't4')
r1(name = 't2', dep = ':t3')
r1(name = 't3')
r1(name = 't4')
filegroup(name = 'tool')
"#],
    );
    let analysis_result = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:t1"])
        .expect("update");

    let key = get_only_element(
        &analysis_result
            .get_aspects_map()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
    )
    .clone();
    let aspect_node = f.find_only_node_entry(|k| k.downcast::<AspectKey>() == Some(&key));
    assert!(aspect_node.is_some());
    let aspect_node_deps = f.get_direct_deps(&aspect_node.unwrap());

    let configured_targets_deps: Vec<String> = aspect_node_deps
        .iter()
        .filter_map(|k| k.downcast::<ConfiguredTargetKey>())
        .map(|k| k.get_label().to_string())
        .collect();
    // aspect depends only on its target and its implicit dependencies not the dependencies of its
    // target
    assert_contains_at_least(&configured_targets_deps, &["//test:tool", "//test:t1"]);
    assert!(!configured_targets_deps.contains(&"//test:t2".to_string()));
    assert!(!configured_targets_deps.contains(&"//test:t3".to_string()));
    assert!(!configured_targets_deps.contains(&"//test:t4".to_string()));

    let aspects_deps: Vec<String> = aspect_node_deps
        .iter()
        .filter_map(|k| k.downcast::<AspectKey>())
        .map(|k| k.get_label().to_string())
        .collect();
    // aspect depends on the result of its application on the target deps if it propagates to them
    assert_contains_exactly(&aspects_deps, &["//test:t2"]);
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:9972 StarlarkDefinedAspectsTest.testTopLevelAspectNotDependsOnConfigeredTopLevelTarget
#[test]
fn test_top_level_aspect_not_depends_on_configered_top_level_target() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"def _a_impl(target, ctx):
  return []
a = aspect(
  implementation = _a_impl,
  attr_aspects = ['dep'],
)

def _rule_impl(ctx):
  pass
r1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 't1', dep = ':t2')
r1(name = 't2', dep = ':t3')
r1(name = 't3')
"#],
    );
    let _unused = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:t1"])
        .expect("update");
    let top_level_aspects_node =
        f.find_only_node_entry(|key| key.downcast::<TopLevelAspectsKey>().is_some());
    assert!(top_level_aspects_node.is_some());
    let top_level_aspects_node_deps = f.get_direct_deps(&top_level_aspects_node.unwrap());
    // top level aspect should not depend on any configured target.
    let configured_targets_deps: Vec<String> = top_level_aspects_node_deps
        .iter()
        .filter_map(|k| k.downcast::<ConfiguredTargetKey>())
        .map(|k| k.get_label().to_string())
        .collect();
    assert!(
        configured_targets_deps.is_empty(),
        "{configured_targets_deps:?}"
    );

    // top level aspect should not even depend on any build configuration.
    let build_configured_targets_deps: Vec<String> = top_level_aspects_node_deps
        .iter()
        .filter_map(|k| k.downcast::<BuildConfigurationKey>())
        .map(|k| format!("{k:?}"))
        .collect();
    assert_eq!(build_configured_targets_deps.len(), 0);

    // top level aspect should depend on the result of aspect's application on the top level target.
    let aspects_deps: Vec<String> = top_level_aspects_node_deps
        .iter()
        .filter_map(|k| k.downcast::<AspectKey>())
        .map(|k| k.get_label().to_string())
        .collect();
    assert_contains_exactly(&aspects_deps, &["//test:t1"]);
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10028 StarlarkDefinedAspectsTest.topLevelAspectRequiredProviderNotSatisfied_aspectKeyNotCreated
#[test]
fn top_level_aspect_required_provider_not_satisfied_aspect_key_not_created() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/defs.bzl",
        &[r#"p1 = provider()
def _a_impl(target, ctx):
  return []
a = aspect(
  implementation = _a_impl,
  attr_aspects = ['dep'],
  required_providers = [p1],
)

def _rule_impl(ctx):
  pass
r1 = rule(
  implementation = _rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[r#"load('//test:defs.bzl', 'r1')
r1(name = 't1', dep = ':t2')
r1(name = 't2')
"#],
    );
    let _unused = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:t1"])
        .expect("update");

    let top_level_aspects_node =
        f.find_only_node_entry(|key| key.downcast::<AspectKey>().is_some());

    // no aspect key should be requested since the aspect's required provider is not satisfied.
    assert!(top_level_aspects_node.is_none());
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10122 StarlarkDefinedAspectsTest.aspectOnAliasForConfiguredTarget_forwardsProvidersButNotActions
#[test]
fn aspect_on_alias_for_configured_target_forwards_providers_but_not_actions() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/BUILD",
        &[r#"alias(
    name = 'alias_target',
    actual = ':actual',
)

genrule(
    name = 'actual',
    outs = ['actual.out'],
    cmd = 'touch $@',
)
"#],
    );
    f.scratch.file(
        "test/simple_label_writing_aspect.bzl",
        &[r#"def _simple_label_writing_aspect_impl(target, ctx):
    label_of_target_file = ctx.actions.declare_file(target.label.name + ".label")
    ctx.actions.write(
        output = label_of_target_file,
        content = target.label.name,
    )
    return [DefaultInfo(files = depset([label_of_target_file]))]

simple_label_writing_aspect = aspect(
    implementation = _simple_label_writing_aspect_impl,
    attr_aspects = ["*"],
)
"#],
    );
    let _unused = f
        .update_aspects(
            &["test/simple_label_writing_aspect.bzl%simple_label_writing_aspect"],
            &["//test:alias_target"],
        )
        .expect("update");

    // The aspect on the actual target has exactly one action.
    let aspect_node = f.find_only_node_entry(|key| {
        key.downcast::<AspectKey>().is_some_and(|aspect_key| {
            aspect_key
                .get_base_configured_target_key()
                .get_label()
                .to_string()
                == "//test:actual"
        })
    });
    assert!(aspect_node.is_some());
    let aspect_value = f.get_aspect_value(&aspect_node.unwrap());
    assert_eq!(aspect_value.get_actions().len(), 1);
    let primary_output = aspect_value.get_actions()[0].get_primary_output();
    assert!(
        primary_output
            .get_exec_path_string()
            .ends_with("bin/test/actual.label"),
        "{}",
        primary_output.get_exec_path_string()
    );

    // The aspect on the alias target has no actions.
    let aspect_on_alias_node = f.find_only_node_entry(|key| {
        key.downcast::<AspectKey>().is_some_and(|aspect_key| {
            aspect_key
                .get_base_configured_target_key()
                .get_label()
                .to_string()
                == "//test:alias_target"
        })
    });
    assert!(aspect_on_alias_node.is_some());
    let aspect_on_alias_value = f.get_aspect_value(&aspect_on_alias_node.unwrap());
    assert!(AspectValue::is_for_alias_target(&aspect_on_alias_value));
    assert!(aspect_on_alias_value.get_actions().is_empty());

    // But the providers should be the same.
    assert_provider_maps_equal(
        aspect_value.get_providers(),
        aspect_on_alias_value.get_providers(),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10195 StarlarkDefinedAspectsTest.topLevelAspectOnAliasTarget_requiredProviderSatisfied
#[test]
fn top_level_aspect_on_alias_target_required_provider_satisfied() {
    let mut f = StarlarkDefinedAspectsTest::new();

    write_aspect_on_alias_test_files(&f);

    // this will select //test:t1 as the alias's actual target.
    f.use_configuration(&["--define=foo=1"]);
    let analysis_result = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:alias_target"])
        .expect("update");

    let evaluator = f.get_evaluator();
    let top_level_aspects_node: Vec<String> = evaluator
        .get_all_keys_for_testing()
        .iter()
        .filter_map(|k| k.downcast::<AspectKey>())
        .map(|k| format!("{} on {}", k.get_aspect_name(), k.get_label()))
        .collect();

    // aspect required provider is satisfied by the alias's actual target.
    assert_contains_exactly(
        &top_level_aspects_node,
        &[
            "//test:defs.bzl%a on //test:t1",
            "//test:defs.bzl%a on //test:alias_target",
        ],
    );

    let configured_aspects = analysis_result.get_aspects_map();
    let aspect_a = get_configured_aspect(configured_aspects, "a");
    assert!(aspect_a.is_some());
    let aspect_a_result = get_value_string(
        &get_starlark_provider(&aspect_a.unwrap(), "//test:defs.bzl", "AspectInfo"),
        "aspect_a_result",
    );
    assert_eq!(aspect_a_result, "target @@//test:t1, p1.val = v1");
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10225 StarlarkDefinedAspectsTest.topLevelAspectOnAliasTarget_requiredProviderNotSatisfied
#[test]
fn top_level_aspect_on_alias_target_required_provider_not_satisfied() {
    let mut f = StarlarkDefinedAspectsTest::new();

    write_aspect_on_alias_test_files(&f);

    // this will select the default //test:t2 as the alias's actual target.
    let _unused = f
        .update_aspects(&["//test:defs.bzl%a"], &["//test:alias_target"])
        .expect("update");

    let top_level_aspects_node =
        f.find_only_node_entry(|key| key.downcast::<AspectKey>().is_some());

    // no aspect key should be requested since the aspect's required provider is not satisfied by
    // the alias's actual target.
    assert!(top_level_aspects_node.is_none());
}

// Java: src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10294 StarlarkDefinedAspectsTest.starlarkDefinedAspectCodec
#[test]
#[ignore = "needs serialization codecs (RoundTripping.roundTripWithSkyframe) and the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn starlark_defined_aspect_codec() {
    let mut f = StarlarkDefinedAspectsTest::new();
    f.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    f.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    // Runs a basic analysis to prime test/aspect.bzl in Skyframe.
    let _unused_analysis_result = f
        .update_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .expect("update");

    // Pulls MyAspect's value out of Skyframe from its BzlLoadValue.
    let aspect_bzl = f
        .get_done_value(&SkyKey::new(BzlLoadKey::key_for_build(parse_label(
            "//test:aspect.bzl",
        ))))
        .downcast::<BzlLoadValue>()
        .unwrap_or_else(|_| panic!("ClassCastException: not a BzlLoadValue"));
    let my_aspect = aspect_bzl
        .get_module()
        .get("MyAspect")
        .unwrap_or_else(|e| panic!("NullPointerException: {e}"));
    assert!(StarlarkDefinedAspect::from_value(my_aspect.value()).is_some());

    let deserialized = round_trip_with_skyframe(&my_aspect);
    assert!(my_aspect.value().ptr_eq(deserialized.value()));
}
