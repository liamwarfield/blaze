//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkAspectsPropagationPredicateTest.java`
//! (an `AnalysisTestCase`): Starlark aspects' `propagation_predicate`.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_events::EventKind;
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_options::Options;
use blaze_packages::aspect::AspectClass;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::structs::StarlarkInfo;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use starlark::values::list::ListRef;

use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::TargetPatternPhaseError;
use crate::testing::build_view_test_case::{LOADING_PHASE_THREADS, parse_label};
use crate::testing::{AnalysisTestCase, BuildViewTestCase, assert_contains_exactly};

fn new_test() -> AnalysisTestCase {
    BuildViewTestCase::analysis_test_case()
}

/// Java `String.format(template, args...)` for templates whose only directives are `%s`.
fn java_format(template: &str, args: &[&str]) -> String {
    let mut parts = template.split("%s");
    let mut out = parts.next().unwrap_or_default().to_string();
    for (part, arg) in parts.zip(args.iter()) {
        out.push_str(arg);
        out.push_str(part);
    }
    out
}

/// Java `AnalysisTestCase.update(aspects, aspectsParameters, labels...)`.
///
/// PORT: the harness has no entry point with `aspectsParameters` (its `update_with` passes
/// none), so this private helper repeats its `update_internal` with
/// `BuildView.update`'s `aspectsParameters` argument
/// ([`build_view::update_with_aspects_parameters`]). It returns the result instead of storing
/// it as `analysisResult`. The fail-fast check is the harness's: an error event fails the test
/// (every caller keeps the fail-fast handler).
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:478 AnalysisTestCase.update
fn update_with_aspects_parameters(
    t: &mut AnalysisTestCase,
    aspects: &[&str],
    aspects_parameters: &[(&str, &str)],
    labels: &[&str],
) -> AnalysisResult {
    t.event_bus.clear();
    let handler = t.event_handler();
    let build_options = t.get_target_configuration().get_options().clone();
    let loading_options: LoadingOptions = Options::<LoadingOptions>::get_defaults();
    let analysis_options: AnalysisOptions = Options::<AnalysisOptions>::get_defaults();
    let targets: Vec<String> = labels.iter().map(|s| s.to_string()).collect();
    let executor = t.skyframe_executor();
    let loading_result = match executor.load_target_patterns_with_filters(
        handler.clone(),
        &targets,
        &PathFragment::create(""),
        &loading_options,
        LOADING_PHASE_THREADS,
        /* keep_going= */ false,
        /* determine_tests= */ false,
    ) {
        Ok(v) => v,
        Err(TargetPatternPhaseError::TargetParsing(e)) => {
            panic!("TargetParsingException: {}", e.get_message())
        }
        Err(TargetPatternPhaseError::Interrupted) => panic!("InterruptedException"),
    };
    let options = BuildViewUpdateOptions {
        keep_going: false,
        check_for_action_conflicts: true,
        aspects: aspects.iter().map(|s| s.to_string()).collect(),
        explicit_target_patterns: IndexSet::new(),
        skip_incompatible_explicit_targets: analysis_options.skip_incompatible_explicit_targets,
        report_incompatible_targets: true,
        include_execution_phase: false,
    };
    let parameters: IndexMap<String, String> = aspects_parameters
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let result = build_view::update_with_aspects_parameters(
        executor,
        &loading_result,
        &build_options,
        &options,
        &parameters,
        handler,
    );
    // failFastHandler.
    for event in t.event_collector.iter() {
        if EventKind::ERRORS.contains(event.get_kind()) {
            panic!("{event}");
        }
    }
    match result {
        Ok(r) => r,
        Err(e) => panic!("{}: {e}", view_error_kind(&e)),
    }
}

fn view_error_kind(e: &BuildViewError) -> &'static str {
    match e {
        BuildViewError::ViewCreationFailed { .. } => "ViewCreationFailedException",
        BuildViewError::InvalidConfiguration(_) => "InvalidConfigurationException",
        BuildViewError::Interrupted => "InterruptedException",
    }
}

// Java: StarlarkAspectsPropagationPredicateTest.createTestDefs
fn create_test_defs(t: &mut AnalysisTestCase, propagation_predicate: &str) {
    t.scratch.file(
        "test/BUILD",
        &["load('//test:defs.bzl', 'rule_1')\nrule_1(name = 'tool')\n"],
    );
    t.scratch.file(
        "test/defs.bzl",
        &[&java_format(
            r#"AspectInfo = provider()
RuleInfo = provider()

%s

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  for dep in ctx.rule.attr.deps:
    if AspectInfo in dep:
      res.extend(dep[AspectInfo].res)
  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
  implementation = _aspect_impl,
  propagation_predicate = _propagation_predicate,
  attrs = {
    'allowed_pkg': attr.string(default = '*'),
    'ignored_tag': attr.string(default = ''),
    'allowed_rule_file': attr.string(default = '*'),
    'allowed_rule_name': attr.string(default = '*'),
    '_tool': attr.label(default = Label('//test:tool'))},
  attr_aspects = ['deps'],
)

aspect_with_required_provider = aspect(
    implementation = _aspect_impl,
    required_providers = [RuleInfo],
    propagation_predicate = _propagation_predicate,
    attr_aspects = ['deps'],
)

def _rule_impl(ctx):
  pass

rule_1 = rule(
  implementation = _rule_impl,
  attrs = {'deps': attr.label_list()},
)

rule_2 = rule(
  implementation = _rule_impl,
  attrs = {'deps': attr.label_list()},
)

def _out_rule_impl(ctx):
  if ctx.outputs.out:
    ctx.actions.write(ctx.outputs.out, 'hi')
  return []

out_rule = rule(
  implementation = _out_rule_impl,
  attrs = {'deps': attr.label_list(), 'out': attr.output()},
)

def _rule_with_provider_impl(ctx):
  return [RuleInfo()]

rule_with_provider = rule(
  implementation = _rule_with_provider_impl,
  attrs = {'deps': attr.label_list()},
  provides = [RuleInfo],
)

aspect_on_rule = aspect(
    implementation = _aspect_impl,
    propagation_predicate = _propagation_predicate,
    attr_aspects = ['deps'],
)

rule_with_aspect_on_deps = rule(
    implementation = _rule_impl,
    attrs = {'deps': attr.label_list(aspects = [aspect_on_rule])},
)
"#,
            &[propagation_predicate],
        )],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.createTestPackages
fn create_test_packages(t: &mut AnalysisTestCase) {
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'rule_1', 'rule_2', 'out_rule', 'rule_with_aspect_on_deps')
rule_1(name = 't1', deps = [':t2', '//pkg2:t2'], tags = ['no-aspect'])
rule_2(name = 't2', tags = ['another-tag'])
out_rule(name = 'target_with_output', out = 'my_out.txt', deps = [':t2'])
rule_with_aspect_on_deps(name = 'target_with_aspect_on_deps', deps = [':t2', '//pkg2:t2'])
alias(name = 'alias_1', actual = ':alias_2')
alias(name = 'alias_2', actual = ':actual')
rule_1(name = 'actual')
"#],
    );
    t.scratch.file(
        "pkg2/BUILD",
        &[r#"load('//test:defs.bzl', 'rule_1', 'rule_2', 'rule_with_provider')
rule_2(name = 't1', deps = [':t2', '//pkg1:t2'])
rule_1(name = 't2', tags = ['no-aspect', 'another-tag'])
rule_with_provider(name = 'target_with_provider', deps = [':t2'])
"#],
    );
}

/// `Splitter.on("%").splitToList(aspectClass.toString()).get(1)`.
fn aspect_name_of(aspect_class: &AspectClass) -> String {
    aspect_class
        .to_string()
        .split('%')
        .nth(1)
        .expect("IndexOutOfBoundsException")
        .to_string()
}

// Java: StarlarkAspectsPropagationPredicateTest.formatAspectKey
fn format_aspect_key(aspect_key: &AspectKey) -> String {
    if aspect_key.get_base_keys().is_empty() {
        return format!(
            "{} on {}",
            aspect_name_of(aspect_key.get_aspect_class()),
            aspect_key.get_label()
        );
    }

    let base_aspects = aspect_key
        .get_base_keys()
        .iter()
        .map(|k| aspect_name_of(k.get_aspect_class()))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{} on {} with base aspects: {}",
        aspect_name_of(aspect_key.get_aspect_class()),
        aspect_key.get_label(),
        base_aspects
    )
}

// Java: StarlarkAspectsPropagationPredicateTest.getFormattedAspectKeys
fn get_formatted_aspect_keys(t: &mut AnalysisTestCase, aspect_label: &str) -> Vec<String> {
    let evaluator = t.get_evaluator();
    evaluator
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|key| evaluator.get_existing_value(key).is_some())
        .filter_map(|key| key.downcast::<AspectKey>().cloned())
        .filter(|key| key.get_aspect_class().to_string() == aspect_label)
        .map(|key| format_aspect_key(&key))
        .collect()
}

/// `getStarlarkProvider(aspect, label, providerName).getValue("res", Sequence.class)` as
/// strings.
fn get_res(aspect_value: &AspectValue, bzl_label: &str, provider_name: &str) -> Vec<String> {
    let key = ProviderKey::Starlark {
        extension_label: parse_label(bzl_label),
        exported_name: provider_name.to_string(),
    };
    let info = aspect_value
        .get_providers()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| panic!("NullPointerException: no provider {provider_name}"));
    let info = StarlarkInfo::from_value(info.value()).expect("a StarlarkInfo");
    let res = info.get_field("res").expect("field res");
    ListRef::from_value(res)
        .expect("a Sequence")
        .iter()
        .map(|v| v.unpack_str().expect("a string").to_string())
        .collect()
}

// Java: StarlarkAspectsPropagationPredicateTest.getAspectResult
fn get_aspect_result(
    aspects_map: &IndexMap<AspectKey, Arc<AspectValue>>,
    aspect_name: &str,
    target_label: Option<&str>,
    provider_name: &str,
) -> Option<Vec<String>> {
    for (key, value) in aspects_map {
        if let AspectClass::Starlark(starlark_aspect_class) = key.get_aspect_class() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            if aspect_exported_name == aspect_name
                && target_label.is_none_or(|l| key.get_label().to_string() == l)
            {
                return Some(get_res(value, "//test:defs.bzl", provider_name));
            }
        }
    }
    None
}

fn get_aspect_result_default(
    aspects_map: &IndexMap<AspectKey, Arc<AspectValue>>,
    aspect_name: &str,
) -> Vec<String> {
    get_aspect_result(aspects_map, aspect_name, None, "AspectInfo")
        .expect("assertThat(null): aspect result not found")
}

// Java: StarlarkAspectsPropagationPredicateTest.propagationPredicateOnTargetPackage_aspectPropagatedToSatisfyingTargets
#[test]
fn propagation_predicate_on_target_package_aspect_propagated_to_satisfying_targets() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_predicate(ctx):
  if ctx.attr.allowed_pkg != '*' and ctx.rule.label.package != ctx.attr.allowed_pkg:
    return False
  return True
"#,
    );
    create_test_packages(&mut t);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%cmdline_aspect"],
        &[("allowed_pkg", "pkg1")],
        &["//pkg1:t1", "//pkg2:t1"],
    );

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%cmdline_aspect");
    // Only the keys to the targets that satisfy the aspect's propagation predicate are present.
    assert_contains_exactly(
        &aspect_keys,
        &["cmdline_aspect on //pkg1:t1", "cmdline_aspect on //pkg1:t2"],
    );

    let aspect_result =
        get_aspect_result_default(analysis_result.get_aspects_map(), "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:t1",
            "cmdline_aspect on @@//pkg1:t2",
        ],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.propagationPredicateOnTargetTags_aspectPropagatedToSatisfyingTargets
#[test]
fn propagation_predicate_on_target_tags_aspect_propagated_to_satisfying_targets() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_predicate(ctx):
  if ctx.attr.ignored_tag != '' and ctx.attr.ignored_tag in ctx.rule.attr.tags.value:
    return False
  return True
"#,
    );
    create_test_packages(&mut t);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%cmdline_aspect"],
        &[("ignored_tag", "no-aspect")],
        &["//pkg1:t1", "//pkg2:t1"],
    );

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%cmdline_aspect");
    // Only the keys to the targets that satisfy the aspect's propagation predicate are present.
    assert_contains_exactly(
        &aspect_keys,
        &["cmdline_aspect on //pkg2:t1", "cmdline_aspect on //pkg1:t2"],
    );

    let aspect_result =
        get_aspect_result_default(analysis_result.get_aspects_map(), "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg2:t1",
            "cmdline_aspect on @@//pkg1:t2",
        ],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.propagationPredicateOnRuleKind_aspectPropagatedToSatisfyingTargets
#[test]
fn propagation_predicate_on_rule_kind_aspect_propagated_to_satisfying_targets() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_predicate(ctx):
  qualified_kind = ctx.rule.qualified_kind
  allowed_rule_file = ctx.attr.allowed_rule_file
  allowed_rule_name = ctx.attr.allowed_rule_name

  if allowed_rule_file != '*' and qualified_kind.file_label != Label(allowed_rule_file):
    return False

  if allowed_rule_name != '*' and qualified_kind.rule_name != allowed_rule_name:
    return False

  return True
"#,
    );
    create_test_packages(&mut t);

    let analysis_result = update_with_aspects_parameters(
        &mut t,
        &["//test:defs.bzl%cmdline_aspect"],
        &[
            ("allowed_rule_file", "@@//test:defs.bzl"),
            ("allowed_rule_name", "rule_1"),
        ],
        &["//pkg1:t1", "//pkg2:t1"],
    );

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%cmdline_aspect");
    // Only the keys to the targets that satisfy the aspect's propagation predicate are present.
    assert_contains_exactly(
        &aspect_keys,
        &["cmdline_aspect on //pkg1:t1", "cmdline_aspect on //pkg2:t2"],
    );

    let aspect_result =
        get_aspect_result_default(analysis_result.get_aspects_map(), "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:t1",
            "cmdline_aspect on @@//pkg2:t2",
        ],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.accessPrivateAspectAttribute_fails
#[test]
fn access_private_aspect_attribute_fails() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        "def _propagation_predicate(ctx):\n  tool = ctx.attr._tool\n  return True\n",
    );
    create_test_packages(&mut t);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event("'_tool' is not a public parameter of the aspect.");
}

// Java: StarlarkAspectsPropagationPredicateTest.aspectOnOutputFile_propagationPredicateNotUsed
#[test]
fn aspect_on_output_file_propagation_predicate_not_used() {
    let mut t = new_test();
    create_test_defs(&mut t, "def _propagation_predicate(ctx):\n  return False\n");
    create_test_packages(&mut t);

    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:my_out.txt"])
        .unwrap();

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%cmdline_aspect");
    // The propagation predicate is not used for output files, so the aspect key is created even
    // if the propagation predicate is not satisfied.
    assert_contains_exactly(&aspect_keys, &["cmdline_aspect on //pkg1:my_out.txt"]);
}

// Java: StarlarkAspectsPropagationPredicateTest.aspectPropagatedFromRule_propagationPredicateIsUsed
#[test]
fn aspect_propagated_from_rule_propagation_predicate_is_used() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_predicate(ctx):
  if ctx.rule.label == Label('//pkg1:t2'):
    return False
  return True
"#,
    );
    create_test_packages(&mut t);

    t.update(&["//pkg1:target_with_aspect_on_deps"]).unwrap();

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_on_rule");
    // The propagation predicate is used for the aspect_on_rule, so the aspect key is not created
    // for //pkg1:t2.
    assert_contains_exactly(&aspect_keys, &["aspect_on_rule on //pkg2:t2"]);
}

// Java: StarlarkAspectsPropagationPredicateTest.aspectOnAlias_propagationPredicateRunOnActualTarget
#[test]
fn aspect_on_alias_propagation_predicate_run_on_actual_target() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_predicate(ctx):
  if ctx.rule.qualified_kind.rule_name == 'rule_1':
    return True
  return False
"#,
    );
    create_test_packages(&mut t);

    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:alias_1"])
        .unwrap();

    let aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%cmdline_aspect");
    // The propagation predicate is evaluated on the actual target of the alias which in this case
    // satisfies the predicate.
    assert_contains_exactly(
        &aspect_keys,
        &[
            "cmdline_aspect on //pkg1:alias_1",
            "cmdline_aspect on //pkg1:alias_2",
            "cmdline_aspect on //pkg1:actual",
        ],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.requiredProviderSatisfied_propagationPredicateNotSatisfied_aspectNotPropagated
#[test]
fn required_provider_satisfied_propagation_predicate_not_satisfied_aspect_not_propagated() {
    let mut t = new_test();
    create_test_defs(&mut t, "def _propagation_predicate(ctx):\n  return False\n");
    create_test_packages(&mut t);

    t.update_with_aspects(
        &["//test:defs.bzl%aspect_with_required_provider"],
        &["//pkg2:target_with_provider"],
    )
    .unwrap();

    let aspect_keys =
        get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_with_required_provider");
    // The propagation predicate is not satisfied, so the aspect is not propagated even though
    // its required provider is satisfied.
    assert!(aspect_keys.is_empty(), "{aspect_keys:?}");
}

// Java: StarlarkAspectsPropagationPredicateTest.requiredProviderSatisfied_propagationPredicateSatisfied_aspectPropagated
#[test]
fn required_provider_satisfied_propagation_predicate_satisfied_aspect_propagated() {
    let mut t = new_test();
    create_test_defs(&mut t, "def _propagation_predicate(ctx):\n  return True\n");
    create_test_packages(&mut t);

    t.update_with_aspects(
        &["//test:defs.bzl%aspect_with_required_provider"],
        &["//pkg2:target_with_provider"],
    )
    .unwrap();

    let aspect_keys =
        get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_with_required_provider");
    // The propagation predicate and the required provider are satisfied, so the aspect is
    // propagated to the target.
    assert_contains_exactly(
        &aspect_keys,
        &["aspect_with_required_provider on //pkg2:target_with_provider"],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.aspectOnAspect_eachPropagationPredicateEvaluatedSeparately
#[test]
fn aspect_on_aspect_each_propagation_predicate_evaluated_separately() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"CInfo = provider()
BInfo = provider()
AInfo = provider()

def _aspect_a_propagation_predicate(ctx):
  return True

def _aspect_b_propagation_predicate(ctx):
  if ctx.rule.label.package == 'pkg1':
    return True
  return False

def _aspect_c_propagation_predicate(ctx):
  return True

def _aspect_a_impl(target, ctx):
  res = 'aspect_a on %s' % target.label
  if BInfo in target:
    res += ' with BInfo'
  if CInfo in target:
    res += ' with CInfo'

  res_list = [res]

  for dep in ctx.rule.attr.deps:
    if AInfo in dep:
      res_list.extend(dep[AInfo].res)

  return [AInfo(res = res_list)]

def _aspect_b_impl(target, ctx):
  return [BInfo()]

def _aspect_c_impl(target, ctx):
  return [CInfo()]

aspect_a = aspect(
  implementation = _aspect_a_impl,
  propagation_predicate = _aspect_a_propagation_predicate,
  attr_aspects = ['deps'],
  required_aspect_providers = [[BInfo], [CInfo]],
)

aspect_b = aspect(
    implementation = _aspect_b_impl,
    propagation_predicate = _aspect_b_propagation_predicate,
    attr_aspects = ['deps'],
    provides = [BInfo],
)

aspect_c = aspect(
    implementation = _aspect_c_impl,
    propagation_predicate = _aspect_c_propagation_predicate,
    attr_aspects = ['deps'],
    provides = [CInfo],
)

def _rule_impl(ctx):
  pass

my_rule = rule(
  implementation = _rule_impl,
  attrs = {'deps': attr.label_list()},
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 't1', deps = [':t2', '//pkg2:t2'])
my_rule(name = 't2')
"#],
    );
    t.scratch.file(
        "pkg2/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 't1', deps = [':t2'])
my_rule(name = 't2')
"#],
    );

    t.update_with_aspects(
        &[
            "//test:defs.bzl%aspect_c",
            "//test:defs.bzl%aspect_b",
            "//test:defs.bzl%aspect_a",
        ],
        &["//pkg1:t1", "//pkg2:t1"],
    )
    .unwrap();

    // The propagation predicate of each aspect is evaluated separately, then the
    // aspect-on-aspect relation is created between the filtered aspects.
    let a_aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_a");
    assert_contains_exactly(
        &a_aspect_keys,
        &[
            "aspect_a on //pkg1:t1 with base aspects: aspect_b,aspect_c",
            "aspect_a on //pkg1:t2 with base aspects: aspect_b,aspect_c",
            "aspect_a on //pkg2:t1 with base aspects: aspect_c",
            "aspect_a on //pkg2:t2 with base aspects: aspect_c",
        ],
    );

    let b_aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_b");
    assert_contains_exactly(
        &b_aspect_keys,
        &["aspect_b on //pkg1:t1", "aspect_b on //pkg1:t2"],
    );

    let c_aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_c");
    assert_contains_exactly(
        &c_aspect_keys,
        &[
            "aspect_c on //pkg1:t1",
            "aspect_c on //pkg1:t2",
            "aspect_c on //pkg2:t1",
            "aspect_c on //pkg2:t2",
        ],
    );

    let aspects_map = t.get_analysis_result().get_aspects_map();
    let aspect_a_on_pkg1_t1 =
        get_aspect_result(aspects_map, "aspect_a", Some("//pkg1:t1"), "AInfo")
            .expect("assertThat(null)");
    assert_contains_exactly(
        &aspect_a_on_pkg1_t1,
        &[
            "aspect_a on @@//pkg1:t1 with BInfo with CInfo",
            "aspect_a on @@//pkg1:t2 with BInfo with CInfo",
            "aspect_a on @@//pkg2:t2 with CInfo",
        ],
    );

    let aspect_a_on_pkg2_t1 =
        get_aspect_result(aspects_map, "aspect_a", Some("//pkg2:t1"), "AInfo")
            .expect("assertThat(null)");
    assert_contains_exactly(
        &aspect_a_on_pkg2_t1,
        &[
            "aspect_a on @@//pkg2:t1 with CInfo",
            "aspect_a on @@//pkg2:t2 with CInfo",
        ],
    );
}

// Java: StarlarkAspectsPropagationPredicateTest.requiredAspects_propagationPredicateOfRequiredAspectIsUsed
#[test]
fn required_aspects_propagation_predicate_of_required_aspect_is_used() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AInfo = provider()
BInfo = provider()

def _aspect_a_propagation_predicate(ctx):
  return True

def _aspect_b_propagation_predicate(ctx):
  if ctx.rule.label.package == 'pkg1':
    return True
  return False

def _aspect_a_impl(target, ctx):
  res = 'aspect_a on %s' % target.label
  if BInfo in target:
    res += ' with BInfo'
  else:
    res += ' without BInfo'

  res_list = [res]

  for dep in ctx.rule.attr.deps:
    if AInfo in dep:
      res_list.extend(dep[AInfo].res)

  return [AInfo(res = res_list)]

def _aspect_b_impl(target, ctx):
  return [BInfo()]

aspect_b = aspect(
    implementation = _aspect_b_impl,
    propagation_predicate = _aspect_b_propagation_predicate,
)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  propagation_predicate = _aspect_a_propagation_predicate,
  attr_aspects = ['deps'],
  requires = [aspect_b],
)

def _rule_impl(ctx):
  pass

my_rule = rule(
  implementation = _rule_impl,
  attrs = {'deps': attr.label_list()},
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 't1', deps = [':t2', '//pkg2:t2'])
my_rule(name = 't2')
"#],
    );
    t.scratch.file(
        "pkg2/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 't1', deps = [':t2'])
my_rule(name = 't2')
"#],
    );

    t.update_with_aspects(&["//test:defs.bzl%aspect_a"], &["//pkg1:t1", "//pkg2:t1"])
        .unwrap();

    // The propagation predicate of both aspects is used.
    let a_aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_a");
    assert_contains_exactly(
        &a_aspect_keys,
        &[
            "aspect_a on //pkg1:t1 with base aspects: aspect_b",
            "aspect_a on //pkg1:t2 with base aspects: aspect_b",
            "aspect_a on //pkg2:t1",
            "aspect_a on //pkg2:t2",
        ],
    );

    let b_aspect_keys = get_formatted_aspect_keys(&mut t, "//test:defs.bzl%aspect_b");
    assert_contains_exactly(
        &b_aspect_keys,
        &["aspect_b on //pkg1:t1", "aspect_b on //pkg1:t2"],
    );

    let aspects_map = t.get_analysis_result().get_aspects_map();
    let aspect_a_on_pkg1_t1 =
        get_aspect_result(aspects_map, "aspect_a", Some("//pkg1:t1"), "AInfo")
            .expect("assertThat(null)");
    assert_contains_exactly(
        &aspect_a_on_pkg1_t1,
        &[
            "aspect_a on @@//pkg1:t1 with BInfo",
            "aspect_a on @@//pkg1:t2 with BInfo",
            "aspect_a on @@//pkg2:t2 without BInfo",
        ],
    );
    let aspect_a_on_pkg2_t1 =
        get_aspect_result(aspects_map, "aspect_a", Some("//pkg2:t1"), "AInfo")
            .expect("assertThat(null)");
    assert_contains_exactly(
        &aspect_a_on_pkg2_t1,
        &[
            "aspect_a on @@//pkg2:t1 without BInfo",
            "aspect_a on @@//pkg2:t2 without BInfo",
        ],
    );
}
