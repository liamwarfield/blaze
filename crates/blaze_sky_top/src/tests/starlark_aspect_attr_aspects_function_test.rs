//! Port of `src/test/java/com/google/devtools/build/lib/starlark/StarlarkAspectAttrAspectsFunctionTest.java`
//! (an `AnalysisTestCase`): Starlark aspects' `attr_aspects` as a function.

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_packages::aspect::AspectClass;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::structs::StarlarkInfo;
use indexmap::IndexMap;
use starlark::values::list::ListRef;

use crate::build_view::BuildViewError;
use crate::testing::build_view_test_case::parse_label;
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

// Java: StarlarkAspectAttrAspectsFunctionTest.createTestDefs(String, String)
fn create_test_defs_with_predicate(
    t: &mut AnalysisTestCase,
    propagation_attrs_function: &str,
    propagation_predicate_function: &str,
) {
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[&java_format(
            r#"AspectInfo = provider()

def _rule_impl(ctx):
  pass

simple_rule = rule(
  implementation = _rule_impl,
  attrs = {
    'deps_1': attr.label_list(),
    'deps_2': attr.label_list(),
    '_tool': attr.label(default = Label('//pkg1:rule_tool'))},
)

tool_rule = rule(
  implementation = _rule_impl,
)

%s

%s

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr
  for dep in (getattr(rule_attr, 'deps_1', []) + getattr(rule_attr, 'deps_2', [])):
    if AspectInfo in dep:
      res.extend(dep[AspectInfo].res)

  if hasattr(rule_attr, '_tool') and AspectInfo in rule_attr._tool:
    res.extend(rule_attr._tool[AspectInfo].res)

  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
  implementation = _aspect_impl,
  attr_aspects = _propagation_attrs,
  propagation_predicate = _propagation_predicate,
)
"#,
            &[propagation_attrs_function, propagation_predicate_function],
        )],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.createTestDefs(String)
fn create_test_defs(t: &mut AnalysisTestCase, propagation_attrs_function: &str) {
    create_test_defs_with_predicate(
        t,
        propagation_attrs_function,
        "def _propagation_predicate(ctx):\n  return True\n",
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.createTestPackages
fn create_test_packages(t: &mut AnalysisTestCase) {
    t.scratch.file(
        "config_setting/BUILD",
        &["config_setting(name='defines', values={'define': 'foo=1'})"],
    );

    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'simple_rule', 'tool_rule')
simple_rule(name = 't1', deps_1 = [':d1'], deps_2 = [':d2'])
simple_rule(name = 'd1', deps_1 = [':d11'], deps_2 = [':d12'])
simple_rule(name = 'd2', deps_2 = [':d22'])
simple_rule(name = 'd11')
simple_rule(name = 'd12')
simple_rule(name = 'd22')
tool_rule(name = 'rule_tool')

simple_rule(
    name = 'with_selects',
    deps_1 = select({
          '//config_setting:defines': [':d1'],
          '//conditions:default': ['d2'],
        })
)

simple_rule(name = 't2', deps_1 = [':d1', ':d11'], deps_2 = [':d2'])
"#],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.getAspectKeys
fn get_aspect_keys(t: &mut AnalysisTestCase, aspect_name: &str) -> Vec<AspectKey> {
    let evaluator = t.get_evaluator();
    let wanted = format!("//test:defs.bzl%{aspect_name}");
    evaluator
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|key| evaluator.get_existing_value(key).is_some())
        .filter_map(|key| key.downcast::<AspectKey>().cloned())
        .filter(|key| key.get_aspect_class().to_string() == wanted)
        .collect()
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

// Java: StarlarkAspectAttrAspectsFunctionTest.formatAspectKey
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

// Java: StarlarkAspectAttrAspectsFunctionTest.getFormattedAspectKeys
fn get_formatted_aspect_keys(t: &mut AnalysisTestCase, aspect_name: &str) -> Vec<String> {
    get_aspect_keys(t, aspect_name)
        .iter()
        .map(format_aspect_key)
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

// Java: StarlarkAspectAttrAspectsFunctionTest.getAspectResult
fn get_aspect_result(
    aspects_map: &IndexMap<AspectKey, std::sync::Arc<AspectValue>>,
    aspect_name: &str,
    target_label: Option<&str>,
    provider_name: &str,
) -> Vec<String> {
    for (key, value) in aspects_map {
        if let AspectClass::Starlark(starlark_aspect_class) = key.get_aspect_class() {
            let aspect_exported_name = starlark_aspect_class.get_exported_name();
            if aspect_exported_name == aspect_name
                && target_label.is_none_or(|l| key.get_label().to_string() == l)
            {
                return get_res(value, "//test:defs.bzl", provider_name);
            }
        }
    }
    panic!("Aspect result not found for aspect: {aspect_name}");
}

fn get_aspect_result_default(t: &AnalysisTestCase, aspect_name: &str) -> Vec<String> {
    get_aspect_result(
        t.get_analysis_result().get_aspects_map(),
        aspect_name,
        None,
        "AspectInfo",
    )
}

// Java: StarlarkAspectAttrAspectsFunctionTest.selectedAttrsReturned_aspectPropagatesAlongReturnedAttrs
#[test]
fn selected_attrs_returned_aspect_propagates_along_returned_attrs() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_attrs(ctx):
  if ctx.rule.label == Label('//pkg1:t1'):
    attr_aspects = []
    for attr_name in dir(ctx.rule.attr):
      attr_value = getattr(ctx.rule.attr, attr_name).value
      if type(attr_value) == type(Label("//foo")):
        attr_aspects.append(attr_name)
      if type(attr_value) == type([]):
        if len(attr_value) > 0 and type(attr_value[0]) == type(Label("//foo")):
          attr_aspects.append(attr_name)
    return attr_aspects

  else:
    return []
"#,
    );
    create_test_packages(&mut t);

    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:t1"])
        .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:t1",
            "cmdline_aspect on //pkg1:d1",
            "cmdline_aspect on //pkg1:d2",
            "cmdline_aspect on //pkg1:rule_tool",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:t1",
            "cmdline_aspect on @@//pkg1:d1",
            "cmdline_aspect on @@//pkg1:d2",
            "cmdline_aspect on @@//pkg1:rule_tool",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.wildCardAttrReturned_buildFails
#[test]
fn wild_card_attr_returned_build_fails() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        "def _propagation_attrs(ctx):\n    return ['*']\n",
    );
    create_test_packages(&mut t);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event("'*' is not allowed in 'attr_aspects' list");
}

// Java: StarlarkAspectAttrAspectsFunctionTest.invalidReturnValue_buildFails
#[test]
fn invalid_return_value_build_fails() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        "def _propagation_attrs(ctx):\n    return [44, 'foo']\n",
    );
    create_test_packages(&mut t);

    t.remove_fail_fast_handler();
    let result = t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:t1"]);
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "expected ViewCreationFailedException"
    );
    t.assert_contains_event("at index 0 of attr_aspects, got element of type int, want string");
}

// Java: StarlarkAspectAttrAspectsFunctionTest.withPropagationPredicate_aspectPropagatedToSatisfyingTargets
#[test]
fn with_propagation_predicate_aspect_propagated_to_satisfying_targets() {
    let mut t = new_test();
    create_test_defs_with_predicate(
        &mut t,
        "def _propagation_attrs(ctx):\n  return ['deps_1']\n",
        r#"def _propagation_predicate(ctx):
  if ctx.rule.label == Label('//pkg1:d1'):
    return False
  return True
"#,
    );
    create_test_packages(&mut t);

    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:t2"])
        .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &["cmdline_aspect on //pkg1:t2", "cmdline_aspect on //pkg1:d11"],
    );

    let aspect_a_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "cmdline_aspect on @@//pkg1:t2",
            "cmdline_aspect on @@//pkg1:d11",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.aspectApplyToGeneratingRule_attrAspectsFuncRunOnGeneratingRule
#[test]
fn aspect_apply_to_generating_rule_attr_aspects_func_run_on_generating_rule() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if ctx.rule.label == Label('//pkg1:target_with_output'):
    if ctx.rule.attr.out.value == Label('//pkg1:my_out.txt'):
      return ['deps']
  return []

def _aspect_impl(target, ctx):
  res = ['generating_rule_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr
  for dep in getattr(rule_attr, 'deps', []):
    if AspectInfo in dep:
      res.extend(dep[AspectInfo].res)
  return [AspectInfo(res = res)]

generating_rule_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs,
    apply_to_generating_rules = True,
)

def _out_rule_impl(ctx):
  if ctx.outputs.out:
    ctx.actions.write(ctx.outputs.out, 'hi')
  return []

out_rule = rule(
  implementation = _out_rule_impl,
  attrs = {'deps': attr.label_list(), 'out': attr.output()},
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'out_rule')
out_rule(name = 'target_with_output', out = 'my_out.txt', deps = [':d1'])
out_rule(name = 'd1')
"#],
    );

    t.update_with_aspects(
        &["//test:defs.bzl%generating_rule_aspect"],
        &["//pkg1:my_out.txt"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "generating_rule_aspect"),
        &[
            "generating_rule_aspect on //pkg1:my_out.txt",
            "generating_rule_aspect on //pkg1:target_with_output",
            "generating_rule_aspect on //pkg1:d1",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "generating_rule_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "generating_rule_aspect on @@//pkg1:target_with_output",
            "generating_rule_aspect on @@//pkg1:d1",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.attrWithSelect_availableToAttrAspectsFunc
#[test]
fn attr_with_select_available_to_attr_aspects_func() {
    let mut t = new_test();
    create_test_defs(
        &mut t,
        r#"def _propagation_attrs(ctx):
  deps_1 = ctx.rule.attr.deps_1.value
  if len(deps_1) == 1 and deps_1[0] == Label('//pkg1:d1'):
    return ['deps_1']
  return []
"#,
    );
    create_test_packages(&mut t);
    t.use_configuration(&["--define=foo=1"]);

    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:with_selects"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:with_selects",
            "cmdline_aspect on //pkg1:d1",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:with_selects",
            "cmdline_aspect on @@//pkg1:d1",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.computedDefaultAttr_availableToAttrAspectsFunc
#[test]
fn computed_default_attr_available_to_attr_aspects_func() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if hasattr(ctx.rule.attr, '_computed'):
    cmp = ctx.rule.attr._computed.value

    if Label('//pkg1:my_prefix01') in cmp and Label('//pkg1:my_prefix02') in cmp:
      return ['_computed']

  return []

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr
  for dep in getattr(rule_attr, '_computed', []):
    if AspectInfo in dep:
      res.extend(dep[AspectInfo].res)
  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs,
)

def _compute_attr(prefix):
  return [
      Label("//pkg1:" + prefix + "01"),
      Label("//pkg1:" + prefix + "02"),
  ]

def _rule_impl(ctx):
  pass

computed_default_rule = rule(
  implementation = _rule_impl,
  attrs = {
      "prefix": attr.string(default = "default_prefix"),
      "_computed": attr.label_list(default = _compute_attr),
  },
)

simple_rule = rule(
  implementation = _rule_impl,
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'computed_default_rule', 'simple_rule')
computed_default_rule(name = 'computed_default_target', prefix = 'my_prefix')
simple_rule(name = 'my_prefix01')
simple_rule(name = 'my_prefix02')
"#],
    );

    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:computed_default_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:computed_default_target",
            "cmdline_aspect on //pkg1:my_prefix01",
            "cmdline_aspect on //pkg1:my_prefix02",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:computed_default_target",
            "cmdline_aspect on @@//pkg1:my_prefix01",
            "cmdline_aspect on @@//pkg1:my_prefix02",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.lateBoundAttributes_availableToAttrAspectsFunc
//
// Java defines a native `rule_with_latebound_attr` (`MockRule`) with a `LabelLateBoundDefault`
// on a test-only configuration fragment (`TestFragment`/`TestOptions`), added to
// `TestRuleClassProvider.addStandardRules` through `useRuleClassProvider`. The scratch files and
// assertions are those of the Java test.
#[test]
#[ignore = "needs MockRule (native rule_with_latebound_attr), a test-only Fragment (TestFragment) and useRuleClassProvider with TestRuleClassProvider.addStandardRules"]
fn late_bound_attributes_available_to_attr_aspects_func() {
    let mut t = new_test();
    // Java: useRuleClassProvider(builder.build()) with rule_with_latebound_attr and TestFragment.
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if hasattr(ctx.rule.attr, '_latebound_attr'):
    if ctx.rule.attr._latebound_attr.value == Label('//pkg1:latebound_dep'):
      return ['_latebound_attr']

  return []

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr
  if hasattr(rule_attr, '_latebound_attr'):
    if AspectInfo in rule_attr._latebound_attr:
      res.extend(rule_attr._latebound_attr[AspectInfo].res)
  return [AspectInfo(res = res)]


cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs
)

def _rule_impl(ctx):
  pass

simple_rule = rule(
  implementation = _rule_impl,
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'simple_rule')
rule_with_latebound_attr(name = 'latebound_target')
simple_rule(name = 'latebound_dep')
"#],
    );

    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:latebound_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:latebound_target",
            "cmdline_aspect on //pkg1:latebound_dep",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:latebound_target",
            "cmdline_aspect on @@//pkg1:latebound_dep",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.toolAttr_correctlyIdentified
#[test]
fn tool_attr_correctly_identified() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  attr_aspects = []
  for attr_name in dir(ctx.rule.attr):
    if getattr(ctx.rule.attr, attr_name).is_tool:
      attr_aspects.append(attr_name)

  return attr_aspects

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr

  if hasattr(rule_attr, '_tool_1') and AspectInfo in rule_attr._tool_1:
    res.extend(rule_attr._tool_1[AspectInfo].res)

  if hasattr(rule_attr, '_tool_2') and AspectInfo in rule_attr._tool_2:
    res.extend(rule_attr._tool_2[AspectInfo].res)

  if hasattr(rule_attr, '_non_tool') and AspectInfo in rule_attr._non_tool:
    res.extend(rule_attr._non_tool[AspectInfo].res)

  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs
)

def _rule_impl(ctx):
  pass

rule_with_tool = rule(
  implementation = _rule_impl,
  attrs = {
      '_tool_1': attr.label(
          default = Label('//pkg1:tool_1'), flags = ['IS_TOOL_DEPENDENCY']),
      '_tool_2': attr.label(default = Label('//pkg1:tool_2'), cfg = 'exec'),
      '_non_tool': attr.label(default = Label('//pkg1:non_tool')),
  },
)

simple_rule = rule(
  implementation = _rule_impl,
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'rule_with_tool', 'simple_rule')
rule_with_tool(name = 'main_target')
simple_rule(name = 'tool_1')
simple_rule(name = 'tool_2')
simple_rule(name = 'non_tool')
"#],
    );

    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:main_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:main_target",
            "cmdline_aspect on //pkg1:tool_1",
            "cmdline_aspect on //pkg1:tool_2",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:main_target",
            "cmdline_aspect on @@//pkg1:tool_1",
            "cmdline_aspect on @@//pkg1:tool_2",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.attrInitializer_availableToAttrAspectsFunc
#[test]
fn attr_initializer_available_to_attr_aspects_func() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if hasattr(ctx.rule.attr, 'deps'):
    deps = ctx.rule.attr.deps.value
    if Label('//pkg1:added') in deps and Label('//pkg1:initial') in deps:
      return ['deps']

  return []

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on ' + str(target.label)]
  rule_attr = ctx.rule.attr
  for dep in getattr(rule_attr, 'deps', []):
    if AspectInfo in dep:
      res.extend(dep[AspectInfo].res)
  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs
)

def _rule_impl(ctx):
  pass

def _initializer(name, deps = []):
  return {"deps": deps + ["//pkg1:added"]}

rule_with_initializer = rule(
  implementation = _rule_impl,
  initializer = _initializer,
  attrs = {
      "deps": attr.label_list(),
  },
)

simple_rule = rule(
  implementation = _rule_impl,
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'rule_with_initializer', 'simple_rule')
rule_with_initializer(name = 'main_target', deps = [":initial"])
simple_rule(name = 'initial')
simple_rule(name = 'added')
"#],
    );
    t.use_configuration(&["--experimental_rule_extension_api"]);

    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:main_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:main_target",
            "cmdline_aspect on //pkg1:added",
            "cmdline_aspect on //pkg1:initial",
        ],
    );

    let aspect_result = get_aspect_result_default(&t, "cmdline_aspect");
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:main_target",
            "cmdline_aspect on @@//pkg1:added",
            "cmdline_aspect on @@//pkg1:initial",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.aspectOnAspect_eachAspectPropagatesSeparately
#[test]
fn aspect_on_aspect_each_aspect_propagates_separately() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AProv = provider()
BProv = provider()
CProv = provider()

def _a_propagation_attrs(ctx):
  return ['dep_1', 'dep_2']

def b_propagation_attrs(ctx):
  return ['dep_1']

def _c_propagation_attrs(ctx):
  return ['dep_2']

def _aspect_a_impl(target, ctx):
  current_res = 'aspect_a on %s' % target.label
  if BProv in target:
    current_res += ' with BProv'
  if CProv in target:
    current_res += ' with CProv'

  res = [current_res]

  if ctx.rule.attr.dep_1 and AProv in ctx.rule.attr.dep_1:
    res.extend(ctx.rule.attr.dep_1[AProv].res)

  if ctx.rule.attr.dep_2 and AProv in ctx.rule.attr.dep_2:
    res.extend(ctx.rule.attr.dep_2[AProv].res)

  return [AProv(res = res)]

def _aspect_b_impl(target, ctx):
  return [BProv(res = 'aspect_b on %s' % target.label)]

def _aspect_c_impl(target, ctx):
  return [CProv(res = 'aspect_c on %s' % target.label)]

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = _a_propagation_attrs,
  required_aspect_providers = [[BProv], [CProv]],
)

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = b_propagation_attrs,
  provides = [BProv],
)

aspect_c = aspect(
  implementation = _aspect_c_impl,
  attr_aspects = _c_propagation_attrs,
  provides = [CProv],
)

def _rule_impl(ctx):
  return []

my_rule = rule(
  implementation = _rule_impl,
  attrs = {
      "dep_1": attr.label(),
      "dep_2": attr.label(),
  },
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 'main_target', dep_1 = ':dep_1', dep_2 = ':dep_2')
my_rule(name = 'dep_1')
my_rule(name = 'dep_2')
"#],
    );

    t.update_with_aspects(
        &[
            "//test:defs.bzl%aspect_c",
            "//test:defs.bzl%aspect_b",
            "//test:defs.bzl%aspect_a",
        ],
        &["//pkg1:main_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "aspect_a"),
        &[
            "aspect_a on //pkg1:main_target with base aspects: aspect_b,aspect_c",
            "aspect_a on //pkg1:dep_1 with base aspects: aspect_b",
            "aspect_a on //pkg1:dep_2 with base aspects: aspect_c",
        ],
    );

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "aspect_b"),
        &["aspect_b on //pkg1:main_target", "aspect_b on //pkg1:dep_1"],
    );

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "aspect_c"),
        &["aspect_c on //pkg1:main_target", "aspect_c on //pkg1:dep_2"],
    );

    let aspect_a_result = get_aspect_result(
        t.get_analysis_result().get_aspects_map(),
        "aspect_a",
        Some("//pkg1:main_target"),
        "AProv",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on @@//pkg1:main_target with BProv with CProv",
            "aspect_a on @@//pkg1:dep_1 with BProv",
            "aspect_a on @@//pkg1:dep_2 with CProv",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.requiredAspect_propagatesWithMainAspect
#[test]
fn required_aspect_propagates_with_main_aspect() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"AProv = provider()
BProv = provider()

def _a_propagation_attrs(ctx):
  return ['dep_1', 'dep_2']

def _aspect_a_impl(target, ctx):
  current_res = 'aspect_a on %s' % target.label
  if BProv in target:
    current_res += ' with BProv'

  res = [current_res]

  if ctx.rule.attr.dep_1 and AProv in ctx.rule.attr.dep_1:
    res.extend(ctx.rule.attr.dep_1[AProv].res)

  if ctx.rule.attr.dep_2 and AProv in ctx.rule.attr.dep_2:
    res.extend(ctx.rule.attr.dep_2[AProv].res)

  return [AProv(res = res)]

def _aspect_b_impl(target, ctx):
  return [BProv(res = 'aspect_b on %s' % target.label)]

aspect_b = aspect(
  implementation = _aspect_b_impl,
  attr_aspects = [],
  provides = [BProv],
)

aspect_a = aspect(
  implementation = _aspect_a_impl,
  attr_aspects = _a_propagation_attrs,
  requires = [aspect_b],
)

def _rule_impl(ctx):
  return []

my_rule = rule(
  implementation = _rule_impl,
  attrs = {
      "dep_1": attr.label(),
      "dep_2": attr.label(),
  },
)
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load('//test:defs.bzl', 'my_rule')
my_rule(name = 'main_target', dep_1 = ':dep_1', dep_2 = ':dep_2')
my_rule(name = 'dep_1')
my_rule(name = 'dep_2')
"#],
    );

    t.update_with_aspects(&["//test:defs.bzl%aspect_a"], &["//pkg1:main_target"])
        .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "aspect_a"),
        &[
            "aspect_a on //pkg1:main_target with base aspects: aspect_b",
            "aspect_a on //pkg1:dep_1 with base aspects: aspect_b",
            "aspect_a on //pkg1:dep_2 with base aspects: aspect_b",
        ],
    );

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "aspect_b"),
        &[
            "aspect_b on //pkg1:main_target",
            "aspect_b on //pkg1:dep_1",
            "aspect_b on //pkg1:dep_2",
        ],
    );

    let aspect_a_result = get_aspect_result(
        t.get_analysis_result().get_aspects_map(),
        "aspect_a",
        Some("//pkg1:main_target"),
        "AProv",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "aspect_a on @@//pkg1:main_target with BProv",
            "aspect_a on @@//pkg1:dep_1 with BProv",
            "aspect_a on @@//pkg1:dep_2 with BProv",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.createDormantDepsTest
fn create_dormant_deps_test(t: &mut AnalysisTestCase, aspect_def: &str) {
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[&java_format(
            r#"ComponentInfo = provider(fields = ["components"])

def _component_impl(ctx):
  current = struct(label=ctx.label, impl = ctx.attr.impl)
  transitive = [d[ComponentInfo].components for d in ctx.attr.deps]
  return [
    ComponentInfo(components = depset(direct = [current], transitive = transitive)),
  ]

component = rule(
  implementation = _component_impl,
  attrs = {
    "deps": attr.label_list(providers = [ComponentInfo]),
    "impl": attr.dormant_label(),
  },
  provides = [ComponentInfo],
  dependency_resolution_rule = True,
)

def _binary_impl(ctx):
  return [DefaultInfo(files=depset(ctx.files._impls))]

def _materializer(ctx):
  all = depset(transitive = [d[ComponentInfo].components for d in ctx.attr.components])
  selected = [c.impl for c in all.to_list() if "yes" in str(c.label)]
  return selected

binary = rule(
  implementation = _binary_impl,
  attrs = {
      "components": attr.label_list(providers = [ComponentInfo], for_dependency_resolution = True),
      "_impls": attr.label_list(materializer = _materializer),
      "regular_deps": attr.label_list(),
  })

def _rule_impl(ctx):
  pass

simple_rule = rule(
    implementation = _rule_impl,
)

%s
"#,
            &[aspect_def],
        )],
    );

    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load("//test:defs.bzl", "component", "binary", "simple_rule")

component(name="a_yes", impl=":a_impl")
component(name="b_no", deps = [":c_yes", ":d_no"], impl=":b_impl")
component(name="c_yes", impl=":c_impl")
component(name="d_no", impl=":d_impl")

binary(name="bin", components=[":a_yes", ":b_no"], regular_deps = [":dep_1"])
[filegroup(name=x + "_impl", srcs=[x]) for x in ["a", "b", "c", "d"]]

simple_rule(name = 'dep_1')
"#],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.aspectOnMaterializingTarget_attrAspectsFuncUsed
#[test]
#[ignore = "BLAZE GAP: unported StarlarkAttrModule.StarlarkMaterializer (attr.label_list(materializer = ...), blaze_packages starlark_defs/attr_module.rs:444)"]
fn aspect_on_materializing_target_attr_aspects_func_used() {
    let mut t = new_test();
    create_dormant_deps_test(
        &mut t,
        r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if ctx.rule.qualified_kind.rule_name == 'binary':
      # the value of _impls should be None as it is not materialized yet
      if ctx.rule.attr._impls.value != None:
        fail("'_impls' should be None")

      # the value of regular_deps should be available
      if Label('//pkg1:dep_1') not in ctx.rule.attr.regular_deps.value:
        fail("regular_deps should be available")

      # the value of components should be available
      components = ctx.rule.attr.components.value
      if len(components) != 2 or Label('//pkg1:a_yes') not in components or Label('//pkg1:b_no') not in components:
        fail("components should be available")

      return ['_impls', 'components', 'regular_deps']

  return []

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on %s' % target.label]

  deps = []
  deps.extend(getattr(ctx.rule.attr, '_impls', []))
  deps.extend(getattr(ctx.rule.attr, 'components', []))
  deps.extend(getattr(ctx.rule.attr, 'regular_deps', []))

  for dep in deps:
    if AspectInfo in dep:
      res += dep[AspectInfo].res

  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs,
)
"#,
    );

    t.use_configuration(&["--experimental_dormant_deps"]);
    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:bin"])
        .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:bin",
            "cmdline_aspect on //pkg1:dep_1",
            "cmdline_aspect on //pkg1:a_yes",
            "cmdline_aspect on //pkg1:b_no",
            "cmdline_aspect on //pkg1:a_impl",
            "cmdline_aspect on //pkg1:c_impl",
        ],
    );

    let aspect_a_result = get_aspect_result(
        t.get_analysis_result().get_aspects_map(),
        "cmdline_aspect",
        Some("//pkg1:bin"),
        "AspectInfo",
    );
    assert_contains_exactly(
        &aspect_a_result,
        &[
            "cmdline_aspect on @@//pkg1:bin",
            "cmdline_aspect on @@//pkg1:dep_1",
            "cmdline_aspect on @@//pkg1:a_yes",
            "cmdline_aspect on @@//pkg1:b_no",
            "cmdline_aspect on @@//pkg1:a_impl",
            "cmdline_aspect on @@//pkg1:c_impl",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.aspectOnDependencyResolutionTargets_attrAspectsFuncUsed
#[test]
#[ignore = "BLAZE GAP: unported StarlarkAttrModule.StarlarkMaterializer (attr.label_list(materializer = ...), blaze_packages starlark_defs/attr_module.rs:444)"]
fn aspect_on_dependency_resolution_targets_attr_aspects_func_used() {
    let mut t = new_test();
    create_dormant_deps_test(
        &mut t,
        r#"AspectInfo = provider()

def _propagation_attrs(ctx):
  if ctx.rule.qualified_kind.rule_name == 'component':
      # the value of impl should be available
      impl_name = ctx.rule.label.name.split('_')[0] + '_impl'
      impl_val = ctx.rule.attr.impl.value
      if type(impl_val) != 'DormantDependency' or impl_val.label.name != impl_name:
        fail("'impl' should be available")

      # the value of deps should be available
      if ctx.rule.label == Label('//pkg1:b_no'):
        deps = ctx.rule.attr.deps.value
        if Label('//pkg1:c_yes') not in deps or Label('//pkg1:d_no') not in deps:
            fail("deps should be available")
      elif len(ctx.rule.attr.deps.value) != 0:
        fail("deps should be empty")

      return ['deps', 'impl']

  return ['components']

def _aspect_impl(target, ctx):
  res = ['cmdline_aspect on %s' % target.label]

  if hasattr(ctx.rule.attr, 'impl') and type(ctx.rule.attr.impl) != 'DormantDependency':
    fail("impl should be a dormant dependency")

  deps = []
  deps.extend(getattr(ctx.rule.attr, 'deps', []))
  deps.extend(getattr(ctx.rule.attr, 'components', []))

  for dep in deps:
    if AspectInfo in dep:
      res += dep[AspectInfo].res

  return [AspectInfo(res = res)]

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs,
)
"#,
    );

    t.use_configuration(&["--experimental_dormant_deps"]);
    t.update_with_aspects(&["//test:defs.bzl%cmdline_aspect"], &["//pkg1:bin"])
        .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &[
            "cmdline_aspect on //pkg1:bin",
            "cmdline_aspect on //pkg1:a_yes",
            "cmdline_aspect on //pkg1:b_no",
            "cmdline_aspect on //pkg1:c_yes",
            "cmdline_aspect on //pkg1:d_no",
        ],
    );

    let aspect_result = get_aspect_result(
        t.get_analysis_result().get_aspects_map(),
        "cmdline_aspect",
        Some("//pkg1:bin"),
        "AspectInfo",
    );
    assert_contains_exactly(
        &aspect_result,
        &[
            "cmdline_aspect on @@//pkg1:bin",
            "cmdline_aspect on @@//pkg1:a_yes",
            "cmdline_aspect on @@//pkg1:b_no",
            "cmdline_aspect on @@//pkg1:c_yes",
            "cmdline_aspect on @@//pkg1:d_no",
        ],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.allRuleAttributesAreAvailableInAttrAspectsFunc
#[test]
fn all_rule_attributes_are_available_in_attr_aspects_func() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"attr_map = {
  'bool': True,
  'int': 100,
  'int_list': [1, 2, 3],
  'label': Label('//pkg1:dep_1'),
  'label_keyed_string_dict': {Label('//pkg1:dep_1'): 'key1', Label('//pkg1:dep_2'): 'key2'},
  'label_list': [Label('//pkg1:dep_1'), Label('//pkg1:dep_2')],
  'output': Label('//pkg1:output.txt'),
  'output_list': [Label('//pkg1:output_2.txt'), Label('//pkg1:output_3.txt')],
  'string': 'string_value',
  'string_dict': {'key1': 'value1', 'key2': 'value2'},
  'string_keyed_label_dict': {'key1': Label('//pkg1:dep_1'), 'key2': Label('//pkg1:dep_2')},
  'string_list': ['string_value_1', 'string_value_2'],
  'string_list_dict': {'key1': ['string_value_1', 'string_value_2'], 'key2': ['string_value_3', 'string_value_4']},
  'label_list_dict': {'key1': [Label('//pkg1:dep_1')], 'key2': [Label('//pkg1:dep_2')]},
}
def _propagation_attrs(ctx):
  for attr_name, expected_val in attr_map.items():
    if not hasattr(ctx.rule.attr, attr_name):
      fail("'%s' is not an attribute of the rule" % attr_name)

    actual_val = getattr(ctx.rule.attr, attr_name).value
    if expected_val != actual_val:
      fail("'%s' has wrong value: expected %s, got %s" % (attr_name, expected_val, actual_val))
  return []

def _aspect_impl(target, ctx):
  return []

cmdline_aspect = aspect(
  implementation = _aspect_impl,
  attr_aspects = _propagation_attrs,
)

def _rule_impl(ctx):
  if ctx.outputs.output:
    ctx.actions.write(ctx.outputs.output, 'hi')
  if ctx.outputs.output_list:
    ctx.actions.write(ctx.outputs.output_list[0], 'hi')
    ctx.actions.write(ctx.outputs.output_list[1], 'hi')
  return []

my_rule = rule(
  implementation = _rule_impl,
  attrs = {
    "bool": attr.bool(),
    "int": attr.int(),
    "int_list": attr.int_list(),
    "label": attr.label(),
    "label_keyed_string_dict": attr.label_keyed_string_dict(),
    "label_list": attr.label_list(),
    "output": attr.output(),
    "output_list": attr.output_list(),
    "string": attr.string(),
    "string_dict": attr.string_dict(),
    "string_keyed_label_dict": attr.string_keyed_label_dict(),
    "string_list": attr.string_list(),
    "string_list_dict": attr.string_list_dict(),
    "label_list_dict": attr.label_list_dict(),
    })
"#],
    );
    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load("//test:defs.bzl", "my_rule")
my_rule(
    name = 'main_target',
    bool = True,
    int = 100,
    int_list = [1, 2, 3],
    label = ':dep_1',
    label_keyed_string_dict = {'//pkg1:dep_1': 'key1', '//pkg1:dep_2': 'key2'},
    label_list = [':dep_1', ':dep_2'],
    output = 'output.txt',
    output_list = ['output_2.txt', 'output_3.txt'],
    string = 'string_value',
    string_dict = {'key1': 'value1', 'key2': 'value2'},
    string_keyed_label_dict = {'key1': ':dep_1', 'key2': ':dep_2'},
    string_list = ['string_value_1', 'string_value_2'],
    string_list_dict = {'key1': ['string_value_1', 'string_value_2'], 'key2': ['string_value_3', 'string_value_4']},
    label_list_dict = {'key1': [':dep_1'], 'key2': [':dep_2']},
)

my_rule(name = 'dep_1')
my_rule(name = 'dep_2')
"#],
    );
    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:main_target"],
    )
    .unwrap();

    assert_contains_exactly(
        &get_formatted_aspect_keys(&mut t, "cmdline_aspect"),
        &["cmdline_aspect on //pkg1:main_target"],
    );
}

// Java: StarlarkAspectAttrAspectsFunctionTest.attributeWithTransition_availableInAttrAspectsFunc
//
// Java adds `DummyTestFragment` (the `--foo` option) to `TestRuleClassProvider.addStandardRules`
// through `useRuleClassProvider`; the assertion reads `DummyTestOptions.foo` from each aspect
// key's configuration.
#[test]
#[ignore = "needs DummyTestFragment/DummyTestOptions (--foo) and useRuleClassProvider with TestRuleClassProvider.addStandardRules"]
fn attribute_with_transition_available_in_attr_aspects_func() {
    let mut t = new_test();
    t.scratch.file("test/BUILD", &[]);
    t.scratch.file(
        "test/defs.bzl",
        &[r#"def _propagation_attrs(ctx):
  if ctx.rule.label.name != 'main_target':
    return []

  dep = ctx.rule.attr.dep.value
  if not dep or type(dep) != type([]) or dep[0] != Label('//pkg1:dep_1'):
    fail("dep is not available")
  return ['dep']

def _aspect_impl(target, ctx):
  return []

cmdline_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _propagation_attrs,
)

def _transition_impl(settings, attr):
    return {
      "t1": {"//command_line_option:foo" : "v1"},
      "t2": {"//command_line_option:foo" : "v2"}
    }

simple_transition = transition(
    implementation = _transition_impl,
    inputs = [],
    outputs = ["//command_line_option:foo"]
)

def _rule_impl(ctx):
  pass

my_rule = rule(
    implementation = _rule_impl,
    attrs = {
        "dep": attr.label(cfg = simple_transition),
    },
)
"#],
    );

    t.scratch.file(
        "pkg1/BUILD",
        &[r#"load("//test:defs.bzl", "my_rule")
my_rule(name = 'main_target', dep = ':dep_1')
my_rule(name = 'dep_1')
"#],
    );
    // Java: useRuleClassProvider(TestRuleClassProvider.addStandardRules + DummyTestFragment).

    t.use_configuration(&["--foo=default"]);
    t.update_with_aspects(
        &["//test:defs.bzl%cmdline_aspect"],
        &["//pkg1:main_target"],
    )
    .unwrap();

    // Java: k.getLabel() + " with foo = " +
    //     k.getConfigurationKey().getOptions().get(DummyTestOptions.class).foo
    // expected: "//pkg1:main_target with foo = default", "//pkg1:dep_1 with foo = v1",
    //     "//pkg1:dep_1 with foo = v2"
    let _aspect_keys = get_aspect_keys(&mut t, "cmdline_aspect");
    unreachable!("needs DummyTestOptions");
}
