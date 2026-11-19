//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java`
//! (an `AnalysisTestCase`), part 2 of 3: the `@Test` methods from
//! `sharedAttributeDefinitionWithAspects` through
//! `testTopLevelAspectRequiresAspect_stackOfRequiredAspects`, in Java's order, with the private
//! helpers of the Java class they use (each part keeps its own copy).
//!
//! PORT: Java runs every test twice, as `StarlarkDefinedAspectsTest` (`keepGoing()` false) and
//! as its subclass `WithKeepGoing` (`Flag.KEEP_GOING`). This file is the base class:
//! [`keep_going`] is false and the keep-going branches are kept as written in Java.
//!
//! PORT: Java's `TestRuleClassProvider` adds the native test aspects and rules of `TestAspects`
//! (`exposeNativeAspectToStarlark`); the harness uses Bazel's rule class provider, so the tests
//! that need them are ignored.

use std::fmt::Debug;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::starlark::starlark_action::StarlarkAction;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::environment::Module;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};
use std::sync::Arc;

use crate::build_view::AnalysisResult;
use crate::testing::build_view_test_case::parse_label;
use crate::testing::{AnalysisTestCase, BuildViewTestCase, has_error};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

/// `StarlarkDefinedAspectsTest.keepGoing()` (the base class; `WithKeepGoing` returns true).
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:88 StarlarkDefinedAspectsTest.keepGoing
fn keep_going() -> bool {
    false
}

/// `AnalysisTestCase.update(labels...)`: Java throws on a failed analysis, which fails the test.
fn update(t: &mut AnalysisTestCase, labels: &[&str]) -> AnalysisSnapshot {
    update_with_aspects(t, &[], labels)
}

/// `AnalysisTestCase.update(aspects, labels...)`.
fn update_with_aspects(
    t: &mut AnalysisTestCase,
    aspects: &[&str],
    labels: &[&str],
) -> AnalysisSnapshot {
    let result = t
        .update_with_aspects(aspects, labels)
        .unwrap_or_else(|e| panic!("update failed: {e:?}"));
    AnalysisSnapshot::of(result)
}

/// What the tests read from an `AnalysisResult`, copied out so that the harness can be used
/// again while the test inspects it.
struct AnalysisSnapshot {
    has_error: bool,
    targets_to_build: Vec<ConfiguredTarget>,
    aspects_map: IndexMap<AspectKey, Arc<AspectValue>>,
}

impl AnalysisSnapshot {
    fn of(result: &AnalysisResult) -> AnalysisSnapshot {
        AnalysisSnapshot {
            has_error: has_error(result),
            targets_to_build: result.get_targets_to_build().to_vec(),
            aspects_map: result.get_aspects_map().clone(),
        }
    }

    /// `AnalysisResult.getAspectsMap()` (Java's values are the `ConfiguredAspect`s).
    fn get_aspects_map(&self) -> Vec<(AspectKey, ConfiguredAspect)> {
        self.aspects_map
            .iter()
            .map(|(k, v)| (k.clone(), v.get_configured_object()))
            .collect()
    }

    /// `Iterables.getOnlyElement(analysisResult.getTargetsToBuild())`.
    fn only_target_to_build(&self) -> ConfiguredTarget {
        assert_eq!(
            self.targets_to_build.len(),
            1,
            "expected one target to build, got {:?}",
            labels_of(&self.targets_to_build)
        );
        self.targets_to_build[0].clone()
    }

    /// `Iterables.getOnlyElement(analysisResult.getAspectsMap().values())`.
    fn only_aspect(&self) -> ConfiguredAspect {
        let aspects = self.get_aspects_map();
        assert_eq!(
            aspects.len(),
            1,
            "expected one aspect, got {:?}",
            aspects.iter().map(|(k, _)| k).collect::<Vec<_>>()
        );
        aspects.into_iter().next().unwrap().1
    }

    /// `analysisResult.getAspectsMap().values().iterator().next()`.
    fn first_aspect(&self) -> ConfiguredAspect {
        self.get_aspects_map()
            .into_iter()
            .next()
            .expect("NoSuchElementException: no aspects")
            .1
    }
}

fn labels_of(targets: &[ConfiguredTarget]) -> Vec<String> {
    targets
        .iter()
        .map(|ct| ct.get_label().get_canonical_form())
        .collect()
}

/// `StarlarkDefinedAspectsTest.getAspectDescriptions`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:260 StarlarkDefinedAspectsTest.getAspectDescriptions
#[allow(dead_code)]
fn get_aspect_descriptions(analysis_result: &AnalysisSnapshot) -> Vec<String> {
    analysis_result
        .aspects_map
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

/// `StarlarkDefinedAspectsTest.getLabelsToBuild`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:318 StarlarkDefinedAspectsTest.getLabelsToBuild
fn get_labels_to_build(analysis_result: &AnalysisSnapshot) -> Vec<String> {
    labels_of(&analysis_result.targets_to_build)
}

/// `StarlarkDefinedAspectsTest.getConfiguredAspect(aspectsMap, aspectName)`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10249 StarlarkDefinedAspectsTest.getConfiguredAspect
fn get_configured_aspect(
    aspects_map: &[(AspectKey, ConfiguredAspect)],
    aspect_name: &str,
) -> Option<ConfiguredAspect> {
    for (key, value) in aspects_map {
        if let Some(starlark_aspect_class) = key.get_aspect_class().as_starlark() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            if aspect_exported_name == aspect_name {
                return Some(value.clone());
            }
        }
    }
    None
}

/// `StarlarkDefinedAspectsTest.getConfiguredAspect(aspectsMap, aspectName, targetName)`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10263 StarlarkDefinedAspectsTest.getConfiguredAspect
fn get_configured_aspect_on(
    aspects_map: &[(AspectKey, ConfiguredAspect)],
    aspect_name: &str,
    target_name: &str,
) -> Option<ConfiguredAspect> {
    for (key, value) in aspects_map {
        if let Some(starlark_aspect_class) = key.get_aspect_class().as_starlark() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            let target = key.get_label().get_name();
            if aspect_exported_name == aspect_name && target == target_name {
                return Some(value.clone());
            }
        }
    }
    None
}

/// `new StarlarkProvider.Key(keyForBuild(Label.parseCanonical(bzl)), symbol)`.
fn starlark_key(bzl_label: &str, symbol: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: parse_label(bzl_label),
        exported_name: symbol.to_string(),
    }
}

/// `AnalysisTestCase.getStarlarkProvider(target, providerSymbol)`: the provider defined in the
/// .bzl file of the target's rule class.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:702 AnalysisTestCase.getStarlarkProvider
fn get_starlark_provider(
    t: &mut AnalysisTestCase,
    target: &ConfiguredTarget,
    provider_symbol: &str,
) -> OwnedFrozenValue {
    let rule_target = t
        .get_target(&target.get_label().get_canonical_form())
        .unwrap_or_else(|e| panic!("{e:?}"));
    let bzl = rule_target
        .get_associated_rule()
        .expect("getAssociatedRule")
        .get_rule_class_object()
        .get_rule_definition_environment_label()
        .expect("getRuleDefinitionEnvironmentLabel")
        .get_canonical_form();
    target
        .get(&starlark_key(&bzl, provider_symbol))
        .unwrap_or_else(|| panic!("no provider {provider_symbol} on {}", target.get_label()))
}

/// A Starlark value as the Java assertions compare it (`String`, `Label`, `StarlarkInt`, ...).
#[derive(Debug, Clone, PartialEq)]
enum J {
    Str(String),
    Label(Label),
    Int(i64),
    Bool(bool),
    None,
    /// Any other value: its type and `repr`.
    Other(String),
}

fn to_j(v: Value) -> J {
    if v.is_none() {
        return J::None;
    }
    if let Some(s) = v.unpack_str() {
        return J::Str(s.to_string());
    }
    if let Some(l) = v.downcast_ref::<LabelValue>() {
        return J::Label(l.0.clone());
    }
    if let Some(b) = v.unpack_bool() {
        return J::Bool(b);
    }
    if let Some(i) = v.unpack_i32() {
        return J::Int(i64::from(i));
    }
    J::Other(format!("{}: {}", v.get_type(), v.to_repr()))
}

fn s(x: &str) -> J {
    J::Str(x.to_string())
}

fn l(x: &str) -> J {
    J::Label(parse_label(x))
}

/// Runs `f` on the field `name` of the provider instance `info` (`StructImpl.getValue(name)`,
/// `None` for a missing field).
fn with_value<R>(
    info: &OwnedFrozenValue,
    name: &str,
    f: impl for<'v> FnOnce(Option<Value<'v>>) -> R,
) -> R {
    Module::with_temp_heap(|module| {
        // SAFETY: `info` keeps its frozen heap alive for the whole closure.
        let v = unsafe { info.unchecked_frozen_value() }
            .to_value()
            .get_attr(name, module.heap())
            .unwrap_or_else(|e| panic!("getValue({name}): {e}"));
        f(v)
    })
}

/// `info.getValue(name)`.
fn get_value(info: &OwnedFrozenValue, name: &str) -> Option<J> {
    with_value(info, name, |v| v.map(to_j))
}

/// `(Sequence<?>) info.getValue(name)`.
fn get_sequence(info: &OwnedFrozenValue, name: &str) -> Vec<J> {
    Module::with_temp_heap(|module| {
        // SAFETY: `info` keeps its frozen heap alive for the whole closure.
        let v = unsafe { info.unchecked_frozen_value() }
            .to_value()
            .get_attr(name, module.heap())
            .unwrap_or_else(|e| panic!("getValue({name}): {e}"))
            .unwrap_or_else(|| panic!("NullPointerException: no field {name}"));
        v.iterate(module.heap())
            .unwrap_or_else(|e| panic!("ClassCastException: {name} is not a Sequence: {e}"))
            .map(to_j)
            .collect()
    })
}

/// `((Depset) info.getValue(name)).toList()`.
fn get_depset_list(info: &OwnedFrozenValue, name: &str) -> Vec<J> {
    with_value(info, name, |v| {
        let v = v.unwrap_or_else(|| panic!("NullPointerException: no field {name}"));
        Depset::from_value(v)
            .unwrap_or_else(|| panic!("ClassCastException: {name} is not a Depset"))
            .to_list()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(to_j)
            .collect()
    })
}

/// `info.getValue(name) instanceof StarlarkList`.
fn is_starlark_list(info: &OwnedFrozenValue, name: &str) -> bool {
    with_value(info, name, |v| {
        v.is_some_and(|v| ListRef::from_value(v).is_some())
    })
}

/// Truth's `assertThat(actual).containsExactly(expected...)` (any order, same multiplicity).
fn assert_contains_exactly<T: PartialEq + Debug>(actual: &[T], expected: &[T]) {
    let mut remaining: Vec<&T> = actual.iter().collect();
    let mut missing = Vec::new();
    for e in expected {
        match remaining.iter().position(|a| *a == e) {
            Some(i) => {
                remaining.remove(i);
            }
            None => missing.push(e),
        }
    }
    assert!(
        missing.is_empty() && remaining.is_empty(),
        "containsExactly failed\n  actual:   {actual:?}\n  expected: {expected:?}\n  missing:  {missing:?}\n  unexpected: {remaining:?}"
    );
}

/// `assertThat(actual).containsExactly(expected...).inOrder()`.
fn assert_contains_exactly_in_order<T: PartialEq + Debug>(actual: &[T], expected: &[T]) {
    assert_eq!(actual, expected, "containsExactly(...).inOrder()");
}

/// Java's
/// `if (keepGoing()) { assertThat(update(aspects, labels).hasError()).isTrue(); } else {
/// assertThrows(<exception>.class, () -> update(aspects, labels)); }`, and the equivalent
/// `try { update(...); assertThat(keepGoing()).isTrue(); assertThat(hasError).isTrue(); }
/// catch (<exception> e) {}`.
///
/// PORT: the harness reports Java's `TargetParsingException` and `ViewCreationFailedException`
/// alike as an `Err` of `update`.
fn update_expecting_failure(
    t: &mut AnalysisTestCase,
    aspects: &[&str],
    labels: &[&str],
    java_exception: &str,
) {
    let result = t.update_with_aspects(aspects, labels);
    if keep_going() {
        let result = result.unwrap_or_else(|e| panic!("update failed with keep_going: {e:?}"));
        assert!(has_error(result), "expected an analysis error");
    } else {
        assert!(
            result.is_err(),
            "expected {java_exception} from update({aspects:?}, {labels:?})"
        );
    }
}

/// `StarlarkDefinedAspectsTest.setupAspectOnAspectTargetGraph`.
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:3632 StarlarkDefinedAspectsTest.setupAspectOnAspectTargetGraph
fn setup_aspect_on_aspect_target_graph(
    t: &mut AnalysisTestCase,
    apply_root_to_generating_rules: bool,
    apply_dep_to_generating_rules: bool,
) {
    // RootAspectInfo.both_labels returns a list of target labels which
    //     were evaluated as [root_aspect(dep_aspect(target))].
    // RootAspectInfo.root_only_labels returns a list of target labels which
    //     were evaluated as [root_aspect(target)].
    // DepAspectInfo.labels returns a list of target labels which were evaluated by dep_aspect.
    let py_bool = |b: bool| if b { "True" } else { "False" };
    let root_apply = format!(
        "  apply_to_generating_rules = {})",
        py_bool(apply_root_to_generating_rules)
    );
    let dep_apply = format!(
        "  apply_to_generating_rules = {})",
        py_bool(apply_dep_to_generating_rules)
    );
    t.scratch.file(
        "test/lib.bzl",
        &[
            "RootAspectInfo = provider()",
            "DepAspectInfo = provider()",
            "def _root_aspect_impl(target, ctx):",
            "  both_labels = []",
            "  root_only_labels = []",
            "  for dep in ctx.rule.attr.deps:",
            "    if RootAspectInfo in dep:",
            "      both_labels += dep[RootAspectInfo].both_labels",
            "      root_only_labels += dep[RootAspectInfo].root_only_labels",
            "      if DepAspectInfo in dep:",
            "        both_labels += [dep.label]",
            "      else:",
            "        root_only_labels += [dep.label]",
            "  return RootAspectInfo(both_labels = both_labels, root_only_labels = root_only_labels)",
            "",
            "def _dep_aspect_impl(target, ctx):",
            "  dep_labels = [ctx.label]",
            "  for dep in ctx.rule.attr.deps:",
            "    if DepAspectInfo in dep:",
            "      dep_labels += dep[DepAspectInfo].labels",
            "  return DepAspectInfo(labels = dep_labels)",
            "",
            "def _root_rule_impl(ctx):",
            "  return [ctx.attr.deps[0][RootAspectInfo], ctx.attr.deps[0][DepAspectInfo]]",
            "",
            "def _dep_with_aspect_rule_impl(ctx):",
            "  return [ctx.attr.deps[0][DepAspectInfo]]",
            "",
            "def _dep_rule_impl(ctx):",
            "  if ctx.outputs.output:",
            "    ctx.actions.run_shell(outputs = [ctx.outputs.output], command = 'dont run me')",
            "  return []",
            "",
            "root_aspect = aspect(",
            "  implementation = _root_aspect_impl,",
            "  attr_aspects = ['deps'],",
            "  required_aspect_providers = [DepAspectInfo],",
            &root_apply,
            "",
            "dep_aspect = aspect(",
            "  implementation = _dep_aspect_impl,",
            "  attr_aspects = ['deps'],",
            "  provides = [DepAspectInfo],",
            &dep_apply,
            "",
            "root_rule = rule(implementation = _root_rule_impl,",
            "  attrs = {'deps' : attr.label_list(aspects = [root_aspect])})",
            "",
            "dep_with_aspect_rule = rule(implementation = _dep_with_aspect_rule_impl,",
            "  attrs = {'deps' : attr.label_list(aspects = [dep_aspect])})",
            "",
            "dep_rule = rule(implementation = _dep_rule_impl,",
            "  attrs = {'deps' : attr.label_list(allow_files = True), 'output' : attr.output()})",
        ],
    );

    // Target graph: test -> aspect_propagating_target -> alpha -> beta_output
    // beta_output is an output of target `beta`
    // beta -> charlie
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:lib.bzl', 'root_rule', 'dep_with_aspect_rule', 'dep_rule')

root_rule(name = 'test', deps = [':aspect_propagating_target'])
dep_with_aspect_rule(name = 'aspect_propagating_target', deps = [':alpha'])
dep_rule(name = 'alpha', deps = [':beta_output'])
dep_rule(name = 'beta', deps = [':charlie'], output = 'beta_output')
dep_rule(name = 'charlie')
",
        ],
    );
}

/// `StarlarkDefinedAspectsTest.exposeNativeAspectToStarlark`: a rule class provider with
/// `TestAspects.STARLARK_NATIVE_ASPECT_WITH_PROVIDER`,
/// `PARAMETRIZED_STARLARK_NATIVE_ASPECT_WITH_PROVIDER`, `BASE_RULE` and `HONEST_RULE`.
///
/// PORT: `TestRuleClassProvider` and `TestAspects` are not ported, and the harness has no
/// `useRuleClassProvider` (module documentation).
// Java: com/google/devtools/build/lib/starlark/StarlarkDefinedAspectsTest.java:10278 StarlarkDefinedAspectsTest.exposeNativeAspectToStarlark
fn expose_native_aspect_to_starlark() {
    blaze_util::unported!("StarlarkDefinedAspectsTest.exposeNativeAspectToStarlark");
}

/// Java's recurring
/// `ConfiguredAspect aN = getConfiguredAspect(configuredAspects, name);
/// assertThat(aN).isNotNull(); StarlarkProvider.Key k = new StarlarkProvider.Key(
/// keyForBuild(Label.parseCanonical("//test:defs.bzl")), symbol);
/// (Sequence<?>) ((StructImpl) aN.get(k)).getValue(field)`.
fn aspect_result_sequence(
    configured_aspects: &[(AspectKey, ConfiguredAspect)],
    aspect_name: &str,
    provider_symbol: &str,
    field: &str,
) -> Vec<J> {
    let aspect = get_configured_aspect(configured_aspects, aspect_name)
        .unwrap_or_else(|| panic!("no configured aspect {aspect_name}"));
    let provider = aspect
        .get(&starlark_key("//test:defs.bzl", provider_symbol))
        .unwrap_or_else(|| panic!("no provider {provider_symbol} on {aspect_name}"));
    get_sequence(&provider, field)
}

// Java: StarlarkDefinedAspectsTest.sharedAttributeDefinitionWithAspects
#[test]
fn shared_attribute_definition_with_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _aspect_impl(target,ctx):
  return []
my_aspect = aspect(implementation = _aspect_impl)
_ATTR = { 'deps' : attr.label_list(aspects = [my_aspect]) }
def _dummy_impl(ctx):
  pass
r1 = rule(_dummy_impl, attrs =  _ATTR)
r2 = rule(_dummy_impl, attrs =  _ATTR)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2')
r1(name = 't1')
r2(name = 't2', deps = [':t1'])
"],
    );
    let analysis_result = update(&mut t, &["//test:t2"]);
    assert!(!analysis_result.has_error);
}

// Java: StarlarkDefinedAspectsTest.multipleAspects
#[test]
fn multiple_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _aspect_impl(target,ctx):
  return []
my_aspect = aspect(implementation = _aspect_impl)
def _dummy_impl(ctx):
  pass
r1 = rule(_dummy_impl,
          attrs = { 'deps' : attr.label_list(aspects = [my_aspect, my_aspect]) })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1')
r1(name = 't1')
"],
    );
    t.remove_fail_fast_handler();
    // This call succeeds if "--keep_going" was passed, which it does in the WithKeepGoing test
    // suite. Otherwise, it fails and throws a TargetParsingException.
    update_expecting_failure(&mut t, &[], &["//test:r1"], "TargetParsingException");
    t.assert_contains_event("aspect //test:aspect.bzl%my_aspect added more than once");
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectsAndExtraActions
#[test]
#[ignore = "BLAZE GAP: unported!(\"RuleConfiguredTargetBuilder.createExtraActionProvider\") (blaze_analysis rule_configured_target_builder.rs)"]
fn top_level_aspects_and_extra_actions() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _aspect_impl(target,ctx):
  f = ctx.actions.declare_file('dummy.txt')
  ctx.actions.run_shell(outputs = [f], command='echo xxx > $(location f)',
                        mnemonic='AspectAction')
  return []
my_aspect = aspect(implementation = _aspect_impl)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
extra_action(
    name = 'xa',
    cmd = 'echo $(EXTRA_ACTION_FILE) > $(output file.xa)',
    out_templates = ['file.xa'],
)
action_listener(
    name = 'al',
    mnemonics = [ 'AspectAction' ],
    extra_actions = [ ':xa' ])
java_library(name = 'xxx')
"#],
    );
    t.use_configuration(&["--experimental_action_listener=//test:al"]);
    let _analysis_result =
        update_with_aspects(&mut t, &["test/aspect.bzl%my_aspect"], &["//test:xxx"]);
    // Java: assertThat(Iterables.transform(analysisResult.getArtifactsToBuild(),
    // Artifact::getFilename)).contains("file.xa");
    // PORT: AnalysisResult.getArtifactsToBuild is not ported.
    blaze_util::unported!("AnalysisResult.getArtifactsToBuild");
}

// Java: StarlarkDefinedAspectsTest.topLevelAspectsAndExtraActionsWithConflict
#[test]
#[ignore = "BLAZE GAP: unported!(\"RuleConfiguredTargetBuilder.createExtraActionProvider\") (blaze_analysis rule_configured_target_builder.rs)"]
fn top_level_aspects_and_extra_actions_with_conflict() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _aspect_impl(target, ctx):
  f = ctx.actions.declare_file('dummy.txt')
  ctx.actions.run_shell(outputs = [f], command='echo xxx > $(location f)',
                        mnemonic='AspectAction')
  return []
my_aspect = aspect(implementation = _aspect_impl)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
extra_action(
    name = 'xa',
    cmd = 'echo $(EXTRA_ACTION_FILE) > $(output file.xa)',
    out_templates = ['file.xa'],
)
action_listener(
    name = 'al',
    mnemonics = ['AspectAction'],
    extra_actions = [':xa'],
)
java_library(name = 'xxx')
java_library(name = 'yyy')
"#],
    );
    t.use_configuration(&["--experimental_action_listener=//test:al"]);
    t.remove_fail_fast_handler(); // We expect an error.

    update_expecting_failure(
        &mut t,
        &["test/aspect.bzl%my_aspect"],
        &["//test:xxx", "//test:yyy"],
        "ViewCreationFailedException",
    );
    t.assert_contains_event(
        "file 'extra_actions/test/xa/test/file.xa' is generated by these conflicting actions",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectsPropagatingToAllAttributes
#[test]
fn aspects_propagating_to_all_attributes() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"MyInfo = provider()
def _impl(target, ctx):
   s = depset([target.label], transitive =
     [i[MyInfo].target_labels for i in ctx.rule.attr.runtime_deps]
     if hasattr(ctx.rule.attr, 'runtime_deps') else [])
   return MyInfo(target_labels = s)

MyAspect = aspect(
    implementation=_impl,
    attrs = { '_tool' : attr.label(default = Label('//test:tool')) },
    attr_aspects=['*'],
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
    name = 'tool',
)
java_library(
     name = 'bar',
     runtime_deps = [':tool'],
)
java_library(
     name = 'foo',
     runtime_deps = [':bar'],
)
"#],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["test/aspect.bzl%MyAspect"], &["//test:foo"]);
    let configured_aspect = analysis_result.first_aspect();
    let names = configured_aspect
        .get(&starlark_key("//test:aspect.bzl", "MyInfo"))
        .expect("MyInfo");
    let names: Vec<String> = get_depset_list(&names, "target_labels")
        .into_iter()
        .map(|o| match o {
            J::Label(label) => label.get_name().to_string(),
            other => panic!("expected a Label, got {other:?}"),
        })
        .collect();
    assert_contains_exactly(
        &names,
        &["foo".to_string(), "bar".to_string(), "tool".to_string()],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectLinear
#[test]
fn aspect_on_aspect_linear() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  return a1p(text = 'random')
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
  value = []
  if  ctx.rule.attr.dep and a2p in ctx.rule.attr.dep:
     value += ctx.rule.attr.dep[a2p].value
  if a1p in target:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=yes')
  else:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=no')
  return a2p(value = value)
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [a1p])
def _r1_impl(ctx):
  pass
r2info = provider()
def _r2_impl(ctx):
  return r2info(result = ctx.attr.dep[a2p].value)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2')
r1(name = 'r0')
r1(name = 'r1', dep = ':r0')
r2(name = 'r2', dep = ':r1')
"],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let result = get_sequence(&get_starlark_provider(&mut t, &target, "r2info"), "result");

    // "yes" means that aspect a2 sees a1's providers.
    assert_contains_exactly(
        &result,
        &[
            s("@@//test:r0[\"//test:aspect.bzl%a1\", \"//test:aspect.bzl%a2\"]=yes"),
            s("@@//test:r1[\"//test:aspect.bzl%a2\"]=no"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectDiamond
#[test]
fn aspect_on_aspect_diamond() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
a2p = provider()
a3p = provider()
def _a1_impl(target,ctx):
  return [a1p(value = 'text from a1')]
a1 = aspect(_a1_impl, attr_aspects = ['deps'], provides = [a1p])

def _a2_impl(target,ctx):
  return [a2p(value = 'text from a2')]
a2 = aspect(_a2_impl, attr_aspects = ['deps'], provides = [a2p])

def _a3_impl(target,ctx):
  value = []
  f = ctx.actions.declare_file('a3.out')
  ctx.actions.write(f, 'text')
  for dep in ctx.rule.attr.deps:
     if a3p in dep:
         value += dep[a3p].value
  s = str(target.label) + str(ctx.aspect_ids) + '='
  if a1p in target:
     s += 'a1p'
  if a2p in target:
     s += 'a2p'
  value.append(s)
  return [a3p(value = value)]
a3 = aspect(_a3_impl, attr_aspects = ['deps'],
            required_aspect_providers = [[a1p], [a2p]])
def _r1_impl(ctx):
  pass
RCollectInfo = provider()
def _rcollect_impl(ctx):
  value = []
  for dep in ctx.attr.deps:
     if a3p in dep:
         value += dep[a3p].value
  return RCollectInfo(result = value)
r1 = rule(_r1_impl, attrs = { 'deps' : attr.label_list(aspects = [a1])})
r2 = rule(_r1_impl, attrs = { 'deps' : attr.label_list(aspects = [a2])})
rcollect = rule(_rcollect_impl, attrs = { 'deps' : attr.label_list(aspects = [a3])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2', 'rcollect')
r1(name = 'r0')
r1(name = 'r1', deps = [':r0'])
r2(name = 'r2', deps = [':r0'])
rcollect(name = 'rcollect', deps = [':r1', ':r2'])
"],
    );
    let analysis_result = update(&mut t, &["//test:rcollect"]);
    let target = analysis_result.only_target_to_build();
    let result = get_sequence(
        &get_starlark_provider(&mut t, &target, "RCollectInfo"),
        "result",
    );
    assert_contains_exactly(
        &result,
        &[
            s("@@//test:r0[\"//test:aspect.bzl%a1\", \"//test:aspect.bzl%a3\"]=a1p"),
            s("@@//test:r1[\"//test:aspect.bzl%a3\"]="),
            s("@@//test:r0[\"//test:aspect.bzl%a2\", \"//test:aspect.bzl%a3\"]=a2p"),
            s("@@//test:r2[\"//test:aspect.bzl%a3\"]="),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectLinearDuplicates
#[test]
fn aspect_on_aspect_linear_duplicates() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  return a1p()
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
  value = []
  if ctx.rule.attr.dep and a2p in ctx.rule.attr.dep:
     value += ctx.rule.attr.dep[a2p].value
  if a1p in target:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=yes')
  else:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=no')
  return a2p(value = value)
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [])
def _r1_impl(ctx):
  pass
r2info = provider()
def _r2_impl(ctx):
  return r2info(result = ctx.attr.dep[a2p].value)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2')
r1(name = 'r0')
r2(name = 'r2_1', dep = ':r0')
r1(name = 'r1', dep = ':r2_1')
r2(name = 'r2', dep = ':r1')
"],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let result = get_sequence(&get_starlark_provider(&mut t, &target, "r2info"), "result");
    // "yes" means that aspect a2 sees a1's providers.
    assert_contains_exactly(
        &result,
        &[
            s("@@//test:r0[\"//test:aspect.bzl%a2\"]=no"),
            s("@@//test:r1[\"//test:aspect.bzl%a2\"]=no"),
            s("@@//test:r2_1[\"//test:aspect.bzl%a2\"]=no"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectLinearAlias
#[test]
fn aspect_on_aspect_linear_alias() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  return [a1p(text = 'random')]
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
  value = []
  if ctx.rule.attr.dep and a2p in ctx.rule.attr.dep:
     value += ctx.rule.attr.dep[a2p].value
  if a1p in target:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=yes')
  else:
     value.append(str(target.label) + str(ctx.aspect_ids) + '=no')
  return [a2p(value = value)]
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [a1p])
def _r1_impl(ctx):
  pass
r2info = provider()
def _r2_impl(ctx):
  return r2info(result = ctx.attr.dep[a2p].value)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2')
r1(name = 'r0')
alias(name = 'a0', actual = ':r0')
r1(name = 'r1', dep = ':a0')
r2(name = 'r2', dep = ':r1')
"],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let result = get_sequence(&get_starlark_provider(&mut t, &target, "r2info"), "result");

    // "yes" means that aspect a2 sees a1's providers.
    assert_contains_exactly(
        &result,
        &[
            s("@@//test:r0[\"//test:aspect.bzl%a1\", \"//test:aspect.bzl%a2\"]=yes"),
            s("@@//test:r1[\"//test:aspect.bzl%a2\"]=no"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectDescriptions
#[test]
fn aspect_descriptions() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"AspectInfo = provider()
def _a_impl(target,ctx):
  s = str(target.label) + str(ctx.aspect_ids) + '='
  value = []
  if ctx.rule.attr.dep:
     d = ctx.rule.attr.dep
     this_id = ctx.aspect_ids[len(ctx.aspect_ids) - 1]
     s += str(d.label) + str(d[AspectInfo].my_ids) + ',' + str(this_id in d[AspectInfo].my_ids)
     value += ctx.rule.attr.dep[AspectInfo].ap
  else:
     s += 'None'
  value.append(s)
  return AspectInfo(ap = value, my_ids = ctx.aspect_ids)
a = aspect(_a_impl, attr_aspects = ['dep'])
RuleInfo = provider()
def _r_impl(ctx):
  if not ctx.attr.dep:
     return RuleInfo(result = [])
  return RuleInfo(result = ctx.attr.dep[AspectInfo].ap)
r = rule(_r_impl, attrs = { 'dep' : attr.label(aspects = [a])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r')
r(name = 'r0')
r(name = 'r1', dep = ':r0')
r(name = 'r2', dep = ':r1')
"],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let result = get_sequence(
        &get_starlark_provider(&mut t, &target, "RuleInfo"),
        "result",
    );

    assert_contains_exactly(
        &result,
        &[
            s("@@//test:r0[\"//test:aspect.bzl%a\"]=None"),
            s("@@//test:r1[\"//test:aspect.bzl%a\"]=@@//test:r0[\"//test:aspect.bzl%a\"],True"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.attributesWithAspectsReused
#[test]
fn attributes_with_aspects_reused() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _impl(target, ctx):
   return []
my_aspect = aspect(_impl)
a_dict = { 'foo' : attr.label_list(aspects = [my_aspect]) }
"],
    );
    t.scratch.file(
        "test/r1.bzl",
        &[r"load(':aspect.bzl', 'my_aspect', 'a_dict')
def _rule_impl(ctx):
   pass
r1 = rule(_rule_impl, attrs = a_dict)
"],
    );
    t.scratch.file(
        "test/r2.bzl",
        &[r"load(':aspect.bzl', 'my_aspect', 'a_dict')
def _rule_impl(ctx):
   pass
r2 = rule(_rule_impl, attrs = a_dict)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':r1.bzl', 'r1')
load(':r2.bzl', 'r2')
r1(name = 'x1')
r2(name = 'x2', foo = [':x1'])
"],
    );
    let analysis_result = update(&mut t, &["//test:x2"]);
    assert!(!analysis_result.has_error);
}

// Java: StarlarkDefinedAspectsTest.aspectAdvertisingProviders
#[test]
fn aspect_advertising_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"MyInfo = provider()
def _impl(target, ctx):
   return []
my_aspect = aspect(_impl, provides = [MyInfo])
a_dict = { 'foo' : attr.label_list(aspects = [my_aspect]) }
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('@rules_java//java:defs.bzl', 'java_library')",
            r"java_library(name = 'xxx',)",
        ],
    );
    t.remove_fail_fast_handler();
    update_expecting_failure(
        &mut t,
        &["//test:aspect.bzl%my_aspect"],
        &["//test:xxx"],
        "ViewCreationFailedException",
    );
    t.assert_contains_event(
        "Aspect '//test:aspect.bzl%my_aspect', applied to '//test:xxx', \
         does not provide advertised provider 'MyInfo'",
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectInconsistentVisibility
#[test]
fn aspect_on_aspect_inconsistent_visibility() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  return [a1p(text = 'random')]
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
  return [a2p(value = 'random')]
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [a1p])
def _r1_impl(ctx):
  pass
r2info = provider()
def _r2_impl(ctx):
  return r2info(result = ctx.attr.dep[a2p].value)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r1', 'r2')
r1(name = 'r0')
r1(name = 'r1', dep = ':r0')
r2(name = 'r2', dep = ':r1')
r1(name = 'r1_1', dep = ':r2')
r2(name = 'r2_1', dep = ':r1_1')
"],
    );
    t.remove_fail_fast_handler();

    update_expecting_failure(&mut t, &[], &["//test:r2_1"], "ViewCreationFailedException");
    let expected = t.ws(
        "ERROR /workspace/test/BUILD:3:3: Aspect //test:aspect.bzl%a2 is \
         applied twice, both before and after aspect //test:aspect.bzl%a1 \
         (when propagating to //test:r1)",
    );
    t.assert_contains_event(&expected);
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectInconsistentVisibilityIndirect
#[test]
fn aspect_on_aspect_inconsistent_visibility_indirect() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  return [a1p(text = 'random')]
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
  return [a2p(value = 'random')]
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [a1p])
def _r1_impl(ctx):
  pass
r2info = provider()
def _r2_impl(ctx):
  return r2info(result = ctx.attr.dep[a2p].value)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
def _r0_impl(ctx):
  pass
r0 = rule(_r0_impl, attrs = { 'dep' : attr.label()})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r1', 'r2')
r0(name = 'r0')
r1(name = 'r1', dep = ':r0')
r2(name = 'r2', dep = ':r1')
r1(name = 'r1_1', dep = ':r2')
r2(name = 'r2_1', dep = ':r1_1')
r0(name = 'r0_2', dep = ':r2_1')
"],
    );
    t.remove_fail_fast_handler();

    update_expecting_failure(&mut t, &[], &["//test:r0_2"], "ViewCreationFailedException");
    let expected = t.ws(
        "ERROR /workspace/test/BUILD:3:3: Aspect //test:aspect.bzl%a2 is \
         applied twice, both before and after aspect //test:aspect.bzl%a1 \
         (when propagating to //test:r1)",
    );
    t.assert_contains_event(&expected);
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectOnAspect
#[test]
fn aspect_on_aspect_on_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"p1 = provider()
def _a1_impl(target, ctx):
   return [p1()]
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [p1])
p2 = provider()
def _a2_impl(target, ctx):
   value = True if p1 in target else False
   return [p2(has_p1 = value)]
a2 = aspect(_a2_impl, attr_aspects = ['dep'],
   required_aspect_providers = [p1], provides = [p2])
p3 = provider()
def _a3_impl(target, ctx):
   list = []
   if ctx.rule.attr.dep:
     list = ctx.rule.attr.dep[p3].value
   my_value = str(target.label) +'=' + str(target[p2].has_p1 if p2 in target else False)
   return [p3(value = list + [my_value])]
a3 = aspect(_a3_impl, attr_aspects = ['dep'],
   required_aspect_providers = [p2])
def _r0_impl(ctx):
  pass
r0 = rule(_r0_impl, attrs = { 'dep' : attr.label()})
def _r1_impl(ctx):
  pass
def _r2_impl(ctx):
  pass
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r1', 'r2')
r0(name = 'r0_1')
r0(name = 'r0_2', dep = ':r0_1')
r0(name = 'r0_3', dep = ':r0_2')
r1(name = 'r1_1', dep = ':r0_3')
r2(name = 'r2_1', dep = ':r1_1')
"],
    );
    let analysis_result = update_with_aspects(&mut t, &["//test:aspect.bzl%a3"], &["//test:r2_1"]);
    let configured_aspect = analysis_result.only_aspect();
    let p3 = starlark_key("//test:aspect.bzl", "p3");
    let p3_provider = configured_aspect.get(&p3).expect("p3");
    assert_contains_exactly(
        &get_sequence(&p3_provider, "value"),
        &[
            s("@@//test:r0_1=True"),
            s("@@//test:r0_2=True"),
            s("@@//test:r0_3=True"),
            s("@@//test:r1_1=False"),
            s("@@//test:r2_1=False"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.multipleDepsDifferentAspects
#[test]
fn multiple_deps_different_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"PAspect = provider()
PCollector = provider()
def _aspect_impl(target, ctx):
   return [PAspect()]
a = aspect(_aspect_impl, attr_aspects = ['dep'], provides = [PAspect])
def _collector_impl(target, ctx):
   suffix = '+PAspect' if PAspect in target else ''
   result = [str(target.label)+suffix]
   for a in ['dep', 'dep1', 'dep2']:
     if hasattr(ctx.rule.attr, a):
        result += getattr(ctx.rule.attr, a)[PCollector].result
   return [PCollector(result=result)]
collector = aspect(_collector_impl, attr_aspects = ['*'],
                   required_aspect_providers = [PAspect])
def _rimpl(ctx):
   pass
r0 = rule(_rimpl)
r1 = rule(_rimpl,
          attrs = {
             'dep1' : attr.label(),
             'dep2' : attr.label(aspects = [a]),
          },
)
def _rcollect_impl(ctx):
    return [ctx.attr.dep[PCollector]]
rcollect = rule(_rcollect_impl,
                attrs = {
                  'dep' : attr.label(aspects = [collector]),
                })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r1', 'rcollect')
r0(name = 'r0')
r1(name = 'r1', dep1 = ':r0', dep2 = ':r0')
rcollect(name = 'rcollect', dep = ':r1')
"],
    );
    let analysis_result = update_with_aspects(&mut t, &[], &["//test:rcollect"]);
    let configured_target = analysis_result.only_target_to_build();
    let p_collector = starlark_key("//test:aspect.bzl", "PCollector");
    let p_collector_provider = configured_target.get(&p_collector).expect("PCollector");
    assert_contains_exactly(
        &get_sequence(&p_collector_provider, "result"),
        &[s("@@//test:r1"), s("@@//test:r0"), s("@@//test:r0+PAspect")],
    );
}

// Java: StarlarkDefinedAspectsTest.aspectSeesOtherAspectAttributes
#[test]
fn aspect_sees_other_aspect_attributes() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"PAspect = provider(fields = [])
PCollector = provider(fields = ['aspect_attr'])
def _a_impl(target, ctx):
  return [PAspect()]
a = aspect(_a_impl,
           provides = [PAspect],
           attrs = {'_a_attr' : attr.label(default = '//test:foo')})
def _rcollect(target, ctx):
  if hasattr(ctx.rule.attr, '_a_attr'):
     return [PCollector(aspect_attr = ctx.rule.attr._a_attr.label)]
  if hasattr(ctx.rule.attr, 'dep'):
     return [ctx.rule.attr.dep[PCollector]]
  return [PCollector()]
acollect = aspect(_rcollect, attr_aspects = ['*'], required_aspect_providers = [PAspect])
def _rimpl(ctx):
  pass
r0 = rule(_rimpl)
r = rule(_rimpl, attrs = { 'dep' : attr.label(aspects = [a]) })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r')
r0(name = 'foo')
r0(name = 'bar')
r(name = 'baz', dep = ':bar')
"],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["//test:aspect.bzl%acollect"], &["//test:baz"]);
    let configured_aspect = analysis_result.only_aspect();
    let p_collector = starlark_key("//test:aspect.bzl", "PCollector");
    let collector = configured_aspect.get(&p_collector).expect("PCollector");
    assert_eq!(get_value(&collector, "aspect_attr"), Some(l("//test:foo")));
}

// Java: StarlarkDefinedAspectsTest.ruleAttributesWinOverAspects
#[test]
fn rule_attributes_win_over_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"PAspect = provider(fields = [])
PCollector = provider(fields = ['attr_value'])
def _a_impl(target, ctx):
  return [PAspect()]
a = aspect(_a_impl,
           provides = [PAspect],
           attrs = {'_same_attr' : attr.int(default = 239)})
def _rcollect(target, ctx):
  if hasattr(ctx.rule.attr, '_same_attr'):
     return [PCollector(attr_value = ctx.rule.attr._same_attr)]
  if hasattr(ctx.rule.attr, 'dep'):
     return [ctx.rule.attr.dep[PCollector]]
  return [PCollector()]
acollect = aspect(_rcollect, attr_aspects = ['*'], required_aspect_providers = [PAspect])
def _rimpl(ctx):
  pass
r0 = rule(_rimpl)
r = rule(_rimpl,
          attrs = {
                  'dep' : attr.label(aspects = [a]),
                  '_same_attr' : attr.int(default = 30)
          })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r')
r0(name = 'foo')
r0(name = 'bar')
r(name = 'baz', dep = ':bar')
"],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["//test:aspect.bzl%acollect"], &["//test:baz"]);
    let configured_aspect = analysis_result.only_aspect();
    let p_collector = starlark_key("//test:aspect.bzl", "PCollector");
    let collector = configured_aspect.get(&p_collector).expect("PCollector");
    assert_eq!(get_value(&collector, "attr_value"), Some(J::Int(30)));
}

// Java: StarlarkDefinedAspectsTest.earlyAspectAttributesWin
#[test]
fn early_aspect_attributes_win() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"PAspect1 = provider(fields = [])
PAspect2 = provider(fields = [])
PCollector = provider(fields = ['attr_value'])
def _a1_impl(target, ctx):
  return [PAspect1()]
def _a2_impl(target, ctx):
  return [PAspect2()]
a1 = aspect(_a1_impl,
            provides = [PAspect1],
            attrs = {'_same_attr' : attr.int(default = 30)})
a2 = aspect(_a2_impl,
            provides = [PAspect2],
            attrs = {'_same_attr' : attr.int(default = 239)})
def _rcollect(target, ctx):
  if hasattr(ctx.rule.attr, 'dep'):
     return [ctx.rule.attr.dep[PCollector]]
  if hasattr(ctx.rule.attr, '_same_attr'):
     return [PCollector(attr_value = ctx.rule.attr._same_attr)]
  fail('???')
  return [PCollector()]
acollect = aspect(_rcollect, attr_aspects = ['*'],
                  required_aspect_providers = [[PAspect1], [PAspect2]])
def _rimpl(ctx):
  pass
r0 = rule(_rimpl)
r1 = rule(_rimpl,
          attrs = {
                  'dep' : attr.label(aspects = [a1]),
          })
r2 = rule(_rimpl,
          attrs = {
                  'dep' : attr.label(aspects = [a2]),
          })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r1', 'r2')
r0(name = 'bar')
r1(name = 'baz', dep = ':bar')
r2(name = 'quux', dep = ':baz')
"],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["//test:aspect.bzl%acollect"], &["//test:quux"]);
    let configured_aspect = analysis_result.only_aspect();
    let p_collector = starlark_key("//test:aspect.bzl", "PCollector");
    let collector = configured_aspect.get(&p_collector).expect("PCollector");
    assert_eq!(get_value(&collector, "attr_value"), Some(J::Int(30)));
}

// Java: StarlarkDefinedAspectsTest.aspectPropagatesOverOtherAspectAttributes
#[test]
fn aspect_propagates_over_other_aspect_attributes() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"PAspect = provider(fields = [])
PCollector = provider(fields = ['visited'])
def _a_impl(target, ctx):
  return [PAspect()]
a = aspect(_a_impl,
       provides = [PAspect],
       attrs = {'_a_attr' : attr.label(default = '//test:referenced_from_aspect_only')})
def _rcollect(target, ctx):
  transitive = []
  if hasattr(ctx.rule.attr, 'dep') and ctx.rule.attr.dep:
     transitive += [ctx.rule.attr.dep[PCollector].visited]
  if hasattr(ctx.rule.attr, '_a_attr') and ctx.rule.attr._a_attr:
     transitive += [ctx.rule.attr._a_attr[PCollector].visited]
  visited = depset([target.label], transitive = transitive, )
  return [PCollector(visited = visited)]
acollect = aspect(_rcollect, attr_aspects = ['*'], required_aspect_providers = [PAspect])
def _rimpl(ctx):
  pass
r0 = rule(_rimpl)
r = rule(_rimpl, attrs = { 'dep' : attr.label(aspects = [a]) })
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r')
r0(name = 'referenced_from_aspect_only')
r0(name = 'bar')
r(name = 'baz', dep = ':bar')
"],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["//test:aspect.bzl%acollect"], &["//test:baz"]);
    let configured_aspect = analysis_result.only_aspect();
    let p_collector = starlark_key("//test:aspect.bzl", "PCollector");
    let collector = configured_aspect.get(&p_collector).expect("PCollector");
    assert_contains_exactly(
        &get_depset_list(&collector, "visited"),
        &[
            l("//test:referenced_from_aspect_only"),
            l("//test:bar"),
            l("//test:baz"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectActionProvider
#[test]
fn test_aspect_action_provider() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"a1p = provider()
def _a1_impl(target,ctx):
  ctx.actions.run_shell(
    outputs = [ctx.actions.declare_file('a1')],
    command = 'touch $@'
  )
  return [a1p()]
a1 = aspect(_a1_impl, attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target, ctx):
  value = []
  if hasattr(ctx.rule.attr, 'dep') and a2p in ctx.rule.attr.dep:
     value += ctx.rule.attr.dep[a2p].value
  value += target.actions
  return [a2p(value = value)]
a2 = aspect(_a2_impl, attr_aspects = ['dep'], required_aspect_providers = [a1p])
def _r0_impl(ctx):
  ctx.actions.run_shell(
    outputs = [ctx.actions.declare_file('r0')],
    command = 'touch $@'
  )
RuleInfo = provider()
def _r1_impl(ctx):
  pass
def _r2_impl(ctx):
  return RuleInfo(result = ctx.attr.dep[a2p].value)
r0 = rule(_r0_impl)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load(':aspect.bzl', 'r0', 'r1', 'r2')
r0(name = 'r0')
r1(name = 'r1', dep = ':r0')
r2(name = 'r2', dep = ':r1')
"],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo");
    // Java: ((Action) a).getPrimaryOutput().getExecPath().getBaseName() of each element of
    // `result` (the Starlark `Action` values, blaze_analysis StarlarkAction).
    let result: Vec<String> = with_value(&info, "result", |v| {
        let v = v.expect("NullPointerException: no field result");
        ListRef::from_value(v)
            .unwrap_or_else(|| panic!("ClassCastException: result is not a Sequence: {v}"))
            .iter()
            .map(|a| {
                StarlarkAction::action_of(a)
                    .unwrap_or_else(|| panic!("ClassCastException: not an Action: {a}"))
                    .get_primary_output()
                    .get_exec_path()
                    .get_base_name()
                    .to_string()
            })
            .collect()
    });

    // We should see both the action from the 'r0' rule, and the action from the 'a1' aspect
    assert_eq!(result.len(), 2);
    let base_names: Vec<String> = result;
    assert_contains_exactly(&base_names, &["r0".to_string(), "a1".to_string()]);
}

// Java: StarlarkDefinedAspectsTest.testRuleAndAspectAttrConflict
#[test]
fn test_rule_and_aspect_attr_conflict() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"MyInfo = provider()
def _impl(target, ctx):
   return [MyInfo(hidden_attr_label = str(ctx.attr._hiddenattr.label))]

def _rule_impl(ctx):
   return []

my_rule = rule(implementation = _rule_impl,
   attrs = { '_hiddenattr' : attr.label(default = Label('//test:xxx')) },
)
MyAspect = aspect(
   implementation=_impl,
   attrs = { '_hiddenattr' : attr.label(default = Label('//test:zzz')) },
)
"],
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
cc_library(
     name = 'zzz',
)
"#],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["test/aspect.bzl%MyAspect"], &["//test:yyy"]);
    let configured_aspect = analysis_result.first_aspect();

    let my_info_key = starlark_key("//test:aspect.bzl", "MyInfo");
    let my_info = configured_aspect.get(&my_info_key).expect("MyInfo");
    assert_eq!(
        get_value(&my_info, "hidden_attr_label"),
        Some(s("@@//test:zzz"))
    );
}

// Java: StarlarkDefinedAspectsTest.aspectOnAspectAttrConflict
#[test]
fn aspect_on_aspect_attr_conflict() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"MyInfo = provider()
a1p = provider()
def _a1_impl(target,ctx):
  return a1p(text = 'random')
a1 = aspect(_a1_impl,
   attrs = { '_hiddenattr' : attr.label(default = Label('//test:xxx')) },
   attr_aspects = ['dep'], provides = [a1p])
a2p = provider()
def _a2_impl(target,ctx):
   return [MyInfo(hidden_attr_label = str(ctx.attr._hiddenattr.label))]
a2 = aspect(_a2_impl,
  attrs = { '_hiddenattr' : attr.label(default = Label('//test:zzz')) },
  attr_aspects = ['dep'], required_aspect_providers = [a1p])
RuleInfo = provider()
def _r1_impl(ctx):
  pass
def _r2_impl(ctx):
  return RuleInfo(result = ctx.attr.dep[MyInfo].hidden_attr_label)
r1 = rule(_r1_impl, attrs = { 'dep' : attr.label(aspects = [a1])})
r2 = rule(_r2_impl, attrs = { 'dep' : attr.label(aspects = [a1, a2])})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load(':aspect.bzl', 'r1', 'r2')
r1(name = 'r0')
r1(name = 'r1', dep = ':r0')
r2(name = 'r2', dep = ':r1')
cc_library(
     name = 'xxx',
)
cc_library(
     name = 'zzz',
)
"#],
    );
    let analysis_result = update(&mut t, &["//test:r2"]);
    let target = analysis_result.only_target_to_build();
    let result = get_value(
        &get_starlark_provider(&mut t, &target, "RuleInfo"),
        "result",
    );

    assert_eq!(result, Some(s("@@//test:zzz")));
}

// Java: StarlarkDefinedAspectsTest.testAllCcLibraryAttrsAreValidTypes
#[test]
fn test_all_cc_library_attrs_are_valid_types() {
    let mut t = new_test();
    t.scratch.file(
        "test/aspect.bzl",
        &[r"def _impl(target, ctx):
  for entry in dir(ctx.rule.attr):
    val = getattr(ctx.rule.attr, entry, None)
    # Only legitimate Starlark values can be passed to dir(), so this effectively
    # verifies val is an appropriate Starlark type.
    _test_dir = dir(val)
  return []

MyAspect = aspect(
  implementation=_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
     name = 'xxx',
)
"#],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["test/aspect.bzl%MyAspect"], &["//test:xxx"]);
    let _ = analysis_result.first_aspect();
}

// Java: StarlarkDefinedAspectsTest.testApplyToGeneratingRules
#[test]
fn test_apply_to_generating_rules() {
    let mut t = new_test();
    // Create test rules:
    // dep_rule: a rule which may depend on other dep_rule targets and may optionally create
    //     an output file.
    // root_{with,no}_files: a rule which depends on dep_rule targets and attaches an aspect.
    //     The rule returns a RootInfo provider which contains two fields:
    //        'from_aspect' : a list of all labels that the aspect propagated to
    //        'non_aspect' : a list of all labels that information was obtained from without aspect
    //     root_with_files uses an aspect with apply_to_generating_rules=True, and root_no_files
    //     uses an aspect with apply_to_generating_rules=False.
    t.scratch.file(
        "test/lib.bzl",
        &[r"RootInfo = provider()
NonAspectInfo = provider()
FromAspectInfo = provider()
def _aspect_impl(target, ctx):
  dep_labels = []
  for dep in ctx.rule.attr.deps:
    if FromAspectInfo in dep:
      dep_labels += [dep[FromAspectInfo].labels]
  return FromAspectInfo(labels = depset(direct = [ctx.label], transitive = dep_labels))

def _rule_impl(ctx):
  non_aspect = []
  from_aspect = []
  for dep in ctx.attr.deps:
    if NonAspectInfo in dep:
      non_aspect +=  dep[NonAspectInfo].labels.to_list()
    if FromAspectInfo in dep:
      from_aspect += dep[FromAspectInfo].labels.to_list()
  return RootInfo(from_aspect = from_aspect, non_aspect = non_aspect)

def _dep_rule_impl(ctx):
  if ctx.outputs.output:
    ctx.actions.run_shell(outputs = [ctx.outputs.output], command = 'dont run me')
  dep_labels = []
  for dep in ctx.attr.deps:
    if NonAspectInfo in dep:
      dep_labels += [dep[NonAspectInfo].labels]
  return NonAspectInfo(labels = depset(direct = [ctx.label], transitive = dep_labels))

aspect_with_files = aspect(
  implementation = _aspect_impl,
  attr_aspects = ['deps'],
  apply_to_generating_rules = True)

aspect_no_files = aspect(
  implementation = _aspect_impl,
  attr_aspects = ['deps'],
  apply_to_generating_rules = False)

root_with_files = rule(implementation = _rule_impl,
  attrs = {'deps' : attr.label_list(aspects = [aspect_with_files])})

root_no_files = rule(implementation = _rule_impl,
  attrs = {'deps' : attr.label_list(aspects = [aspect_no_files])})

dep_rule = rule(implementation = _dep_rule_impl,
  attrs = {'deps' : attr.label_list(allow_files = True), 'output' : attr.output()})
"],
    );
    // Create a target graph such that two graph roots each point to a common subgraph
    // alpha -> beta_output -> charlie, where beta_output is a generated output file of target
    // 'beta'.
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:lib.bzl', 'root_with_files', 'root_no_files', 'dep_rule')

root_with_files(name = 'test_with_files', deps = [':alpha'])
root_no_files(name = 'test_no_files', deps = [':alpha'])
dep_rule(name = 'alpha', deps = [':beta_output'])
dep_rule(name = 'beta', deps = [':charlie'], output = 'beta_output')
dep_rule(name = 'charlie')
",
        ],
    );
    let root_info_key = starlark_key("//test:lib.bzl", "RootInfo");
    let analysis_result_with_files = update(&mut t, &["//test:test_with_files"]);
    let target_with_files = analysis_result_with_files.only_target_to_build();
    let root_info_with_files = target_with_files.get(&root_info_key).expect("RootInfo");
    // With apply_to_generating_rules=True, the aspect should have traversed :beta_output and
    // applied to both :beta and :charlie.
    assert_contains_exactly(
        &get_sequence(&root_info_with_files, "from_aspect"),
        &[l("//test:charlie"), l("//test:beta"), l("//test:alpha")],
    );
    assert_contains_exactly(
        &get_sequence(&root_info_with_files, "non_aspect"),
        &[l("//test:alpha")],
    );
    let analysis_result_no_files = update(&mut t, &["//test:test_no_files"]);
    let target_no_files = analysis_result_no_files.only_target_to_build();
    let root_info_no_files = target_no_files.get(&root_info_key).expect("RootInfo");
    // With apply_to_generating_rules=False, the aspect should have only accessed :alpha, as it
    // must have stopped before :beta_output.
    assert_contains_exactly(
        &get_sequence(&root_info_no_files, "from_aspect"),
        &[l("//test:alpha")],
    );
    assert_contains_exactly(
        &get_sequence(&root_info_with_files, "non_aspect"),
        &[l("//test:alpha")],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectOnAspectApplyToGeneratingRules_bothPropagate
#[test]
fn test_aspect_on_aspect_apply_to_generating_rules_both_propagate() {
    let mut t = new_test();
    setup_aspect_on_aspect_target_graph(
        &mut t, /* apply_root_to_generating_rules= */ true,
        /* apply_dep_to_generating_rules= */ true,
    );
    let root_info_key = starlark_key("//test:lib.bzl", "RootAspectInfo");
    let dep_info_key = starlark_key("//test:lib.bzl", "DepAspectInfo");
    let analysis_result = update(&mut t, &["//test:test"]);
    let target = analysis_result.only_target_to_build();
    let root_info = target.get(&root_info_key).expect("RootAspectInfo");
    let dep_info = target.get(&dep_info_key).expect("DepAspectInfo");
    assert_contains_exactly(
        &get_sequence(&root_info, "both_labels"),
        &[
            l("//test:alpha"),
            l("//test:beta_output"),
            l("//test:charlie"),
        ],
    );
    assert!(get_sequence(&root_info, "root_only_labels").is_empty());
    assert_contains_exactly(
        &get_sequence(&dep_info, "labels"),
        &[l("//test:alpha"), l("//test:beta"), l("//test:charlie")],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectOnAspectApplyToGeneratingRules_neitherPropagate
#[test]
fn test_aspect_on_aspect_apply_to_generating_rules_neither_propagate() {
    let mut t = new_test();
    setup_aspect_on_aspect_target_graph(
        &mut t, /* apply_root_to_generating_rules= */ false,
        /* apply_dep_to_generating_rules= */ false,
    );
    let root_info_key = starlark_key("//test:lib.bzl", "RootAspectInfo");
    let dep_info_key = starlark_key("//test:lib.bzl", "DepAspectInfo");
    let analysis_result = update(&mut t, &["//test:test"]);
    let target = analysis_result.only_target_to_build();
    let root_info = target.get(&root_info_key).expect("RootAspectInfo");
    let dep_info = target.get(&dep_info_key).expect("DepAspectInfo");
    assert_contains_exactly(
        &get_sequence(&root_info, "both_labels"),
        &[l("//test:alpha")],
    );
    assert!(get_sequence(&root_info, "root_only_labels").is_empty());
    assert_contains_exactly(&get_sequence(&dep_info, "labels"), &[l("//test:alpha")]);
}

// Java: StarlarkDefinedAspectsTest.testAspectOnAspectApplyToGeneratingRules_rootOnly
#[test]
fn test_aspect_on_aspect_apply_to_generating_rules_root_only() {
    let mut t = new_test();
    setup_aspect_on_aspect_target_graph(
        &mut t, /* apply_root_to_generating_rules= */ true,
        /* apply_dep_to_generating_rules= */ false,
    );
    let root_info_key = starlark_key("//test:lib.bzl", "RootAspectInfo");
    let dep_info_key = starlark_key("//test:lib.bzl", "DepAspectInfo");
    let analysis_result = update(&mut t, &["//test:test"]);
    let target = analysis_result.only_target_to_build();
    let root_info = target.get(&root_info_key).expect("RootAspectInfo");
    let dep_info = target.get(&dep_info_key).expect("DepAspectInfo");
    assert_contains_exactly(
        &get_sequence(&root_info, "both_labels"),
        &[l("//test:alpha")],
    );
    assert_contains_exactly(
        &get_sequence(&root_info, "root_only_labels"),
        &[l("//test:beta_output"), l("//test:charlie")],
    );
    assert_contains_exactly(&get_sequence(&dep_info, "labels"), &[l("//test:alpha")]);
}

// Java: StarlarkDefinedAspectsTest.testAspectOnAspectApplyToGeneratingRules_depOnly
#[test]
fn test_aspect_on_aspect_apply_to_generating_rules_dep_only() {
    let mut t = new_test();
    setup_aspect_on_aspect_target_graph(
        &mut t, /* apply_root_to_generating_rules= */ false,
        /* apply_dep_to_generating_rules= */ true,
    );
    let root_info_key = starlark_key("//test:lib.bzl", "RootAspectInfo");
    let dep_info_key = starlark_key("//test:lib.bzl", "DepAspectInfo");
    let analysis_result = update(&mut t, &["//test:test"]);
    let target = analysis_result.only_target_to_build();
    let root_info = target.get(&root_info_key).expect("RootAspectInfo");
    let dep_info = target.get(&dep_info_key).expect("DepAspectInfo");
    assert_contains_exactly(
        &get_sequence(&root_info, "both_labels"),
        &[l("//test:alpha")],
    );
    assert!(get_sequence(&root_info, "root_only_labels").is_empty());
    assert_contains_exactly(
        &get_sequence(&dep_info, "labels"),
        &[l("//test:alpha"), l("//test:beta"), l("//test:charlie")],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectActionsDontInheritExecProperties
#[test]
fn test_aspect_actions_dont_inherit_exec_properties() {
    let mut t = new_test();
    t.scratch.file(
        "test/BUILD",
        &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")
load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'my_target',
  deps = [':my_dep'],
)
cc_binary(
  name = 'my_dep',
  srcs = ['dep.cc'],
  exec_properties = {'mem': '16g'},
)
"#],
    );
    t.scratch.file(
        "test/defs.bzl",
        &[r"def _aspect_impl(target, ctx):
  f = ctx.actions.declare_file('f.txt')
  ctx.actions.write(f, 'f')
  return []
my_aspect = aspect(
  implementation = _aspect_impl,
  attr_aspects = ['deps'],
)
def _rule_impl(ctx):
  pass
my_rule = rule(
  implementation = _rule_impl,
  attrs = {
    'deps': attr.label_list(aspects = [my_aspect])
  },
)
"],
    );
    t.scratch.file("test/dep.cc", &[r"int main() {return 0;}"]);
    let analysis_result =
        update_with_aspects(&mut t, &["test/defs.bzl%my_aspect"], &["//test:my_target"]);
    let aspect = analysis_result.only_aspect();
    assert_eq!(
        aspect.get_actions().len(),
        1,
        "getOnlyElement(getActions())"
    );
    let owner = aspect.get_actions()[0].get_owner().clone();
    assert!(owner.get_exec_properties().is_empty());
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredProviders_defaultNoRequiredProviders
#[test]
fn test_aspect_required_providers_default_no_required_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r#"prov_a = provider()
prov_b = provider()

AspectInfo = provider()
def _my_aspect_impl(target, ctx):
  targets_labels = ["//test:defs.bzl%my_aspect({})".format(target.label)]
  for dep in ctx.rule.attr.deps:
    if AspectInfo in dep:
      targets_labels.extend(dep[AspectInfo].target_labels)
  return [AspectInfo(target_labels = targets_labels)]

my_aspect = aspect(
  implementation = _my_aspect_impl,
  attr_aspects = ['deps'],
)

RuleInfo = provider()
def _rule_without_providers_impl(ctx):
  s = []
  for dep in ctx.attr.deps:
    if AspectInfo in dep:
      s.extend(dep[AspectInfo].target_labels)
  return RuleInfo(rule_deps = s)

rule_without_providers = rule(
  implementation = _rule_without_providers_impl,
  attrs = {
    'deps': attr.label_list(aspects = [my_aspect])
  },
)

def _rule_with_providers_impl(ctx):
  return [prov_a(), prov_b()]

rule_with_providers = rule(
  implementation = _rule_with_providers_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_a, prov_b]
)

rule_with_providers_not_advertised = rule(
  implementation = _rule_with_providers_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = []
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:defs.bzl', 'rule_with_providers', 'rule_without_providers',
                        'rule_with_providers_not_advertised')
rule_without_providers(
  name = 'main',
  deps = [':target_without_providers', ':target_with_providers',
          ':target_with_providers_not_advertised'],
)
rule_without_providers(
  name = 'target_without_providers',
)
rule_with_providers(
  name = 'target_with_providers',
)
rule_with_providers(
  name = 'target_with_providers_indeps',
)
rule_with_providers_not_advertised(
  name = 'target_with_providers_not_advertised',
  deps = [':target_with_providers_indeps'],
)
",
        ],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // my_aspect does not require any providers so it will be applied to all the dependencies of
    // main target
    let expected = vec![
        s("//test:defs.bzl%my_aspect(@@//test:target_without_providers)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_providers)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_providers_not_advertised)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_providers_indeps)"),
    ];
    assert_contains_exactly(
        &get_labels_to_build(&analysis_result),
        &["//test:main".to_string()],
    );
    let target = analysis_result.targets_to_build[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo");
    assert!(
        is_starlark_list(&info, "rule_deps"),
        "isInstanceOf(StarlarkList.class)"
    );
    assert_contains_exactly(&get_sequence(&info, "rule_deps"), &expected);
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredProviders_flatSetOfRequiredProviders
#[test]
fn test_aspect_required_providers_flat_set_of_required_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r#"prov_a = provider()
prov_b = provider()

AspectInfo = provider()
def _my_aspect_impl(target, ctx):
  targets_labels = ["//test:defs.bzl%my_aspect({})".format(target.label)]
  for dep in ctx.rule.attr.deps:
    if AspectInfo in dep:
      targets_labels.extend(dep[AspectInfo].target_labels)
  return AspectInfo(target_labels = targets_labels)

my_aspect = aspect(
  implementation = _my_aspect_impl,
  attr_aspects = ['deps'],
  required_providers = [prov_a, prov_b],
)

RuleInfo = provider()
def _rule_without_providers_impl(ctx):
  s = []
  for dep in ctx.attr.deps:
    if AspectInfo in dep:
      s.extend(dep[AspectInfo].target_labels)
  return RuleInfo(rule_deps = s)

rule_without_providers = rule(
  implementation = _rule_without_providers_impl,
  attrs = {
    'deps': attr.label_list(aspects=[my_aspect])
  },
  provides = []
)

def _rule_with_a_impl(ctx):
  return [prov_a()]

rule_with_a = rule(
  implementation = _rule_with_a_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_a]
)

def _rule_with_ab_impl(ctx):
  return [prov_a(), prov_b()]

rule_with_ab = rule(
  implementation = _rule_with_ab_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_a, prov_b]
)

rule_with_ab_not_advertised = rule(
  implementation = _rule_with_ab_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = []
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:defs.bzl', 'rule_without_providers', 'rule_with_a', 'rule_with_ab',
                        'rule_with_ab_not_advertised')
rule_without_providers(
  name = 'main',
  deps = [':target_without_providers', ':target_with_a', ':target_with_ab',
          ':target_with_ab_not_advertised'],
)
rule_without_providers(
  name = 'target_without_providers',
)
rule_with_a(
  name = 'target_with_a',
  deps = [':target_with_ab_indeps_not_reached']
)
rule_with_ab(
  name = 'target_with_ab',
  deps = [':target_with_ab_indeps_reached']
)
rule_with_ab(
  name = 'target_with_ab_indeps_not_reached',
)
rule_with_ab(
  name = 'target_with_ab_indeps_reached',
)
rule_with_ab_not_advertised(
  name = 'target_with_ab_not_advertised',
)
",
        ],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // my_aspect will only be applied on target_with_ab and target_with_ab_indeps_reached since
    // their rule (rule_with_ab) is the only rule that advertises the aspect required providers.
    // However, my_aspect cannot be propagated to target_with_ab_indeps_not_reached because it was
    // not applied to its parent (target_with_a)
    let expected = vec![
        s("//test:defs.bzl%my_aspect(@@//test:target_with_ab)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_ab_indeps_reached)"),
    ];
    assert_contains_exactly(
        &get_labels_to_build(&analysis_result),
        &["//test:main".to_string()],
    );
    let target = analysis_result.targets_to_build[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo");
    assert!(
        is_starlark_list(&info, "rule_deps"),
        "isInstanceOf(StarlarkList.class)"
    );
    assert_contains_exactly(&get_sequence(&info, "rule_deps"), &expected);
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredProviders_listOfRequiredProvidersLists
#[test]
fn test_aspect_required_providers_list_of_required_providers_lists() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r#"prov_a = provider()
prov_b = provider()
prov_c = provider()
AspectInfo = provider()
def _my_aspect_impl(target, ctx):
  targets_labels = ["//test:defs.bzl%my_aspect({})".format(target.label)]
  for dep in ctx.rule.attr.deps:
    if AspectInfo in dep:
      targets_labels.extend(dep[AspectInfo].target_labels)
  return [AspectInfo(target_labels = targets_labels)]

my_aspect = aspect(
  implementation = _my_aspect_impl,
  attr_aspects = ['deps'],
  required_providers = [[prov_a, prov_b], [prov_c]],
)

RuleInfo = provider()
def _rule_without_providers_impl(ctx):
  s = []
  for dep in ctx.attr.deps:
    if AspectInfo in dep:
      s.extend(dep[AspectInfo].target_labels)
  return RuleInfo(rule_deps = s)

rule_without_providers = rule(
  implementation = _rule_without_providers_impl,
  attrs = {
    'deps': attr.label_list(aspects=[my_aspect])
  },
  provides = []
)

def _rule_with_a_impl(ctx):
  return [prov_a()]

rule_with_a = rule(
  implementation = _rule_with_a_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_a]
)

def _rule_with_c_impl(ctx):
  return [prov_c()]

rule_with_c = rule(
  implementation = _rule_with_c_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_c]
)

def _rule_with_ab_impl(ctx):
  return [prov_a(), prov_b()]

rule_with_ab = rule(
  implementation = _rule_with_ab_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = [prov_a, prov_b]
)

rule_with_ab_not_advertised = rule(
  implementation = _rule_with_ab_impl,
  attrs = {
    'deps': attr.label_list()
  },
  provides = []
)
"#],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:defs.bzl', 'rule_without_providers', 'rule_with_a', 'rule_with_c',
                        'rule_with_ab', 'rule_with_ab_not_advertised')
rule_without_providers(
  name = 'main',
  deps = [':target_without_providers', ':target_with_a', ':target_with_c',
          ':target_with_ab', 'target_with_ab_not_advertised'],
)
rule_without_providers(
  name = 'target_without_providers',
)
rule_with_a(
  name = 'target_with_a',
  deps = [':target_with_c_indeps_not_reached'],
)
rule_with_c(
  name = 'target_with_c',
)
rule_with_c(
  name = 'target_with_c_indeps_reached',
)
rule_with_c(
  name = 'target_with_c_indeps_not_reached',
)
rule_with_ab(
  name = 'target_with_ab',
  deps = [':target_with_c_indeps_reached'],
)
rule_with_ab_not_advertised(
  name = 'target_with_ab_not_advertised',
)
",
        ],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // my_aspect will only be applied on target_with_ab, target_wtih_c and
    // target_with_c_indeps_reached because their rules (rule_with_ab and rule_with_c) are the only
    // rules advertising the aspect required providers
    // However, my_aspect cannot be propagated to target_with_c_indeps_not_reached because it was
    // not applied to its parent (target_with_a)
    let expected = vec![
        s("//test:defs.bzl%my_aspect(@@//test:target_with_ab)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_c)"),
        s("//test:defs.bzl%my_aspect(@@//test:target_with_c_indeps_reached)"),
    ];
    assert_contains_exactly(
        &get_labels_to_build(&analysis_result),
        &["//test:main".to_string()],
    );
    let target = analysis_result.targets_to_build[0].clone();
    let info = get_starlark_provider(&mut t, &target, "RuleInfo");
    assert!(
        is_starlark_list(&info, "rule_deps"),
        "isInstanceOf(StarlarkList.class)"
    );
    assert_contains_exactly(&get_sequence(&info, "rule_deps"), &expected);
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredByMultipleAspects_inheritsAttrAspects
#[test]
fn test_aspect_required_by_multiple_aspects_inherits_attr_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"prov_a = provider()
prov_b = provider()
prov_c = provider()

def _aspect_c_impl(target, ctx):
  res = ['aspect_c runs on target {}'.format(target.label)]
  return [prov_c(val = res)]
aspect_c = aspect(
  implementation = _aspect_c_impl,
)

def _aspect_b_impl(target, ctx):
  res = []
  res += target[prov_c].val
  res += ['aspect_b runs on target {}'.format(target.label)]
  if ctx.rule.attr.dep_b:
    res += ctx.rule.attr.dep_b[prov_b].val
  return [prov_b(val = res)]
aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep_b'],
  requires = [aspect_c],
)

def _aspect_a_impl(target, ctx):
  res = []
  res += target[prov_c].val
  res += ['aspect_a runs on target {}'.format(target.label)]
  if ctx.rule.attr.dep_a:
    res += ctx.rule.attr.dep_a[prov_a].val
  return [prov_a(val = res)]
aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep_a'],
  requires = [aspect_c],
)

def _my_rule_impl(ctx):
  pass

my_rule = rule(
  implementation = _my_rule_impl,
  attrs = {
    'dep_a': attr.label(),
    'dep_b': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep_a = ':dep_target_a',
  dep_b = ':dep_target_b',
)
my_rule(
  name = 'dep_target_a',
)
my_rule(
  name = 'dep_target_b',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
        &["//test:main_target"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    // aspect_a should run on main_target and dep_target_a and can retrieve aspect_c provider value
    // on both of them
    let aspect_a = get_configured_aspect(&configured_aspects, "aspect_a").expect("aspect_a");
    let a_result = starlark_key("//test:defs.bzl", "prov_a");
    let a_result_provider = aspect_a.get(&a_result).expect("prov_a");
    assert_contains_exactly(
        &get_sequence(&a_result_provider, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_target_a"),
            s("aspect_a runs on target @@//test:dep_target_a"),
            s("aspect_c runs on target @@//test:main_target"),
            s("aspect_a runs on target @@//test:main_target"),
        ],
    );
    // aspect_b should run on main_target and dep_target_b and can retrieve aspect_c provider value
    // on both of them
    let aspect_b = get_configured_aspect(&configured_aspects, "aspect_b").expect("aspect_b");
    let b_result = starlark_key("//test:defs.bzl", "prov_b");
    let b_result_provider = aspect_b.get(&b_result).expect("prov_b");
    assert_contains_exactly(
        &get_sequence(&b_result_provider, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_target_b"),
            s("aspect_b runs on target @@//test:dep_target_b"),
            s("aspect_c runs on target @@//test:main_target"),
            s("aspect_b runs on target @@//test:main_target"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredByMultipleAspects_inheritsRequiredProviders
#[test]
fn test_aspect_required_by_multiple_aspects_inherits_required_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"aspect_prov_a = provider()
aspect_prov_b = provider()
aspect_prov_c = provider()
rule_prov_a = provider()
rule_prov_b = provider()
rule_prov_c = provider()

def _aspect_c_impl(target, ctx):
  res = ['aspect_c runs on target {}'.format(target.label)]
  return [aspect_prov_c(val = res)]
aspect_c = aspect(
  implementation = _aspect_c_impl,
  required_providers = [rule_prov_c],
)

def _aspect_b_impl(target, ctx):
  res = []
  if aspect_prov_c in target:
    res += target[aspect_prov_c].val
  res += ['aspect_b runs on target {}'.format(target.label)]
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      if aspect_prov_b in dep:
        res += dep[aspect_prov_b].val
  return [aspect_prov_b(val = res)]
aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['deps'],
  required_providers = [[rule_prov_b], [rule_prov_c]],
  requires = [aspect_c],
)

def _aspect_a_impl(target, ctx):
  res = []
  if aspect_prov_c in target:
    res += target[aspect_prov_c].val
  res += ['aspect_a runs on target {}'.format(target.label)]
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      if aspect_prov_a in dep:
        res += dep[aspect_prov_a].val
  return [aspect_prov_a(val = res)]
aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['deps'],
  required_providers = [[rule_prov_a], [rule_prov_c]],
  requires = [aspect_c],
)

def _my_rule_impl(ctx):
  return [rule_prov_a(), rule_prov_b()]

my_rule = rule(
  implementation = _my_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [rule_prov_a, rule_prov_b]
)

def _rule_with_prov_a_impl(ctx):
  return [rule_prov_a()]

rule_with_prov_a = rule(
  implementation = _rule_with_prov_a_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [rule_prov_a]
)

def _rule_with_prov_b_impl(ctx):
  return [rule_prov_b()]

rule_with_prov_b = rule(
  implementation = _rule_with_prov_b_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [rule_prov_b]
)

def _rule_with_prov_c_impl(ctx):
  return [rule_prov_c()]
rule_with_prov_c = rule(
  implementation = _rule_with_prov_c_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [rule_prov_c]
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'my_rule', 'rule_with_prov_a',
                        'rule_with_prov_b', 'rule_with_prov_c')
my_rule(
  name = 'main_target',
  deps = [':dep_target_with_prov_a', ':dep_target_with_prov_b']
)
rule_with_prov_a(
  name = 'dep_target_with_prov_a',
  deps = [':dep_target_with_prov_c'],
)
rule_with_prov_b(
  name = 'dep_target_with_prov_b',
  deps = [':dep_target_with_prov_c'],
)
rule_with_prov_c(
  name = 'dep_target_with_prov_c',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%aspect_a", "test/defs.bzl%aspect_b"],
        &["//test:main_target"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    // aspect_a runs on main_target, dep_target_with_prov_a and dep_target_with_prov_c and it can
    // only retrieve aspect_c provider value on dep_target_with_prov_c
    let aspect_a = get_configured_aspect(&configured_aspects, "aspect_a").expect("aspect_a");
    let a_result = starlark_key("//test:defs.bzl", "aspect_prov_a");
    let a_result_provider = aspect_a.get(&a_result).expect("aspect_prov_a");
    assert_contains_exactly(
        &get_sequence(&a_result_provider, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_target_with_prov_c"),
            s("aspect_a runs on target @@//test:dep_target_with_prov_c"),
            s("aspect_a runs on target @@//test:dep_target_with_prov_a"),
            s("aspect_a runs on target @@//test:main_target"),
        ],
    );
    // aspect_b runs on main_target, dep_target_with_prov_b and dep_target_with_prov_c and it can
    // only retrieve aspect_c provider value on dep_target_with_prov_c
    let aspect_b = get_configured_aspect(&configured_aspects, "aspect_b").expect("aspect_b");
    let b_result = starlark_key("//test:defs.bzl", "aspect_prov_b");
    let b_result_provider = aspect_b.get(&b_result).expect("aspect_prov_b");
    assert_contains_exactly(
        &get_sequence(&b_result_provider, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_target_with_prov_c"),
            s("aspect_b runs on target @@//test:dep_target_with_prov_c"),
            s("aspect_b runs on target @@//test:dep_target_with_prov_b"),
            s("aspect_b runs on target @@//test:main_target"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiredByMultipleAspects_withDifferentParametersValues
#[test]
fn test_aspect_required_by_multiple_aspects_with_different_parameters_values() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"prov_a = provider()
prov_b = provider()
prov_c = provider()

def _aspect_c_impl(target, ctx):
  res = ['aspect_c runs on target {} and param = {}'.format(target.label, ctx.attr.p)]
  return [prov_c(val = res)]
aspect_c = aspect(
  implementation = _aspect_c_impl,
  attrs = {
    'p': attr.string(values=['rule_1_val', 'rule_2_val']),
  },
)

def _aspect_b_impl(target, ctx):
  res = []
  res += target[prov_c].val
  res += ['aspect_b runs on target {}'.format(target.label)]
  if ctx.rule.attr.dep:
    res += ctx.rule.attr.dep[prov_b].val
  return [prov_b(val = res)]
aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = ['dep'],
  requires = [aspect_c],
)

def _aspect_a_impl(target, ctx):
  res = []
  res += target[prov_c].val
  res += ['aspect_a runs on target {}'.format(target.label)]
  if ctx.rule.attr.dep:
    res += ctx.rule.attr.dep[prov_a].val
  return [prov_a(val = res)]
aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = ['dep'],
  requires = [aspect_c],
)

def _rule_1_impl(ctx):
  return ctx.attr.dep[prov_a]

rule_1 = rule(
  implementation = _rule_1_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_a]),
    'p': attr.string(values = ['rule_1_val', 'rule_2_val'])
  },
)

def _rule_2_impl(ctx):
  return ctx.attr.dep[prov_b]

rule_2 = rule(
  implementation = _rule_2_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_b]),
    'p': attr.string(values = ['rule_1_val', 'rule_2_val'])
  },
)

def _rule_3_impl(ctx):
  pass

rule_3 = rule(
  implementation = _rule_3_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'rule_1', 'rule_2', 'rule_3')
rule_1(
  name = 'target_1',
  dep = ':dep_1',
  p = 'rule_1_val'
)
rule_2(
  name = 'target_2',
  dep = ':dep_2',
  p = 'rule_2_val'
)
rule_3(
  name = 'dep_1',
  dep = ':dep_3',
)
rule_3(
  name = 'dep_2',
  dep = ':dep_3',
)
rule_3(
  name = 'dep_3',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:target_1", "//test:target_2"]);
    let mut it = analysis_result.targets_to_build.iter();
    // aspect_a runs on dep_1 and dep_3 and it can retrieve aspect_c provider value on them
    // aspect_c here should get its parameter value from rule_2
    let target1 = it.next().expect("target_1");
    let prov_akey = starlark_key("//test:defs.bzl", "prov_a");
    let prov_a = target1.get(&prov_akey).expect("prov_a");
    assert_contains_exactly(
        &get_sequence(&prov_a, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_1 and param = rule_1_val"),
            s("aspect_a runs on target @@//test:dep_1"),
            s("aspect_c runs on target @@//test:dep_3 and param = rule_1_val"),
            s("aspect_a runs on target @@//test:dep_3"),
        ],
    );
    // aspect_b runs on dep_2 and dep_3 and it can retrieve aspect_c provider value on them.
    // aspect_c here should get its parameter value from rule_2
    let target2 = it.next().expect("target_2");
    let prov_bkey = starlark_key("//test:defs.bzl", "prov_b");
    let prov_b = target2.get(&prov_bkey).expect("prov_b");
    assert_contains_exactly(
        &get_sequence(&prov_b, "val"),
        &[
            s("aspect_c runs on target @@//test:dep_2 and param = rule_2_val"),
            s("aspect_b runs on target @@//test:dep_2"),
            s("aspect_c runs on target @@//test:dep_3 and param = rule_2_val"),
            s("aspect_b runs on target @@//test:dep_3"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_requireNativeAspect
#[test]
#[ignore = "needs TestRuleClassProvider's test rules and TestAspects native aspects (exposeNativeAspectToStarlark: starlark_native_aspect, honest)"]
fn test_aspect_requires_aspect_require_native_aspect() {
    let mut t = new_test();
    expose_native_aspect_to_starlark();
    t.scratch.file(
        "test/defs.bzl",
        &[r"prov_a = provider()
def _impl(target, ctx):
  res = 'aspect_a on target {} '.format(target.label)
  if hasattr(target, 'native_aspect_prov'):
    res += 'can see native aspect provider'
  else:
    res += 'cannot see native aspect provider'
  complete_res = [res]
  if hasattr(ctx.rule.attr, 'dep'):
    complete_res += ctx.rule.attr.dep[prov_a].val
  return [prov_a(val = complete_res)]
aspect_a = aspect(implementation = _impl,
                  requires = [starlark_native_aspect],
                  attr_aspects = ['dep'],)

def _my_rule_impl(ctx):
  pass
my_rule = rule(implementation = _my_rule_impl,
               attrs = {'dep': attr.label()})
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'my_rule')
my_rule(
  name = 'main_target',
  dep = ':dep_1',
)
my_rule(
  name = 'dep_1',
  dep = ':dep_2',
)
honest(
  name = 'dep_2',
)
"],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["test/defs.bzl%aspect_a"], &["//test:main_target"]);
    let configured_aspects = analysis_result.get_aspects_map();
    // aspect_a runs on main_target, dep_1 and dep_2 but it can only see the required native aspect
    // run on dep_2 because its rule satisfies its required provider.
    let aspect_a = get_configured_aspect(&configured_aspects, "aspect_a").expect("aspect_a");
    let a_result = starlark_key("//test:defs.bzl", "prov_a");
    let a_result_provider = aspect_a.get(&a_result).expect("prov_a");
    assert_contains_exactly(
        &get_sequence(&a_result_provider, "val"),
        &[
            s("aspect_a on target @@//test:main_target cannot see native aspect provider"),
            s("aspect_a on target @@//test:dep_1 cannot see native aspect provider"),
            s("aspect_a on target @@//test:dep_2 can see native aspect provider"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_aspectsParameters
#[test]
fn test_aspect_requires_aspect_aspects_parameters() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"RequiredAspectProv = provider()
BaseAspectProv = provider()

def _required_aspect_impl(target, ctx):
  p1_val = 'In required_aspect, p1 = {} on target {}'.format(ctx.attr.p1, target.label)
  p2_val = 'invalid value'
  if not hasattr(ctx.attr, 'p2'):
    p2_val = 'In required_aspect, p2 not found on target {}'.format(target.label)
  return [RequiredAspectProv(p1_val = p1_val, p2_val = p2_val)]
required_aspect = aspect(
  implementation = _required_aspect_impl,
  attr_aspects = ['dep'],
  attrs = {'p1' : attr.string(values = ['p1_v1', 'p1_v2'])}
)

def _base_aspect_impl(target, ctx):
  p2_val = 'In base_aspect, p2 = {} on target {}'.format(ctx.attr.p2, target.label)
  p1_val = 'invalid value'
  if not hasattr(ctx.attr, 'p1'):
    p1_val = 'In base_aspect, p1 not found on target {}'.format(target.label)
  return [BaseAspectProv(p1_val = p1_val, p2_val = p2_val)]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  attrs = {'p2' : attr.string(values = ['p2_v1', 'p2_v2'])},
  requires = [required_aspect],
)

def _main_rule_impl(ctx):
  return [ctx.attr.dep[RequiredAspectProv], ctx.attr.dep[BaseAspectProv]]
def _dep_rule_impl(ctx):
  pass

main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects=[base_aspect]),
    'p1' : attr.string(values = ['p1_v1', 'p1_v2']),
    'p2' : attr.string(values = ['p2_v1', 'p2_v2'])
  },
)

dep_rule = rule(
  implementation = _dep_rule_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
  p1 = 'p1_v1',
  p2 = 'p2_v1'
)
dep_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // Both base_aspect and required_aspect can get their parameters values from the base rule
    let configured_target = analysis_result.only_target_to_build();
    let required_aspect_prov = starlark_key("//test:defs.bzl", "RequiredAspectProv");
    let required_aspect_provider = configured_target
        .get(&required_aspect_prov)
        .expect("RequiredAspectProv");
    assert_eq!(
        get_value(&required_aspect_provider, "p1_val"),
        Some(s(
            "In required_aspect, p1 = p1_v1 on target @@//test:dep_target"
        ))
    );
    assert_eq!(
        get_value(&required_aspect_provider, "p2_val"),
        Some(s(
            "In required_aspect, p2 not found on target @@//test:dep_target"
        ))
    );
    let base_aspect_prov = starlark_key("//test:defs.bzl", "BaseAspectProv");
    let base_aspect_provider = configured_target
        .get(&base_aspect_prov)
        .expect("BaseAspectProv");
    assert_eq!(
        get_value(&base_aspect_provider, "p1_val"),
        Some(s(
            "In base_aspect, p1 not found on target @@//test:dep_target"
        ))
    );
    assert_eq!(
        get_value(&base_aspect_provider, "p2_val"),
        Some(s(
            "In base_aspect, p2 = p2_v1 on target @@//test:dep_target"
        ))
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_ruleAttributes
#[test]
fn test_aspect_requires_aspect_rule_attributes() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"RequiredAspectProv = provider()
BaseAspectProv = provider()

def _required_aspect_impl(target, ctx):
  p_val = 'In required_aspect, p = {} on target {}'.format(ctx.rule.attr.p, target.label)
  return [RequiredAspectProv(p_val = p_val)]
required_aspect = aspect(
  implementation = _required_aspect_impl,
)

def _base_aspect_impl(target, ctx):
  p_val = 'In base_aspect, p = {} on target {}'.format(ctx.rule.attr.p, target.label)
  return [BaseAspectProv(p_val = p_val)]
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  requires = [required_aspect],
)

def _main_rule_impl(ctx):
  return [ctx.attr.dep[RequiredAspectProv], ctx.attr.dep[BaseAspectProv]]
def _dep_rule_impl(ctx):
  pass

main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects=[base_aspect]),
  },
)

dep_rule = rule(
  implementation = _dep_rule_impl,
  attrs = {
    'p' : attr.string(values = ['p_v1', 'p_v2']),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
  p = 'p_v2',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // Both base_aspect and required_aspect can see the attributes of the rule they run on
    let configured_target = analysis_result.only_target_to_build();
    let required_aspect_prov = starlark_key("//test:defs.bzl", "RequiredAspectProv");
    let required_aspect_provider = configured_target
        .get(&required_aspect_prov)
        .expect("RequiredAspectProv");
    assert_eq!(
        get_value(&required_aspect_provider, "p_val"),
        Some(s(
            "In required_aspect, p = p_v2 on target @@//test:dep_target"
        ))
    );
    let base_aspect_prov = starlark_key("//test:defs.bzl", "BaseAspectProv");
    let base_aspect_provider = configured_target
        .get(&base_aspect_prov)
        .expect("BaseAspectProv");
    assert_eq!(
        get_value(&base_aspect_provider, "p_val"),
        Some(s("In base_aspect, p = p_v2 on target @@//test:dep_target"))
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_inheritPropagationAttributes
#[test]
fn test_aspect_requires_aspect_inherit_propagation_attributes() {
    let mut t = new_test();
    // base_aspect propagates over base_dep attribute and requires first_required_aspect which
    // propagates over first_dep attribute and requires second_required aspect which propagates
    // over second_dep attribute
    t.scratch.file(
        "test/defs.bzl",
        &[r"BaseAspectProv = provider()
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

def _main_rule_impl(ctx):
  return [ctx.attr.dep[BaseAspectProv]]
def _dep_rule_impl(ctx):
  pass

main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects=[base_aspect]),
  },
)

dep_rule = rule(
  implementation = _dep_rule_impl,
  attrs = {
    'base_dep': attr.label(),
    'first_dep': attr.label(),
    'second_dep': attr.label()
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
  base_dep = ':base_dep_target',
  first_dep = ':first_dep_target',
  second_dep = ':second_dep_target',
)
dep_rule(
  name = 'base_dep_target',
)
dep_rule(
  name = 'first_dep_target',
)
dep_rule(
  name = 'second_dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // base_aspect should propagate only along its attr_aspects: 'base_dep'
    // first_required_aspect should propagate along 'base_dep' and 'first_dep'
    // second_required_aspect should propagate along 'base_dep', 'first_dep' and `second_dep`
    let configured_target = analysis_result.only_target_to_build();
    let base_aspect_prov = starlark_key("//test:defs.bzl", "BaseAspectProv");
    let base_aspect_provider = configured_target
        .get(&base_aspect_prov)
        .expect("BaseAspectProv");
    assert_contains_exactly(
        &get_sequence(&base_aspect_provider, "result"),
        &[
            s("second_required_aspect run on target @@//test:second_dep_target"),
            s("second_required_aspect run on target @@//test:dep_target"),
            s("second_required_aspect run on target @@//test:first_dep_target"),
            s("first_required_aspect run on target @@//test:first_dep_target"),
            s("first_required_aspect run on target @@//test:dep_target"),
            s("second_required_aspect run on target @@//test:base_dep_target"),
            s("first_required_aspect run on target @@//test:base_dep_target"),
            s("base_aspect run on target @@//test:base_dep_target"),
            s("base_aspect run on target @@//test:dep_target"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_inheritRequiredProviders
#[test]
fn test_aspect_requires_aspect_inherit_required_providers() {
    let mut t = new_test();
    // aspect_a requires provider Prov_A and requires aspect_b which requires
    // provider Prov_B and requires aspect_c which requires provider Prov_C
    t.scratch.file(
        "test/defs.bzl",
        &[
            r"Prov_A = provider()",
            r"Prov_B = provider()",
            r"Prov_C = provider()",
            r"",
            r"CollectorProv = provider()",
            r"",
            r"def _aspect_c_impl(target, ctx):",
            r"  collector_result = ['aspect_c run on target {} and value of Prov_C = {}'.format(target.label, target[Prov_C].val)]",
            r"  return [CollectorProv(result = collector_result)]",
            r"aspect_c = aspect(",
            r"  implementation = _aspect_c_impl,",
            r"  required_providers = [Prov_C],",
            r"  attr_aspects = ['dep'],",
            r")",
            r"",
            r"def _aspect_b_impl(target, ctx):",
            r"  collector_result = []",
            r"  collector_result += ctx.rule.attr.dep[CollectorProv].result",
            r"  collector_result += ['aspect_b run on target {} and value of Prov_B = {}'.format(target.label, target[Prov_B].val)]",
            r"  return [ CollectorProv(result = collector_result)]",
            r"aspect_b = aspect(",
            r"  implementation = _aspect_b_impl,",
            r"  required_providers = [Prov_B],",
            r"  requires = [aspect_c],",
            r"  attr_aspects = ['dep'],",
            r")",
            r"",
            r"def _aspect_a_impl(target, ctx):",
            r"  collector_result = []",
            r"  collector_result += ctx.rule.attr.dep[CollectorProv].result",
            r"  collector_result += ['aspect_a run on target {} and value of Prov_A = {}'.format(target.label, target[Prov_A].val)]",
            r"  return [CollectorProv(result = collector_result)]",
            r"aspect_a = aspect(",
            r"  implementation = _aspect_a_impl,",
            r"  attr_aspects = ['dep'],",
            r"  required_providers = [Prov_A],",
            r"  requires = [aspect_b],",
            r")",
            r"",
            r"def _main_rule_impl(ctx):",
            r"  return [ctx.attr.dep[CollectorProv]]",
            r"main_rule = rule(",
            r"  implementation = _main_rule_impl,",
            r"  attrs = {",
            r"    'dep': attr.label(aspects = [aspect_a]),",
            r"  },",
            r")",
            r"",
            r"def _rule_with_prov_a_impl(ctx):",
            r"  return [Prov_A(val='val_a')]",
            r"rule_with_prov_a = rule(",
            r"  implementation = _rule_with_prov_a_impl,",
            r"  attrs = {",
            r"    'dep': attr.label(),",
            r"  },",
            r"  provides = [Prov_A]",
            r")",
            r"",
            r"def _rule_with_prov_b_impl(ctx):",
            r"  return [Prov_B(val = 'val_b')]",
            r"rule_with_prov_b = rule(",
            r"  implementation = _rule_with_prov_b_impl,",
            r"  attrs = {",
            r"    'dep': attr.label(),",
            r"  },",
            r"  provides = [Prov_B]",
            r")",
            r"",
            r"def _rule_with_prov_c_impl(ctx):",
            r"  return [Prov_C(val = 'val_c')]",
            r"rule_with_prov_c = rule(",
            r"  implementation = _rule_with_prov_c_impl,",
            r"  provides = [Prov_C]",
            r")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:defs.bzl', 'main_rule', 'rule_with_prov_a', 'rule_with_prov_b', 'rule_with_prov_c')",
            r"main_rule(",
            r"  name = 'main',",
            r"  dep = ':target_with_prov_a',",
            r")",
            r"rule_with_prov_a(",
            r"  name = 'target_with_prov_a',",
            r"  dep = ':target_with_prov_b'",
            r")",
            r"rule_with_prov_b(",
            r"  name = 'target_with_prov_b',",
            r"  dep = ':target_with_prov_c'",
            r")",
            r"rule_with_prov_c(",
            r"  name = 'target_with_prov_c'",
            r")",
        ],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    // aspect_a should only run on target_with_prov_a, aspect_b should only run on
    // target_with_prov_b and aspect_c should only run on target_with_prov_c.
    // aspect_c will reach target target_with_prov_c because it inherits the required_providers of
    // aspect_b otherwise it would have stopped propagating after target_with_prov_b.
    let configured_target = analysis_result.only_target_to_build();
    let collector_prov = starlark_key("//test:defs.bzl", "CollectorProv");
    let collector_provider = configured_target
        .get(&collector_prov)
        .expect("CollectorProv");
    assert_contains_exactly_in_order(
        &get_sequence(&collector_provider, "result"),
        &[
            s("aspect_c run on target @@//test:target_with_prov_c and value of Prov_C = val_c"),
            s("aspect_b run on target @@//test:target_with_prov_b and value of Prov_B = val_b"),
            s("aspect_a run on target @@//test:target_with_prov_a and value of Prov_A = val_a"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_inspectRequiredAspectActions
#[test]
fn test_aspect_requires_aspect_inspect_required_aspect_actions() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"def _required_aspect_impl(target, ctx):
  f = ctx.actions.declare_file('dummy.txt')
  ctx.actions.run_shell(outputs = [f], command='echo xxx > $(location f)',
                        mnemonic='RequiredAspectAction')
  return []
required_aspect = aspect(
  implementation = _required_aspect_impl,
)

AspectInfo = provider()
def _base_aspect_impl(target, ctx):
  required_aspect_action = None
  for action in target.actions:
    if action.mnemonic == 'RequiredAspectAction':
      required_aspect_action = action
  if required_aspect_action:
    return AspectInfo(result = 'base_aspect can see required_aspect action')
  else:
    return AspectInfo(result = 'base_aspect cannot see required_aspect action')
base_aspect = aspect(
  implementation = _base_aspect_impl,
  attr_aspects = ['dep'],
  requires = [required_aspect]
)

RuleInfo = provider()
def _main_rule_impl(ctx):
  return RuleInfo(result = ctx.attr.dep[AspectInfo].result)
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects = [base_aspect]),
  },
)

def _dep_rule_impl(ctx):
  pass
dep_rule = rule(
  implementation = _dep_rule_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    let configured_target = analysis_result.only_target_to_build();
    let result = get_value(
        &get_starlark_provider(&mut t, &configured_target, "RuleInfo"),
        "result",
    );
    assert_eq!(
        result,
        Some(s("base_aspect can see required_aspect action"))
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_inspectRequiredAspectGeneratedFiles
#[test]
fn test_aspect_requires_aspect_inspect_required_aspect_generated_files() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[
            r"def _required_aspect_impl(target, ctx):",
            r"  file = ctx.actions.declare_file('required_aspect_file')",
            r"  ctx.actions.write(file, 'data')",
            r"  return [OutputGroupInfo(out = [file])]",
            r"required_aspect = aspect(",
            r"  implementation = _required_aspect_impl,",
            r")",
            r"",
            r"AspectInfo = provider()",
            r"def _base_aspect_impl(target, ctx):",
            r"  files = ['base_aspect can see file ' + f.path.split('/')[-1] for f in target[OutputGroupInfo].out.to_list()]",
            r"  return AspectInfo(my_files = files)",
            r"base_aspect = aspect(",
            r"  implementation = _base_aspect_impl,",
            r"  attr_aspects = ['dep'],",
            r"  requires = [required_aspect]",
            r")",
            r"",
            r"RuleInfo = provider()",
            r"def _main_rule_impl(ctx):",
            r"  return RuleInfo(my_files = ctx.attr.dep[AspectInfo].my_files)",
            r"main_rule = rule(",
            r"  implementation = _main_rule_impl,",
            r"  attrs = {",
            r"    'dep': attr.label(aspects = [base_aspect]),",
            r"  },",
            r")",
            r"",
            r"def _dep_rule_impl(ctx):",
            r"  pass",
            r"dep_rule = rule(",
            r"  implementation = _dep_rule_impl,",
            r")",
        ],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    let configured_target = analysis_result.only_target_to_build();
    let info = get_starlark_provider(&mut t, &configured_target, "RuleInfo");
    assert!(is_starlark_list(&info, "my_files"), "(StarlarkList) cast");
    assert_contains_exactly(
        &get_sequence(&info, "my_files"),
        &[s("base_aspect can see file required_aspect_file")],
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_withRequiredAspectProvidersSatisfied
#[test]
fn test_aspect_requires_aspect_with_required_aspect_providers_satisfied() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"prov_a = provider()
prov_b = provider()
prov_b_forwarded = provider()
AspectAInfo = provider()
AspectBInfo = provider()
RuleInfo = provider()
def _aspect_b_impl(target, ctx):
  result = 'aspect_b on target {} '.format(target.label)
  if prov_b in target:
    result += 'found prov_b = {}'.format(target[prov_b].val)
    return [AspectBInfo(aspect_b_result = result),
            prov_b_forwarded(val = target[prov_b].val)]
  else:
    result += 'cannot find prov_b'
    return [AspectBInfo(aspect_b_result = result)]
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
  return AspectAInfo(aspect_a_result = result)

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

def _main_rule_impl(ctx):
  return RuleInfo(aspect_a_result = ctx.attr.dep[AspectAInfo].aspect_a_result,
                aspect_b_result = ctx.attr.dep[AspectBInfo].aspect_b_result)
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_with_prov_a, aspect_with_prov_b, aspect_a]),
  },
)

def _dep_rule_impl(ctx):
  pass
dep_rule = rule(
  implementation = _dep_rule_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    let configured_target = analysis_result.only_target_to_build();
    let target_info = get_starlark_provider(&mut t, &configured_target, "RuleInfo");
    let aspect_a_result = get_value(&target_info, "aspect_a_result");
    assert_eq!(
        aspect_a_result,
        Some(s(
            "aspect_a on target @@//test:dep_target found prov_a = a1 and found prov_b = b1"
        ))
    );
    let aspect_b_result = get_value(&target_info, "aspect_b_result");
    assert_eq!(
        aspect_b_result,
        Some(s(
            "aspect_b on target @@//test:dep_target found prov_b = b1"
        ))
    );
}

// Java: StarlarkDefinedAspectsTest.testAspectRequiresAspect_withRequiredAspectProvidersNotFound
#[test]
fn test_aspect_requires_aspect_with_required_aspect_providers_not_found() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"prov_a = provider()
prov_b = provider()
AspectAInfo = provider()
AspectBInfo = provider()
def _aspect_b_impl(target, ctx):
  result = 'aspect_b on target {} '.format(target.label)
  if prov_b in target:
    result += 'found prov_b = {}'.format(target[prov_b].val)
  else:
    result += 'cannot find prov_b'
  return AspectBInfo(aspect_b_result = result)
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
  return AspectAInfo(aspect_a_result = result)

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

RuleInfo = provider()
def _main_rule_impl(ctx):
  return RuleInfo(aspect_a_result = ctx.attr.dep[AspectAInfo].aspect_a_result,
                aspect_b_result = ctx.attr.dep[AspectBInfo].aspect_b_result)
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'dep': attr.label(aspects = [aspect_with_prov_a, aspect_a]),
  },
)

def _dep_rule_impl(ctx):
  pass
dep_rule = rule(
  implementation = _dep_rule_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'main_rule', 'dep_rule')
main_rule(
  name = 'main',
  dep = ':dep_target',
)
dep_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update(&mut t, &["//test:main"]);
    let configured_target = analysis_result.only_target_to_build();
    let rule_info = get_starlark_provider(&mut t, &configured_target, "RuleInfo");
    let aspect_a_result = get_value(&rule_info, "aspect_a_result");
    assert_eq!(
        aspect_a_result,
        Some(s(
            "aspect_a on target @@//test:dep_target found prov_a = a1"
        ))
    );
    let aspect_b_result = get_value(&rule_info, "aspect_b_result");
    assert_eq!(
        aspect_b_result,
        Some(s(
            "aspect_b on target @@//test:dep_target cannot find prov_b"
        ))
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_stackOfAspects
#[test]
fn test_top_level_aspect_on_aspect_stack_of_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a2p = provider()
a1_result = provider()
a2_result = provider()
a3_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  if a2p in target:
    result += ' and sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' and cannot see a2p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  result = 'aspect a2 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  if a2p in target:
    result += ' and sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' and cannot see a2p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a2_result].value + [result]
  else:
    complete_result = [result]
  return [a2_result(value = complete_result), a1p(value = 'a1p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
  required_aspect_providers = [a2p],
)

def _a3_impl(target, ctx):
  result = 'aspect a3 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  if a2p in target:
    result += ' and sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' and cannot see a2p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a3_result].value + [result]
  else:
    complete_result = [result]
  return [a3_result(value = complete_result), a2p(value = 'a2p_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['dep'],
  provides = [a2p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a3", "a3_result", "value"),
        &[
            s("aspect a3 on target @@//test:dep_target cannot see a1p and cannot see a2p"),
            s("aspect a3 on target @@//test:main cannot see a1p and cannot see a2p"),
        ],
    );
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a2", "a2_result", "value"),
        &[
            s("aspect a2 on target @@//test:dep_target cannot see a1p and sees a2p = a2p_val"),
            s("aspect a2 on target @@//test:main cannot see a1p and sees a2p = a2p_val"),
        ],
    );
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a1p = a1p_val and cannot see a2p"),
            s("aspect a1 on target @@//test:main sees a1p = a1p_val and cannot see a2p"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_requiredProviderProvidedTwiceFailed
#[test]
fn test_top_level_aspect_on_aspect_required_provider_provided_twice_failed() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  return [a1p(value = 'a1p_a2_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _a3_impl(target, ctx):
  return [a1p(value = 'a1p_a3_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    t.remove_fail_fast_handler();
    // The call to `update` does not throw an exception when "--keep_going" is passed in the
    // WithKeepGoing test suite. Otherwise, it throws ViewCreationFailedException.
    update_expecting_failure(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
        "ViewCreationFailedException",
    );
    let expected = t.ws("ERROR /workspace/test/BUILD:2:12: Provider a1p provided twice");
    t.assert_contains_event(&expected);
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_requiredProviderProvidedTwicePassed
#[test]
fn test_top_level_aspect_on_aspect_required_provider_provided_twice_passed() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  return [a1p(value = 'a1p_a2_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _a3_impl(target, ctx):
  return [a1p(value = 'a1p_a3_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a1", "test/defs.bzl%a2"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a1p = a1p_a3_val"),
            s("aspect a1 on target @@//test:main sees a1p = a1p_a3_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_requiredProviderNotProvided
#[test]
fn test_top_level_aspect_on_aspect_required_provider_not_provided() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a2p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a2p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target cannot see a1p"),
            s("aspect a1 on target @@//test:main cannot see a1p"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_requiredProviderProvidedAfterTheAspect
#[test]
fn test_top_level_aspect_on_aspect_required_provider_provided_after_the_aspect() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  return [a1p(value = 'a1p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a1", "test/defs.bzl%a2"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target cannot see a1p"),
            s("aspect a1 on target @@//test:main cannot see a1p"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_differentAttrAspects
#[test]
fn test_top_level_aspect_on_aspect_different_attr_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result += ctx.rule.attr.dep[a1_result].value
  if ctx.rule.attr.extra_dep:
    complete_result += ctx.rule.attr.extra_dep[a1_result].value
  complete_result += [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep', 'extra_dep'],
  required_aspect_providers = [a1p]
)

def _a2_impl(target, ctx):
  return [a1p(value = 'a1p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  provides = [a1p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
    'extra_dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
  extra_dep = ':extra_dep_target',
)
simple_rule(
  name = 'dep_target',
)
simple_rule(
  name = 'extra_dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a1p = a1p_val"),
            s("aspect a1 on target @@//test:extra_dep_target cannot see a1p"),
            s("aspect a1 on target @@//test:main sees a1p = a1p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_differentRequiredRuleProviders
#[test]
fn test_top_level_aspect_on_aspect_different_required_rule_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a1_result = provider()
rule_prov_a = provider()
rule_prov_b = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  complete_result = []
  if hasattr(ctx.rule.attr, 'deps'):
    for dep in ctx.rule.attr.deps:
      complete_result += dep[a1_result].value
  complete_result += [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [a1p],
  required_providers = [[rule_prov_a], [rule_prov_b]],
)

def _a2_impl(target, ctx):
  return [a1p(value = 'a1p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a1p],
  required_providers = [rule_prov_a],
)

def _main_rule_impl(ctx):
  return [rule_prov_a(), rule_prov_b()]
main_rule = rule(
  implementation = _main_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [rule_prov_a, rule_prov_b],
)

def _rule_with_prov_a_impl(ctx):
  return [rule_prov_a()]
rule_with_prov_a = rule(
  implementation = _rule_with_prov_a_impl,
  provides = [rule_prov_a]
)

def _rule_with_prov_b_impl(ctx):
  return [rule_prov_b()]
rule_with_prov_b = rule(
  implementation = _rule_with_prov_b_impl,
  provides = [rule_prov_b]
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('//test:defs.bzl', 'main_rule', 'rule_with_prov_a', 'rule_with_prov_b')
main_rule(
  name = 'main',
  deps = [':target_with_prov_a', ':target_with_prov_b'],
)
rule_with_prov_a(
  name = 'target_with_prov_a',
)
rule_with_prov_b(
  name = 'target_with_prov_b',
)
",
        ],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:target_with_prov_a sees a1p = a1p_val"),
            s("aspect a1 on target @@//test:target_with_prov_b cannot see a1p"),
            s("aspect a1 on target @@//test:main sees a1p = a1p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_providerRequiredByMultipleAspects
#[test]
fn test_top_level_aspect_on_aspect_provider_required_by_multiple_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a3p = provider()
a1_result = provider()
a2_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a3p in target:
    result += ' sees a3p = {}'.format(target[a3p].value)
  else:
    result += ' cannot see a3p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a1_result].value + [result]
  else:
    complete_result = [result]
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a3p]
)

def _a2_impl(target, ctx):
  result = 'aspect a2 on target {}'.format(target.label)
  if a3p in target:
    result += ' sees a3p = {}'.format(target[a3p].value)
  else:
    result += ' cannot see a3p'
  complete_result = []
  if ctx.rule.attr.dep:
    complete_result = ctx.rule.attr.dep[a2_result].value + [result]
  else:
    complete_result = [result]
  return [a2_result(value = complete_result)]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['dep'],
  required_aspect_providers = [a3p]
)

def _a3_impl(target, ctx):
  return [a3p(value = 'a3p_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['dep'],
  provides = [a3p],
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'dep': attr.label(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  dep = ':dep_target',
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a2", "a2_result", "value"),
        &[
            s("aspect a2 on target @@//test:dep_target sees a3p = a3p_val"),
            s("aspect a2 on target @@//test:main sees a3p = a3p_val"),
        ],
    );
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a3p = a3p_val"),
            s("aspect a1 on target @@//test:main sees a3p = a3p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_diamondCase
#[test]
fn test_top_level_aspect_on_aspect_diamond_case() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a2p = provider()
a3_result = provider()

r1p = provider()
r2p = provider()

def _a1_impl(target, ctx):
  return [a1p(value = 'a1p_val')]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_providers = [r1p],
  provides = [a1p]
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  required_providers = [r2p],
  provides = [a2p]
)

def _a3_impl(target, ctx):
  result = 'aspect a3 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  if a2p in target:
    result += ' and sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' and cannot see a2p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a3_result].value)
  complete_result.append(result)
  return [a3_result(value = complete_result)]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [[a1p], [a2p]],
)

def _r0_impl(ctx):
  return [r1p(), r2p()]
r0 = rule(
  implementation = _r0_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [r1p, r2p]
)
def _r1_impl(ctx):
  return [r1p()]
r1 = rule(
  implementation = _r1_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [r1p]
)
def _r2_impl(ctx):
  return [r2p()]
r2 = rule(
  implementation = _r2_impl,
  attrs = {
    'deps': attr.label_list(),
  },
  provides = [r2p]
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'r0', 'r1', 'r2')
r0(
  name = 'main',
  deps = [':t1', ':t2'],
)
r1(
  name = 't1',
  deps = [':t0'],
)
r2(
  name = 't2',
  deps = [':t0'],
)
r0(
  name = 't0',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a1", "test/defs.bzl%a2", "test/defs.bzl%a3"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a3", "a3_result", "value"),
        &[
            s("aspect a3 on target @@//test:t0 sees a1p = a1p_val and cannot see a2p"),
            s("aspect a3 on target @@//test:t0 cannot see a1p and sees a2p = a2p_val"),
            s("aspect a3 on target @@//test:t1 sees a1p = a1p_val and cannot see a2p"),
            s("aspect a3 on target @@//test:t2 cannot see a1p and sees a2p = a2p_val"),
            s("aspect a3 on target @@//test:main sees a1p = a1p_val and sees a2p = a2p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_duplicateAspectsNotAllowed
#[test]
fn test_top_level_aspect_on_aspect_duplicate_aspects_not_allowed() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a2p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a2p in target:
    result += ' sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' cannot see a2p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a1_result].value)
  complete_result.append(result)
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [a2p]
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a2p]
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  deps = [':dep_target'],
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    t.remove_fail_fast_handler();
    // The call to `update` does not throw an exception when "--keep_going" is passed in the
    // WithKeepGoing test suite. Otherwise, it throws ViewCreationFailedException.
    update_expecting_failure(
        &mut t,
        &["test/defs.bzl%a1", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
        "ViewCreationFailedException",
    );
    t.assert_contains_event("aspect //test:defs.bzl%a1 added more than once");
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_requiredAspectProviderOnlyAvailableOnDep
#[test]
fn test_top_level_aspect_on_aspect_required_aspect_provider_only_available_on_dep() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a2p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a2p in target:
    result += ' sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' cannot see a2p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a1_result].value)
  complete_result.append(result)
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [a2p]
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a2p]
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'deps': attr.label_list(aspects=[a2]),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  deps = [':dep_target'],
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(&mut t, &["test/defs.bzl%a1"], &["//test:main"]);
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a2p = a2p_val"),
            s("aspect a1 on target @@//test:main cannot see a2p"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_multipleTopLevelTargets
#[test]
fn test_top_level_aspect_on_aspect_multiple_top_level_targets() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a2p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a2p in target:
    result += ' sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' cannot see a2p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a1_result].value)
  complete_result.append(result)
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [a2p],
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a2p]
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 't1',
)
simple_rule(
  name = 't2',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:t2", "//test:t1"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    let a1_ont1 = get_configured_aspect_on(&configured_aspects, "a1", "t1").expect("a1 on t1");
    let a1_result = starlark_key("//test:defs.bzl", "a1_result");
    let a1_result_provider = a1_ont1.get(&a1_result).expect("a1_result");
    assert_contains_exactly(
        &get_sequence(&a1_result_provider, "value"),
        &[s("aspect a1 on target @@//test:t1 sees a2p = a2p_val")],
    );
    let a1_ont2 = get_configured_aspect_on(&configured_aspects, "a1", "t2").expect("a1 on t2");
    let a1_result_provider = a1_ont2.get(&a1_result).expect("a1_result");
    assert_contains_exactly(
        &get_sequence(&a1_result_provider, "value"),
        &[s("aspect a1 on target @@//test:t2 sees a2p = a2p_val")],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_multipleRequiredProviders
#[test]
fn test_top_level_aspect_on_aspect_multiple_required_providers() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a2p = provider()
a3p = provider()
a1_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a2p in target:
    result += ' sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' cannot see a2p'
  if a3p in target:
    result += ' and sees a3p = {}'.format(target[a3p].value)
  else:
    result += ' and cannot see a3p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a1_result].value)
  complete_result.append(result)
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [[a2p], [a3p]],
)

def _a2_impl(target, ctx):
  return [a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a2p]
)

def _a3_impl(target, ctx):
  return [a3p(value = 'a3p_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['deps'],
  provides = [a3p]
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  deps = [':dep_target'],
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a2p = a2p_val and sees a3p = a3p_val"),
            s("aspect a1 on target @@//test:main sees a2p = a2p_val and sees a3p = a3p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectOnAspect_multipleRequiredProviders2
#[test]
fn test_top_level_aspect_on_aspect_multiple_required_providers2() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a2p = provider()
a3p = provider()
a1_result = provider()
a2_result = provider()

def _a1_impl(target, ctx):
  result = 'aspect a1 on target {}'.format(target.label)
  if a2p in target:
    result += ' sees a2p = {}'.format(target[a2p].value)
  else:
    result += ' cannot see a2p'
  if a3p in target:
    result += ' and sees a3p = {}'.format(target[a3p].value)
  else:
    result += ' and cannot see a3p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a1_result].value)
  complete_result.append(result)
  return [a1_result(value = complete_result)]
a1 = aspect(
  implementation = _a1_impl,
  attr_aspects = ['deps'],
  required_aspect_providers = [[a2p], [a3p]],
)

def _a2_impl(target, ctx):
  result = 'aspect a2 on target {}'.format(target.label)
  if a3p in target:
    result += ' sees a3p = {}'.format(target[a3p].value)
  else:
    result += ' cannot see a3p'
  complete_result = []
  if ctx.rule.attr.deps:
    for dep in ctx.rule.attr.deps:
      complete_result.extend(dep[a2_result].value)
  complete_result.append(result)
  return [a2_result(value = complete_result), a2p(value = 'a2p_val')]
a2 = aspect(
  implementation = _a2_impl,
  attr_aspects = ['deps'],
  provides = [a2p],
  required_aspect_providers = [a3p]
)

def _a3_impl(target, ctx):
  return [a3p(value = 'a3p_val')]
a3 = aspect(
  implementation = _a3_impl,
  attr_aspects = ['deps'],
  provides = [a3p]
)

def _simple_rule_impl(ctx):
  pass
simple_rule = rule(
  implementation = _simple_rule_impl,
  attrs = {
    'deps': attr.label_list(),
  },
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'simple_rule')
simple_rule(
  name = 'main',
  deps = [':dep_target'],
)
simple_rule(
  name = 'dep_target',
)
"],
    );
    let analysis_result = update_with_aspects(
        &mut t,
        &["test/defs.bzl%a3", "test/defs.bzl%a2", "test/defs.bzl%a1"],
        &["//test:main"],
    );
    let configured_aspects = analysis_result.get_aspects_map();
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a1", "a1_result", "value"),
        &[
            s("aspect a1 on target @@//test:dep_target sees a2p = a2p_val and sees a3p = a3p_val"),
            s("aspect a1 on target @@//test:main sees a2p = a2p_val and sees a3p = a3p_val"),
        ],
    );
    assert_contains_exactly(
        &aspect_result_sequence(&configured_aspects, "a2", "a2_result", "value"),
        &[
            s("aspect a2 on target @@//test:dep_target sees a3p = a3p_val"),
            s("aspect a2 on target @@//test:main sees a3p = a3p_val"),
        ],
    );
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspects_duplicateRuleProviderError
#[test]
fn test_top_level_aspects_duplicate_rule_provider_error() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"a1p = provider()
a2p = provider()

def _a1_impl(target, ctx):
  return [a1p(value = 'aspect_a1p_val')]
a1 = aspect(
  implementation = _a1_impl,
  provides = [a1p],
)

def _a2_impl(target, ctx):
  result = 'aspect a2 on target {}'.format(target.label)
  if a1p in target:
    result += ' sees a1p = {}'.format(target[a1p].value)
  else:
    result += ' cannot see a1p'
  return [a2p(value = result)]
a2 = aspect(
  implementation = _a2_impl,
  provides = [a2p],
  required_aspect_providers = [a1p]
)

def _my_rule_impl(ctx):
  return [a1p(value = 'rule_a1p_val')]
my_rule = rule(
  implementation = _my_rule_impl,
)
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[r"load('//test:defs.bzl', 'my_rule')
my_rule(name = 'main')
"],
    );
    t.remove_fail_fast_handler();
    // The call to `update` does not throw an exception when "--keep_going" is passed in the
    // WithKeepGoing test suite. Otherwise, it throws ViewCreationFailedException.
    update_expecting_failure(
        &mut t,
        &["test/defs.bzl%a1", "test/defs.bzl%a2"],
        &["//test:main"],
        "ViewCreationFailedException",
    );
    t.assert_contains_event("Provider a1p provided twice");
}

// Java: StarlarkDefinedAspectsTest.testTopLevelAspectRequiresAspect_stackOfRequiredAspects
#[test]
fn test_top_level_aspect_requires_aspect_stack_of_required_aspects() {
    let mut t = new_test();
    t.scratch.file(
        "test/defs.bzl",
        &[r"def _impl(target, ctx):
   return []
aspect_c = aspect(implementation = _impl)
aspect_b = aspect(implementation = _impl, requires = [aspect_c])
aspect_a = aspect(implementation = _impl, requires = [aspect_b])
"],
    );
    t.scratch.file(
        "test/BUILD",
        &[
            r"load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
            r"cc_binary(name = 'main_target')",
        ],
    );
    let analysis_result =
        update_with_aspects(&mut t, &["test/defs.bzl%aspect_a"], &["//test:main_target"]);
    let configured_aspects = analysis_result.get_aspects_map();
    assert_eq!(configured_aspects.len(), 3);
    assert!(get_configured_aspect(&configured_aspects, "aspect_a").is_some());
    assert!(get_configured_aspect(&configured_aspects, "aspect_b").is_some());
    assert!(get_configured_aspect(&configured_aspects, "aspect_c").is_some());
}
