//! Port of `src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java`:
//! dormant dependencies (`attr.dormant_label`, materializers, `for_dependency_resolution`), on
//! the in-process runtime (see `analysis_common`).

mod analysis_common;

#[allow(unused_imports)]
use analysis_common::{
    Fixture, assert_contains_at_least, assert_contains_exactly, assert_contains_none_of,
    base_artifact_names,
};

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:37 DormantDependencyTest.enableDormantDeps
fn set_up() -> Fixture {
    let mut t = Fixture::new();
    t.use_configuration(&["--experimental_dormant_deps"]);
    t
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:42 testDormantLabelDisabledWithoutExperimentalFlag
#[test]
fn test_dormant_label_disabled_without_experimental_flag() {
    let mut t = set_up();
    t.use_configuration(&["--noexperimental_dormant_deps"]);
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  fail(\"should not be called\")",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = {",
            "    \"dormant\": attr.dormant_label(),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("no field or method 'dormant_label'");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:71 testDormantAttribute
#[test]
fn test_dormant_attribute() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  print(\"dormant label is \" + str(ctx.attr.dormant.label))",
            "  print(\"dormant label list is \" + str(ctx.attr.dormant_list[0].label))",
            "  return [DefaultInfo()]",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {",
            "    \"dormant\": attr.dormant_label(),",
            "    \"dormant_list\": attr.dormant_label_list(),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &[
            "load(\":dormant.bzl\", \"r\")",
            "",
            "filegroup(name=\"a\")",
            "filegroup(name=\"b\")",
            "r(name=\"r\", dormant=\":a\", dormant_list=[\":b\"])",
        ],
    );
    t.update(&["//dormant:r"]).expect("update");
    t.assert_contains_event("dormant label is @@//dormant:a");
    t.assert_contains_event("dormant label list is @@//dormant:b");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:105 testDormantAttributeComputedDefaultsFail
#[test]
fn test_dormant_attribute_computed_defaults_fail() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  fail(\"should not happen\")",
            "",
            "def computed_default():",
            "  fail(\"should not happen\")",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {",
            "    \"dormant\": attr.dormant_label(default=computed_default),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("got value of type 'function'");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:136 testDormantAttributeDefaultValues
#[test]
fn test_dormant_attribute_default_values() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  print(\"dormant label is \" + str(ctx.attr.dormant.label))",
            "  print(\"dormant label list is \" + str(ctx.attr.dormant_list[0].label))",
            "  return [DefaultInfo()]",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {",
            "    \"dormant\": attr.dormant_label(default=\"//dormant:a\"),",
            "    \"dormant_list\": attr.dormant_label_list(default=[\"//dormant:b\"]),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &[
            "load(\":dormant.bzl\", \"r\")",
            "",
            "filegroup(name=\"a\")",
            "filegroup(name=\"b\")",
            "r(name=\"r\")",
        ],
    );
    t.update(&["//dormant:r"]).expect("update");
    t.assert_contains_event("dormant label is @@//dormant:a");
    t.assert_contains_event("dormant label list is @@//dormant:b");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:170 testExistenceOfMaterializerParameter
#[test]
fn test_existence_of_materializer_parameter() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "def _label_materializer(*args, **kwargs):",
            "  return None",
            "",
            "def _list_materializer(*args, **kwargs):",
            "  return []",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = {",
            "    \"_materialized\": attr.label(materializer=_label_materializer),",
            "    \"_materialized_list\": attr.label_list(materializer=_list_materializer),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    t.update(&["//dormant:r"]).expect("update");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:202 testMaterializedOnNonHiddenAttribute
#[test]
fn test_materialized_on_non_hidden_attribute() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "def _label_materializer(*args, **kwargs):",
            "  return None",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = {",
            "    \"materialized\": attr.label(materializer=_label_materializer),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("attribute must be private");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:232 testMaterializerAndDefaultAreIncompatible
#[test]
fn test_materializer_and_default_are_incompatible() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "def _label_materializer(*args, **kwargs):",
            "  return None",
            "",
            "def _list_materializer(*args, **kwargs):",
            "  return []",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = {",
            "    \"_materialized\": attr.label(",
            "        materializer=_label_materializer,",
            "        default=Label(\"//dormant:default\")),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("parameters are incompatible");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:267 testMaterializerAndMandatoryAreIncompatible
#[test]
fn test_materializer_and_mandatory_are_incompatible() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "def _label_materializer(*args, **kwargs):",
            "  return None",
            "",
            "def _list_materializer(*args, **kwargs):",
            "  return []",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = {",
            "    \"_materialized\": attr.label(materializer=_label_materializer, mandatory=True),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("parameters are incompatible");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:300 testMaterializerAndConfigurableAreIncompatible
#[test]
fn test_materializer_and_configurable_are_incompatible() {
    let mut t = set_up();
    t.write("dormant/dormant.bzl", &[
        "def _r_impl(ctx):",
        "  return [DefaultInfo()]",
        "",
        "def _label_materializer(*args, **kwargs):",
        "  return None",
        "",
        "def _list_materializer(*args, **kwargs):",
        "  return []",
        "",
        "r = rule(",
        "  implementation = _r_impl,",
        "  attrs = {",
        "    \"_materialized\": attr.label(materializer=_label_materializer, configurable=True),",
        "  })",
    ]);
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:r"]).is_err());
    t.assert_contains_event("parameters are incompatible");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:333 testMaterializerOnAspectNotAllowed
#[test]
fn test_materializer_on_aspect_not_allowed() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _r_impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "def _a_impl(target, ctx):",
            "  fail(\"aspect implementation should not be called\")",
            "",
            "def _materializer(ctx):",
            "   fail(\"materializer should not be called\")",
            "",
            "a = aspect(",
            "  implementation = _a_impl,",
            "  attrs = { \"_materialized\": attr.label_list(materializer=_materializer)})",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  attrs = { \"dep\": attr.label(aspects=[a])})",
        ],
    );
    t.write(
        "a/BUILD",
        &[
            "load(\":a.bzl\", \"r\")",
            "",
            "filegroup(name=\"f\")",
            "r(name=\"r\", dep=\":f\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:r"]).is_err());
    t.assert_contains_event("has a materializer, which is not allowed on aspects");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:370 testAttributesOfDependencyResolutionRulesCannotBeMarkedOtherwise
#[test]
fn test_attributes_of_dependency_resolution_rules_cannot_be_marked_otherwise() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _a_impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "a = rule(",
            "  implementation = _a_impl,",
            "  attrs = {\"dep\": attr.label(for_dependency_resolution = False)},",
            "  dependency_resolution_rule = True)",
        ],
    );
    t.write("a/BUILD", &["load(\"//a:a.bzl\", \"a\")", "a(name=\"x\")"]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:x"]).is_err());
    t.assert_contains_event("explicitly marked as not for dependency resolution");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:396 testAttributesOfDependencyResolutionRulesAreNonconfigurable
#[test]
fn test_attributes_of_dependency_resolution_rules_are_nonconfigurable() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _a_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "a = rule(",
            "  implementation = _a_impl,",
            "  attrs = {\"dep\": attr.label()},",
            "  dependency_resolution_rule = True)",
        ],
    );
    t.write("a/BUILD", &[]);
    t.write("x/BUILD", &["load(\"//a:a.bzl\", \"a\")", "a(name=\"x\")"]);
    t.write(
        "y/BUILD",
        &[
            "load(\"//a:a.bzl\", \"a\")",
            "config_setting(name = \"cs\", values = {\"define\": \"cs\"})",
            "a(name=\"y\", dep=select({\":cs\": \":y1\", \"//conditions:default\": \":y2\"}))",
        ],
    );
    t.update(&["//x"]).expect("update");
    let x_rule = t.get_target("//x");
    let x_rule = x_rule.as_rule().expect("a rule");
    let dep_attribute = x_rule
        .get_rule_class_object()
        .get_attribute_provider()
        .get_attribute_by_name("dep");
    assert!(!dep_attribute.is_configurable());
    assert!(dep_attribute.is_for_dependency_resolution());
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//y"]).is_err());
    t.assert_contains_event("attribute \"dep\" is not configurable");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:438 testRuleMustBeMarkedAsForDependencyResolution
#[test]
fn test_rule_must_be_marked_as_for_dependency_resolution() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _a_impl(ctx):",
            "  return []",
            "",
            "a = rule(",
            "  implementation = _a_impl,",
            "  attrs = {\"dep\": attr.dormant_label()})",
        ],
    );
    t.write("a/BUILD", &["load(\":a.bzl\", \"a\")", "a(name=\"a\")"]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:a"]).is_err());
    t.assert_contains_event("Has dormant attributes ('dep')");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:463 testNoDormantDepsOnAspects
#[test]
fn test_no_dormant_deps_on_aspects() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _impl(*args):",
            "  fail(\"should not be called\")",
            "",
            "a = aspect(",
            "  implementation = _impl,",
            "  attrs = { \"_dormant\": attr.dormant_label(default=\"//x:x\")})",
            "",
            "r = rule(",
            "  implementation = _impl,",
            "  attrs = { \"dep\": attr.label(aspects=[a]) })",
        ],
    );
    t.write("a/BUILD", &["load(\":a.bzl\", \"r\")", "r(name=\"a\")"]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a"]).is_err());
    t.assert_contains_event("'_dormant' has a dormant label type");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:492 testNoAspectsOnDependencyResolutionRules
#[test]
fn test_no_aspects_on_dependency_resolution_rules() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _impl(*args):",
            "  fail(\"should not be called\")",
            "",
            "a = aspect(implementation = _impl)",
            "",
            "r = rule(",
            "  implementation = _impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {\"dep\": attr.label_list(aspects=[a])},",
            ")",
        ],
    );
    t.write("a/BUILD", &["load(\":a.bzl\", \"r\")", "r(name=\"r\")"]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:r"]).is_err());
    t.assert_contains_event("cannot propagate aspects");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:521 testNoToolchainsOnDependencyResolutionRules
#[test]
fn test_no_toolchains_on_dependency_resolution_rules() {
    let mut t = set_up();
    t.write(
        "a/a.bzl",
        &[
            "def _impl(*args):",
            "  fail(\"should not be called\")",
            "",
            "a = aspect(implementation = _impl)",
            "",
            "r = rule(",
            "  implementation = _impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {\"dep\": attr.label_list()},",
            "  toolchains = [\"//a:nonexistent_toolchain\"],",
            ")",
        ],
    );
    t.write("a/BUILD", &["load(\":a.bzl\", \"r\")", "r(name=\"r\")"]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:r"]).is_err());
    t.assert_contains_event("cannot depend on toolchains");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:551 testErrorOnUnmarkedRuleInAttributeAvailableInMaterializers
#[test]
fn test_error_on_unmarked_rule_in_attribute_available_in_materializers() {
    let mut t = set_up();
    t.write(
        "a/dormant.bzl",
        &[
            "def _component_impl(ctx):",
            "  fail(\"should not be called\")",
            "",
            "component = rule(",
            "  implementation = _component_impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {",
            "      \"impl\": attr.label(),",
            "  })",
        ],
    );
    t.write(
        "a/BUILD",
        &[
            "load(\":dormant.bzl\", \"component\")",
            "component(name=\"c\", impl=\":bad\")",
            "filegroup(name=\"bad\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//a:c"]).is_err());
    t.assert_contains_event(
        "marked as available in materializers but prerequisite filegroup rule '//a:bad' isn't",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:581 testSubrulesCannotHaveDormantDeps
#[test]
fn test_subrules_cannot_have_dormant_deps() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _impl(ctx):",
            "  fail(\"implementation should not be called\")",
            "",
            "sub = subrule(implementation = _impl, attrs = {",
            "  \"_dormant\": attr.dormant_label(default=\"//dormant:dormant\"),",
            "})",
            "real = rule(implementation = _impl, attrs = {}, subrules = [sub])",
        ],
    );
    t.write(
        "dormant/BUILD",
        &["load(\":dormant.bzl\", \"real\")", "real(name=\"real\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//dormant:real"]).is_err());
    t.assert_contains_event("subrule attributes may only be");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:607 testMarkedRulesCannotBeParents
#[test]
fn test_marked_rules_cannot_be_parents() {
    let mut t = set_up();
    t.write(
        "parent/parent.bzl",
        &[
            "def _impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "p = rule(",
            "  implementation = _impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {",
            "      \"dormant\": attr.dormant_label(),",
            "  })",
        ],
    );
    t.write("parent/BUILD", &[]);
    t.write(
        "unmarked/unmarked.bzl",
        &[
            "load(\"//parent:parent.bzl\", \"p\")",
            "",
            "def _impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "unmarked = rule(",
            "  implementation = _impl,",
            "  parent = p,",
            "  attrs = {})",
        ],
    );
    t.write(
        "unmarked/BUILD",
        &[
            "load(\":unmarked.bzl\", \"unmarked\")",
            "unmarked(name=\"unmarked\")",
        ],
    );
    t.write(
        "marked/marked.bzl",
        &[
            "load(\"//parent:parent.bzl\", \"p\")",
            "",
            "def _impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "marked = rule(",
            "  implementation = _impl,",
            "  dependency_resolution_rule = True,",
            "  parent = p,",
            "  attrs = {})",
        ],
    );
    t.write(
        "marked/BUILD",
        &[
            "load(\":marked.bzl\", \"marked\")",
            "marked(name=\"marked\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//unmarked:unmarked"]).is_err());
    t.assert_contains_event("cannot be parents");
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//marked:marked"]).is_err());
    t.assert_contains_event("cannot be parents");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:676 testMarkedRulesCannotHaveParents
#[test]
fn test_marked_rules_cannot_have_parents() {
    let mut t = set_up();
    t.write(
        "a/dormant.bzl",
        &[
            "def _impl(ctx):",
            "  fail(\"rule implementation should not be called\")",
            "",
            "",
            "p = rule(",
            "  implementation = _impl,",
            "  attrs = {})",
            "",
            "r = rule(",
            "  implementation = _impl,",
            "  dependency_resolution_rule = True,",
            "  parent = p,",
            "  attrs = {})",
        ],
    );
    t.write(
        "a/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:r"]).is_err());
    t.assert_contains_event("cannot have a parent");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:708 testMarkedRulesCannotCreateActions
#[test]
fn test_marked_rules_cannot_create_actions() {
    let mut t = set_up();
    t.write(
        "a/dormant.bzl",
        &[
            "def _r_impl(ctx):",
            "  a = ctx.actions.declare_file(\"f\")",
            "  ctx.actions.write(a, \"foo\")",
            "  return [DefaultInfo(files=depset([a]))]",
            "",
            "r = rule(",
            "  implementation = _r_impl,",
            "  dependency_resolution_rule = True,",
            "  attrs = {})",
        ],
    );
    t.write(
        "a/BUILD",
        &["load(\":dormant.bzl\", \"r\")", "r(name=\"r\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//a:r"]).is_err());
    t.assert_contains_event("shouldn't have actions");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:736 testAllowlistForDormantAttributes
#[test]
fn test_allowlist_for_dormant_attributes() {
    let mut t = set_up();
    t.write_tools(
        "tools/allowlists/dormant_dependency_allowlist/BUILD",
        &[
            "package_group(",
            "name = 'dormant_dependency_allowlist',",
            "  # This rule is in @bazel_tools but must reference a package in the main repository.",
            "  # (the value of packages= can't cross repositories at the moment)",
            "  includes = ['@@//pkg:pkg'])",
        ],
    );
    t.write(
        "pkg/BUILD",
        &["package_group(name='pkg', packages=['//ok/...'])"],
    );
    let dormant_rule: &[&str] = &[
        "def _impl(ctx):",
        "  return [DefaultInfo()]",
        "r = rule(",
        "    implementation = _impl,",
        "    dependency_resolution_rule = True,",
        "    attrs={\"dormant\": attr.dormant_label()})",
    ];
    let dormant_build_file: &[&str] = &[
        "load(\":r.bzl\", \"r\")",
        "filegroup(name=\"dep\")",
        "r(name=\"r\", dormant=\":dep\")",
    ];
    t.write("ok/r.bzl", dormant_rule);
    t.write("ok/BUILD", dormant_build_file);
    t.write("bad/r.bzl", dormant_rule);
    t.write("bad/BUILD", dormant_build_file);
    t.update(&["//ok:r"]).expect("update");
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//bad:r"]).is_err());
    t.assert_contains_event("Non-allowlisted use of dormant dependencies");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:825 testSmoke
#[test]
fn test_smoke() {
    let mut t = set_up();
    write_simple_dormant_rules(&mut t);
    t.write(
        "a/BUILD",
        &[
            "load(\"//dormant:dormant.bzl\", \"component\", \"binary\")",
            "",
            "component(name=\"a_yes\", impl=\":a_impl\")",
            "component(name=\"b_no\", deps = [\":c_yes\", \":d_no\"], impl=\":b_impl\")",
            "component(name=\"c_yes\", impl=\":c_impl\")",
            "component(name=\"d_no\", impl=\":d_impl\")",
            "",
            "binary(name=\"bin\", components=[\":a_yes\", \":b_no\"])",
            "[filegroup(name=x + \"_impl\", srcs=[x]) for x in [\"a\", \"b\", \"c\", \"d\"]]",
        ],
    );
    t.update(&["//a:bin"]).expect("update");
    let target = "//a:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(&base_artifact_names(&files_to_build), &["a", "c"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:848 testErrorOnUnmarkedAttribute
#[test]
fn test_error_on_unmarked_attribute() {
    let mut t = set_up();
    t.write(
        "a/dormant.bzl",
        &[
            "ComponentInfo = provider(fields = [\"components\"])",
            "",
            "def _binary_impl(ctx):",
            "  return [DefaultInfo(files=depset([]))]",
            "",
            "def _materializer(ctx):",
            "  return ctx.attr.dep[ComponentInfo].components",
            "",
            "binary = rule(",
            "  implementation = _binary_impl,",
            "  attrs = {",
            "      \"dep\": attr.label(),",
            "      \"_impls\": attr.label_list(materializer = _materializer),",
            "  })",
        ],
    );
    t.write(
        "a/BUILD",
        &[
            "load(\":dormant.bzl\", \"binary\")",
            "binary(name=\"bin\", dep=\":dep\")",
            "filegroup(name=\"dep\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//a:bin"]).is_err());
    t.assert_contains_event("not available in materializer");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:882 testMaterializersOnDependencyResolutionRulesDisallowed
#[test]
fn test_materializers_on_dependency_resolution_rules_disallowed() {
    let mut t = set_up();
    t.write(
        "a/dormant.bzl",
        &[
            "def _impl(ctx):",
            "    return [DefaultInfo()]",
            "",
            "def _materializer(ctx):",
            "    return []",
            "",
            "rr = rule(",
            "    dependency_resolution_rule = True,",
            "    implementation = _impl,",
            "    attrs = {",
            "        \"_mat\": attr.label_list(materializer = _materializer),",
            "    },",
            ")",
        ],
    );
    t.write(
        "a/BUILD",
        &["load(\":dormant.bzl\", \"rr\")", "rr(name=\"rr\")"],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(TargetParsingException)
    assert!(t.update(&["//a:rr"]).is_err());
    t.assert_contains_event("has a materializer which is not allowed");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:914 testSplitConfigurationOnMaterializingAttribute
#[test]
fn test_split_configuration_on_materializing_attribute() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "ComponentInfo = provider(fields = [\"impls\"])",
            "",
            "def _component_impl(ctx):",
            "  return [ComponentInfo(impls = ctx.attr.impls)]",
            "",
            "component = rule(",
            "  implementation = _component_impl,",
            "  attrs = { \"impls\": attr.dormant_label_list() },",
            "  provides = [ComponentInfo],",
            "  dependency_resolution_rule = True,",
            ")",
            "",
            "def _binary_impl(ctx):",
            "  return [DefaultInfo(files=depset(ctx.files._impls))]",
            "",
            "def _materializer(ctx):",
            "  return ctx.attr.component[ComponentInfo].impls",
            "",
            "def _transition_impl(settings, attr):",
            "  return [",
            "    {\"//command_line_option:compilation_mode\": \"dbg\"},",
            "    {\"//command_line_option:compilation_mode\": \"opt\"},",
            "  ]",
            "",
            "_transition = transition(",
            "  implementation = _transition_impl,",
            "  inputs = [],",
            "  outputs = [\"//command_line_option:compilation_mode\"])",
            "",
            "binary = rule(",
            "  implementation = _binary_impl,",
            "  attrs = {",
            "      \"component\": attr.label(",
            "          providers = [ComponentInfo], for_dependency_resolution = True),",
            "      \"_impls\": attr.label_list(materializer = _materializer, cfg = _transition),",
            "  })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &[
            "load(\":dormant.bzl\", \"component\", \"binary\")",
            "genrule(name=\"g1\", srcs=[], outs=[\"g1o\"], cmd=\"echo GO >$@\")",
            "genrule(name=\"g2\", srcs=[], outs=[\"g2o\"], cmd=\"echo GO >$@\")",
            "component(name=\"c\", impls=[\":g1\", \":g2\"])",
            "binary(name=\"b\", component=\":c\")",
        ],
    );
    t.update(&["//dormant:b"]).expect("update");
    let target = "//dormant:b";
    let files_to_build = t.files_to_build(target);
    // Two deps in two configurations must be four artifacts altogether
    assert_eq!(files_to_build.len(), 4);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:975 testAspectOnDependencyResolutionRule
#[test]
fn test_aspect_on_dependency_resolution_rule() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _propagator_impl(ctx):",
            "  return [DefaultInfo(files=depset(ctx.files.dep))]",
            "",
            "def _asp_impl(target, ctx):",
            "  o = ctx.actions.declare_file(\"o\")",
            "  ctx.actions.write(o, \"CONTENT\")",
            "  return [DefaultInfo(files=depset([o]))]",
            "",
            "def _dep_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "dep = rule(",
            "  implementation = _dep_impl,",
            "  dependency_resolution_rule=True,",
            "  attrs = {})",
            "",
            "asp = aspect(implementation = _asp_impl)",
            "",
            "propagator = rule(",
            "    implementation = _propagator_impl,",
            "    attrs = {\"dep\": attr.label(for_dependency_resolution=True, aspects=[asp])})",
        ],
    );
    t.write(
        "dormant/BUILD",
        &[
            "load(\":dormant.bzl\", \"propagator\", \"dep\")",
            "dep(name=\"dep\")",
            "propagator(name=\"propagator\", dep=\":dep\")",
        ],
    );
    t.update(&["//dormant:propagator"]).expect("update");
    let propagator = "//dormant:propagator";
    let files_to_build = t.files_to_build(propagator);
    assert_contains_exactly(&base_artifact_names(&files_to_build), &["o"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:1017 testLabelInMaterializer
#[test]
fn test_label_in_materializer() {
    let mut t = set_up();
    t.write(
        "dormant/dormant.bzl",
        &[
            "def _binary_impl(ctx):",
            "  return [DefaultInfo()]",
            "",
            "def _materializer(ctx):",
            "  print(\"MATERIALIZER LABEL \" + str(ctx.label))",
            "  return []",
            "",
            "binary = rule(",
            "  implementation = _binary_impl,",
            "  attrs = { \"_materialized\": attr.label_list(materializer = _materializer) })",
        ],
    );
    t.write(
        "dormant/BUILD",
        &[
            "load(\":dormant.bzl\", \"binary\")",
            "binary(name=\"binary\")",
        ],
    );
    t.update(&["//dormant:binary"]).expect("update");
    t.assert_contains_event("MATERIALIZER LABEL @@//dormant:binary");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/DormantDependencyTest.java:782 writeSimpleDormantRules
fn write_simple_dormant_rules(t: &mut Fixture) {
    t.write("dormant/dormant.bzl", &[
        "ComponentInfo = provider(fields = [\"components\"])",
        "",
        "def _component_impl(ctx):",
        "  current = struct(label=ctx.label, impl = ctx.attr.impl)",
        "  transitive = [d[ComponentInfo].components for d in ctx.attr.deps]",
        "  return [",
        "    ComponentInfo(components = depset(direct = [current], transitive = transitive)),",
        "  ]",
        "",
        "component = rule(",
        "  implementation = _component_impl,",
        "  attrs = {",
        "    \"deps\": attr.label_list(providers = [ComponentInfo]),",
        "    \"impl\": attr.dormant_label(),",
        "  },",
        "  provides = [ComponentInfo],",
        "  dependency_resolution_rule = True,",
        ")",
        "",
        "def _binary_impl(ctx):",
        "  return [DefaultInfo(files=depset(ctx.files._impls))]",
        "",
        "def _materializer(ctx):",
        "  all = depset(transitive = [d[ComponentInfo].components for d in ctx.attr.components])",
        "  selected = [c.impl for c in all.to_list() if \"yes\" in str(c.label)]",
        "  return selected",
        "",
        "binary = rule(",
        "  implementation = _binary_impl,",
        "  attrs = {",
        "      \"components\": attr.label_list(providers = [ComponentInfo], for_dependency_resolution = True),",
        "      \"_impls\": attr.label_list(materializer = _materializer),",
        "  })",
    ]);
    t.write("dormant/BUILD", &[]);
}
