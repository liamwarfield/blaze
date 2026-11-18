//! Tests for the S2 findings of docs/reviews/2026-11-18-loading.md about BUILD natives and rule-definition builtins.
//!
//! Expected texts come from the Java tests named at each test, or from the Java source cited
//! there. Java tests that call `getConfiguredTarget` only to load the package become
//! `get_package` (see `package_loading_test_case`).

use blaze_cmdline::Label;
use blaze_packages::types::AttrValue;
use indexmap::IndexMap;

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

/// Java `PackageFactoryTest.assertGlobFails`: `dummypackage/BUILD` is `x = <glob call>`; the
/// package has errors and an event contains `expected_error`.
fn assert_glob_fails(glob_call_expression: &str, expected_error: &str) {
    let t = PackageLoadingTestCase::new();
    let build_file = format!("x = {glob_call_expression}");
    t.scratch_file("dummypackage/BUILD", &[build_file.as_str()]);
    let pkg = t.get_package("dummypackage");
    pkg.assert_contains_event(expected_error);
    assert!(pkg.package.contains_errors());
}

/// A package whose BUILD file is `lines` fails with an event containing `expected_error`.
fn assert_build_file_fails(lines: &[&str], expected_error: &str) {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("dummypackage/BUILD", lines);
    let pkg = t.get_package("dummypackage");
    pkg.assert_contains_event(expected_error);
    assert!(pkg.package.contains_errors());
}

// ---- S2-1: private-API allowlist

// Java: StarlarkRuleClassFunctionsTest.initializer_failsSettingPrivateAttribute_outsideBuiltins
#[test]
fn initializer_fails_setting_private_attribute_outside_builtins() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    return {\"srcs\": srcs, \"_tool\": \":my_tool\"}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"_tool\": attr.label(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "",
            "filegroup(name = \"my_tool\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let pkg = t.get_package("initializer_testing");
    pkg.assert_contains_event("file '//initializer_testing:b.bzl' cannot use private API");
}

// Java: StarlarkRuleClassFunctionsTest.initializer_settingPrivateAttribute_insideBuiltins
// (the Java test reads `ctx.attr._tool.label` in analysis; here the loaded rule's `$tool`).
#[test]
fn initializer_setting_private_attribute_inside_builtins() {
    let t = PackageLoadingTestCase::new();
    // Because it's hard to test something that needs to be in builtins, this is also allowed in
    // a special testing location: StarlarkRuleClassFunctions.ALLOWLIST_RULE_EXTENSION_API_EXPERIMENTAL
    t.scratch_file(
        "initializer_testing/builtins/BUILD",
        &["filegroup(name='my_tool')"],
    );
    t.scratch_file(
        "initializer_testing/builtins/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    return {\"srcs\": srcs, \"_tool\": \":my_tool\"}",
            "",
            "MyInfo = provider()",
            "",
            "def impl(ctx):",
            "    return MyInfo(_tool = str(ctx.attr._tool.label))",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"_tool\": attr.label(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\"//initializer_testing/builtins:b.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let pkg = t.get_package_without_errors("initializer_testing");
    // Java: `info.getValue("_tool")` is "@@//initializer_testing/builtins:my_tool".
    assert_eq!(
        pkg.attr("my_target", "$tool"),
        AttrValue::Label(Label::parse_canonical("//initializer_testing/builtins:my_tool").unwrap())
    );
}

// ---- S2-5: initializers run without the package builder on the thread

// Java: StarlarkRuleClassFunctionsTest.initializer_failsCreatingAnotherRule. The Java test
// instantiates `java_library` loaded from rules_java, which a scratch workspace does not have;
// `native.filegroup` goes through the same check (RuleFactory.java:315
// `TargetDefinitionContext.fromOrFail(thread, "a rule", "instantiated")`).
#[test]
fn initializer_fails_creating_another_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    native.filegroup(name = \"jl\", srcs = [\"a.java\"])",
            "    return {\"srcs\": srcs, \"deps\": deps}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let pkg = t.get_package("initializer_testing");
    pkg.assert_contains_event(
        "a rule can only be instantiated while evaluating a BUILD file or a legacy or symbolic \
         macro",
    );
    // The initializer created nothing, and the builder is back on the thread afterwards: the
    // package is in error because of the failed call only.
    assert!(pkg.package.contains_errors());
    assert!(pkg.package.get_rule("jl").is_none());
}

// Java: StarlarkRuleClassFunctionsTest.initializer_failsWithExistingRules
#[test]
fn initializer_fails_with_existing_rules() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = [], deps = []):",
            "    native.existing_rules()",
            "    return {\"srcs\": srcs, \"deps\": deps}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {",
            "        \"srcs\": attr.label_list(allow_files = [\"ml\"]),",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "",
            "my_rule(",
            "    name = \"my_target\",",
            "    srcs = [\"a.ml\"],",
            ")",
        ],
    );
    let pkg = t.get_package("initializer_testing");
    pkg.assert_contains_event(
        "existing_rules() can only be used while evaluating a BUILD file, a legacy macro, or a \
         rule finalizer",
    );
}

// Java: StarlarkRuleClassFunctionsTest.initializer_nativeModule (the Java test reads the
// `props` attribute through a provider in analysis; here the loaded rule's attribute).
#[test]
fn initializer_native_module() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("initializer_testing/rules/BUILD", &[]);
    t.scratch_file(
        "initializer_testing/rules/b.bzl",
        &[
            "MyInfo = provider()",
            "def initializer(name, **kwargs):",
            "  return {'props': {",
            "    'package_relative_label': str(native.package_relative_label(':target')),",
            "  }}",
            "def impl(ctx): ",
            "  return [MyInfo(props = ctx.attr.props)]",
            "my_rule = rule(impl,",
            "  initializer = initializer,",
            "  attrs = {",
            "    'props': attr.string_dict(),",
            "  })",
        ],
    );
    t.scratch_file(
        "initializer_testing/targets/BUILD",
        &[
            "load('//initializer_testing/rules:b.bzl','my_rule')",
            "my_rule(name = 'my_target')",
        ],
    );
    let pkg = t.get_package_without_errors("initializer_testing/targets");
    let mut expected: IndexMap<String, String> = IndexMap::new();
    expected.insert(
        "package_relative_label".to_owned(),
        "@@//initializer_testing/targets:target".to_owned(),
    );
    assert_eq!(
        pkg.attr("my_target", "props"),
        AttrValue::StringDict(expected)
    );
}

// The builder is restored after the initializers ran (Java's `finally`:
// StarlarkRuleClassFunctions.java:1880-1882 `targetDefinitionContext.storeInThread(thread)`):
// the rule itself and later BUILD statements still reach the package.
#[test]
fn initializer_restores_the_package_context() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = []):",
            "    return {\"srcs\": srcs}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {\"srcs\": attr.label_list(allow_files = True)},",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\", srcs = [\"a.ml\"])",
            "filegroup(name = \"after_\" + package_name().replace(\"/\", \"_\"))",
        ],
    );
    let pkg = t.get_package_without_errors("initializer_testing");
    pkg.rule("my_target");
    pkg.rule("after_initializer_testing");
}

// ---- S2-6: values Java makes immutable

// Java: Type.java:770 ListType.copyAndLiftStarlarkValue returns
// `StarlarkList.immutableCopyOf(...)`, so the initializer's `srcs` is frozen; mutating it is
// Starlark.java:264 checkMutable "trying to mutate a frozen %s value".
#[test]
fn initializer_cannot_mutate_lifted_list_value() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, srcs = []):",
            "    srcs.append(\"//x:y\")",
            "    return {\"srcs\": srcs}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {\"srcs\": attr.label_list(allow_files = True)},",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\", srcs = [\"a.ml\"])",
        ],
    );
    let pkg = t.get_package("initializer_testing");
    pkg.assert_contains_event("trying to mutate a frozen list value");
    assert!(pkg.package.contains_errors());
}

// Java: Type.java:598 DictType.copyAndLiftStarlarkValue returns `Dict.immutableCopyOf(result)`.
#[test]
fn initializer_cannot_mutate_lifted_dict_value() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "initializer_testing/b.bzl",
        &[
            "def initializer(name, props = {}):",
            "    props[\"k\"] = \"v\"",
            "    return {\"props\": props}",
            "",
            "def impl(ctx):",
            "    pass",
            "",
            "my_rule = rule(",
            "    impl,",
            "    initializer = initializer,",
            "    attrs = {\"props\": attr.string_dict()},",
            ")",
        ],
    );
    t.scratch_file(
        "initializer_testing/BUILD",
        &[
            "load(\":b.bzl\", \"my_rule\")",
            "my_rule(name = \"my_target\", props = {\"a\": \"b\"})",
        ],
    );
    let pkg = t.get_package("initializer_testing");
    pkg.assert_contains_event("trying to mutate a frozen dict value");
    assert!(pkg.package.contains_errors());
}

// Java: NativeExistingRulesTest.existingRules_returnsDeeplyImmutableView
#[test]
fn existing_rules_returns_deeply_immutable_view() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("test/BUILD", &["load(\"inc.bzl\", \"f\")", "", "f()"]);
    t.scratch_file(
        "test/inc.bzl",
        &[
            "def f():",
            "    native.config_setting(name = \"x\", define_values = {\"key\": \"value\"})",
            "    rs = native.existing_rules()",
            "    rs[\"x\"][\"define_values\"][\"key\"] = 123",
        ],
    ); // mutate an attribute value within the view
    let pkg = t.get_package("test");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event("trying to mutate a frozen dict value");
}

// Java: StarlarkNativeModule.java:177-216 DictLikeView.keys / starlarkValues / items return
// `StarlarkList.immutableCopyOf(...)`; mutating one is Starlark.java:264 checkMutable.
#[test]
fn existing_rule_keys_values_items_are_immutable() {
    for method in ["keys", "values", "items"] {
        let t = PackageLoadingTestCase::new();
        t.scratch_file("test/BUILD", &["load(\"inc.bzl\", \"f\")", "", "f()"]);
        let mutation = format!("    native.existing_rule(\"x\").{method}().append(1)");
        t.scratch_file(
            "test/inc.bzl",
            &[
                "def f():",
                "    native.config_setting(name = \"x\", define_values = {\"key\": \"value\"})",
                mutation.as_str(),
            ],
        );
        let pkg = t.get_package("test");
        assert!(pkg.package.contains_errors(), "{method}");
        pkg.assert_contains_event("trying to mutate a frozen list value");
    }
}

// ---- S2-8: @Param Sequence checks of the BUILD natives

// Java: PackageFactoryTest.testGlobEnforcesListArgument
#[test]
fn test_glob_enforces_list_argument() {
    assert_glob_fails(
        "glob(1, exclude=2)",
        "in call to glob(), parameter 'include' got value of type 'int', want 'sequence'",
    );
}

// Java: PackageFactoryTest.testGlobEnforcesListOfStringsArguments
#[test]
fn test_glob_enforces_list_of_strings_arguments() {
    assert_glob_fails(
        "glob(['a', 'b'], exclude=['c', 42])",
        "expected value of type 'string' for element 1 of 'glob' argument, but got 42 (int)",
    );
}

// Java: StarlarkNativeModuleApi.java:63-73 (`include` is
// `@ParamType(type = Sequence.class, generic1 = String.class)`); a dict is not a Sequence, so
// BuiltinFunction.java:516 checkParamValue reports it.
#[test]
fn glob_rejects_a_dict() {
    assert_glob_fails(
        "glob({'*.cc': 1})",
        "in call to glob(), parameter 'include' got value of type 'dict', want 'sequence'",
    );
    assert_glob_fails(
        "glob(['*.cc'], exclude = {'a.cc': 1})",
        "in call to glob(), parameter 'exclude' got value of type 'dict', want 'sequence'",
    );
}

// Java: RangeList implements Sequence (RangeList.java:45), so `range(3)` passes the parameter
// check; StarlarkNativeModule.java:102 `Types.STRING_LIST.convert(include, "'glob' argument")`
// then fails on the first element as in testGlobEnforcesListOfStringsArguments.
#[test]
fn glob_of_a_range_fails_on_its_elements() {
    assert_glob_fails(
        "glob(range(3))",
        "expected value of type 'string' for element 0 of 'glob' argument, but got 0 (int)",
    );
}

// Java: StarlarkNativeModuleApi.java (`subpackages`, `exports_files`, `package_group`) and
// BuildGlobals.java:48-63,116-120 (`environment_group`, `licenses`): Sequence parameters,
// `exports_files`' `visibility` and `licenses` being Sequence or NoneType
// (ParamDescriptor.java:139 joins two types with " or "). The text is
// BuiltinFunction.java:516 checkParamValue.
#[test]
fn build_natives_enforce_sequence_parameters() {
    assert_build_file_fails(
        &["x = subpackages(include = '*')"],
        "in call to subpackages(), parameter 'include' got value of type 'string', want \
         'sequence'",
    );
    assert_build_file_fails(
        &["x = subpackages(include = ['*'], exclude = {'a': 1})"],
        "in call to subpackages(), parameter 'exclude' got value of type 'dict', want 'sequence'",
    );
    assert_build_file_fails(
        &["exports_files({'a.txt': 1})"],
        "in call to exports_files(), parameter 'srcs' got value of type 'dict', want 'sequence'",
    );
    assert_build_file_fails(
        &["exports_files(['a.txt'], visibility = '//visibility:public')"],
        "in call to exports_files(), parameter 'visibility' got value of type 'string', want \
         'sequence or NoneType'",
    );
    assert_build_file_fails(
        &["exports_files(['a.txt'], licenses = 'notice')"],
        "in call to exports_files(), parameter 'licenses' got value of type 'string', want \
         'sequence or NoneType'",
    );
    assert_build_file_fails(
        &["package_group(name = 'g', packages = '//foo')"],
        "in call to package_group(), parameter 'packages' got value of type 'string', want \
         'sequence'",
    );
    assert_build_file_fails(
        &["package_group(name = 'g', includes = {':h': 1})"],
        "in call to package_group(), parameter 'includes' got value of type 'dict', want \
         'sequence'",
    );
    assert_build_file_fails(
        &["environment_group(name = 'g', environments = ':a', defaults = [])"],
        "in call to environment_group(), parameter 'environments' got value of type 'string', \
         want 'sequence'",
    );
    assert_build_file_fails(
        &["environment_group(name = 'g', environments = [], defaults = ':a')"],
        "in call to environment_group(), parameter 'defaults' got value of type 'string', want \
         'sequence'",
    );
    assert_build_file_fails(
        &["licenses('notice')"],
        "in call to licenses(), parameter 'license_strings' got value of type 'string', want \
         'sequence'",
    );
}

// A tuple is a Sequence (Tuple.java), so the checks accept it as they accept a list.
#[test]
fn build_natives_accept_tuples() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/a.txt", &[]);
    t.scratch_file(
        "pkg/BUILD",
        &[
            "exports_files(('a.txt',), visibility = ('//visibility:public',))",
            "filegroup(name = 'g', srcs = glob(('*.txt',), exclude = ()))",
        ],
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_eq!(pkg.label_list_attr("g", "srcs"), vec!["//pkg:a.txt"]);
}

// ---- S2-10: repository_name and --incompatible_enable_deprecated_label_apis

// Java: StarlarkNativeModuleApi.java:261 `enableOnlyWithFlag =
// INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS`. The function stays bound
// (StarlarkNativeModule.java:88 `Starlark.addMethods(bindings, new StarlarkNativeModule())`
// uses the default semantics); calling it without the flag is MethodDescriptor.java:421-424.
#[test]
fn repository_name_is_unavailable_without_the_deprecated_label_apis_flag() {
    let expected = "function repository_name() is experimental and thus unavailable with the \
                    current flags. It may be enabled by setting \
                    --incompatible_enable_deprecated_label_apis";

    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--noincompatible_enable_deprecated_label_apis"]);
    t.scratch_file("pkg/BUILD", &["x = repository_name()"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(expected);

    // `native.repository_name` in a macro.
    t.scratch_file(
        "macro/defs.bzl",
        &["def f():", "    return native.repository_name()"],
    );
    t.scratch_file("macro/BUILD", &["load(':defs.bzl', 'f')", "x = f()"]);
    let pkg = t.get_package("macro");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(expected);
}

// Java: BuildLanguageOptions.java:732-740: the flag defaults to true, and
// StarlarkNativeModule.java:637 repositoryName returns "@" + repoName (the main repository's
// name is empty).
#[test]
fn repository_name_is_available_by_default() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/defs.bzl",
        &["def f():", "    return native.repository_name()"],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(':defs.bzl', 'f')",
            "filegroup(name = 'build_' + repository_name().replace('@', 'at'))",
            "filegroup(name = 'macro_' + f().replace('@', 'at'))",
        ],
    );
    let pkg = t.get_package_without_errors("pkg");
    pkg.rule("build_at");
    pkg.rule("macro_at");
}
