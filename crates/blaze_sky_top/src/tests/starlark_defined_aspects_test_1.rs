//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java`
//! (an `AnalysisTestCase`), part 1 of 3: the tests from `simpleAspect` through
//! `toplevelAspectOnFile`, in file order, with the private helpers of the Java class they use.
//!
//! PORT: the Java class's `WithKeepGoing` subclass (every test again with `--keep_going`) is not
//! ported here; `keep_going()` is the base class's `false`.

use std::sync::Arc;

use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_analysis::providers::output_group_info::{
    HIDDEN_TOP_LEVEL, INTERNAL_SUFFIX, OutputGroupInfo,
};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO;
use blaze_skyframe::SkyKey;
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

use crate::build_view::{AnalysisResult, BuildViewError};
use crate::testing::build_view_test_case::parse_label;
use crate::testing::{
    AnalysisTestCase, BuildViewTestCase, assert_contains_exactly, base_artifact_names, has_error,
};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:88 StarlarkDefinedAspectsTest.keepGoing
fn keep_going() -> bool {
    false
}

// -------------------------------------------------------------------------------------------
// Private helpers of the Java test class (and of AnalysisTestCase).

/// `getAspectDescriptions(analysisResult)`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:260 StarlarkDefinedAspectsTest.getAspectDescriptions
fn get_aspect_descriptions(analysis_result: &AnalysisResult) -> Vec<String> {
    analysis_result
        .get_aspects_map()
        .keys()
        .map(|aspect_key| {
            format!(
                "{}({})",
                aspect_key.get_aspect_class().get_name(),
                aspect_key.get_label()
            )
        })
        .collect()
}

/// `getLabelsToBuild(analysisResult)`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:318 StarlarkDefinedAspectsTest.getLabelsToBuild
fn get_labels_to_build(analysis_result: &AnalysisResult) -> Vec<String> {
    analysis_result
        .get_targets_to_build()
        .iter()
        .map(|configured_target| configured_target.get_label().to_string())
        .collect()
}

/// `getOutputGroupContents(outputGroupInfo, groupName)`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:1593 StarlarkDefinedAspectsTest.getOutputGroupContents
fn get_output_group_contents(output_group_info: &OutputGroupInfo, group_name: &str) -> Vec<String> {
    output_group_info
        .get_output_group(group_name)
        .to_list()
        .iter()
        .map(Artifact::get_root_relative_path_string)
        .collect()
}

/// `Iterables.getOnlyElement(analysisResult.getAspectsMap().values())`.
fn only_aspect(analysis_result: &AnalysisResult) -> Arc<AspectValue> {
    let values: Vec<_> = analysis_result.get_aspects_map().values().collect();
    assert_eq!(
        values.len(),
        1,
        "expected exactly one aspect, got {:?}",
        analysis_result.get_aspects_map().keys().collect::<Vec<_>>()
    );
    values[0].clone()
}

/// `analysisResult.getAspectsMap().values().iterator().next()`.
fn first_aspect(analysis_result: &AnalysisResult) -> Arc<AspectValue> {
    analysis_result
        .get_aspects_map()
        .values()
        .next()
        .expect("NoSuchElementException: no aspect")
        .clone()
}

/// `Iterables.getOnlyElement(analysisResult.getTargetsToBuild())`.
fn only_target(analysis_result: &AnalysisResult) -> ConfiguredTarget {
    let targets = analysis_result.get_targets_to_build();
    assert_eq!(targets.len(), 1, "expected exactly one target to build");
    targets[0].clone()
}

/// `new StarlarkProvider.Key(keyForBuild(Label.parseCanonical(label)), symbol)`.
fn starlark_provider_key(label: &str, symbol: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: parse_label(label),
        exported_name: symbol.to_string(),
    }
}

/// `AnalysisTestCase.getStarlarkProvider(ProviderCollection, label, providerSymbol)` for an
/// aspect.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:715 AnalysisTestCase.getStarlarkProvider
fn get_starlark_provider_of_aspect(
    aspect: &AspectValue,
    label: &str,
    provider_symbol: &str,
) -> Option<OwnedFrozenValue> {
    aspect
        .get_providers()
        .get(&starlark_provider_key(label, provider_symbol))
        .cloned()
}

/// `AnalysisTestCase.getStarlarkProvider(ConfiguredTarget, providerSymbol)`: the provider
/// defined by the .bzl file of the target's rule class.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:702 AnalysisTestCase.getStarlarkProvider
fn get_starlark_provider(
    t: &mut AnalysisTestCase,
    target: &ConfiguredTarget,
    provider_symbol: &str,
) -> Option<OwnedFrozenValue> {
    let rule_target = t
        .get_target(&target.get_label().to_string())
        .unwrap_or_else(|e| panic!("{e:?}"));
    let rule = rule_target
        .get_associated_rule()
        .expect("getAssociatedRule")
        .clone();
    let bzl = rule
        .get_rule_class_object()
        .get_rule_definition_environment_label()
        .expect("getRuleDefinitionEnvironmentLabel")
        .to_string();
    target.get(&starlark_provider_key(&bzl, provider_symbol))
}

/// `((StarlarkInfo) info).getValue(name)`, handed to `f`.
fn with_field<R>(info: &OwnedFrozenValue, name: &str, f: impl FnOnce(Value) -> R) -> R {
    let v = info.value();
    let info = StarlarkInfo::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a StarlarkInfo"));
    let field = info
        .get_field(name)
        .unwrap_or_else(|| panic!("no field '{name}' in {v}"));
    f(field)
}

/// `info.getValue(name, String.class)`.
fn string_field(info: &OwnedFrozenValue, name: &str) -> String {
    with_field(info, name, |v| {
        v.unpack_str()
            .unwrap_or_else(|| panic!("ClassCastException: {v} is not a String"))
            .to_string()
    })
}

/// `((Depset) v).toList()`, each element mapped by `f`.
fn depset_elements<R>(v: Value, f: impl Fn(Value) -> R) -> Vec<R> {
    let depset =
        Depset::from_value(v).unwrap_or_else(|| panic!("ClassCastException: {v} is not a Depset"));
    depset
        .to_list()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(f)
        .collect()
}

fn label_of(v: Value) -> blaze_cmdline::Label {
    v.downcast_ref::<LabelValue>()
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a Label"))
        .label()
        .clone()
}

fn artifact_of(v: Value) -> Artifact {
    artifact_from_value(v).unwrap_or_else(|| panic!("ClassCastException: {v} is not an Artifact"))
}

fn string_of(v: Value) -> String {
    v.unpack_str()
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a String"))
        .to_string()
}

/// `OutputGroupInfo.get(configuredAspect)`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:137 OutputGroupInfo.get
fn output_group_info_of_aspect(aspect: &AspectValue) -> Option<OutputGroupInfo> {
    let info = aspect.get_providers().get(&OUTPUT_GROUP_INFO.key())?;
    Some(
        blaze_analysis::info::downcast_info::<OutputGroupInfo>(info)
            .expect("ClassCastException: OutputGroupInfo")
            .clone(),
    )
}

/// `OutputGroupInfo.get(configuredTarget)`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:132 OutputGroupInfo.get
fn output_group_info_of_target(target: &ConfiguredTarget) -> Option<OutputGroupInfo> {
    let info = target.get(&OUTPUT_GROUP_INFO.key())?;
    Some(
        blaze_analysis::info::downcast_info::<OutputGroupInfo>(&info)
            .expect("ClassCastException: OutputGroupInfo")
            .clone(),
    )
}

/// `getProvider(FileProvider.class).getFilesToBuild()` of the only target to build, as base
/// names.
fn only_target_files_to_build_base_names(analysis_result: &AnalysisResult) -> Vec<String> {
    let target = only_target(analysis_result);
    let files_to_build = target
        .get_provider::<FileProvider>()
        .expect("FileProvider")
        .get_files_to_build()
        .to_list();
    base_artifact_names(files_to_build.iter())
}

/// The Java pattern
/// `try { result = update(...); assertThat(keepGoing()).isTrue(); assertThat(result.hasError()).isTrue(); }
/// catch (ViewCreationFailedException e) { /* expect to fail */ }`.
///
/// PORT: the harness reports Java's `TargetParsingException` as
/// `BuildViewError::ViewCreationFailed` too (`BuildViewTestCase::update_internal`).
fn expect_failure(result: Result<&AnalysisResult, BuildViewError>) {
    match result {
        Ok(result) => {
            assert!(keep_going(), "update() succeeded without --keep_going");
            assert!(has_error(result));
        }
        Err(BuildViewError::ViewCreationFailed { .. }) => {
            // expect to fail.
        }
        Err(e) => panic!("unexpected exception: {e:?}"),
    }
}

// -------------------------------------------------------------------------------------------
// Tests.

// Java: StarlarkDefinedAspectsTest.simpleAspect
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn simple_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//test:aspect.bzl%MyAspect(//test:xxx)"],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectCanBeDefinedUsingFactory
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn aspect_can_be_defined_using_factory() {
    let mut t = new_test();
    t.scratch.file(
        "pkg/foo.bzl",
        &[r#"def _impl(target, ctx):
    return []

def aspect_factory():
    return aspect(implementation=_impl)

my_aspect = aspect_factory()
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(name = "abc")
"#],
    );

    let analysis_result = t
        .update_with_aspects(&["pkg/foo.bzl%my_aspect"], &["//pkg:abc"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//pkg:abc"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//pkg:foo.bzl%my_aspect(//pkg:abc)"],
    );
}

// Regression test for b/409532322
// Java: StarlarkDefinedAspectsTest.aspectCannotBeDefinedInBuildFileThread
#[test]
fn aspect_cannot_be_defined_in_build_file_thread() {
    let mut t = new_test();
    t.scratch.file(
        "pkg/foo.bzl",
        &[r#"def _impl(target, ctx):
    return []

def aspect_factory():
    return aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"load(":foo.bzl", "aspect_factory")
my_aspect = aspect_factory()
"#],
    );

    t.remove_fail_fast_handler();
    // PORT: assertThrows(TargetParsingException.class, ...): the harness reports it as an error
    // of update().
    assert!(t.update(&["//pkg:BUILD"]).is_err());
    t.assert_contains_event("aspect() can only be used during .bzl initialization");
}

// Java: StarlarkDefinedAspectsTest.aspectWithSingleDeclaredProvider
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_with_single_declared_provider() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"foo = provider()
def _impl(target, ctx):
   return foo()
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//test:aspect.bzl%MyAspect(//test:xxx)"],
    );
    let configured_aspect = only_aspect(analysis_result);

    let foo_key = starlark_provider_key("//test:aspect.bzl", "foo");

    let foo = configured_aspect.get_providers().get(&foo_key).unwrap();
    assert_eq!(
        blaze_analysis::info::info_provider_key(foo.value()),
        Some(foo_key)
    );
}

// Java: StarlarkDefinedAspectsTest.aspectWithDeclaredProviders
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_with_declared_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"foo = provider()
bar = provider()
def _impl(target, ctx):
   return [foo(), bar()]
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//test:aspect.bzl%MyAspect(//test:xxx)"],
    );
    let configured_aspect = only_aspect(analysis_result);

    let foo_key = starlark_provider_key("//test:aspect.bzl", "foo");
    let bar_key = starlark_provider_key("//test:aspect.bzl", "bar");

    let foo = configured_aspect.get_providers().get(&foo_key).unwrap();
    assert_eq!(
        blaze_analysis::info::info_provider_key(foo.value()),
        Some(foo_key)
    );
    let bar = configured_aspect.get_providers().get(&bar_key).unwrap();
    assert_eq!(
        blaze_analysis::info::info_provider_key(bar.value()),
        Some(bar_key)
    );
}

// Java: StarlarkDefinedAspectsTest.aspectWithDeclaredProvidersInAStruct
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_with_declared_providers_in_a_struct() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"foo = provider()
bar = provider()
def _impl(target, ctx):
   return [foo(), bar()]
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//test:aspect.bzl%MyAspect(//test:xxx)"],
    );

    let configured_aspect = only_aspect(analysis_result);

    let foo_key = starlark_provider_key("//test:aspect.bzl", "foo");
    let bar_key = starlark_provider_key("//test:aspect.bzl", "bar");

    let foo = configured_aspect.get_providers().get(&foo_key).unwrap();
    assert_eq!(
        blaze_analysis::info::info_provider_key(foo.value()),
        Some(foo_key)
    );
    let bar = configured_aspect.get_providers().get(&bar_key).unwrap();
    assert_eq!(
        blaze_analysis::info::info_provider_key(bar.value()),
        Some(bar_key)
    );
}

// Java: StarlarkDefinedAspectsTest.aspectCommandLineLabel
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_command_line_label() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["//test:aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["//test:aspect.bzl%MyAspect(//test:xxx)"],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectCommandLineRepoLabel
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_command_line_repo_label() {
    let mut t = new_test();
    t.scratch.append_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='local')",
            "local_path_override(module_name='local', path='local/repo')",
        ],
    );
    t.scratch
        .file("local/repo/MODULE.bazel", &["module(name='local')"]);
    t.scratch.file(
        "local/repo/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file("local/repo/BUILD", &[]);

    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["@local//:aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    assert_contains_exactly(
        &get_aspect_descriptions(analysis_result),
        &["@@local+//:aspect.bzl%MyAspect(//test:xxx)"],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectAllowsFragmentsToBeSpecified
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_allows_fragments_to_be_specified() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl, fragments=['java'])
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();

    let keys: Vec<_> = analysis_result.get_aspects_map().keys().cloned().collect();
    assert_eq!(keys.len(), 1);
    let key = keys[0].clone();
    let aspect_value = t
        .get_evaluator()
        .get_existing_value(&SkyKey::new(key))
        .expect("existing value")
        .downcast::<AspectValue>()
        .unwrap_or_else(|_| panic!("ClassCastException: AspectValue"));
    let aspect_definition = aspect_value.get_aspect().get_definition();
    // PORT: isLegalConfigurationFragment(Class) takes the class's simple name and its
    // @StarlarkBuiltin name.
    assert!(
        aspect_definition
            .get_configuration_fragment_policy()
            .is_legal_configuration_fragment("JavaConfiguration", Some("java"))
    );
    assert!(
        !aspect_definition
            .get_configuration_fragment_policy()
            .is_legal_configuration_fragment("CppConfiguration", Some("cpp"))
    );
}

// Java: StarlarkDefinedAspectsTest.aspectPropagating
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_propagating() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"Info = provider()
def _impl(target, ctx):
   s = depset([target.label], transitive = [i[Info].target_labels for i in ctx.rule.attr.deps])
   c = depset([ctx.rule.kind], transitive = [i[Info].rule_kinds for i in ctx.rule.attr.deps])
   return Info(target_labels = s, rule_kinds = c)

MyAspect = aspect(
   implementation=_impl,
   attr_aspects=['deps'],
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
java_library(
     name = 'xxx',
     srcs = ['A.java'],
     deps = [':yyy'],
)
"#],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    let configured_aspect = first_aspect(analysis_result);
    let info = get_starlark_provider_of_aspect(&configured_aspect, "//test:aspect.bzl", "Info")
        .expect("Info");
    let names = with_field(&info, "target_labels", |v| {
        depset_elements(v, |e| label_of(e).to_string())
    });
    assert_contains_exactly(&names, &["//test:xxx", "//test:yyy"]);
    let rule_kinds = with_field(&info, "rule_kinds", |v| {
        assert!(Depset::from_value(v).is_some(), "not a Depset: {v}");
        depset_elements(v, string_of)
    });
    assert_contains_exactly(&rule_kinds, &["java_library"]);
}

// Java: StarlarkDefinedAspectsTest.aspectsPropagatingForDefaultAndImplicit
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn aspects_propagating_for_default_and_implicit() {
    let mut t = new_test();
    t.use_configuration(&[
        "--experimental_builtins_injection_override=+cc_library",
        "--incompatible_enable_cc_toolchain_resolution",
    ]);
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"Info = provider()
def _impl(target, ctx):
   s = []
   c = []
   a = ctx.rule.attr
   if getattr(a, '_defaultattr', None):
       s += [a._defaultattr[Info].target_labels]
       c += [a._defaultattr[Info].rule_kinds]
   if getattr(a, '_cc_toolchain', None):
       s += [a._cc_toolchain[Info].target_labels]
       c += [a._cc_toolchain[Info].rule_kinds]
   return Info(
       target_labels = depset([target.label], transitive = s),
       rule_kinds = depset([ctx.rule.kind], transitive = c))

def _rule_impl(ctx):
   pass

my_rule = rule(implementation = _rule_impl,
   attrs = { '_defaultattr' : attr.label(default = Label('//test:xxx')) },
)
MyAspect = aspect(
   implementation=_impl,
   attr_aspects=['_defaultattr', '_cc_toolchain'],
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load('//test:aspect.bzl', 'my_rule')
cc_library(
     name = 'xxx',
)
my_rule(
     name = 'yyy',
)
"#],
    );
    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:yyy"])
        .unwrap();
    let configured_aspect = first_aspect(analysis_result);
    let info_key = starlark_provider_key("//test:aspect.bzl", "Info");
    let info = configured_aspect
        .get_providers()
        .get(&info_key)
        .expect("Info")
        .clone();
    let names = with_field(&info, "target_labels", |v| {
        depset_elements(v, |o| label_of(o).get_name().to_string())
    });

    // containsAtLeast("xxx", "yyy")
    for expected in ["xxx", "yyy"] {
        assert!(
            names.iter().any(|n| n == expected),
            "{names:?} does not contain {expected}"
        );
    }
}

// Java: StarlarkDefinedAspectsTest.aspectsDirOnMergedTargets
#[test]
#[ignore = "BLAZE BUG: dir() of a MergedConfiguredTarget lists 'actions' twice; Java Starlark.dir collects getFieldNames into a TreeSet (dedup), StarlarkTarget::dir_attr (blaze_analysis/src/configuredtargets/target_value.rs:388) returns the duplicate from MergedConfiguredTarget.addExtraStarlarkKeys over a rule base"]
fn aspects_dir_on_merged_targets() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"Info = provider()
def _impl(target, ctx):
   return Info(aspect_provider = 'data')

p = provider()
MyAspect = aspect(implementation=_impl)
def _rule_impl(ctx):
   if ctx.attr.dep:
      return [p(dir = dir(ctx.attr.dep))]
   return [p()]

my_rule = rule(implementation = _rule_impl,
   attrs = { 'dep' : attr.label(aspects = [MyAspect]) },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx',)
my_rule(name = 'yyy', dep = ':xxx')
"#],
    );
    let target = only_target(t.update(&["//test:yyy"]).unwrap());

    let names = get_starlark_provider(&mut t, &target, "p").expect("p");
    let dir = with_field(&names, "dir", |v| {
        ListRef::from_value(v)
            .unwrap_or_else(|| panic!("not a list: {v}"))
            .iter()
            .map(string_of)
            .collect::<Vec<_>>()
    });
    assert_contains_exactly(
        &dir,
        &[
            "actions",
            "data_runfiles",
            "default_runfiles",
            "files",
            "files_to_run",
            "label",
            "output_groups",
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectWithOutputGroupsDeclaredProvider
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_with_output_groups_declared_provider() {
    let mut t = new_test();
    let line = format!("   f = target[OutputGroupInfo]._hidden_top_level{INTERNAL_SUFFIX}");
    t.scratch.file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            &line,
            "   return [OutputGroupInfo(my_result = f)]",
            "",
            "MyAspect = aspect(",
            "   implementation=_impl,",
            ")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'xxx',
     srcs = ['A.java'],
)
"#],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    let configured_aspect = first_aspect(analysis_result);
    let output_group_info = output_group_info_of_aspect(&configured_aspect);

    let output_group_info = output_group_info.expect("OutputGroupInfo");
    let names = output_group_info.get_output_group("my_result");
    assert!(!names.to_list().is_empty());

    // Configuration of the true Artifact may diverge slightly (e.g. be trimmed) causing owners to
    // also diverge so just compare paths instead of the whole Artifact.
    let paths: Vec<String> = names
        .to_list()
        .iter()
        .map(|a| a.get_path().get_path_string().to_string())
        .collect();
    let xxx = t.get_configured_target("//test:xxx").unwrap();
    let expected_paths: Vec<String> = output_group_info_of_target(&xxx)
        .expect("OutputGroupInfo")
        .get_output_group(HIDDEN_TOP_LEVEL)
        .to_list()
        .iter()
        .map(|a| a.get_path().get_path_string().to_string())
        .collect();
    let expected: Vec<&str> = expected_paths.iter().map(String::as_str).collect();
    assert_contains_exactly(&paths, &expected);
}

// Java: StarlarkDefinedAspectsTest.aspectWithOutputGroupsAsListDeclaredProvider
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_with_output_groups_as_list_declared_provider() {
    let mut t = new_test();
    let line = format!("   g = target[OutputGroupInfo]._hidden_top_level{INTERNAL_SUFFIX}");
    t.scratch.file(
        "test/aspect.bzl",
        &[
            "def _impl(target, ctx):",
            &line,
            "   return [OutputGroupInfo(my_result=g.to_list())]",
            "",
            "MyAspect = aspect(",
            "   implementation=_impl,",
            ")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'xxx',
     srcs = ['A.java'],
)
"#],
    );

    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert_contains_exactly(
        &analysis_result
            .get_targets_to_build()
            .iter()
            .map(|configured_target| configured_target.get_label().to_string())
            .collect::<Vec<_>>(),
        &["//test:xxx"],
    );
    let configured_aspect = first_aspect(analysis_result);
    let output_group_info = output_group_info_of_aspect(&configured_aspect);
    let output_group_info = output_group_info.expect("OutputGroupInfo");
    let names = output_group_info.get_output_group("my_result");
    assert!(!names.to_list().is_empty());

    // Configuration of the true Artifact may diverge slightly (e.g. be trimmed) causing owners to
    // also diverge so just compare paths instead of the whole Artifact.
    let paths: Vec<String> = names
        .to_list()
        .iter()
        .map(|a| a.get_path().get_path_string().to_string())
        .collect();
    let xxx = t.get_configured_target("//test:xxx").unwrap();
    let expected_paths: Vec<String> = output_group_info_of_target(&xxx)
        .expect("OutputGroupInfo")
        .get_output_group(HIDDEN_TOP_LEVEL)
        .to_list()
        .iter()
        .map(|a| a.get_path().get_path_string().to_string())
        .collect();
    let expected: Vec<&str> = expected_paths.iter().map(String::as_str).collect();
    assert_contains_exactly(&paths, &expected);
}

// Java: StarlarkDefinedAspectsTest.aspectsFromStarlarkRules
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspects_from_starlark_rules() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
   s = depset([target.label], transitive =
       [i[AspectInfo].target_labels for i in ctx.rule.attr.deps])
   return AspectInfo(target_labels = s)

TargetInfo = provider()
def _rule_impl(ctx):
   s = depset(transitive = [i[AspectInfo].target_labels for i in ctx.attr.attr])
   return TargetInfo(rule_deps = s)

MyAspect = aspect(
   implementation=_aspect_impl,
   attr_aspects=['deps'],
)
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' :
             attr.label_list(mandatory=True, allow_files=True, aspects = [MyAspect]) },
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
my_rule(
     name = 'xxx',
     attr = [':yyy'],
)
"#],
    );

    let analysis_result = t.update(&["//test:xxx"]).unwrap();
    assert_contains_exactly(&get_labels_to_build(analysis_result), &["//test:xxx"]);
    let target = analysis_result.get_targets_to_build()[0].clone();
    let info = get_starlark_provider(&mut t, &target, "TargetInfo").expect("TargetInfo");
    let names = with_field(&info, "rule_deps", |v| {
        depset_elements(v, |e| label_of(e).to_string())
    });
    assert_contains_exactly(&names, &["//test:yyy"]);
}

// Java: StarlarkDefinedAspectsTest.aspectsNonExported
#[test]
fn aspects_non_exported() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _aspect_impl(target, ctx):
   return []

def _rule_impl(ctx):
   pass

def mk_aspect():
   return aspect(implementation=_aspect_impl)
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' : attr.label_list(aspects = [mk_aspect()]) },  # line 11
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
my_rule(
     name = 'xxx',
     attr = [':yyy'],
)
"#],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update(&["//test:xxx"]));

    // attr.label_list() fails, stack=[<toplevel>@rules.bzl:11:38, label_list:<builtin>]
    t.assert_contains_event(
        &t.ws("File \"/workspace/test/aspect.bzl\", line 11, column 38, in <toplevel>"),
    );
    t.assert_contains_event(
        "Error in label_list: Aspects should be top-level values in extension files that define \
         them.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectReturnsNonExportedProvider
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_returns_non_exported_provider() {
    let mut t = new_test();
    t.scratch.file(
        "test/inc.bzl",
        &[
            r#"a = aspect(implementation = lambda target, ctx: [provider()()])
r = rule(
  implementation = lambda ctx: [],
  attrs = {'a': attr.label_list(aspects = [a])})
"#,
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:inc.bzl', 'r')
load("@rules_java//java:defs.bzl", "java_library")
java_library(name = 'j')
r(name = 'test', a = [':j'])
"#],
    );

    t.remove_fail_fast_handler();
    match t.update(&["//test"]) {
        Ok(_) => { /* reached if --keep_going=true */ }
        Err(BuildViewError::ViewCreationFailed { .. }) => { /* reached if --keep_going=false */ }
        Err(e) => panic!("unexpected exception: {e:?}"),
    }
    t.assert_contains_event(&t.ws("aspect function returned an instance of a provider \
         (defined at /workspace/test/inc.bzl:1:58) that is not a global"));
}

// Java: StarlarkDefinedAspectsTest.providerNonExported
#[test]
fn provider_non_exported() {
    let mut t = new_test();
    t.scratch.file(
        "test/rule.bzl",
        &[r#"def mk_provider():
   return provider()
def _rule_impl(ctx):
   pass
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' : attr.label_list(providers = [mk_provider()]) },  # line 7
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:rule.bzl', 'my_rule')
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
my_rule(
     name = 'xxx',
     attr = [':yyy'],
)
"#],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update(&["//test:xxx"]));

    // attr.label_list() fails, stack=[<toplevel>@rules.bzl:7:38, label_list:<builtin>]
    t.assert_contains_event(
        &t.ws("File \"/workspace/test/rule.bzl\", line 7, column 38, in <toplevel>"),
    );
    t.assert_contains_event(
        "Error in label_list: Providers should be top-level values in extension files that define \
         them.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnLabelAttr
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_on_label_attr() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
   return AspectInfo(aspect_data='foo')

RuleInfo = provider()
def _rule_impl(ctx):
   return RuleInfo(data=ctx.attr.attr[AspectInfo].aspect_data)

MyAspect = aspect(
   implementation=_aspect_impl,
)
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' :
             attr.label(aspects = [MyAspect]) },
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
my_rule(
     name = 'xxx',
     attr = ':yyy',
)
"#],
    );

    let target = t.update(&["//test:xxx"]).unwrap().get_targets_to_build()[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo").expect("RuleInfo");
    let value = string_field(&info, "data");
    assert_eq!(value, "foo");
}

// Java: StarlarkDefinedAspectsTest.labelKeyedStringDictAllowsAspects
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn label_keyed_string_dict_allows_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
   return AspectInfo(aspect_data=target.label.name)

RuleInfo = provider()
def _rule_impl(ctx):
   return RuleInfo(
       data=','.join(['{}:{}'.format(dep[AspectInfo].aspect_data, val)
                      for dep, val in ctx.attr.attr.items()]))

MyAspect = aspect(
   implementation=_aspect_impl,
)
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' :
             attr.label_keyed_string_dict(aspects = [MyAspect]) },
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'yyy',
)
my_rule(
     name = 'xxx',
     attr = {':yyy': 'zzz'},
)
"#],
    );

    let target = t.update(&["//test:xxx"]).unwrap().get_targets_to_build()[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo").expect("RuleInfo");
    let value = string_field(&info, "data");
    assert_eq!(value, "yyy:zzz");
}

// Java: StarlarkDefinedAspectsTest.labelListDictAllowsAspects
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn label_list_dict_allows_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
   return AspectInfo(aspect_data=target.label.name)

RuleInfo = provider()
def _rule_impl(ctx):
   return RuleInfo(
       data=','.join(['{}:{}'.format(dep[AspectInfo].aspect_data, val)
                      for val, deps in ctx.attr.attr.items() for dep in deps]))

MyAspect = aspect(
   implementation=_aspect_impl,
)
my_rule = rule(
   implementation=_rule_impl,
   attrs = { 'attr' : attr.label_list_dict(aspects = [MyAspect]) },
)
"#],
    );

    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
     name = 'yyy1',
)
cc_library(
     name = 'yyy2',
)
my_rule(
     name = 'xxx',
     attr = {'zzz': [':yyy1', ':yyy2']},
)
"#],
    );

    let target = t.update(&["//test:xxx"]).unwrap().get_targets_to_build()[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo").expect("RuleInfo");
    let value = string_field(&info, "data");
    assert_eq!(value, "yyy1:zzz,yyy2:zzz");
}

// Java: StarlarkDefinedAspectsTest.aspectsDoNotAttachToFiles
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspects_do_not_attach_to_files() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []

MyAspect = aspect(
   implementation=_impl,
   attr_aspects=['deps'],
)
"#],
    );
    t.scratch.file("test/zzz.jar", &[]);
    t.scratch.file(
        "test/BUILD",
        &[r#"exports_files(['zzz.jar'])
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'xxx',
     srcs = ['A.java'],
     deps = ['//test:zzz.jar'],
)
"#],
    );

    let result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])
        .unwrap();
    assert!(!has_error(result));
}

// Java: StarlarkDefinedAspectsTest.aspectsDoNotAttachToTopLevelFiles
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspects_do_not_attach_to_top_level_files() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"p = provider()
def _impl(target, ctx):
   return [p()]

MyAspect = aspect(
   implementation=_impl,
   attr_aspects=['deps'],
)
"#],
    );
    t.scratch.file("test/zzz.jar", &[]);
    t.scratch.file(
        "test/BUILD",
        &[r#"exports_files(['zzz.jar'])
load("@rules_java//java:defs.bzl", "java_library")
java_library(
     name = 'xxx',
     srcs = ['A.java'],
     deps = ['//test:zzz.jar'],
)
"#],
    );

    let result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:zzz.jar"])
        .unwrap();
    assert!(!has_error(result));
    assert_eq!(only_aspect(result).get_providers().get_provider_count(), 0);
}

// Java: StarlarkDefinedAspectsTest.aspectFailingExecution
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_failing_execution() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return 1 // 0

MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    // Stack doesn't include source lines because we haven't told EvalException
    // how to read from scratch.
    t.assert_contains_event(&t.ws("ERROR /workspace/test/BUILD:2:13: in \
         //test:aspect.bzl%MyAspect aspect on java_library rule //test:xxx: \n\
         Traceback (most recent call last):\n\
         \tFile \"/workspace/test/aspect.bzl\", line 2, column 13, in _impl\n\
         \t\treturn 1 // 0\n\
         Error: integer division by zero"));
}

// Java: StarlarkDefinedAspectsTest.aspectFailingReturnsNotAStruct
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_failing_returns_not_a_struct() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return 0

MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event(
        "Aspect implementation should return a list, or a provider instance, but got int",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectFailingOrphanArtifacts
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn aspect_failing_orphan_artifacts() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
  ctx.actions.declare_file('missing_in_action.txt')
  return []

MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx',)",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event(&t.ws("ERROR /workspace/test/BUILD:2:13: in \
         //test:aspect.bzl%MyAspect aspect on java_library rule //test:xxx: \n\
         The following files have no generating action:\n\
         test/missing_in_action.txt"));
}

// Java: StarlarkDefinedAspectsTest.aspectSkippingOrphanArtifactsWithLocation
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value (Attribute::value_to_starlark, blaze_packages/src/attribute/attribute.rs:1406; Java Attribute.valueToStarlark returns the License StarlarkValue) reached when the aspect ctx.rule.attr converts the licenses attribute (StarlarkAttributesCollection.maybe_direct_val)"]
fn aspect_skipping_orphan_artifacts_with_location() {
    let mut t = new_test();
    t.scratch.file(
        "simple/print.bzl",
        &[r#"Info = provider()
def _print_expanded_location_impl(target, ctx):
    return Info(result=ctx.expand_location(ctx.rule.attr.cmd, []))

print_expanded_location = aspect(
    implementation = _print_expanded_location_impl,
)
"#],
    );
    t.scratch.file(
        "simple/BUILD",
        &[r#"filegroup(
    name = "files",
    srcs = ["afile"],
)

genrule(
    name = "concat_all_files",
    srcs = [":files"],
    outs = ["concatenated.txt"],
    cmd = "$(location :files)"
)
"#],
    );

    t.remove_fail_fast_handler();
    let analysis_result = t
        .update_with_aspects(
            &["//simple:print.bzl%print_expanded_location"],
            &["//simple:concat_all_files"],
        )
        .unwrap();
    assert!(!has_error(analysis_result));
    let configured_aspect = first_aspect(analysis_result);
    let info = get_starlark_provider_of_aspect(&configured_aspect, "//simple:print.bzl", "Info")
        .expect("Info");
    let result = string_field(&info, "result");

    assert_eq!(result, "simple/afile");
}

// Java: StarlarkDefinedAspectsTest.expandLocationFailsForTargetsWithSameLabel
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn expand_location_fails_for_targets_with_same_label() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "tools/allowlists/function_transition_allowlist/BUILD",
        &[r#"package_group(
    name = 'function_transition_allowlist',
    packages = [
        '//a/...',
    ],
)
"#],
    );
    t.scratch.file(
        "a/defs.bzl",
        &[
            "def _transition_impl(settings, attr):",
            "    return {",
            "        'opt': {'//command_line_option:compilation_mode': 'opt'},",
            "        'dbg': {'//command_line_option:compilation_mode': 'dbg'},",
            "    }",
            "split_transition = transition(",
            "    implementation = _transition_impl,",
            "    inputs = [],",
            "    outputs = ['//command_line_option:compilation_mode'])",
            "def _split_deps_rule_impl(ctx):",
            "    pass",
            "split_deps_rule = rule(",
            "    implementation = _split_deps_rule_impl,",
            "    attrs = {",
            "        'my_dep': attr.label(cfg = split_transition),",
            "    })",
            "",
            "Info = provider()",
            "def _print_expanded_location_impl(target, ctx):",
            "    return Info(result=ctx.expand_location('$(location //a:lib)', \
             [ctx.rule.attr.my_dep[0], ctx.rule.attr.my_dep[1]]))",
            "",
            "print_expanded_location = aspect(",
            "    implementation = _print_expanded_location_impl,",
            ")",
        ],
    );
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load('//a:defs.bzl', 'split_deps_rule')
cc_library(name = 'lib', srcs = ['lib.cc'])
split_deps_rule(
    name = 'a',
    my_dep = ':lib')
"#],
    );

    t.remove_fail_fast_handler();

    expect_failure(t.update_with_aspects(&["//a:defs.bzl%print_expanded_location"], &["//a"]));
    t.assert_contains_event("Label \"//a:lib\" is found more than once in 'targets' list.");
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectIsNotAnAspect
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn top_level_aspect_is_not_an_aspect() {
    let mut t = new_test();
    t.scratch.file("test/aspect.bzl", &["MyAspect = 4"]);
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx')",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event("MyAspect from //test:aspect.bzl is not an aspect");
}

// Java: StarlarkDefinedAspectsTest.duplicateOutputGroups
#[test]
fn duplicate_output_groups() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _aspect_impl(target, ctx):
  a = ctx.actions.declare_file('aspect.txt')
  ctx.actions.write(a, 'f')
  return [OutputGroupInfo(duplicate = depset([a]))]

MyAspect = aspect(implementation=_aspect_impl)
def _rule_impl(ctx):
  r = ctx.actions.declare_file('rule.txt')
  ctx.actions.write(r, 'r')
  return [OutputGroupInfo(duplicate = depset([r]))]
my_rule = rule(_rule_impl)
def _rbase_impl(ctx):
  return [DefaultInfo(files=ctx.attr.dep[OutputGroupInfo].duplicate)]
rbase = rule(_rbase_impl, attrs = { 'dep' : attr.label(aspects = [MyAspect]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule', 'rbase')
my_rule(name = 'xxx')
rbase(name = 'yyy', dep = ':xxx')
"#],
    );

    let analysis_result = t.update(&["//test:yyy"]).unwrap();
    assert_contains_exactly(
        &only_target_files_to_build_base_names(analysis_result),
        &["aspect.txt", "rule.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.mergesFileProvider
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value (Attribute::value_to_starlark, blaze_packages/src/attribute/attribute.rs:1406; Java Attribute.valueToStarlark returns the License StarlarkValue) reached when the aspect ctx.rule.attr converts the licenses attribute (StarlarkAttributesCollection.maybe_direct_val)"]
fn merges_file_provider() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _asp_impl(target, ctx):
  f = ctx.actions.declare_file('aspectfile')
  ctx.actions.write(f, 'f')
  return [DefaultInfo(files=depset([f]))]

asp = aspect(implementation=_asp_impl)
def _rule_impl(ctx):
  return [DefaultInfo(files=depset(ctx.files.dep))]
my_rule = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [asp]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule')
my_rule(name='a', dep=':b')
filegroup(name='b', srcs=['sourcefile'])
"#],
    );

    let analysis_result = t.update(&["//test:a"]).unwrap();
    assert_contains_exactly(
        &only_target_files_to_build_base_names(analysis_result),
        &["sourcefile", "aspectfile"],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectDefaultInfoWithNonArtifacts
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value (Attribute::value_to_starlark, blaze_packages/src/attribute/attribute.rs:1406; Java Attribute.valueToStarlark returns the License StarlarkValue) reached when the aspect ctx.rule.attr converts the licenses attribute (StarlarkAttributesCollection.maybe_direct_val)"]
fn aspect_default_info_with_non_artifacts() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _asp_impl(target, ctx):
  return [DefaultInfo(files=depset([1]))]

asp = aspect(implementation=_asp_impl)
def _rule_impl(ctx):
  return [DefaultInfo()]
my_rule = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [asp]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule')
my_rule(name='a', dep=':b')
filegroup(name='b', srcs=['sourcefile'])
"#],
    );

    t.remove_fail_fast_handler();
    match t.update(&["//test:a"]) {
        Ok(_) | Err(BuildViewError::ViewCreationFailed { .. }) => {
            // expected.
        }
        Err(e) => panic!("unexpected exception: {e:?}"),
    }
    t.assert_contains_event("should contain a depset of files");
}

// Java: StarlarkDefinedAspectsTest.aspectDefaultInfoSomethingElseThanFiles
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value (Attribute::value_to_starlark, blaze_packages/src/attribute/attribute.rs:1406; Java Attribute.valueToStarlark returns the License StarlarkValue) reached when the aspect ctx.rule.attr converts the licenses attribute (StarlarkAttributesCollection.maybe_direct_val)"]
fn aspect_default_info_something_else_than_files() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _asp_impl(target, ctx):
  return [DefaultInfo(runfiles=ctx.runfiles([]))]

asp = aspect(implementation=_asp_impl)
def _rule_impl(ctx):
  return [DefaultInfo()]
my_rule = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [asp]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule')
my_rule(name='a', dep=':b')
filegroup(name='b', srcs=['sourcefile'])
"#],
    );

    t.remove_fail_fast_handler();
    match t.update(&["//test:a"]) {
        Ok(_) | Err(BuildViewError::ViewCreationFailed { .. }) => {
            // expected.
        }
        Err(e) => panic!("unexpected exception: {e:?}"),
    }
    t.assert_contains_event("must only have the 'files' field set");
}

// Java: StarlarkDefinedAspectsTest.aspectEmptyDefaultInfo
#[test]
#[ignore = "BLAZE GAP: unported License as a Starlark value (Attribute::value_to_starlark, blaze_packages/src/attribute/attribute.rs:1406; Java Attribute.valueToStarlark returns the License StarlarkValue) reached when the aspect ctx.rule.attr converts the licenses attribute (StarlarkAttributesCollection.maybe_direct_val)"]
fn aspect_empty_default_info() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _asp_impl(target, ctx):
  return [DefaultInfo()]

asp = aspect(implementation=_asp_impl)
def _rule_impl(ctx):
  return [DefaultInfo(files=depset(ctx.files.dep))]
my_rule = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [asp]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule')
my_rule(name='a', dep=':b')
filegroup(name='b', srcs=['sourcefile'])
"#],
    );

    let analysis_result = t.update(&["//test:a"]).unwrap();
    assert_contains_exactly(
        &only_target_files_to_build_base_names(analysis_result),
        &["sourcefile"],
    );
}

// Java: StarlarkDefinedAspectsTest.outputGroupsFromOneAspect
#[test]
fn output_groups_from_one_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  f = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(f, 'f')
  return [OutputGroupInfo(a1_group = depset([f]))]

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])
def _rule_impl(ctx):
  if not ctx.attr.dep:
     return []
  og = {k:ctx.attr.dep.output_groups[k] for k in ctx.attr.dep[OutputGroupInfo]}
  return [OutputGroupInfo(**og)]
my_rule1 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a1]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule1')
my_rule1(name = 'base')
my_rule1(name = 'xxx', dep = ':base')
"#],
    );

    let analysis_result = t.update(&["//test:xxx"]).unwrap();
    let output_group_info =
        output_group_info_of_target(&only_target(analysis_result)).expect("OutputGroupInfo");
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a1_group"),
        &["test/base_a1.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.returningOutputGroupsNotList
#[test]
fn returning_output_groups_not_list() {
    let mut t = new_test();
    // OutputGroupInfo is also a list, tests that an aspect can return it without a list
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  f = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(f, 'f')
  return OutputGroupInfo(a1_group = depset([f]))

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])
def _rule_impl(ctx):
  if not ctx.attr.dep:
     return []
  og = {k:ctx.attr.dep.output_groups[k] for k in ctx.attr.dep[OutputGroupInfo]}
  return [OutputGroupInfo(**og)]
my_rule1 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a1]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule1')
my_rule1(name = 'base')
my_rule1(name = 'xxx', dep = ':base')
"#],
    );

    let analysis_result = t.update(&["//test:xxx"]).unwrap();
    let output_group_info =
        output_group_info_of_target(&only_target(analysis_result)).expect("OutputGroupInfo");
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a1_group"),
        &["test/base_a1.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.outputGroupsDeclaredProviderFromOneAspect
#[test]
fn output_groups_declared_provider_from_one_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  f = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(f, 'f')
  return [OutputGroupInfo(a1_group = depset([f]))]

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])
def _rule_impl(ctx):
  if not ctx.attr.dep:
     return []
  return [OutputGroupInfo(a1_group = ctx.attr.dep[OutputGroupInfo].a1_group)]
my_rule1 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a1]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule1')
my_rule1(name = 'base')
my_rule1(name = 'xxx', dep = ':base')
"#],
    );

    let analysis_result = t.update(&["//test:xxx"]).unwrap();
    let output_group_info =
        output_group_info_of_target(&only_target(analysis_result)).expect("OutputGroupInfo");
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a1_group"),
        &["test/base_a1.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.outputGroupsFromTwoAspects
#[test]
fn output_groups_from_two_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  f = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(f, 'f')
  return [OutputGroupInfo(a1_group = depset([f]))]

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])
def _rule_impl(ctx):
  if not ctx.attr.dep:
     return []
  og = {k:ctx.attr.dep.output_groups[k] for k in ctx.attr.dep.output_groups}
  return [OutputGroupInfo(**og)]
my_rule1 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a1]) })
def _a2_impl(target, ctx):
  g = ctx.actions.declare_file(target.label.name + '_a2.txt')
  ctx.actions.write(g, 'f')
  return [OutputGroupInfo(a2_group = depset([g]))]

a2 = aspect(implementation=_a2_impl, attr_aspects = ['dep'])
my_rule2 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a2]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule1', 'my_rule2')
my_rule1(name = 'base')
my_rule1(name = 'xxx', dep = ':base')
my_rule2(name = 'yyy', dep = ':xxx')
"#],
    );

    let analysis_result = t.update(&["//test:yyy"]).unwrap();
    let output_group_info =
        output_group_info_of_target(&only_target(analysis_result)).expect("OutputGroupInfo");
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a1_group"),
        &["test/base_a1.txt"],
    );
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a2_group"),
        &["test/xxx_a2.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.outputGroupsDeclaredProvidersFromTwoAspects
#[test]
fn output_groups_declared_providers_from_two_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  f = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(f, 'f')
  return [OutputGroupInfo(a1_group = depset([f]))]

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])
def _rule_impl(ctx):
  if not ctx.attr.dep:
     return []
  og = dict()
  dep_og = ctx.attr.dep[OutputGroupInfo]
  if hasattr(dep_og, 'a1_group'):
     og['a1_group'] = dep_og.a1_group
  if hasattr(dep_og, 'a2_group'):
     og['a2_group'] = dep_og.a2_group
  return [OutputGroupInfo(**og)]
my_rule1 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a1]) })
def _a2_impl(target, ctx):
  g = ctx.actions.declare_file(target.label.name + '_a2.txt')
  ctx.actions.write(g, 'f')
  return [OutputGroupInfo(a2_group = depset([g]))]

a2 = aspect(implementation=_a2_impl, attr_aspects = ['dep'])
my_rule2 = rule(_rule_impl, attrs = { 'dep' : attr.label(aspects = [a2]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule1', 'my_rule2')
my_rule1(name = 'base')
my_rule1(name = 'xxx', dep = ':base')
my_rule2(name = 'yyy', dep = ':xxx')
"#],
    );

    let analysis_result = t.update(&["//test:yyy"]).unwrap();
    let output_group_info =
        output_group_info_of_target(&only_target(analysis_result)).expect("OutputGroupInfo");
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a1_group"),
        &["test/base_a1.txt"],
    );
    assert_contains_exactly(
        &get_output_group_contents(&output_group_info, "a2_group"),
        &["test/xxx_a2.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.duplicateOutputGroupsFromTwoAspects
#[test]
fn duplicate_output_groups_from_two_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _a1_impl(target, ctx):
  a1 = ctx.actions.declare_file(target.label.name + '_a1.txt')
  ctx.actions.write(a1, 'f')
  return [OutputGroupInfo(aspect_group = depset([a1]))]

a1 = aspect(implementation=_a1_impl, attr_aspects = ['dep'])

def _a2_impl(target, ctx):
  a2 = ctx.actions.declare_file(target.label.name + '_a2.txt')
  ctx.actions.write(a2, 'f')
  return [OutputGroupInfo(aspect_group = depset([a2]))]

a2 = aspect(implementation=_a2_impl, attr_aspects = ['dep'])

def _base_impl(ctx):
  return []

base = rule(_base_impl, attrs = {})

def _top_impl(ctx):
  return [DefaultInfo(files=ctx.attr.dep[OutputGroupInfo].aspect_group)]

top = rule(_top_impl, attrs = { 'dep' : attr.label(aspects = [a1, a2]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'base', 'top')
base(name = 'base')
top(name = 'top', dep = ':base')
"#],
    );

    let analysis_result = t.update(&["//test:top"]).unwrap();
    assert_contains_exactly(
        &only_target_files_to_build_base_names(analysis_result),
        &["base_a1.txt", "base_a2.txt"],
    );
}

// Java: StarlarkDefinedAspectsTest.duplicateStarlarkProviders
#[test]
fn duplicate_starlark_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"Info = provider()
def _impl(target, ctx):
  return Info(duplicate = 'x')

MyAspect = aspect(implementation=_impl)
def _rule_impl(ctx):
  return Info(duplicate = 'y')
my_rule = rule(_rule_impl)
def _noop(ctx):
  pass
rbase = rule(_noop, attrs = { 'dep' : attr.label(aspects = [MyAspect]) })
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':aspect.bzl', 'my_rule', 'rbase')
my_rule(name = 'xxx')
rbase(name = 'yyy', dep = ':xxx')
"#],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update(&["//test:yyy"]));
    t.assert_contains_event(&t.ws("ERROR /workspace/test/BUILD:3:6: Provider Info provided twice"));
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectDoesNotExist
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn top_level_aspect_does_not_exist() {
    let mut t = new_test();
    t.scratch.file("test/aspect.bzl", &[""]);
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx')",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event("MyAspect is not exported from //test:aspect.bzl");
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectDoesNotExist2
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn top_level_aspect_does_not_exist2() {
    let mut t = new_test();
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx')",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event("cannot load '//test:aspect.bzl': no such file");
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectDoesNotExistNoBuildFile
#[test]
fn top_level_aspect_does_not_exist_no_build_file() {
    let mut t = new_test();
    t.scratch.file(
        "test/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_library')",
            "java_library(name = 'xxx')",
        ],
    );

    t.remove_fail_fast_handler();
    expect_failure(t.update_with_aspects(&["foo/aspect.bzl%MyAspect"], &["//test:xxx"]));
    t.assert_contains_event("Every .bzl file must have a corresponding package");
}

/// Tests that a loading-level error (missing bzl file) is properly transformed by the configured
/// target that requested the relevant package, and doesn't bubble up to a higher configured
/// target/aspect that wasn't expecting a loading-level error. The complication is that the
/// configured target that depends directly on the error tries to do configuration resolution
/// after noticing the error, and configuration resolution is interruptible, so it is
/// interrupted. It needs to then throw the error, rather than the interruption.
///
/// This test covers error propagation up to both the configured target that depends on the one
/// in error, as well as the aspect on that configured target, since the error goes through both.
// Java: StarlarkDefinedAspectsTest.aspectBaseConfiguredTargetTransitivelyDependingOnPackageInError
#[test]
fn aspect_base_configured_target_transitively_depending_on_package_in_error() {
    let mut t = new_test();
    // PORT: setRulesAndAspectsAvailableInTests(ImmutableList.of(), ImmutableList.of()): the
    // harness's rule class provider has no test rules or aspects to begin with.
    t.scratch.overwrite_file(
        "tools/allowlists/function_transition_allowlist/BUILD",
        &["package_group(name = 'function_transition_allowlist', packages = ['//aspect/...'])"],
    );
    t.scratch.file(
        "aspect/aspect.bzl",
        &[r#"def _setting_impl(ctx):
  return []

string_flag = rule(
  implementation = _setting_impl,
  build_setting = config.string(flag=True),
)

def _rule_impl(ctx):
  pass

def _transition_impl(settings, attr):
  return {'//aspect:formation': 'mesa'}

formation_transition = transition(
  implementation = _transition_impl,
  inputs = ['//aspect:formation'],
  outputs = ['//aspect:formation'],
)

def _aspect_impl(target, ctx):
  pass

myaspect = aspect(implementation = _aspect_impl)

cfgrule = rule(
  implementation = _rule_impl,
  attrs = {
    'to': attr.label(),
    'innocent': attr.label(cfg = formation_transition),
  }
)
"#],
    );
    t.scratch.file(
        "aspect/BUILD",
        &[r#"load('aspect.bzl', 'cfgrule', 'string_flag')
string_flag(name = 'formation', build_setting_default = 'canyon')
filegroup(name = 'innocent')
cfgrule(name = 'top', to = '//baz:baz', innocent = ':innocent')
"#],
    );
    t.scratch.file(
        "bar/BUILD",
        &[
            "load('//test_defs:foo_library.bzl', 'foo_library')",
            "foo_library(name = 'bar', deps = ['//baz:baz'])",
        ],
    );
    t.scratch.file(
        "baz/BUILD",
        &[r#"load('//test_defs:foo_library.bzl', 'foo_library')
load('//baz/subdir:missing.bzl', 'sym')
foo_library(name = 'baz')
"#],
    );
    t.scratch.file("baz/subdir/missing.bzl", &[]);
    t.remove_fail_fast_handler();
    match t.update_with_aspects(&["//aspect:aspect.bzl%myaspect"], &["//aspect:top"]) {
        Ok(result) => {
            assert!(keep_going());
            assert!(has_error(result));
        }
        Err(BuildViewError::ViewCreationFailed { .. }) => {
            // expect to fail.
            assert!(!keep_going());
        }
        Err(e) => panic!("unexpected exception: {e:?}"),
    }
    t.assert_contains_event("Label '//baz/subdir:missing.bzl' is invalid");
}

// Java: StarlarkDefinedAspectsTest.aspectParametersUncovered
#[test]
fn aspect_parameters_uncovered() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectUncovered = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['aaa']) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectUncovered]) },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx')
"#],
    );

    t.remove_fail_fast_handler();
    if let Ok(result) = t.update_with_aspects(&[], &["//test:xxx"]) {
        assert!(keep_going());
        assert!(has_error(result));
    }
    t.assert_contains_event(
        // "ERROR /workspace/test/aspect.bzl:9:11: "
        "Aspect //test:aspect.bzl%MyAspectUncovered requires rule my_rule to specify attribute \
         'my_attr' with type string.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersTypeMismatch
#[test]
fn aspect_parameters_type_mismatch() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectMismatch = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['aaa']) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectMismatch]),
              'my_attr' : attr.int() },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx', my_attr = 4)
"#],
    );

    t.remove_fail_fast_handler();
    if let Ok(result) = t.update_with_aspects(&[], &["//test:xxx"]) {
        assert!(keep_going());
        assert!(has_error(result));
    }
    t.assert_contains_event(
        "Aspect //test:aspect.bzl%MyAspectMismatch requires rule my_rule to specify attribute \
         'my_attr' with type string.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersDontSupportSelect
#[test]
fn aspect_parameters_dont_support_select() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectMismatch = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['aaa']) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectMismatch]),
              'my_attr' : attr.string() },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("//test:aspect.bzl", "my_rule")

# Needed to avoid the select() being eliminated as trivial.
config_setting(
    name = "config",
    values = {"defines": "something"},
)

my_rule(
    name = "xxx",
    my_attr = select({
        ":config": "foo",
        "//conditions:default": "bar",
    }),
)
"#],
    );

    t.remove_fail_fast_handler();
    if let Ok(result) = t.update_with_aspects(&[], &["//test:xxx"]) {
        assert!(keep_going());
        assert!(has_error(result));
    }
    t.assert_contains_event(
        "//test:xxx: attribute 'my_attr' has a select() and aspect \
         //test:aspect.bzl%MyAspectMismatch also declares '//test:xxx'. Aspect attributes \
         don't currently support select().",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersBadDefault
#[test]
fn aspect_parameters_bad_default() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectBadDefault = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['a'], default='b') },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectBadDefault]) },  # line 11
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx')
"#],
    );

    t.remove_fail_fast_handler();
    if let Ok(result) = t.update_with_aspects(&[], &["//test:xxx"]) {
        assert!(keep_going());
        assert!(has_error(result));
    }

    // aspect fails, stack = [<toplevel>@:5:28, aspect@<builtin>]
    t.assert_contains_event(
        &t.ws("File \"/workspace/test/aspect.bzl\", line 5, column 28, in <toplevel>"),
    );
    t.assert_contains_event(
        "Error in aspect: Aspect parameter attribute 'my_attr' has a bad default value: has to be \
         one of 'a' instead of 'b'",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersBadValue
#[test]
fn aspect_parameters_bad_value() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectBadValue = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['a']) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectBadValue]),
              'my_attr' : attr.string() },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx', my_attr='b')
"#],
    );

    t.remove_fail_fast_handler();
    if let Ok(result) = t.update_with_aspects(&[], &["//test:xxx"]) {
        assert!(keep_going());
        assert!(has_error(result));
    }
    t.assert_contains_event(&t.ws(
        "ERROR /workspace/test/BUILD:2:8: //test:xxx: invalid value in 'my_attr' \
         attribute: has to be one of 'a' instead of 'b'",
    ));
}

// Java: StarlarkDefinedAspectsTest.aspectParameters
#[test]
fn aspect_parameters() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspect = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['aaa']) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspect]),
              'my_attr' : attr.string() },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx', my_attr = 'aaa')
"#],
    );

    let result = t.update_with_aspects(&[], &["//test:xxx"]).unwrap();
    assert!(!has_error(result));
}

// Java: StarlarkDefinedAspectsTest.aspectParametersConfigurationField
#[test]
fn aspect_parameters_configuration_field() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspect = aspect(
    implementation=_impl,
    attrs = { '_my_attr' : attr.label(default=
             configuration_field(fragment = "coverage", name = "output_generator")) },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspect]) },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx')
"#],
    );

    let result = t.update_with_aspects(&[], &["//test:xxx"]).unwrap();
    assert!(!has_error(result));
}

// Java: StarlarkDefinedAspectsTest.aspectParameterComputedDefault
#[test]
fn aspect_parameter_computed_default() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
def _defattr():
   return Label('//foo/bar:baz')
MyAspect = aspect(
    implementation=_impl,
    attrs = { '_extra' : attr.label(default = _defattr) }
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspect]) },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx')
"#],
    );
    t.remove_fail_fast_handler();

    if keep_going() {
        let result = t.update(&["//test:xxx"]).unwrap();
        assert!(has_error(result));
    } else {
        // PORT: assertThrows(TargetParsingException.class, ...): the harness reports it as an
        // error of update().
        assert!(t.update(&["//test:xxx"]).is_err());
    }
    t.assert_contains_event(
        "Aspect attribute '_extra' (label) with computed default value is unsupported.",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectParametersOptional
#[test]
fn aspect_parameters_optional() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   return []
def _rule_impl(ctx):
   return []
MyAspectOptParam = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['aaa'], default='aaa') },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectOptParam]) },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx')
"#],
    );

    let result = t.update_with_aspects(&[], &["//test:xxx"]).unwrap();
    assert!(!has_error(result));
}

// Java: StarlarkDefinedAspectsTest.aspectParametersOptionalOverride
#[test]
fn aspect_parameters_optional_override() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   if (ctx.attr.my_attr == 'a'):
       fail('Rule is not overriding default, still has value ' + ctx.attr.my_attr)
   return []
def _rule_impl(ctx):
   return []
MyAspectOptOverride = aspect(
    implementation=_impl,
    attrs = { 'my_attr' : attr.string(values=['a', 'b'], default='a') },
)
my_rule = rule(
    implementation=_rule_impl,
    attrs = { 'deps' : attr.label_list(aspects=[MyAspectOptOverride]),
              'my_attr' : attr.string() },
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
my_rule(name = 'xxx', my_attr = 'b')
"#],
    );

    let result = t.update_with_aspects(&[], &["//test:xxx"]).unwrap();
    assert!(!has_error(result));
}

// Java: StarlarkDefinedAspectsTest.testMultipleExecutablesInTarget
#[test]
fn test_multiple_executables_in_target() {
    let mut t = new_test();
    t.scratch.file(
        "foo/extension.bzl",
        &[r#"def _aspect_impl(target, ctx):
   return []
my_aspect = aspect(_aspect_impl)
def _main_rule_impl(ctx):
   pass
my_rule = rule(_main_rule_impl,
   attrs = {
      'exe1' : attr.label(executable = True, allow_files = True, cfg = 'exec'),
      'exe2' : attr.label(executable = True, allow_files = True, cfg = 'exec'),
   },
)
"#],
    );

    t.scratch.file("foo/tool.sh", &["#!/bin/bash"]);
    t.scratch.file(
        "foo/BUILD",
        &[r#"load(':extension.bzl',  'my_rule')
my_rule(name = 'main', exe1 = ':tool.sh', exe2 = ':tool.sh')
"#],
    );
    let analysis_result_of_rule = t.update_with_aspects(&[], &["//foo:main"]).unwrap();
    assert!(!has_error(analysis_result_of_rule));

    let analysis_result_of_aspect = t
        .update_with_aspects(&["/foo/extension.bzl%my_aspect"], &["//foo:main"])
        .unwrap();
    assert!(!has_error(analysis_result_of_aspect));
}

// Java: StarlarkDefinedAspectsTest.aspectFragmentAccessSuccess
#[test]
#[ignore = "BLAZE GAP: unported Fragment Starlark API for JavaConfiguration (ctx.fragments.java: no Starlark value factory registered, blaze_analysis/src/starlark/fragment_collection.rs:241)"]
fn aspect_fragment_access_success() {
    let mut t = new_test();
    analyze_configured_target_for_aspect_fragment(
        &mut t,
        "ctx.fragments.java.strict_java_deps",
        "'java'",
        "",
    )
    .unwrap();
    t.assert_no_events();
}

// Java: StarlarkDefinedAspectsTest.aspectFragmentAccessError
#[test]
#[ignore = "BLAZE BUG: fragment error names the rule class ('my_rule has to declare') instead of RuleContext.ruleClassNameForLogging ('//test:aspect.bzl%MyAspect aspect on my_rule has to declare'); blaze_analysis/src/starlark/fragment_collection.rs:95 get_starlark_fragment uses get_rule().get_rule_class()"]
fn aspect_fragment_access_error() {
    let mut t = new_test();
    t.remove_fail_fast_handler();
    assert!(matches!(
        analyze_configured_target_for_aspect_fragment(
            &mut t,
            "ctx.fragments.java.strict_java_deps",
            "'cpp'",
            "'cpp'",
        ),
        Err(BuildViewError::ViewCreationFailed { .. })
    ));
    t.assert_contains_event(
        "//test:aspect.bzl%MyAspect aspect on my_rule has to declare 'java' as a \
         required fragment in order to access it. Please update the 'fragments' argument of \
         the rule definition (for example: fragments = [\"java\"])",
    );
}

// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:2198 StarlarkDefinedAspectsTest.analyzeConfiguredTargetForAspectFragment
fn analyze_configured_target_for_aspect_fragment(
    t: &mut AnalysisTestCase,
    full_field_name: &str,
    fragments: &str,
    rule_fragments: &str,
) -> Result<(), BuildViewError> {
    let result_line = format!("   return AspectInfo(result = str({full_field_name}))");
    let fragments_line = format!("   fragments=[{fragments}],");
    let rule_fragments_line = format!("   fragments=[{rule_fragments}],");
    t.scratch.file(
        "test/aspect.bzl",
        &[
            "AspectInfo = provider()",
            "def _aspect_impl(target, ctx):",
            &result_line,
            "",
            "RuleInfo = provider()",
            "def _rule_impl(ctx):",
            "   return RuleInfo(stuff = '...')",
            "",
            "MyAspect = aspect(",
            "   implementation=_aspect_impl,",
            "   attr_aspects=['deps'],",
            &fragments_line,
            ")",
            "my_rule = rule(",
            "   implementation=_rule_impl,",
            "   attrs = { 'attr' : ",
            "             attr.label_list(mandatory=True, allow_files=True, aspects = [MyAspect]) },",
            &rule_fragments_line,
            ")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load('//test:aspect.bzl', 'my_rule')
exports_files(['zzz'])
my_rule(
     name = 'yyy',
     attr = ['zzz'],
)
my_rule(
     name = 'xxx',
     attr = ['yyy'],
)
"#],
    );

    let result = t.update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:xxx"])?;
    if has_error(result) {
        assert!(keep_going());
        let error_message = "Analysis failed";
        return Err(BuildViewError::ViewCreationFailed {
            message: error_message.to_string(),
            failure_detail: Default::default(),
        });
    }

    assert!(t.get_configured_target("//test:xxx").is_some());
    Ok(())
}

// Java: StarlarkDefinedAspectsTest.invalidateAspectOnBzlFileChange
#[test]
fn invalidate_aspect_on_bzl_file_change() {
    let mut t = new_test();
    let file = aspect_bzl_file("'deps'");
    t.scratch.file(
        "test/build_defs.bzl",
        &file.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load(':build_defs.bzl', 'repro', 'repro_no_aspect')
repro_no_aspect(name = 'r0')
repro_no_aspect(name = 'r1', deps = [':r0'])
repro(name = 'r2', deps = [':r1'])
"#],
    );
    build_target_and_check_rule_info(&mut t, &["@@//test:r0", "@@//test:r1"]);

    // Make aspect propagation list empty.
    let file = aspect_bzl_file("");
    t.scratch.overwrite_file(
        "test/build_defs.bzl",
        &file.iter().map(String::as_str).collect::<Vec<_>>(),
    );

    // The aspect should not propagate to //test:r0 anymore.
    build_target_and_check_rule_info(&mut t, &["@@//test:r1"]);
}

// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:2271 StarlarkDefinedAspectsTest.buildTargetAndCheckRuleInfo
fn build_target_and_check_rule_info(t: &mut AnalysisTestCase, expected_labels: &[&str]) {
    let configured_target = t
        .update_with_aspects(&[], &["//test:r2"])
        .unwrap()
        .get_targets_to_build()[0]
        .clone();
    let info = get_starlark_provider(t, &configured_target, "RuleInfo").expect("RuleInfo");
    let rule_info_value = with_field(&info, "rule_info", |v| depset_elements(v, string_of));
    assert_contains_exactly(&rule_info_value, expected_labels);
}

// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:2280 StarlarkDefinedAspectsTest.aspectBzlFile
fn aspect_bzl_file(attr_aspects: &str) -> Vec<String> {
    [
        "AspectInfo = provider()",
        "def _repro_aspect_impl(target, ctx):",
        "    s = depset([str(target.label)], transitive =",
        "      [d[AspectInfo].aspect_info for d in ctx.rule.attr.deps if AspectInfo in d])",
        "    return AspectInfo(aspect_info = s)",
        "",
        "_repro_aspect = aspect(",
        "    _repro_aspect_impl,",
        &format!("    attr_aspects = [{attr_aspects}],"),
        ")",
        "",
        "RuleInfo = provider()",
        "def repro_impl(ctx):",
        "    s = depset(transitive = ",
        "      [d[AspectInfo].aspect_info for d in ctx.attr.deps if AspectInfo in d])",
        "    return RuleInfo(rule_info = s)",
        "",
        "def repro_no_aspect_impl(ctx):",
        "    pass",
        "",
        "repro_no_aspect = rule(implementation = repro_no_aspect_impl,",
        "             attrs = {",
        "                       'deps': attr.label_list(",
        "                             allow_files = True,",
        "                       )",
        "                      },",
        ")",
        "",
        "repro = rule(implementation = repro_impl,",
        "             attrs = {",
        "                       'deps': attr.label_list(",
        "                             allow_files = True,",
        "                             aspects = [_repro_aspect],",
        "                       )",
        "                      },",
        ")",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// Java: StarlarkDefinedAspectsTest.aspectOutputsToBinDirectory
#[test]
fn aspect_outputs_to_bin_directory() {
    let mut t = new_test();
    t.scratch.file(
        "foo/extension.bzl",
        &[r#"AspectInfo = provider()
def _aspect_impl(target, ctx):
   file = ctx.actions.declare_file('aspect-output-' + target.label.name)
   ctx.actions.write(file, 'data')
   return AspectInfo(aspect_file = file)
my_aspect = aspect(_aspect_impl)
def _rule_impl(ctx):
   pass
rule_bin_out = rule(_rule_impl, output_to_genfiles=False)
rule_gen_out = rule(_rule_impl, output_to_genfiles=True)
RuleInfo = provider()
def _main_rule_impl(ctx):
   s = depset([d[AspectInfo].aspect_file for d in ctx.attr.deps])
   return RuleInfo(aspect_files = s)
main_rule = rule(_main_rule_impl,
   attrs = { 'deps' : attr.label_list(aspects = [my_aspect]) },
)
"#],
    );

    t.scratch.file(
        "foo/BUILD",
        &[
            r#"load(':extension.bzl', 'rule_bin_out', 'rule_gen_out', 'main_rule')
rule_bin_out(name = 'rbin')
rule_gen_out(name = 'rgen')
main_rule(name = 'main', deps = [':rbin', ':rgen'])
"#,
        ],
    );
    let target = t
        .update_with_aspects(&[], &["//foo:main"])
        .unwrap()
        .get_targets_to_build()[0]
        .clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo").expect("RuleInfo");
    let aspect_files = with_field(&info, "aspect_files", |v| depset_elements(v, artifact_of));
    assert_contains_exactly(
        &aspect_files
            .iter()
            .map(|a| a.get_filename().to_string())
            .collect::<Vec<_>>(),
        &["aspect-output-rbin", "aspect-output-rgen"],
    );
    for aspect_file in &aspect_files {
        let root_path = aspect_file
            .get_root()
            .get_exec_path()
            .get_path_string()
            .to_string();
        assert!(
            !root_path.contains("genfiles"),
            "Artifact {aspect_file:?} should not be in genfiles"
        );
        assert!(
            root_path.ends_with("bin"),
            "Artifact {aspect_file:?} should be in bin"
        );
    }
}

// Java: StarlarkDefinedAspectsTest.toplevelAspectOnFile
#[test]
fn toplevel_aspect_on_file() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r#"def _impl(target, ctx):
   print('This aspect does nothing')
   return []
MyAspect = aspect(implementation=_impl)
"#],
    );
    t.scratch
        .file("test/BUILD", &["exports_files(['file.txt'])"]);
    t.scratch.file("test/file.txt", &[""]);
    let analysis_result = t
        .update_with_aspects(&["test/aspect.bzl%MyAspect"], &["//test:file.txt"])
        .unwrap();
    assert!(!has_error(analysis_result));
    assert_eq!(
        only_aspect(analysis_result)
            .get_providers()
            .get_provider_count(),
        0
    );
}
