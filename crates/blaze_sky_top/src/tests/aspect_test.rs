//! Port of `src/test/java/com/google/devtools/build/lib/analysis/AspectTest.java` (an
//! `AnalysisTestCase`): aspect creation and merging with configured targets.
//!
//! Most of the Java tests use the native test rules and aspects of `TestAspects` (`base`,
//! `aspect`, `honest`, `simple`, `SIMPLE_ASPECT`, `AspectApplyingToFiles`,
//! `ExtraAttributeAspect`, ...) registered with `TestRuleClassProvider.addStandardRules` through
//! `setRulesAvailableInTests` / `setRulesAndAspectsAvailableInTests`. Blaze has none of them;
//! those tests keep their scratch files and calls, with the assertions on the Java-only
//! provider classes in comments, and are ignored with the exact class they need.
//!
//! PORT: `setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of())` installs
//! `TestRuleClassProvider.addStandardRules` and calls `update()`; the harness's rule class
//! provider is Bazel's (see `testing/build_view_test_case.rs`), so those tests call
//! `update()` only.

use blaze_analysis::info::downcast_info;
use blaze_analysis::test::instrumented_files_info::InstrumentedFilesInfo;
use blaze_events::{EventKind, EventKindSet};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::INSTRUMENTED_FILES_INFO;
use blaze_starlark::structs::StarlarkInfo;

use crate::build_view::BuildViewError;
use crate::testing::build_view_test_case::parse_label;
use crate::testing::{AnalysisTestCase, BuildViewTestCase, base_artifact_names};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

// Java: AspectTest.pkg
fn pkg(t: &mut AnalysisTestCase, name: &str, contents: &[&str]) {
    t.scratch.file(&format!("{name}/BUILD"), contents);
}

/// `MoreAsserts.assertContainsEvent(eventCollector, Pattern, kinds...)`: the first event whose
/// `toString()` the pattern finds, of one of `kinds`. The pattern is a predicate here (no regex
/// crate in this crate's dependencies).
// Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:320 MoreAsserts.assertContainsEvent
fn assert_contains_event_pattern(
    t: &AnalysisTestCase,
    pattern: &str,
    kinds: EventKindSet,
    find: impl Fn(&str) -> bool,
) {
    for event in t.event_collector.iter() {
        if find(&event.to_string()) && kinds.contains(event.get_kind()) {
            return;
        }
    }
    let events: Vec<String> = t.event_collector.iter().map(|e| e.to_string()).collect();
    panic!("Event matching '{pattern}' not found; found these though: {events:?}");
}

/// The Java pattern `Aspects: \[//foo:aspect.bzl%aspect[12]], \[//foo:aspect.bzl%aspect[12]]`.
fn find_two_top_level_aspects(s: &str) -> bool {
    ["1", "2"].iter().any(|a| {
        ["1", "2"].iter().any(|b| {
            s.contains(&format!(
                "Aspects: [//foo:aspect.bzl%aspect{a}], [//foo:aspect.bzl%aspect{b}]"
            ))
        })
    })
}

/// `assertThrows(AssertionError.class, f)`: the fail-fast handler's panic message.
fn assert_throws_assertion_error(f: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("expected an AssertionError");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

// Java: AspectTest.testAspectAppliedToAliasWithSelect
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE, TestAspects.RuleInfo"]
fn test_aspect_applied_to_alias_with_select() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "aspect(name='a', foo=[':b'])",
            "alias(name='b', actual=select({'//conditions:default': ':c'}))",
            "base(name='c')",
        ],
    );
    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:c", "rule //a:a");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.testAspectAppliedToChainedAliases
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE, TestAspects.RuleInfo"]
fn test_aspect_applied_to_chained_aliases() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "aspect(name='a', foo=[':b'])",
            "alias(name='b', actual=':c')",
            "alias(name='c', actual=':d')",
            "alias(name='d', actual=':e')",
            "base(name='e')",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:e", "rule //a:a");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.testAspectAppliedToChainedAliasesAndSelect
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE, TestAspects.RuleInfo"]
fn test_aspect_applied_to_chained_aliases_and_select() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "aspect(name='a', foo=[':b'])",
            "alias(name='b', actual=select({'//conditions:default': ':c'}))",
            "alias(name='c', actual=select({'//conditions:default': ':d'}))",
            "base(name='d')",
        ],
    );
    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:d", "rule //a:a");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.fileProviderMerged
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.FILE_PROVIDER_ASPECT_REQUIRING_RULE"]
fn file_provider_merged() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(
    //     TestAspects.BASE_RULE, TestAspects.FILE_PROVIDER_ASPECT_REQUIRING_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "file_provider_aspect(name='a', dep=':b')",
            "filegroup(name='b', srcs=['source'])",
        ],
    );

    let a = t.get_configured_target("//a:a").unwrap();
    let files_to_build = crate::testing::get_files_to_build(&a);
    crate::testing::assert_contains_exactly(
        &base_artifact_names(files_to_build.to_list().iter()),
        &["file_provider_aspect_file", "source"],
    );
}

// Java: AspectTest.providersOfAspectAreMergedIntoDependency
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE, TestAspects.RuleInfo"]
fn providers_of_aspect_are_merged_into_dependency() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE);
    pkg(
        &mut t,
        "a",
        &["aspect(name='a', foo=[':b'])", "aspect(name='b', foo=[])"],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:b", "rule //a:a");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectPropagatedThroughAliasRule
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.ASPECT_REQUIRING_PROVIDER_RULE, TestAspects.RuleInfo"]
fn aspect_propagated_through_alias_rule() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.ASPECT_REQUIRING_PROVIDER_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "aspect_requiring_provider(name='a', foo=[':b_alias'])",
            "alias(name = 'b_alias', actual = ':b')",
            "honest(name='b', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("rule //a:a", "aspect //a:b");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectPropagatedThroughAliasRuleAndHonestRules
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.ASPECT_REQUIRING_PROVIDER_RULE, TestAspects.RuleInfo"]
fn aspect_propagated_through_alias_rule_and_honest_rules() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.ASPECT_REQUIRING_PROVIDER_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "aspect_requiring_provider(name='a', foo=[':b'])",
            "alias(name = 'b_alias', actual = ':b')",
            "honest(name='b', foo=[':c'])",
            "honest(name='c', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("rule //a:a", "aspect //a:b", "aspect //a:c");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectCreatedIfAdvertisedProviderIsPresent
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.ASPECT_REQUIRING_PROVIDER_RULE, TestAspects.RuleInfo"]
fn aspect_created_if_advertised_provider_is_present() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.ASPECT_REQUIRING_PROVIDER_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "aspect_requiring_provider(name='a', foo=[':b'])",
            "honest(name='b', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("rule //a:a", "aspect //a:b");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectCreatedIfAtLeastOneSetOfAdvertisedProvidersArePresent
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.HONEST_RULE_2, TestAspects.ASPECT_REQUIRING_PROVIDER_SETS_RULE, TestAspects.RuleInfo"]
fn aspect_created_if_at_least_one_set_of_advertised_providers_are_present() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.HONEST_RULE_2, TestAspects.ASPECT_REQUIRING_PROVIDER_SETS_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "aspect_requiring_provider_sets(name='a', foo=[':b', ':c'])",
            "honest(name='b', foo=[])",
            "honest2(name='c', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("rule //a:a", "aspect //a:b", "aspect //a:c");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectWithParametrizedDefinition
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.PARAMETERIZED_DEFINITION_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_with_parametrized_definition() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.PARAMETERIZED_DEFINITION_ASPECT_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "honest(name='q', foo=[])",
            "parametrized_definition_aspect(name='a', foo=[':b'], baz='//a:q')",
            "honest(name='c', foo=[])",
            "honest(name='b', foo=[':c'])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(TestAspects.RuleInfo.class).getData().toList())
    //     .containsExactly(
    //         "rule //a:a",
    //         "aspect //a:b data //a:q $dep:[ //a:q]",
    //         "aspect //a:c data //a:q $dep:[ //a:q]");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectInError
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ERROR_ASPECT_RULE, TestAspects.SIMPLE_RULE"]
fn aspect_in_error() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ERROR_ASPECT_RULE,
    //     TestAspects.SIMPLE_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "simple(name='a', foo=[':b'])",
            "error_aspect(name='b', foo=[':c'])",
            "simple(name='c')",
        ],
    );

    t.remove_fail_fast_handler();
    // getConfiguredTarget() uses a separate code path that does not hit
    // SkyframeBuildView#configureTargets
    assert!(matches!(
        t.update(&["//a:a"]),
        Err(BuildViewError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("Aspect error");
}

// Java: AspectTest.transitiveAspectInError
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ERROR_ASPECT_RULE, TestAspects.SIMPLE_RULE"]
fn transitive_aspect_in_error() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ERROR_ASPECT_RULE,
    //     TestAspects.SIMPLE_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "error_aspect(name='a', foo=[':b'])",
            "error_aspect(name='b', bar=[':c'])",
            "error_aspect(name='c', bar=[':d'])",
            "error_aspect(name='d')",
        ],
    );

    t.remove_fail_fast_handler();
    // getConfiguredTarget() uses a separate code path that does not hit
    // SkyframeBuildView#configureTargets
    assert!(matches!(
        t.update(&["//a:a"]),
        Err(BuildViewError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event("Aspect error");
}

// Java: AspectTest.aspectDependenciesDontShowDeprecationWarnings
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.EXTRA_ATTRIBUTE_ASPECT_RULE"]
fn aspect_dependencies_dont_show_deprecation_warnings() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE,
    //     TestAspects.EXTRA_ATTRIBUTE_ASPECT_RULE);

    pkg(
        &mut t,
        "extra",
        &["base(name='extra', deprecation='bad aspect')"],
    );

    pkg(
        &mut t,
        "a",
        &[
            "rule_with_extra_deps_aspect(name='a', foo=[':b'])",
            "base(name='b')",
        ],
    );

    t.get_configured_target("//a:a");
    t.assert_contains_event_with_frequency("bad aspect", 0);
}

// Java: AspectTest.aspectDependsOnPackageGroup
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.PACKAGE_GROUP_ATTRIBUTE_ASPECT_RULE"]
fn aspect_depends_on_package_group() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(
    //     TestAspects.BASE_RULE, TestAspects.PACKAGE_GROUP_ATTRIBUTE_ASPECT_RULE);
    pkg(&mut t, "extra", &["package_group(name='extra')"]);
    pkg(
        &mut t,
        "a",
        &[
            "rule_with_package_group_deps_aspect(name='a', foo=[':b'])",
            "base(name='b')",
        ],
    );

    t.get_configured_target("//a:a");
    t.assert_contains_event_with_frequency("bad aspect", 0);
}

// Java: AspectTest.aspectWithComputedAttribute
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.COMPUTED_ATTRIBUTE_ASPECT_RULE"]
fn aspect_with_computed_attribute() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE,
    //     TestAspects.COMPUTED_ATTRIBUTE_ASPECT_RULE);

    pkg(
        &mut t,
        "a",
        &[
            "rule_with_computed_deps_aspect(name='a', foo=[':b'])",
            "base(name='b')",
        ],
    );

    t.get_configured_target("//a:a");
}

// Java: AspectTest.ruleDependencyDeprecationWarningsAbsentDuringAspectEvaluations
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE"]
fn rule_dependency_deprecation_warnings_absent_during_aspect_evaluations() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE);

    pkg(&mut t, "a", &["aspect(name='a', foo=['//b:b'])"]);
    pkg(&mut t, "b", &["aspect(name='b', bar=['//d:d'])"]);
    pkg(&mut t, "d", &["base(name='d', deprecation='bad rule')"]);

    t.get_configured_target("//a:a");
    t.assert_contains_event_with_frequency("bad rule", 1);
}

// Java: AspectTest.aspectWarningsFilteredByOutputFiltersForAssociatedRules
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.WARNING_ASPECT_RULE; and Reporter.setOutputFilter(RegexOutputFilter)"]
fn aspect_warnings_filtered_by_output_filters_for_associated_rules() {
    let mut t = new_test();
    // Java: if (getInternalTestExecutionMode() != NORMAL) return; (always NORMAL here)
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.WARNING_ASPECT_RULE);
    pkg(
        &mut t,
        "a",
        &["warning_aspect(name='a', foo=['//b:b', '//c:c'])"],
    );
    pkg(&mut t, "b", &["base(name='b')"]);
    pkg(&mut t, "c", &["base(name='c')"]);

    // Java: reporter.setOutputFilter(RegexOutputFilter.forPattern(Pattern.compile("^//b:")));

    t.get_configured_target("//a:a");
    t.assert_contains_event_with_frequency("Aspect warning on //b:b", 1);
    t.assert_contains_event_with_frequency("Aspect warning on //c:c", 0);
}

// Java: AspectTest.sameTargetInDifferentAttributes
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE, TestAspects.SIMPLE_RULE, TestAspects.RuleInfo"]
fn same_target_in_different_attributes() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.ASPECT_REQUIRING_RULE,
    //     TestAspects.SIMPLE_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "aspect(name='a', foo=[':b'], bar=[':b'])",
            "aspect(name='b', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:b", "rule //a:a");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.sameTargetInDifferentAttributesWithDifferentAspects
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.MULTI_ASPECT_RULE, TestAspects.SIMPLE_RULE, TestAspects.RuleInfo"]
fn same_target_in_different_attributes_with_different_aspects() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.MULTI_ASPECT_RULE,
    //     TestAspects.SIMPLE_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "multi_aspect(name='a', foo=':b', bar=':b')",
            "simple(name='b')",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList()).containsExactly("foo", "bar");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.informationFromBaseRulePassedToAspect
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.HONEST_RULE, TestAspects.ASPECT_REQUIRING_PROVIDER_RULE, TestAspects.RuleInfo"]
fn information_from_base_rule_passed_to_aspect() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.HONEST_RULE,
    //     TestAspects.ASPECT_REQUIRING_PROVIDER_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "aspect_requiring_provider(name='a', foo=[':b'], baz='hello')",
            "honest(name='b', foo=[])",
        ],
    );

    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("rule //a:a", "aspect //a:b data hello");
    unreachable!("needs TestAspects.RuleInfo");
}

/// An Aspect has a late-bound attribute with no value (that is, a LateBoundDefault whose
/// getDefault() returns `null`). Test that this attribute is available in the RuleContext which
/// is provided to the Aspect's `create()` method.
// Java: AspectTest.emptyAspectAttributesAreAvailableInRuleContext
#[test]
#[ignore = "needs TestRuleClassProvider's test rules and native aspects: TestAspects.SIMPLE_ASPECT, TestAspects.BASE_RULE, MockRule testrule, NativeAspectClass AspectWithEmptyLateBoundAttribute, TestAspects.RuleInfo"]
fn empty_aspect_attributes_are_available_in_rule_context() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(
    //     ImmutableList.of(TestAspects.SIMPLE_ASPECT,
    //         EmptyAspectAttributesAreAvailableInRuleContext.AspectWithEmptyLateBoundAttribute
    //             .INSTANCE),
    //     ImmutableList.of(
    //         TestAspects.BASE_RULE, EmptyAspectAttributesAreAvailableInRuleContext.TEST_RULE));
    pkg(
        &mut t,
        "a",
        &["testrule(name='a', foo=[':b'])", "testrule(name='b')"],
    );
    let _a = t.get_configured_target("//a:a");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList()).contains("empty");
    unreachable!("needs TestAspects.RuleInfo");
}

/// Test that actions registered in an Aspect are reported as extra-actions on the attached rule.
/// AspectThatRegistersAction registers a NullAction, whose mnemonic is "Null". We have an
/// action_listener that targets that mnemonic, which makes sure the Aspect machinery will expose
/// an ExtraActionArtifactsProvider. The rule //a:a doesn't have an aspect, so the only action we
/// get is the one on //a:b (which does have an aspect).
// Java: AspectTest.extraActionsAreEmitted
#[test]
#[ignore = "needs TestRuleClassProvider's test rules and native aspects: TestAspects.SIMPLE_ASPECT, TestAspects.BASE_RULE, MockRule testrule, NativeAspectClass AspectThatRegistersAction; and ExtraActionArtifactsProvider"]
fn extra_actions_are_emitted() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(
    //     ImmutableList.of(TestAspects.SIMPLE_ASPECT,
    //         ExtraActionsAreEmitted.AspectThatRegistersAction.INSTANCE),
    //     ImmutableList.of(TestAspects.BASE_RULE, ExtraActionsAreEmitted.TEST_RULE));
    t.use_configuration(&["--experimental_action_listener=//extra_actions:listener"]);
    t.scratch.file(
        "extra_actions/BUILD",
        &[r#"extra_action(
    name = "xa",
    cmd = "echo dont-care",
)

action_listener(
    name = "listener",
    extra_actions = [":xa"],
    mnemonics = ["Null"],
)
"#],
    );
    pkg(
        &mut t,
        "a",
        &["testrule(name='a', foo=[':b'])", "testrule(name='b')"],
    );
    t.update(&[]).unwrap();

    let _a = t.get_configured_target("//a:a");
    // NestedSet<Artifact.DerivedArtifact> extraActionArtifacts =
    //     a.getProvider(ExtraActionArtifactsProvider.class).getTransitiveExtraActionArtifacts();
    // for (Artifact artifact : extraActionArtifacts.toList()) {
    //   assertThat(artifact.getOwnerLabel()).isEqualTo(Label.create("@//a", "b"));
    // }
    unreachable!("needs ExtraActionArtifactsProvider");
}

// Java: AspectTest.aspectPropagatesToAllAttributes
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.ALL_ATTRIBUTES_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_propagates_to_all_attributes() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.ALL_ATTRIBUTES_ASPECT_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name='a', foo=[':b'], foo1=':c', txt='some text')",
            "simple(name='b', foo=[], txt='some text')",
            "simple(name='c', foo=[], txt='more text')",
            "all_attributes_aspect(name='x', foo=[':a'])",
        ],
    );

    let _a = t.get_configured_target("//a:x");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:a", "aspect //a:b", "aspect //a:c", "rule //a:x");
    unreachable!("needs TestAspects.RuleInfo");
}

const EXTRA_ACTIONS_TOP_LEVEL_X_BUILD: &str = r#"load(":extension.bzl", "injector1", "injector2", "null_rule")

injector1(
    name = "a",
    deps = [":b"],
)

null_rule(
    name = "b",
    deps = [":c"],
)

null_rule(
    name = "c",
    deps = [":d"],
)

injector2(
    name = "d",
    extra_deps = [":e"],
)

null_rule(name = "e")
"#;

/// Tests that when --experimental_extra_action_top_level_only, Blaze reports extra-actions for
/// actions registered by Aspects injected by a top-level rule. Because we can't know whether an
/// aspect was injected by a top-level target or one of its children, we approximate it by only
/// reporting extra-actions from Aspects that the top-level target could have injected.
///
/// Here, injector1() and injector2() inject aspects into their children. null_rule() just passes
/// the aspects to its children. The test makes sure that actions registered by aspect1 (injected
/// by injector1()) are reported to the extra-action mechanism. Actions registered by aspect2
/// (from injector2) are not reported, because the target under test (//x:a) doesn't inject
/// aspect2.
// Java: AspectTest.extraActionsAreEmitted_topLevel
#[test]
#[ignore = "BLAZE GAP: unported AnalysisResult.getArtifactsToBuild (Java BuildView.update collects the extra-action artifacts with addExtraActionsIfRequested; blaze_sky_top build_view.rs AnalysisResult has no artifactsToBuild)"]
fn extra_actions_are_emitted_top_level() {
    let mut t = new_test();
    t.use_configuration(&[
        "--experimental_action_listener=//pkg1:listener",
        "--experimental_extra_action_top_level_only",
    ]);

    t.scratch
        .file("x/BUILD", &[EXTRA_ACTIONS_TOP_LEVEL_X_BUILD]);

    t.scratch.file(
        "x/extension.bzl",
        &[r#"def _aspect_impl(target, ctx):
    ctx.actions.do_nothing(mnemonic = "Mnemonic")
    return []

aspect1 = aspect(_aspect_impl, attr_aspects = ["deps"])
aspect2 = aspect(_aspect_impl, attr_aspects = ["extra_deps"])

def _rule_impl(ctx):
    return []

injector1 = rule(_rule_impl, attrs = {"deps": attr.label_list(aspects = [aspect1])})
null_rule = rule(_rule_impl, attrs = {"deps": attr.label_list()})
injector2 = rule(
    _rule_impl,
    attrs = {"extra_deps": attr.label_list(aspects = [aspect2])},
)
"#],
    );

    t.scratch.file(
        "pkg1/BUILD",
        &[r#"extra_action(
    name = "xa",
    cmd = "echo dont-care",
)

action_listener(
    name = "listener",
    extra_actions = [":xa"],
    mnemonics = ["Mnemonic"],
)
"#],
    );

    // Check: //x:d injects an aspect which produces some extra-action.
    {
        t.update(&["//x:d"]).unwrap();

        // Get owners of all extra-action artifacts.
        // for (Artifact artifact : analysisResult.getArtifactsToBuild()) {
        //   if (artifact.getRootRelativePathString().endsWith(".xa")) {
        //     extraArtifactOwners.add(artifact.getOwnerLabel());
        //   }
        // }
        // assertThat(extraArtifactOwners).containsExactly(Label.create("@//x", "e"));
    }

    // Actual test: //x:a reports actions registered by the aspect it injects.
    {
        t.update(&["//x:a"]).unwrap();

        // Get owners of all extra-action artifacts (as above).
        // assertThat(extraArtifactOwners)
        //     .containsExactly(
        //         Label.create("@//x", "b"), Label.create("@//x", "c"), Label.create("@//x", "d"));
    }
    unreachable!("needs AnalysisResult.getArtifactsToBuild");
}

// Java: AspectTest.extraActionsFromDifferentAspectsDontConflict
#[test]
#[ignore = "BLAZE GAP: unported native rule factory ExtraActionFactory (extra_action rule; blaze_analysis rule/configured_target_factory.rs:523 RuleConfiguredTargetFactory.create)"]
fn extra_actions_from_different_aspects_dont_conflict() {
    let mut t = new_test();
    t.use_configuration(&[
        "--experimental_action_listener=//pkg1:listener",
        "--experimental_extra_action_top_level_only",
    ]);

    t.scratch.file(
        "x/BUILD",
        &[
            r#"load(":extension.bzl", "injector1", "injector2", "null_rule")

injector2(
    name = "i2_a",
    deps = [":i1_a"],
)

injector1(
    name = "i1_a",
    param = "a",
    deps = [":n"],
)

injector1(
    name = "i1_b",
    param = "b",
    deps = [":n"],
)

injector2(
    name = "i2",
    deps = [":n"],
)

null_rule(name = "n")
"#,
        ],
    );

    t.scratch.file(
        "x/extension.bzl",
        &[r#"def _aspect_impl(target, ctx):
    ctx.actions.do_nothing(mnemonic = "Mnemonic")
    return []

aspect1 = aspect(
    _aspect_impl,
    attr_aspects = ["deps"],
    attrs = {"param": attr.string(values = ["a", "b"])},
)
aspect2 = aspect(_aspect_impl, attr_aspects = ["deps"])

def _rule_impl(ctx):
    return []

injector1 = rule(
    _rule_impl,
    attrs = {"deps": attr.label_list(aspects = [aspect1]), "param": attr.string()},
)
injector2 = rule(_rule_impl, attrs = {"deps": attr.label_list(aspects = [aspect2])})
null_rule = rule(_rule_impl, attrs = {"deps": attr.label_list()})
"#],
    );

    t.scratch.file(
        "pkg1/BUILD",
        &[r#"extra_action(
    name = "xa",
    cmd = "echo dont-care",
)

action_listener(
    name = "listener",
    extra_actions = [":xa"],
    mnemonics = ["Mnemonic"],
)
"#],
    );

    t.update(&["//x:i1_a", "//x:i1_b", "//x:i2", "//x:i2_a"])
        .unwrap();

    // Implicitly check that update() didn't throw an exception because of two actions producing
    // the same outputs.
}

// Java: AspectTest.sharedArtifactsInAspect
#[test]
fn shared_artifacts_in_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "foo/shared_aspect.bzl",
        &[r#"MyInfo = provider()
def _shared_aspect_impl(target, ctx):
    shared_file = ctx.actions.declare_file("shared_file")
    ctx.actions.write(output = shared_file, content = "Shared content")
    lib = ctx.rule.attr.lib
    if lib:
        result = depset([shared_file], transitive = [ctx.rule.attr.lib[MyInfo].prov])
    else:
        result = depset([shared_file])
    return MyInfo(prov = result)

shared_aspect = aspect(
    implementation = _shared_aspect_impl,
    attr_aspects = ["lib"],
)

def _rule_impl(ctx):
    pass

simple_rule = rule(
    implementation = _rule_impl,
    attrs = {"lib": attr.label(
        providers = [MyInfo],
        aspects = [shared_aspect],
    )},
)
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &[
            r#"load(":shared_aspect.bzl", "shared_aspect", "simple_rule")

simple_rule(
    name = "top_rule",
    lib = ":first_dep",
)

simple_rule(
    name = "first_dep",
    lib = ":second_dep",
)

simple_rule(name = "second_dep")
"#,
        ],
    );
    // Confirm that load is successful and doesn't crash.
    t.update(&["//foo:top_rule"]).unwrap();
}

// Java: AspectTest.aspectPropagatesToAllAttributesImplicit
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.IMPLICIT_DEP_RULE, TestAspects.ALL_ATTRIBUTES_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_propagates_to_all_attributes_implicit() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.IMPLICIT_DEP_RULE, TestAspects.ALL_ATTRIBUTES_ASPECT_RULE);
    t.scratch.file("extra/BUILD", &["simple(name ='extra')"]);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name='a', foo=[':b'], foo1=':c', txt='some text')",
            "simple(name='b', foo=[], txt='some text')",
            "implicit_dep(name='c')",
            "all_attributes_aspect(name='x', foo=[':a'])",
        ],
    );
    t.update(&[]).unwrap();

    let _a = t.get_configured_target("//a:x");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly(
    //         "aspect //a:a", "aspect //a:b", "aspect //a:c", "aspect //extra:extra",
    //         "rule //a:x");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectPropagatesToAllAttributesLateBound
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.LATE_BOUND_DEP_RULE, TestAspects.ALL_ATTRIBUTES_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_propagates_to_all_attributes_late_bound() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.LATE_BOUND_DEP_RULE, TestAspects.ALL_ATTRIBUTES_ASPECT_RULE);

    t.scratch.file("extra/BUILD", &["simple(name ='extra')"]);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name='a', foo=[':b'], foo1=':c', txt='some text')",
            "simple(name='b', foo=[], txt='some text')",
            "late_bound_dep(name='c')",
            "all_attributes_aspect(name='x', foo=[':a'])",
        ],
    );
    t.use_configuration(&["--plugin=//extra:extra"]);
    t.update(&[]).unwrap();

    let _a = t.get_configured_target("//a:x");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly(
    //         "aspect //a:a", "aspect //a:b", "aspect //a:c", "aspect //extra:extra",
    //         "rule //a:x");
    unreachable!("needs TestAspects.RuleInfo");
}

/// Ensures an aspect with attr = '*' doesn't try to propagate to its own implicit attributes.
/// Doing so leads to a dependency cycle.
// Java: AspectTest.aspectWithAllAttributesDoesNotPropagateToOwnImplicitAttributes
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.ALL_ATTRIBUTES_WITH_TOOL_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_with_all_attributes_does_not_propagate_to_own_implicit_attributes() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.ALL_ATTRIBUTES_WITH_TOOL_ASPECT_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name='tool')",
            "simple(name='a')",
            "all_attributes_with_tool_aspect(name='x', foo=[':a'])",
        ],
    );

    let _a = t.get_configured_target("//a:x");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly("aspect //a:a", "rule //a:x");
    unreachable!("needs TestAspects.RuleInfo");
}

/// Makes sure the aspect *will* propagate to its implicit attributes if there is a "regular"
/// dependency path to it (i.e. not through its own implicit attributes).
// Java: AspectTest.aspectWithAllAttributesPropagatesToItsToolIfThereIsPath
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.ALL_ATTRIBUTES_WITH_TOOL_ASPECT_RULE, TestAspects.RuleInfo"]
fn aspect_with_all_attributes_propagates_to_its_tool_if_there_is_path() {
    let mut t = new_test();
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.ALL_ATTRIBUTES_WITH_TOOL_ASPECT_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name='tool')",
            "simple(name='a', foo=[':b'], foo1=':c', txt='some text')",
            "simple(name='b', foo=[], txt='some text')",
            "simple(name='c', foo=[':tool'], txt='more text')",
            "all_attributes_with_tool_aspect(name='x', foo=[':a'])",
        ],
    );

    let _a = t.get_configured_target("//a:x");
    // assertThat(a.getProvider(RuleInfo.class).getData().toList())
    //     .containsExactly(
    //         "aspect //a:a", "aspect //a:b", "aspect //a:c", "aspect //a:tool", "rule //a:x");
    unreachable!("needs TestAspects.RuleInfo");
}

// Java: AspectTest.aspectTruthInAdvertisement
#[test]
#[ignore = "needs TestRuleClassProvider's test rules: TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE, TestAspects.FALSE_ADVERTISEMENT_ASPECT_RULE"]
fn aspect_truth_in_advertisement() {
    let mut t = new_test();
    t.remove_fail_fast_handler(); // expect errors
    // Java: setRulesAvailableInTests(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE,
    //     TestAspects.FALSE_ADVERTISEMENT_ASPECT_RULE);
    pkg(
        &mut t,
        "a",
        &[
            "simple(name = 's')",
            "false_advertisement_aspect(name = 'x', deps = [':s'])",
        ],
    );
    // ViewCreationFailedException expected.
    let _ = t.update(&["//a:x"]);
    t.assert_contains_event(
        "Aspect 'FalseAdvertisementAspect', applied to '//a:s', does not provide advertised \
         provider 'RequiredProvider'",
    );
}

// Java: AspectTest.aspectApplyingToFiles
#[test]
#[ignore = "BLAZE GAP: unported!(\"ConfiguredRuleClassProvider.getNativeAspectClassMap\") in build_view.rs for the native test aspect AspectApplyingToFiles (TestAspects, registered by Java's test rule class provider)"]
fn aspect_applying_to_files() {
    let mut t = new_test();
    // Java: AspectApplyingToFiles aspectApplyingToFiles = new AspectApplyingToFiles();
    //     setRulesAndAspectsAvailableInTests(ImmutableList.of(aspectApplyingToFiles), of());
    pkg(
        &mut t,
        "a",
        &[
            "load('@rules_java//java:defs.bzl', 'java_binary')",
            "java_binary(name = 'x', main_class = 'x.FooBar', srcs = ['x.java'])",
        ],
    );

    // Java: eventBus.register(new AspectConfiguredCollector());
    t.update_with_aspects(&["AspectApplyingToFiles"], &["//a:x_deploy.jar"])
        .unwrap();
    // ConfiguredAspect aspect = getOnlyElement(analysisResult.getAspectsMap().values());
    // AspectApplyingToFiles.Provider provider =
    //     aspect.getProvider(AspectApplyingToFiles.Provider.class);
    // Label label = Label.parseCanonicalUnchecked("//a:x_deploy.jar");
    // assertThat(provider.getLabel()).isEqualTo(label);
    //
    // AspectConfiguredEvent configuredEvent = getOnlyElement(collector.events);
    // BuildEventId targetCompletedId = getOnlyElement(configuredEvent.getChildrenEvents());
    // AspectKey key = getOnlyElement(analysisResult.getAspectsMap().keySet());
    // assertThat(targetCompletedId)
    //     .isEqualTo(BuildEventIdUtil.aspectCompleted(label,
    //         BuildEventIdUtil.configurationId(key.getConfigurationKey()),
    //         "AspectApplyingToFiles"));
    unreachable!("needs TestAspects.AspectApplyingToFiles");
}

// Java: AspectTest.aspectApplyingToSourceFilesIgnored
#[test]
#[ignore = "BLAZE GAP: unported!(\"ConfiguredRuleClassProvider.getNativeAspectClassMap\") in build_view.rs for the native test aspect AspectApplyingToFiles (TestAspects, registered by Java's test rule class provider)"]
fn aspect_applying_to_source_files_ignored() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(ImmutableList.of(aspectApplyingToFiles), of());
    pkg(
        &mut t,
        "a",
        &[
            "load('@rules_java//java:defs.bzl', 'java_binary')",
            "java_binary(name = 'x', main_class = 'x.FooBar', srcs = ['x.java'])",
        ],
    );
    t.scratch.file("a/x.java", &[""]);
    let result = t
        .update_with_aspects(&["AspectApplyingToFiles"], &["//a:x.java"])
        .unwrap();
    assert_eq!(result.get_aspects_map().len(), 1);
    // assertThat(aspect.getProvider(AspectApplyingToFiles.Provider.class)).isNull();
    unreachable!("needs TestAspects.AspectApplyingToFiles.Provider");
}

// Java: AspectTest.aspectApplyingToPackageGroupIgnored
#[test]
#[ignore = "BLAZE GAP: unported!(\"ConfiguredRuleClassProvider.getNativeAspectClassMap\") in build_view.rs for the native test aspect AspectApplyingToFiles (TestAspects, registered by Java's test rule class provider)"]
fn aspect_applying_to_package_group_ignored() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(ImmutableList.of(aspectApplyingToFiles), of());
    pkg(&mut t, "b", &[]);
    pkg(
        &mut t,
        "a",
        &[
            "load('@rules_java//java:defs.bzl', 'java_binary')",
            "package_group(name = 'group', packages = ['//b'])",
            "java_binary(name = 'x', main_class = 'x.F', srcs = ['x.java'], visibility = [':group'])",
        ],
    );
    t.scratch.file("a/x.java", &[""]);

    // This exercises a code path that crashes if the PackageGroup is matched as an aspect
    // provider.
    let result = t
        .update_with_aspects(&["AspectApplyingToFiles"], &["//a:group"])
        .unwrap();
    assert_eq!(result.get_aspects_map().len(), 1);
}

// Java: AspectTest.duplicateTopLevelAspects_allowedAndDeduplicated
#[test]
#[ignore = "BLAZE GAP: unported!(\"ConfiguredRuleClassProvider.getNativeAspectClassMap\") in build_view.rs for the native test aspect AspectApplyingToFiles (TestAspects, registered by Java's test rule class provider)"]
fn duplicate_top_level_aspects_allowed_and_deduplicated() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(ImmutableList.of(aspectApplyingToFiles), of());
    pkg(
        &mut t,
        "a",
        &[
            "load('@rules_java//java:defs.bzl', 'java_binary')",
            "java_binary(name = 'x', main_class = 'x.FooBar', srcs = ['x.java'])",
        ],
    );
    t.remove_fail_fast_handler();
    let result = t
        .update_with_aspects(
            &["AspectApplyingToFiles", "AspectApplyingToFiles"],
            &["//a:x_deploy.jar"],
        )
        .unwrap();
    assert_eq!(result.get_aspects_map().len(), 1);
}

// Java: AspectTest.aspectWithExtraAttribute_ignoredForOutputFile
#[test]
#[ignore = "needs the native test aspect TestAspects.ExtraAttributeAspect(\"//nonexistent\", false) and its PROVIDER"]
fn aspect_with_extra_attribute_ignored_for_output_file() {
    let mut t = new_test();
    // Java: ExtraAttributeAspect aspect =
    //     new ExtraAttributeAspect("//nonexistent", /*applyToFiles=*/ false);
    //     setRulesAndAspectsAvailableInTests(ImmutableList.of(aspect), ImmutableList.of());
    t.scratch.file(
        "a/BUILD",
        &["genrule(name='gen_a', outs=['a'], cmd='touch $@')"],
    );

    let result = t
        .update_with_aspects(&["ExtraAttributeAspect_//nonexistent_false"], &["//a"])
        .unwrap();

    assert_eq!(result.get_aspects_map().len(), 1);
    // assertThat(configuredAspect.get(ExtraAttributeAspect.PROVIDER.getKey())).isNull();
    unreachable!("needs ExtraAttributeAspect.PROVIDER");
}

// Java: AspectTest.aspectWithExtraAttributeApplyToFiles_outputFile_hasResolvedAttribute
#[test]
#[ignore = "needs the native test aspect TestAspects.ExtraAttributeAspect(\"//extra\", true) and its PROVIDER, and TestRuleClassProvider's test rules TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE"]
fn aspect_with_extra_attribute_apply_to_files_output_file_has_resolved_attribute() {
    let mut t = new_test();
    // Java: ExtraAttributeAspect aspect = new ExtraAttributeAspect("//extra", true);
    //     setRulesAndAspectsAvailableInTests(
    //         ImmutableList.of(aspect),
    //         ImmutableList.of(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE));
    t.scratch.file("extra/BUILD", &["simple(name='extra')"]);
    t.scratch.file(
        "a/BUILD",
        &["genrule(name='gen_a', outs=['a'], cmd='touch $@')"],
    );

    t.update_with_aspects(&["ExtraAttributeAspect_//extra_true"], &["//a"])
        .unwrap();

    // StarlarkInfo provider =
    //     (StarlarkInfo) configuredAspect.get(ExtraAttributeAspect.PROVIDER.getKey());
    // assertThat(provider.getValue("label")).isEqualTo("//extra:extra");
    unreachable!("needs ExtraAttributeAspect.PROVIDER");
}

// Java: AspectTest.aspectWithExtraAttributeApplyToFiles_ignoredForSourceFile
#[test]
#[ignore = "needs the native test aspect TestAspects.ExtraAttributeAspect(\"//nonexistent\", true) and its PROVIDER"]
fn aspect_with_extra_attribute_apply_to_files_ignored_for_source_file() {
    let mut t = new_test();
    // Java: ExtraAttributeAspect aspect = new ExtraAttributeAspect("//nonexistent", true);
    //     setRulesAndAspectsAvailableInTests(ImmutableList.of(aspect), ImmutableList.of());
    t.scratch.file("a/BUILD", &["exports_files(['a.txt'])"]);
    t.scratch.file("a/a.txt", &["hello"]);

    let result = t
        .update_with_aspects(&["ExtraAttributeAspect_//nonexistent_true"], &["//a:a.txt"])
        .unwrap();

    assert_eq!(result.get_aspects_map().len(), 1);
    // assertThat(configuredAspect.get(ExtraAttributeAspect.PROVIDER.getKey())).isNull();
    unreachable!("needs ExtraAttributeAspect.PROVIDER");
}

// Java: AspectTest.aspectWithExtraAttributeApplyToFilesAndNot_outputFile_onlyApplyToFilesIsResolved
#[test]
#[ignore = "needs the native test aspects TestAspects.ExtraAttributeAspect(\"//extra\", true) and (\"//nonexistent\", false) and their PROVIDER, and TestRuleClassProvider's test rules TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE"]
fn aspect_with_extra_attribute_apply_to_files_and_not_output_file_only_apply_to_files_is_resolved()
{
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(
    //     ImmutableList.of(aspectApplies, aspectDoesNotApply),
    //     ImmutableList.of(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE));
    t.scratch.file("extra/BUILD", &["simple(name='extra')"]);
    t.scratch.file(
        "a/BUILD",
        &["genrule(name='gen_a', outs=['a'], cmd='touch $@')"],
    );

    let result = t
        .update_with_aspects(
            &[
                "ExtraAttributeAspect_//extra_true",
                "ExtraAttributeAspect_//nonexistent_false",
            ],
            &["//a"],
        )
        .unwrap();

    assert_eq!(result.get_aspects_map().len(), 2);
    // StarlarkInfo provider = (StarlarkInfo) getAspectByName(analysisResult.getAspectsMap(),
    //     aspectApplies.getName()).get(ExtraAttributeAspect.PROVIDER.getKey());
    // assertThat(provider.getValue("label")).isEqualTo("//extra:extra");
    // assertThat(getAspectByName(analysisResult.getAspectsMap(), aspectDoesNotApply.getName())
    //     .getProviders().getProviderCount()).isEqualTo(0);
    unreachable!("needs ExtraAttributeAspect.PROVIDER");
}

// Java: AspectTest.aspectWithExtraAttributeDependsOnNotApplicable_usesItsOwnAttribute
#[test]
#[ignore = "needs the native test aspects TestAspects.ExtraAttributeAspect(\"//extra\", true, PROVIDER) and (\"//extra:extra2\", false) and their PROVIDER, and TestRuleClassProvider's test rules TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE"]
fn aspect_with_extra_attribute_depends_on_not_applicable_uses_its_own_attribute() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(
    //     ImmutableList.of(aspectApplies, aspectDoesNotApply),
    //     ImmutableList.of(TestAspects.BASE_RULE, TestAspects.SIMPLE_RULE));
    t.scratch.file(
        "extra/BUILD",
        &["simple(name = \"extra\")\n\nsimple(name = \"extra2\")\n"],
    );
    t.scratch.file(
        "a/BUILD",
        &["genrule(name='gen_a', outs=['a'], cmd='touch $@')"],
    );

    let result = t
        .update_with_aspects(
            &[
                "ExtraAttributeAspect_//extra:extra2_false",
                "ExtraAttributeAspect_//extra_true",
            ],
            &["//a"],
        )
        .unwrap();

    assert_eq!(result.get_aspects_map().len(), 2);
    // StarlarkInfo provider = (StarlarkInfo) getAspectByName(analysisResult.getAspectsMap(),
    //     aspectApplies.getName()).get(ExtraAttributeAspect.PROVIDER.getKey());
    // assertThat(provider.getValue("label")).isEqualTo("//extra:extra");
    // assertThat(getAspectByName(analysisResult.getAspectsMap(), aspectDoesNotApply.getName())
    //     .getProviders().getProviderCount()).isEqualTo(0);
    unreachable!("needs ExtraAttributeAspect.PROVIDER");
}

// Java: AspectTest.sameConfiguredAttributeOnAspectAndRule
#[test]
fn same_configured_attribute_on_aspect_and_rule() {
    let mut t = new_test();
    t.scratch.file(
        "a/a.bzl",
        &[r#"def _a_impl(t, ctx):
    return [DefaultInfo()]

def _r_impl(ctx):
    return [DefaultInfo()]

a = aspect(
    implementation = _a_impl,
    attrs = {"_f": attr.label(
        default = configuration_field(fragment = "coverage", name = "output_generator"),
    )},
)
r = rule(
    implementation = _r_impl,
    attrs = {
        "_f": attr.label(
            default =
                configuration_field(fragment = "coverage", name = "output_generator"),
        ),
        "dep": attr.label(aspects = [a]),
    },
)
"#],
    );

    t.scratch
        .file("a/BUILD", &["load(\":a.bzl\", \"r\")\n\nr(name = \"r\")\n"]);

    // Java: setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of()) (PORT:
    // module documentation).
    t.update(&[]).unwrap();
    t.get_configured_target("//a:r");
}

/// The bzl file of `topLevelConflictDetected` (`String.join("\n", ...)`, one `%s`).
const TOP_LEVEL_CONFLICT_BZL_TEMPLATE: &str = "def _aspect1_impl(target, ctx):
  outfile = ctx.actions.declare_file('aspect.out')
  ctx.actions.run_shell(
    outputs = [outfile],
    progress_message = 'Action for aspect 1',
    command = 'echo \"1\" > ' + outfile.path,
  )
  return [OutputGroupInfo(files = [outfile])]
def _aspect2_impl(target, ctx):
  outfile = ctx.actions.declare_file('aspect.out')
  ctx.actions.run_shell(
    outputs = [outfile],
    progress_message = 'Action for aspect 2',
    command = 'echo \"%s\" > ' + outfile.path,
  )
  return [OutputGroupInfo(files = [outfile])]
aspect1 = aspect(implementation = _aspect1_impl)
aspect2 = aspect(implementation = _aspect2_impl)";

// Java: AspectTest.topLevelConflictDetected
#[test]
fn top_level_conflict_detected() {
    let mut t = new_test();
    let bzl_file = |arg: &str| TOP_LEVEL_CONFLICT_BZL_TEMPLATE.replacen("%s", arg, 1);
    let aspects_pattern =
        "Aspects: \\[//foo:aspect.bzl%aspect[12]], \\[//foo:aspect.bzl%aspect[12]]";
    t.scratch.file("foo/aspect.bzl", &[&bzl_file("2")]);
    t.scratch
        .file("foo/BUILD", &["filegroup(name = 'foo', srcs = ['foo.sh'])"]);
    // Expect errors.
    t.remove_fail_fast_handler();
    let flags = t.default_flags();
    let exception = match t.update_with(
        &flags,
        &["//foo:aspect.bzl%aspect1", "//foo:aspect.bzl%aspect2"],
        &["//foo:foo"],
    ) {
        Err(e @ BuildViewError::ViewCreationFailed { .. }) => e,
        Err(e) => panic!("expected ViewCreationFailedException, got {e:?}"),
        Ok(_) => panic!("expected ViewCreationFailedException"),
    };
    let message = exception.to_string();
    assert!(
        message.contains("file 'foo/aspect.out' is generated by these conflicting actions:"),
        "{message}"
    );
    assert_contains_event_pattern(
        &t,
        aspects_pattern,
        EventKindSet::of(&[EventKind::Error]),
        find_two_top_level_aspects,
    );

    // Fix bzl file so actions are shared: analysis should succeed now.
    t.scratch
        .overwrite_file("foo/aspect.bzl", &[&bzl_file("1")]);
    t.reporter.add_handler(t.fail_fast_handler.clone());
    let result = t
        .update_with(
            &flags,
            &["//foo:aspect.bzl%aspect1", "//foo:aspect.bzl%aspect2"],
            &["//foo:foo"],
        )
        .unwrap();
    assert_eq!(result.get_aspects_map().len(), 2);

    // Break bzl file again: we should notice.
    t.scratch
        .overwrite_file("foo/aspect.bzl", &[&bzl_file("2")]);
    // Expect errors.
    t.remove_fail_fast_handler();
    assert!(matches!(
        t.update_with(
            &flags,
            &["//foo:aspect.bzl%aspect1", "//foo:aspect.bzl%aspect2"],
            &["//foo:foo"],
        ),
        Err(BuildViewError::ViewCreationFailed { .. })
    ));
    assert_contains_event_pattern(
        &t,
        aspects_pattern,
        EventKindSet::of(&[EventKind::Error]),
        find_two_top_level_aspects,
    );
}

// Java: AspectTest.conflictBetweenTargetAndAspect
#[test]
fn conflict_between_target_and_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "foo/aspect.bzl",
        &[r#"def _aspect_impl(target, ctx):
    outfile = ctx.actions.declare_file("conflict.out")
    ctx.actions.run_shell(
        outputs = [outfile],
        progress_message = "Action for aspect",
        command = 'echo "aspect" > ' + outfile.path,
    )
    return [OutputGroupInfo(files = [outfile])]

def _rule_impl(ctx):
    outfile = ctx.actions.declare_file("conflict.out")
    ctx.actions.run_shell(
        outputs = [outfile],
        progress_message = "Action for target",
        command = 'echo "target" > ' + outfile.path,
    )
    return [DefaultInfo(files = depset([outfile]))]

my_aspect = aspect(implementation = _aspect_impl)
my_rule = rule(
    implementation = _rule_impl,
    attrs = {"deps": attr.label_list(aspects = [my_aspect])},
)
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load("//foo:aspect.bzl", "my_aspect", "my_rule")

my_rule(
    name = "foo",
    deps = [":dep"],
)

filegroup(
    name = "dep",
    srcs = ["dep.sh"],
)
"#],
    );
    // Expect errors.
    t.remove_fail_fast_handler();
    let exception = match t.update(&["//foo:foo"]) {
        Err(e @ BuildViewError::ViewCreationFailed { .. }) => e,
        Err(e) => panic!("expected ViewCreationFailedException, got {e:?}"),
        Ok(_) => panic!("expected ViewCreationFailedException"),
    };
    let message = exception.to_string();
    assert!(
        message.contains("file 'foo/conflict.out' is generated by these conflicting actions"),
        "{message}"
    );
    assert_contains_event_pattern(
        &t,
        "Aspects: (\\[], \\[//foo:aspect.bzl%my_aspect]|\\[//foo:aspect.bzl%my_aspect], \\[])",
        EventKindSet::of(&[EventKind::Error]),
        |s| {
            s.contains("Aspects: [], [//foo:aspect.bzl%my_aspect]")
                || s.contains("Aspects: [//foo:aspect.bzl%my_aspect], []")
        },
    );
}

// Java: AspectTest.aspectDuplicatesRuleProviderError
#[test]
fn aspect_duplicates_rule_provider_error() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of()) (PORT:
    // module documentation).
    t.update(&[]).unwrap();
    t.scratch.file(
        "aspect/build_defs.bzl",
        &[r#"DuplicateInfo = provider(fields=[])
def _aspect_impl(target, ctx):
    return [DuplicateInfo()]

returns_duplicate_aspect = aspect(implementation = _aspect_impl)

def _duplicate_rule_impl(ctx):
  return [DefaultInfo(), DuplicateInfo()]

duplicate_rule = rule(implementation = _duplicate_rule_impl, attrs = {})

def _rule_impl(ctx):
    pass

duplicate_aspect_applying_rule = rule(
    implementation = _rule_impl,
    attrs = {"to": attr.label(aspects = [returns_duplicate_aspect])},
)
"#],
    );
    t.scratch.file(
        "aspect/BUILD",
        &[
            r#"load("build_defs.bzl", "duplicate_aspect_applying_rule", "duplicate_rule")

duplicate_rule(name = "duplicate")

duplicate_aspect_applying_rule(
    name = "applies_aspect",
    to = ":duplicate",
)
"#,
        ],
    );
    let message = assert_throws_assertion_error(|| {
        t.get_configured_target("//aspect:applies_aspect");
    });
    assert!(
        message.contains("Provider DuplicateInfo provided twice"),
        "{message}"
    );
}

// Java: AspectTest.instrumentedFilesInfoFromBaseRuleAndAspectUsesAspect
#[test]
#[ignore = "BLAZE GAP: unported BaselineCoverageAction.create (blaze_analysis test/instrumented_files_info.rs:338, coverage_common.instrumented_files_info under --collect_code_coverage)"]
fn instrumented_files_info_from_base_rule_and_aspect_uses_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "aspect/build_defs.bzl",
        &[r#"def _instrumented_files_info_aspect_impl(target, ctx):
    return [coverage_common.instrumented_files_info(ctx, source_attributes = ["a"])]

instrumented_files_info_aspect = aspect(
    implementation = _instrumented_files_info_aspect_impl,
)

def _no_instrumented_files_info_aspect_impl(target, ctx):
    return []

no_instrumented_files_info_aspect = aspect(
    implementation = _no_instrumented_files_info_aspect_impl,
)

def _applies_aspect_impl(ctx):
    return coverage_common.instrumented_files_info(ctx, dependency_attributes = ["to"])

instrumented_files_info_aspect_rule = rule(
    implementation = _applies_aspect_impl,
    attrs = {"to": attr.label(aspects = [instrumented_files_info_aspect])},
)

no_instrumented_files_info_aspect_rule = rule(
    implementation = _applies_aspect_impl,
    attrs = {"to": attr.label(aspects = [no_instrumented_files_info_aspect])},
)

def _base_rule_impl(ctx):
    return [coverage_common.instrumented_files_info(ctx, source_attributes = ["b"])]

base_rule = rule(
    implementation = _base_rule_impl,
    attrs = {"a": attr.label(allow_files = True), "b": attr.label(allow_files = True)},
)

def _base_rule_no_coverage_impl(ctx):
    return []

base_rule_no_coverage = rule(
    implementation = _base_rule_no_coverage_impl,
    attrs = {"a": attr.label(allow_files = True), "b": attr.label(allow_files = True)},
)
"#],
    );
    t.scratch.file(
        "aspect/BUILD",
        &[r#"load(
    "build_defs.bzl",
    "base_rule",
    "base_rule_no_coverage",
    "instrumented_files_info_aspect_rule",
    "no_instrumented_files_info_aspect_rule",
)

base_rule(
    name = "rule_target",
    # Ends up in coverage sources when instrumented_files_info_aspect is applied
    a = "a",
    # Ends up in coverage sources for the base rule's InstrumentedFilesInfo is used
    b = "b",
)

instrumented_files_info_aspect_rule(
    name = "duplicate_instrumented_file_info",
    to = ":rule_target",
)

no_instrumented_files_info_aspect_rule(
    name = "instrumented_file_info_from_base_target",
    to = ":rule_target",
)

base_rule_no_coverage(
    name = "rule_target_no_coverage",
    # Ends up in coverage sources when instrumented_files_info_aspect is applied
    a = "a",
    # Ends up in coverage sources never
    b = "b",
)

instrumented_files_info_aspect_rule(
    name = "instrumented_files_info_only_from_aspect",
    to = ":rule_target_no_coverage",
)

no_instrumented_files_info_aspect_rule(
    name = "no_instrumented_files_info",
    to = ":rule_target_no_coverage",
)
"#],
    );
    t.use_configuration(&["--collect_code_coverage", "--instrumentation_filter=.*"]);
    t.update(&[]).unwrap();
    assert_eq!(
        get_instrumented_files(&mut t, "//aspect:rule_target"),
        ["b"]
    );
    assert_eq!(
        get_instrumented_files(&mut t, "//aspect:duplicate_instrumented_file_info"),
        ["a"]
    );
    assert_eq!(
        get_instrumented_files(&mut t, "//aspect:instrumented_file_info_from_base_target"),
        ["b"]
    );
    assert!(get_instrumented_files(&mut t, "//aspect:rule_target_no_coverage").is_empty());
    assert_eq!(
        get_instrumented_files(&mut t, "//aspect:instrumented_files_info_only_from_aspect"),
        ["a"]
    );
    assert!(get_instrumented_files(&mut t, "//aspect:no_instrumented_files_info").is_empty());
}

// Java: AspectTest.getInstrumentedFiles
fn get_instrumented_files(t: &mut AnalysisTestCase, label: &str) -> Vec<String> {
    let target = t
        .get_configured_target(label)
        .unwrap_or_else(|| panic!("no configured target {label}"));
    let info = target
        .get(&INSTRUMENTED_FILES_INFO.key())
        .unwrap_or_else(|| panic!("NullPointerException: no InstrumentedFilesInfo on {label}"));
    let info = downcast_info::<InstrumentedFilesInfo>(&info).expect("InstrumentedFilesInfo");
    base_artifact_names(info.get_instrumented_files().to_list().iter())
}

// Java: AspectTest.aspectSeesAspectHintsAttributeOnNativeRule
#[test]
fn aspect_sees_aspect_hints_attribute_on_native_rule() {
    let mut t = new_test();
    setup_aspect_hints(&mut t);
    t.scratch.file(
        "aspect_hints/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("//aspect_hints:hints.bzl", "hint")
load("//aspect_hints:hints_counter.bzl", "count_hints")

hint(
    name = "my_hint",
    hints_cnt = 3,
)

cc_library(
    name = "lib1",
    deps = [":lib2"],
)

cc_library(
    name = "lib2",
    aspect_hints = [":my_hint"],
)

count_hints(
    name = "cnt",
    deps = [":lib1"],
)
"#],
    );
    t.update(&[]).unwrap();

    let a = t.get_configured_target("//aspect_hints:cnt").unwrap();
    assert_eq!(get_hints_cnt(&a), 3);
}

// Java: AspectTest.aspectSeesAspectHintsAttributeOnStarlarkRule
#[test]
fn aspect_sees_aspect_hints_attribute_on_starlark_rule() {
    let mut t = new_test();
    setup_aspect_hints(&mut t);
    setup_starlark_rule(&mut t);
    t.scratch.file(
        "aspect_hints/BUILD",
        &[r#"load("//aspect_hints:custom_rule.bzl", "custom_rule")
load("//aspect_hints:hints.bzl", "hint")
load("//aspect_hints:hints_counter.bzl", "count_hints")

hint(
    name = "my_hint",
    hints_cnt = 2,
)

custom_rule(
    name = "lib1",
    deps = [":lib2"],
)

custom_rule(
    name = "lib2",
    aspect_hints = [":my_hint"],
)

count_hints(
    name = "cnt",
    deps = [":lib1"],
)
"#],
    );
    t.update(&[]).unwrap();

    let a = t.get_configured_target("//aspect_hints:cnt").unwrap();
    assert_eq!(get_hints_cnt(&a), 2);
}

// Java: AspectTest.ruleDepsVisibilityNotAffectNativeAspect
#[test]
#[ignore = "needs the native test aspect TestAspects.ALL_ATTRIBUTES_ASPECT (setRulesAndAspectsAvailableInTests)"]
fn rule_deps_visibility_not_affect_native_aspect() {
    let mut t = new_test();
    // Java: setRulesAndAspectsAvailableInTests(
    //     ImmutableList.of(TestAspects.ALL_ATTRIBUTES_ASPECT), ImmutableList.of());
    t.scratch.file("defs/BUILD", &[]);
    t.scratch.file(
        "defs/build_defs.bzl",
        &[r#"def _rule_impl(ctx):
    pass

implicit_dep_rule = rule(
    implementation = _rule_impl,
    attrs = {
        "_tool": attr.label(default = "//tool:tool"),
        "deps": attr.label_list(),
    },
)
"#],
    );
    t.scratch.file(
        "tool/BUILD",
        &["filegroup(name='tool', visibility = ['//defs:__pkg__'])"],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("//defs:build_defs.bzl", "implicit_dep_rule")

implicit_dep_rule(name = "y")

implicit_dep_rule(
    name = "x",
    deps = [":y"],
)
"#],
    );

    // Java: ImmutableList.of(TestAspects.ALL_ATTRIBUTES_ASPECT.getName())
    let result = t
        .update_with_aspects(&["AllAttributesAspect"], &["//pkg:x"])
        .unwrap();

    assert!(!crate::testing::has_error(result));
}

// Java: AspectTest.nativeAspectFailIfDepsNotVisible
#[test]
#[ignore = "needs the native test aspect TestAspects.ExtraAttributeAspect(\"//tool:tool\", false) (setRulesAndAspectsAvailableInTests)"]
fn native_aspect_fail_if_deps_not_visible() {
    let mut t = new_test();
    t.scratch.file(
        "tool/BUILD",
        &["filegroup(name='tool', visibility = ['//visibility:private'])"],
    );
    // Java: ExtraAttributeAspect extraAttributeAspect =
    //     new ExtraAttributeAspect("//tool:tool", false);
    //     setRulesAndAspectsAvailableInTests(ImmutableList.of(extraAttributeAspect), of());
    t.scratch.file(
        "pkg/build_defs.bzl",
        &[r#"def _rule_impl(ctx):
    pass

simple_rule = rule(
    implementation = _rule_impl,
)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &["load(\"//pkg:build_defs.bzl\", \"simple_rule\")\n\nsimple_rule(name = \"x\")\n"],
    );
    t.remove_fail_fast_handler();

    assert!(matches!(
        t.update_with_aspects(&["ExtraAttributeAspect_//tool:tool_false"], &["//pkg:x"]),
        Err(BuildViewError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event(
        "ExtraAttributeAspect_//tool:tool_false aspect on simple_rule rule //pkg:x: \
         Visibility error:\n\
         target '//tool:tool' is not visible from\n\
         target '//pkg:x'",
    );
}

// Java: AspectTest.setupAspectHints
fn setup_aspect_hints(t: &mut AnalysisTestCase) {
    t.scratch.file(
        "aspect_hints/hints.bzl",
        &[r#"HintInfo = provider(fields = ["hints_cnt"])

def _hint_impl(ctx):
    return [HintInfo(hints_cnt = ctx.attr.hints_cnt)]

hint = rule(
    implementation = _hint_impl,
    attrs = {"hints_cnt": attr.int(default = 0)},
)
"#],
    );
    t.scratch.file(
        "aspect_hints/hints_counter.bzl",
        &[r#"load("//aspect_hints:hints.bzl", "HintInfo")

HintsCntInfo = provider(fields = ["cnt"])

def _my_aspect_impl(target, ctx):
    transitive_hints = 0
    for dep in ctx.rule.attr.deps:
        transitive_hints = transitive_hints + dep[HintsCntInfo].cnt

    hints = 0
    for hint in ctx.rule.attr.aspect_hints:
        hints = hints + hint[HintInfo].hints_cnt

    return [HintsCntInfo(cnt = hints + transitive_hints)]

my_aspect = aspect(
    implementation = _my_aspect_impl,
    attr_aspects = ["deps"],
)

def _count_hints_impl(ctx):
    hints = 0
    for dep in ctx.attr.deps:
        hints = hints + dep[HintsCntInfo].cnt
    return [HintsCntInfo(cnt = hints)]

count_hints = rule(
    implementation = _count_hints_impl,
    attrs = {
        "deps": attr.label_list(aspects = [my_aspect]),
    },
)
"#],
    );
}

// Java: AspectTest.setupStarlarkRule
fn setup_starlark_rule(t: &mut AnalysisTestCase) {
    t.scratch.file(
        "aspect_hints/custom_rule.bzl",
        &[r#"def _custom_rule_impl(ctx):
    return []

custom_rule = rule(
    implementation = _custom_rule_impl,
    attrs = {
        "deps": attr.label_list(),
    },
)
"#],
    );
}

/// `((StarlarkInt) getHintsCntInfo(a).getValue("cnt")).truncateToInt()`.
// Java: AspectTest.getHintsCntInfo
fn get_hints_cnt(configured_target: &blaze_analysis::configured_target::ConfiguredTarget) -> i32 {
    let key = ProviderKey::Starlark {
        extension_label: parse_label("//aspect_hints:hints_counter.bzl"),
        exported_name: "HintsCntInfo".to_string(),
    };
    let info = configured_target
        .get(&key)
        .expect("NullPointerException: no HintsCntInfo");
    let info = StarlarkInfo::from_value(info.value()).expect("ClassCastException: StructImpl");
    info.get_field("cnt")
        .expect("field cnt")
        .unpack_i32()
        .expect("ClassCastException: StarlarkInt")
}
