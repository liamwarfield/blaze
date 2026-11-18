//! Tests for the S2 findings of docs/reviews/2026-11-18-loading.md about the builtins
//! environment (S2-9, guarded symbols), the stand-in builtin objects (S2-12) and Java hash order
//! that reaches events (S2-11).
//!
//! Expected texts and orders come from the Java tests named at each test, or from the Java
//! source (or the JDK run) cited there; never from Blaze.

use blaze_events::{Event, EventKind};
use blaze_packages::types::AttrValue;

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

fn error_messages(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get_kind() == EventKind::Error)
        .map(|e| e.get_message().into_owned())
        .collect()
}

/// Java `assertContainsEvent(expected)`.
fn assert_contains_event(events: &[Event], expected: &str) {
    assert!(
        events.iter().any(|e| e.get_message().contains(expected)),
        "no event contains {expected:?}; events: {:#?}",
        events
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect::<Vec<_>>()
    );
}

/// The string list `tags` of `//pkg:values`, which the BUILD files of these tests fill with the
/// values under test.
fn values(t: &PackageLoadingTestCase, pkg: &str) -> Vec<String> {
    let loaded = t.get_package(pkg);
    assert!(
        !loaded.package.contains_errors(),
        "package '{pkg}' contains errors: {:#?}",
        error_messages(&loaded.events)
    );
    let rule = loaded
        .package
        .get_rule("values")
        .unwrap_or_else(|| panic!("no rule 'values' in package '{pkg}'"));
    match blaze_packages::target::RuleOrMacroInstance::get_attr(&**rule, "tags") {
        Some(AttrValue::StringList(tags)) => tags,
        other => panic!("tags of //{pkg}:values: {other:?}"),
    }
}

// ---- S2-9: guarded symbols

// Java: BuiltinsIntegrationTest.builtinsDummyIsNotAPublicApi
//   scratch.file("pkg/BUILD", "load(':foo.bzl', 'foo')");
//   scratch.file("pkg/foo.bzl", "foo = _builtins_dummy");
//   Object result = getConfiguredTarget("//pkg:BUILD");
//   assertThat(result).isNull();
//   assertContainsEvent("_builtins_dummy is experimental");
#[test]
fn builtins_dummy_is_not_a_public_api() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["load(':foo.bzl', 'foo')"]);
    t.scratch_file("pkg/foo.bzl", &["foo = _builtins_dummy"]);

    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    assert_contains_event(&events, "_builtins_dummy is experimental");
    // Java: net/starlark/java/eval/FlagGuardedValue.java:36 (the whole message; GenericRules.java:57
    // guards the symbol with EXPERIMENTAL_BUILTINS_DUMMY = "-experimental_builtins_dummy").
    assert_contains_event(
        &events,
        "_builtins_dummy is experimental and thus unavailable with the current flags. It may be \
         enabled by setting --experimental_builtins_dummy",
    );
}

// Java: BuiltinsIntegrationTest.builtinsInjectionWorksInBuildViewTestCase, without the
// injection half (the harness has no @_builtins): with --experimental_builtins_dummy=true the
// guard is enabled and the name resolves to the guarded object (GenericRules.java:57
// `"original value"`).
#[test]
fn builtins_dummy_is_accessible_with_its_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_builtins_dummy=true"]);
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':foo.bzl', 'foo')",
            "filegroup(name = 'values', tags = [foo])",
        ],
    );
    t.scratch_file("pkg/foo.bzl", &["foo = _builtins_dummy"]);
    assert_eq!(values(&t, "pkg"), vec!["original value"]);
}

// Java: src/test/shell/integration/incompatible_autoload_externally_test.sh test_removed_provider
//   expect_log "my_rule.bzl:2:11: The CcInfo symbol has been removed, add the following to your BUILD/bzl file:"
//   expect_log 'load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")'
// The error is a syntax error of BzlCompileFunction (BzlCompileFunction.java:236), so the load
// fails with BzlLoadFunction.java:782's "compilation of module '%s'%s failed".
#[test]
fn removed_provider_reports_the_load_statement_to_add() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\"//pkg:my_rule.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            ")",
        ],
    );
    t.scratch_file(
        "pkg/my_rule.bzl",
        &[
            "def _impl(ctx):",
            "  return [CcInfo()]",
            "",
            "my_rule = rule(",
            "    implementation = _impl,",
            "    provides = [CcInfo],",
            ")",
        ],
    );

    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    let event = events
        .iter()
        .find(|e| {
            e.get_message().contains(
                "The CcInfo symbol has been removed, add the following to your BUILD/bzl file:",
            )
        })
        .unwrap_or_else(|| panic!("no such event: {:#?}", error_messages(&events)));
    assert_eq!(event.get_kind(), EventKind::Error);
    // BaseRuleClasses.java:502 createEmptySymbol: the whole message.
    assert_eq!(
        event.get_message(),
        "The CcInfo symbol has been removed, add the following to your BUILD/bzl file:\n\n\
         load(\"@rules_cc//cc/common:cc_info.bzl\", \"CcInfo\")\n"
    );
    // "my_rule.bzl:2:11".
    let location = event.get_location().expect("the identifier's location");
    assert!(
        location.file().ends_with("pkg/my_rule.bzl"),
        "{}",
        location.file()
    );
    assert_eq!((location.line(), location.column()), (2, 11));
    // No "name 'CcInfo' is not defined" and none of starlark-rust's own text.
    for message in error_messages(&events) {
        assert!(!message.contains("not found"), "{message}");
        assert!(!message.contains("is not defined"), "{message}");
    }

    let exception = t.get_package_exception("pkg");
    assert!(
        exception
            .get_message()
            .contains("compilation of module 'pkg/my_rule.bzl' failed"),
        "{}",
        exception.get_message()
    );
}

// Java: net/starlark/java/eval/Module.java:229 Module.resolve
//   // global?
//   if (globalIndex.containsKey(name)) {
//     return Resolver.Scope.GLOBAL;
//   }
//   // predeclared?
//   Object v = getPredeclared(name);
// A file that binds the name itself never reaches the guarded predeclared symbol.
#[test]
fn a_file_may_bind_the_name_of_a_guarded_symbol() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'x')",
            "filegroup(name = 'values', tags = [x])",
        ],
    );
    t.scratch_file("pkg/defs.bzl", &["CcInfo = 'my own CcInfo'", "x = CcInfo"]);
    assert_eq!(values(&t, "pkg"), vec!["my own CcInfo"]);
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/core/ContextAndFlagGuardedValue.java:53
//   return name
//       + " may only be used from one of the following repositories or prefixes: "
//       + allowedEntries.stream().map(PackageIdentifier::toString).collect(Collectors.joining(", "))
//       + ". It may be temporarily re-enabled for general use by setting --"
//       + flag.substring(1)
//       + "=false";
// with BazelRuleClassProvider.java:270 `PackageIdentifier.createUnchecked("rules_android", "")`
// and INCOMPATIBLE_STOP_EXPORTING_LANGUAGE_MODULES.
#[test]
fn context_guarded_symbol_reports_the_allowed_repositories() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_stop_exporting_language_modules"]);
    t.scratch_file("pkg/BUILD", &["load(':defs.bzl', 'x')"]);
    t.scratch_file("pkg/defs.bzl", &["x = android_common"]);

    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    assert_contains_event(
        &events,
        "android_common may only be used from one of the following repositories or prefixes: \
         @@rules_android//. It may be temporarily re-enabled for general use by setting \
         --incompatible_stop_exporting_language_modules=false",
    );
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcBootstrap.java:31 allowedRepositories
// (`PackageIdentifier.createUnchecked("", "tools/build_defs/cc")`) and
// ContextGuardedValue.java:41 isObjectAccessibleUsingSemantics: a .bzl file under an allowed
// main-repository prefix may use `cc_common` although the flag is set; elsewhere it gets the
// message of ContextAndFlagGuardedValue.java:53 (PackageIdentifier.toString of a main-repository
// package is its path, of another repository `@@repo//pkg`).
#[test]
fn context_guarded_symbol_is_accessible_under_an_allowed_prefix() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_stop_exporting_language_modules"]);
    t.scratch_file(
        "tools/build_defs/cc/BUILD",
        &[
            "load(':defs.bzl', 'x')",
            "filegroup(name = 'values', tags = [str(x)])",
        ],
    );
    t.scratch_file("tools/build_defs/cc/defs.bzl", &["x = cc_common"]);
    // CcBootstrap.java:53 guards `Starlark.NONE`.
    assert_eq!(values(&t, "tools/build_defs/cc"), vec!["None"]);

    t.scratch_file("pkg/BUILD", &["load(':defs.bzl', 'x')"]);
    t.scratch_file("pkg/defs.bzl", &["x = cc_common"]);
    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    assert_contains_event(
        &events,
        "cc_common may only be used from one of the following repositories or prefixes: \
         @@_builtins//, @@bazel_tools//, @@local_config_cc//, @@rules_cc//, \
         tools/build_defs/cc. It may be temporarily re-enabled for general use by setting \
         --incompatible_stop_exporting_language_modules=false",
    );
}

// ---- S2-12: stand-in builtin objects have exactly their Java class's members

// Java: net/starlark/java/eval/Starlark.java:967 hasattr, :978 getattr, :1024 dir, over
// AndroidStarlarkCommon's two @StarlarkMethods (rules/android/AndroidStarlarkCommon.java:52
// `resource_source_directory`, :73 `create_dex_merger_actions`). `android_common` is
// accessible in any .bzl file while --incompatible_stop_exporting_language_modules is false
// (its default).
#[test]
fn android_common_has_only_its_java_members() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "HAS_METHOD = hasattr(android_common, 'create_dex_merger_actions')",
            "HAS_ANYTHING = hasattr(android_common, 'anything')",
            "DEFAULTED = getattr(android_common, 'anything', 'the default')",
            "NAMES = dir(android_common)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'DEFAULTED', 'HAS_ANYTHING', 'HAS_METHOD', 'NAMES')",
            "filegroup(",
            "    name = 'values',",
            // One string: `tags` is order-independent and Bazel sorts it (BuildType.java:249).
            "    tags = ['|'.join([str(HAS_METHOD), str(HAS_ANYTHING), DEFAULTED] + NAMES)],",
            ")",
        ],
    );
    assert_eq!(
        values(&t, "pkg")[0].split('|').collect::<Vec<_>>(),
        vec![
            "True",
            "False",
            "the default",
            "create_dex_merger_actions",
            "resource_source_directory",
        ]
    );
}

// Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
//   throw Starlark.errorf(
//       "'%s' value has no field or method '%s'%s",
//       Starlark.type(x), name, SpellChecker.didYouMean(name, dir(mu, semantics, x)));
#[test]
fn android_common_missing_member_is_an_error() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["load(':defs.bzl', 'x')"]);
    t.scratch_file("pkg/defs.bzl", &["x = android_common.anything"]);
    let (result, events) = t.evaluate_package("pkg");
    assert!(result.has_error());
    assert_contains_event(
        &events,
        "'android_common' value has no field or method 'anything'",
    );
}

// ---- S2-11: Java HashSet order of duplicate labels

// Java: com/google/devtools/build/lib/packages/RuleClass.java:2114 checkForDuplicateLabels
//   for (Label label : duplicates) {
//     rule.reportError(
//         String.format(
//             "Label '%s' is duplicated in the '%s' attribute of rule '%s'",
//             label, attribute.getName(), rule.getName()),
//         eventHandler);
//   }
// where `duplicates` is the `HashSet<Label>` of CollectionUtils.duplicatedElementsOf
// (lib/collect/CollectionUtils.java:37). The order d, x, a, c, b is the iteration order that
// Bazel 9.3.2's own CollectionUtils.duplicatedElementsOf returns for these labels (run from the
// pinned server jar on the JDK the pin embeds); the order of second occurrence is x, d, c, b, a.
#[test]
fn duplicate_labels_are_reported_in_java_hash_set_order() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "p/BUILD",
        &[
            "filegroup(",
            "    name = 't',",
            "    srcs = ['x', 'd', 'c', 'b', 'a', 'x', 'd', 'c', 'b', 'a'],",
            ")",
        ],
    );
    let loaded = t.get_package("p");
    assert!(loaded.package.contains_errors());
    let suffix = " is duplicated in the 'srcs' attribute of rule 't'";
    let duplicated: Vec<String> = error_messages(&loaded.events)
        .into_iter()
        .filter(|m| m.contains(suffix))
        .map(|m| {
            let start = m.find("Label '").expect("the message names the label");
            let end = m.find(suffix).expect("checked above");
            m[start..end].to_owned()
        })
        .collect();
    assert_eq!(
        duplicated,
        vec![
            "Label '//p:d'",
            "Label '//p:x'",
            "Label '//p:a'",
            "Label '//p:c'",
            "Label '//p:b'",
        ]
    );
}

// Java: lib/collect/CollectionUtils.java:37 duplicatedElementsOf: a label that appears three
// times is added to the HashSet twice and reported once.
#[test]
fn a_label_repeated_twice_is_reported_once() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "p/BUILD",
        &["filegroup(name = 't', srcs = ['a', 'a', 'a'])"],
    );
    let loaded = t.get_package("p");
    assert!(loaded.package.contains_errors());
    let duplicated: Vec<String> = error_messages(&loaded.events)
        .into_iter()
        .filter(|m| m.contains("is duplicated in the"))
        .collect();
    assert_eq!(duplicated.len(), 1, "{duplicated:#?}");
    assert!(
        duplicated[0].contains("Label '//p:a' is duplicated in the 'srcs' attribute of rule 't'"),
        "{}",
        duplicated[0]
    );
}

// ---- S2-11: Java HashMap order of a Starlark computed default's lookup table

// Java: com/google/devtools/build/lib/packages/Attribute.java:1563
// StarlarkComputedDefault.getPossibleValues
//   for (Object obj : lookupTable.values()) {
//     result.add(type.cast(obj));
//   }
// The lookup table is `new HashMap<>(valueMap)` (Attribute.java:1436) of the
// `Maps.newHashMapWithExpectedSize(depMaps.size())` map that
// ComputationStrategy.computeValuesForAllCombinations (Attribute.java:1221) fills in the order
// of the select's values, keyed by `List<Object>` tuples of the dependency values. For the keys
// [zeta], [eta], [theta], [iota], [kappa] put in this order, both maps iterate as eta, iota,
// kappa, theta, zeta on the JDK 25 that Bazel 9.3.2 embeds (measured with Guava 33.5.0 from the
// pinned server jar).
#[test]
fn starlark_computed_default_values_are_in_java_hash_map_order() {
    use blaze_packages::target::RuleOrMacroInstance;
    use blaze_packages::types::Type;

    let t = PackageLoadingTestCase::new();
    t.scratch_file("c/BUILD", &[]);
    t.scratch_file("deps/BUILD", &[]);
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "def _compute(mode):",
            "    return Label('//deps:' + mode)",
            "def _impl(ctx):",
            "    pass",
            "r = rule(_impl, attrs = {",
            "    'mode': attr.string(),",
            "    '_dep': attr.label(default = _compute),",
            "})",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'r')",
            "r(",
            "    name = 't',",
            "    mode = select({",
            "        '//c:1': 'zeta',",
            "        '//c:2': 'eta',",
            "        '//c:3': 'theta',",
            "        '//c:4': 'iota',",
            "        '//c:5': 'kappa',",
            "    }),",
            ")",
        ],
    );
    let loaded = t.get_package("pkg");
    assert!(
        !loaded.package.contains_errors(),
        "{:#?}",
        error_messages(&loaded.events)
    );
    let rule = loaded.package.get_rule("t").expect("the rule //pkg:t");
    // Java `Attribute.getNativeAttrName("_dep")` is "$dep".
    let computed_default = match RuleOrMacroInstance::get_attr(&**rule, "$dep") {
        Some(AttrValue::ComputedDefault(computed_default)) => computed_default,
        other => panic!("the value of $dep: {other:?}"),
    };
    let values: Vec<String> = computed_default
        .get_possible_values(Type::Label, &**rule)
        .into_iter()
        .map(|value| match value {
            Some(AttrValue::Label(label)) => label.to_string(),
            other => panic!("a possible value of $dep: {other:?}"),
        })
        .collect();
    assert_eq!(
        values,
        vec![
            "//deps:eta",
            "//deps:iota",
            "//deps:kappa",
            "//deps:theta",
            "//deps:zeta",
        ]
    );
}
