//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/starlark/StarlarkSubruleTest.java`
//! (a `BuildViewTestCase`): Starlark subrules (`subrule()`, subrule attributes, toolchains,
//! fragments, transitive subrules).
//!
//! How the Java test environment maps onto the harness:
//! - The Java class's `BazelEvaluationTestCase ev` (a .bzl thread labelled
//!   `//subrule_testing:label`) is [`Ev`]: the lines executed so far plus the new ones are the
//!   .bzl file `//subrule_testing:label.bzl`, which `//ev_test:BUILD` loads (the same mapping as
//!   the `Ev` of `blaze_sky_loading`'s `starlark_rule_class_functions_test.rs`). An exception of
//!   `ev.checkEvalErrorContains` is an ERROR event of loading the package. `ev.eval(expr)` is the
//!   value's Starlark `type(...)` carried out through the `tags` of a `filegroup`, so Java's
//!   `isInstanceOf(BuiltinFunction.class)` / `isInstanceOf(StarlarkSubruleApi.class)` is the
//!   Starlark type name of that class (`builtin_function_or_method`, `Subrule`).
//! - `assertThrows(AssertionError.class, () -> getConfiguredTarget(...))` is the panic of the
//!   harness's fail-fast handler (its message is the first error event, as Java's).
//! - `getProvider(target, bzl, name)` is [`BuildViewTestCase::get_starlark_provider`].

use std::sync::Arc;

use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configuredtargets::target_value::StarlarkTarget;
use blaze_events::EventKind;
use blaze_packages::attribute::AttributeValueSource;
use blaze_packages::starlark_defs::subrule::get_rule_attr_name;
use blaze_packages::types::AttrValue;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value};

use crate::testing::BuildViewTestCase;
use crate::testing::build_view_test_case::parse_label;

// -------------------------------------------------------------------------------------------
// Helpers.

/// Java `BazelEvaluationTestCase ev = new BazelEvaluationTestCase("//subrule_testing:label")`
/// (see the module documentation).
struct Ev {
    /// The lines executed so far.
    lines: Vec<String>,
}

impl Ev {
    fn new() -> Ev {
        Ev { lines: Vec::new() }
    }

    /// Writes the .bzl file (the lines executed so far, `lines`, one assignment per expression
    /// of `exprs`) and `//ev_test:BUILD`, which loads it and stores each expression's value
    /// (a string) in the `tags` of `//ev_test:ev_value_<i>`.
    fn write(&self, t: &BuildViewTestCase, lines: &[&str], exprs: &[&str]) {
        let mut file: Vec<String> = self.lines.clone();
        file.extend(lines.iter().map(|l| (*l).to_owned()));
        let mut symbols = vec!["ev_loaded".to_owned()];
        for (i, expr) in exprs.iter().enumerate() {
            file.push(format!("ev_value_{i} = {expr}"));
            symbols.push(format!("ev_value_{i}"));
        }
        file.push("ev_loaded = True".to_owned());
        let file: Vec<&str> = file.iter().map(String::as_str).collect();
        t.scratch.overwrite_file("subrule_testing/BUILD", &[]);
        t.scratch.overwrite_file("subrule_testing/label.bzl", &file);
        let mut build = vec![format!(
            "load(\"//subrule_testing:label.bzl\", {})",
            symbols
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )];
        build.push("filegroup(name = \"ev_loaded_target\")".to_owned());
        for symbol in &symbols[1..] {
            build.push(format!("filegroup(name = \"{symbol}\", tags = [{symbol}])"));
        }
        let build: Vec<&str> = build.iter().map(String::as_str).collect();
        t.scratch.overwrite_file("ev_test/BUILD", &build);
    }

    /// Java `ev.execAndExport(lines...)`: the lines execute without error; their globals stay
    /// defined.
    fn exec_and_export(&mut self, t: &mut BuildViewTestCase, lines: &[&str]) {
        self.write(t, lines, &[]);
        t.get_target("//ev_test:ev_loaded_target")
            .unwrap_or_else(|e| panic!("{e:?}"));
        self.lines.extend(lines.iter().map(|l| (*l).to_owned()));
    }

    /// Java `ev.checkEvalErrorContains(msg, lines...)`.
    fn check_eval_error_contains(&self, t: &mut BuildViewTestCase, msg: &str, lines: &[&str]) {
        self.write(t, lines, &[]);
        t.remove_fail_fast_handler();
        assert!(
            t.get_target("//ev_test:ev_loaded_target").is_err(),
            "Expected error containing '{msg}' but got no error"
        );
        t.assert_contains_event_of_kinds(msg, EventKind::ERRORS);
    }

    /// The Starlark value of `expr` (a string) in the environment of the lines executed so far
    /// (Java `ev.eval(expr)`).
    fn eval_str(&self, t: &mut BuildViewTestCase, expr: &str) -> String {
        self.write(t, &[], &[expr]);
        let target = t
            .get_target("//ev_test:ev_value_0")
            .unwrap_or_else(|e| panic!("{e:?}"));
        let rule = target.as_rule().expect("a rule");
        match blaze_packages::target::RuleOrMacroInstance::get_attr(&**rule, "tags") {
            Some(AttrValue::StringList(mut tags)) if tags.len() == 1 => tags.remove(0),
            other => panic!("tags of //ev_test:ev_value_0: {other:?}"),
        }
    }
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

/// `assertThat(error).hasMessageThat().contains(expected)`.
fn assert_message_contains(message: &str, expected: &str) {
    assert!(
        message.contains(expected),
        "expected message to contain {expected:?}; message: {message:?}"
    );
}

/// `getConfiguredTarget(label)` that must throw (`assertThrows(AssertionError.class, ...)`).
fn configured_target_error(t: &mut BuildViewTestCase, label: &str) -> String {
    assert_throws_assertion_error(|| {
        t.get_configured_target(label);
    })
}

/// `getProvider(targetLabel, providerLabel, providerName)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubruleTest.java:1934 StarlarkSubruleTest.getProvider
fn get_provider(
    t: &mut BuildViewTestCase,
    target_label: &str,
    provider_label: &str,
    provider_name: &str,
) -> Option<OwnedFrozenValue> {
    let target = t
        .get_configured_target(target_label)
        .unwrap_or_else(|| panic!("no configured target {target_label}"));
    BuildViewTestCase::get_starlark_provider(&target, provider_label, provider_name)
}

/// `provider.getValue(name)`, handed to `f`.
fn with_value<R>(info: &OwnedFrozenValue, name: &str, f: impl FnOnce(Value) -> R) -> R {
    let v = info.value();
    let info = StarlarkInfo::from_value(v)
        .unwrap_or_else(|| panic!("ClassCastException: {v} is not a StructImpl"));
    let field = info
        .get_field(name)
        .unwrap_or_else(|| panic!("no field '{name}' in {v}"));
    f(field)
}

/// `provider.getValue(name, String.class)`.
fn string_value(info: &OwnedFrozenValue, name: &str) -> String {
    with_value(info, name, |v| {
        v.unpack_str()
            .unwrap_or_else(|| panic!("ClassCastException: {v} is not a String"))
            .to_string()
    })
}

/// `Sequence.cast(provider.getValue(name), String.class, ...).getImmutableList()`.
fn string_list_value(info: &OwnedFrozenValue, name: &str) -> Vec<String> {
    with_value(info, name, |v| {
        ListRef::from_value(v)
            .unwrap_or_else(|| panic!("ClassCastException: {v} is not a Sequence"))
            .iter()
            .map(|e| {
                e.unpack_str()
                    .unwrap_or_else(|| panic!("ClassCastException: {e} is not a String"))
                    .to_string()
            })
            .collect()
    })
}

/// `((ConfiguredTarget) provider.getValue(name)).getLabel().toString()`.
fn configured_target_label_value(info: &OwnedFrozenValue, name: &str) -> String {
    with_value(info, name, |v| {
        StarlarkTarget::configured_target_of(v)
            .unwrap_or_else(|| panic!("ClassCastException: {v} is not a ConfiguredTarget"))
            .get_label()
            .get_canonical_form()
    })
}

/// `(Artifact) provider.getValue(name)`.
fn artifact_value(info: &OwnedFrozenValue, name: &str) -> Artifact {
    with_value(info, name, |v| {
        artifact_from_value(v)
            .unwrap_or_else(|| panic!("ClassCastException: {v} is not an Artifact"))
    })
}

/// `getAspect(label)`: the only done `AspectValue` whose aspect name is `label`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1205 BuildViewTestCase.getAspect
fn get_aspect(t: &mut BuildViewTestCase, label: &str) -> Arc<AspectValue> {
    let ev = t.get_evaluator();
    let values: Vec<Arc<AspectValue>> = ev
        .get_all_keys_for_testing()
        .into_iter()
        .filter(|k| {
            k.downcast::<AspectKey>()
                .is_some_and(|a| a.get_aspect_name() == label)
        })
        .filter_map(|k| ev.get_existing_value(&k))
        .map(|v| {
            v.downcast::<AspectValue>()
                .unwrap_or_else(|_| panic!("ClassCastException: AspectValue"))
        })
        .collect();
    assert_eq!(
        values.len(),
        1,
        "onlyElement: {} aspect values named {label}",
        values.len()
    );
    values.into_iter().next().unwrap()
}

/// `getRuleAttrName(Label.parseCanonical(bzl), subrule, attr, AttributeValueSource.DIRECT)`.
fn rule_attr_name(bzl: &str, exported_name: &str, attr_name: &str) -> String {
    get_rule_attr_name(
        &parse_label(bzl),
        exported_name,
        attr_name,
        AttributeValueSource::Direct,
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// `TestConstants.JAVA_TOOLCHAIN_TYPE`.
// Java: com/google/devtools/build/lib/testutil/TestConstants.java:182 TestConstants.JAVA_TOOLCHAIN_TYPE
const JAVA_TOOLCHAIN_TYPE: &str = "@@bazel_tools//tools/jdk:toolchain_type";

// -------------------------------------------------------------------------------------------
// Tests.

// Java: StarlarkSubruleTest.testSubruleFunctionSymbol_notVisibleInBUILD
#[test]
fn test_subrule_function_symbol_not_visible_in_build() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("foo/BUILD", &["subrule"]);

    t.check_loading_phase_error("//foo", "'subrule' is not defined");
}

// Java: StarlarkSubruleTest.testSubruleFunctionSymbol_isVisibleInBzl
// checks that 'subrule' symbol visibility in bzl files, not whether it's callable
#[test]
fn test_subrule_function_symbol_is_visible_in_bzl() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    let subrule_function_type = ev.eval_str(&mut t, "type(subrule)");

    t.assert_no_events();
    assert_eq!(subrule_function_type, "builtin_function_or_method");
}

// Java: StarlarkSubruleTest.testSubruleInstantiation_inAllowlistedPackage_succeeds
#[test]
fn test_subrule_instantiation_in_allowlisted_package_succeeds() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    let subrule_type = ev.eval_str(&mut t, "type(subrule(implementation = lambda : 0 ))");

    assert_eq!(subrule_type, "Subrule");
}

// Java: StarlarkSubruleTest.testSubrule_isCallableOnlyFromRuleOrAspectImplementation
#[test]
fn test_subrule_is_callable_only_from_rule_or_aspect_implementation() {
    let mut t = BuildViewTestCase::new();
    let mut ev = Ev::new();
    ev.exec_and_export(
        &mut t,
        &["x = subrule(implementation = lambda : 'dummy result')"],
    );

    ev.check_eval_error_contains(
        &mut t,
        "x can only be called from a rule or aspect implementation",
        &["x()"],
    );
}

// Java: StarlarkSubruleTest.testSubrule_isCallableOnlyAfterExport
#[test]
fn test_subrule_is_callable_only_after_export() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "Invalid subrule hasn't been exported by a bzl file",
        &[
            "unexported = [subrule(implementation = lambda: None)]",
            "unexported[0]()",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubrule_ruleMustDeclareSubrule
#[test]
fn test_subrule_rule_must_declare_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"_my_subrule = subrule(implementation = lambda: "")

def _rule_impl(ctx):
    _my_subrule()

my_rule = rule(implementation = _rule_impl)
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in _my_subrule: rule 'my_rule' must declare '_my_subrule' in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testSubrule_attrLengthLimitAppliedOnUserDefinedName
#[test]
fn test_subrule_attr_length_limit_applied_on_user_defined_name() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[&format!(
            r#"_my_subrule = subrule(
  implementation = lambda ctx: "dummy rule result",
  attrs = {{
    "_{}": attr.label(default = "//subrule_testing:bar")
  }}
)

my_rule = rule(implementation = lambda ctx: [], subrules = [_my_subrule])
"#,
            "a".repeat(130)
        )],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
cc_binary(name = "bar")
"#],
    );

    let error = assert_throws_assertion_error(|| {
        let _ = t.get_target("//subrule_testing:foo");
    });

    assert_message_contains(&error, "name is too long (131 > 128)");
}

// Java: StarlarkSubruleTest.testSubrule_attrLengthLimitIgnoresBzlLabel
#[test]
fn test_subrule_attr_length_limit_ignores_bzl_label() {
    let mut t = BuildViewTestCase::new();
    // deserialized package does not have a reference to subrules
    // PORT: `initializeSkyframeExecutor(/* doPackageLoadingChecks= */ false)`: the harness does
    // no package loading checks (no serialization round trip of packages).

    let long_package_name = "a".repeat(130);
    t.scratch
        .file(&format!("subrule_testing/{long_package_name}/BUILD"), &[]);
    t.scratch.file(
        &format!("subrule_testing/{long_package_name}/myrule.bzl"),
        &[r#"_my_subrule = subrule(
  implementation = lambda ctx: "dummy rule result",
  attrs = {
    "_a": attr.label(default = "//subrule_testing:bar")
  }
)

my_rule = rule(implementation = lambda ctx: [], subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[&format!(
            r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")
load("//subrule_testing/{long_package_name}:myrule.bzl", "my_rule")

my_rule(name = "foo")
cc_binary(name = "bar")
"#
        )],
    );

    t.get_target("//subrule_testing:foo")
        .unwrap_or_else(|e| panic!("{e:?}"));

    t.assert_no_events();
}

// Java: StarlarkSubruleTest.testSubrule_childAndParentSubrules
#[test]
fn test_subrule_child_and_parent_subrules() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/parent.bzl",
        &[
            "_parent_subrule = subrule(implementation = lambda ctx: None)",
            "def _rule_impl(ctx):",
            "  _parent_subrule()",
            "parent_rule = rule(implementation = _rule_impl, extendable = True, subrules = [_parent_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/child.bzl",
        &[
            "load('parent.bzl', 'parent_rule')",
            "_child_subrule = subrule(implementation = lambda ctx: None)",
            "def _rule_impl(ctx):",
            "  ctx.super()",
            "  _child_subrule()",
            "child_rule = rule(implementation = _rule_impl, parent = parent_rule, subrules = [_child_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("child.bzl", "child_rule")

child_rule(name = "foo")
"#],
    );

    t.get_configured_target("//subrule_testing:foo");

    t.assert_no_events();
}

// Java: StarlarkSubruleTest.testSubrule_childAndParent_canUseTheSameSubrule
#[test]
fn test_subrule_child_and_parent_can_use_the_same_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/parent.bzl",
        &[
            "parent_subrule = subrule(",
            "  implementation = lambda ctx, _tool: None,",
            "  attrs = {'_tool': attr.label(default = ':tool')}",
            ")",
            "def _rule_impl(ctx):",
            "  parent_subrule()",
            "parent_rule = rule(implementation = _rule_impl, extendable = True, subrules = [parent_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/child.bzl",
        &[
            "load('parent.bzl', 'parent_rule', 'parent_subrule')",
            "def _rule_impl(ctx):",
            "  ctx.super()",
            "  parent_subrule()",
            "child_rule = rule(implementation = _rule_impl, parent = parent_rule, subrules = [parent_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("child.bzl", "child_rule")

child_rule(name = "foo")

filegroup(name = "tool")
"#],
    );

    t.get_configured_target("//subrule_testing:foo");

    t.assert_no_events();
}

// Java: StarlarkSubruleTest.testSubrule_childCantUseParentsSubrule
#[test]
fn test_subrule_child_cant_use_parents_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/parent.bzl",
        &[
            "parent_subrule = subrule(implementation = lambda ctx: None)",
            "def _rule_impl(ctx):",
            "  parent_subrule()",
            "parent_rule = rule(implementation = _rule_impl, extendable = True, subrules = [parent_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/child.bzl",
        &[r#"load("parent.bzl", "parent_rule", "parent_subrule")

_child_subrule = subrule(implementation = lambda ctx: None)

def _rule_impl(ctx):
    ctx.super()
    parent_subrule()

child_rule = rule(implementation = _rule_impl, parent = parent_rule)
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("child.bzl", "child_rule")

child_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in parent_subrule: rule 'child_rule' must declare 'parent_subrule' in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testSubrule_parentCantUseChildsSubrule
#[test]
fn test_subrule_parent_cant_use_childs_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/parent.bzl",
        &[r#"my_subrule = subrule(implementation = lambda ctx: None)

def _rule_impl(ctx):
    my_subrule()

parent_rule = rule(implementation = _rule_impl, extendable = True)
"#],
    );
    t.scratch.file(
        "subrule_testing/child.bzl",
        &[
            "load('parent.bzl', 'parent_rule', 'my_subrule')",
            "def _rule_impl(ctx):",
            "  ctx.super()",
            "  my_subrule()",
            "child_rule = rule(implementation = _rule_impl, parent = parent_rule, subrules = [my_subrule])",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("child.bzl", "child_rule")

child_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in my_subrule: rule 'parent_rule' must declare 'my_subrule' in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testSubrule_aspectMustDeclareSubrule
#[test]
fn test_subrule_aspect_must_declare_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            r#"_my_subrule = subrule(implementation = lambda ctx: "dummy aspect result")

def _aspect_impl(ctx, target):
    res = _my_subrule()

_my_aspect = aspect(implementation = _aspect_impl)

my_rule = rule(
    implementation = lambda ctx: [],
    attrs = {"dep": attr.label(mandatory = True, aspects = [_my_aspect])},
)
"#,
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
load("myrule.bzl", "my_rule")

java_library(name = "bar")

my_rule(
    name = "foo",
    dep = "bar",
)
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in _my_subrule: aspect '//subrule_testing:myrule.bzl%_my_aspect' must declare '_my_subrule' in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testSubruleCallingUndeclaredSibling_fails
#[test]
fn test_subrule_calling_undeclared_sibling_fails() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule1_impl(ctx):
    return "result from subrule1"

_my_subrule1 = subrule(implementation = _subrule1_impl)

def _subrule2_impl(ctx):
    return _my_subrule1()

_my_subrule2 = subrule(implementation = _subrule2_impl)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule2()
    return [MyInfo(result = res)]

my_rule = rule(_rule_impl, subrules = [_my_subrule2, _my_subrule1])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "subrule _my_subrule2 must declare _my_subrule1 in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testSubrule_implementationMustAcceptSubruleContext
#[test]
fn test_subrule_implementation_must_accept_subrule_context() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"_my_subrule = subrule(implementation = lambda: "")

def _rule_impl(ctx):
    _my_subrule()

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error: lambda() does not accept positional arguments, but got 1",
    );
}

// Java: StarlarkSubruleTest.testSubrule_isCallableFromRule
#[test]
fn test_subrule_is_callable_from_rule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            r#"_my_subrule = subrule(implementation = lambda ctx: "dummy rule result")

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#,
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(string_value(&provider, "result"), "dummy rule result");
}

// Java: StarlarkSubruleTest.testSubrule_isCallableFromAspect
#[test]
fn test_subrule_is_callable_from_aspect() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            r#"_my_subrule = subrule(implementation = lambda ctx: "dummy aspect result")

MyInfo = provider()

def _aspect_impl(ctx, target):
    res = _my_subrule()
    return MyInfo(result = res)

_my_aspect = aspect(implementation = _aspect_impl, subrules = [_my_subrule])

my_rule = rule(
    implementation = lambda ctx: [ctx.attr.dep[MyInfo]],
    attrs = {"dep": attr.label(mandatory = True, aspects = [_my_aspect])},
)
"#,
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
load("myrule.bzl", "my_rule")

java_library(name = "bar")

my_rule(
    name = "foo",
    dep = "bar",
)
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(string_value(&provider, "result"), "dummy aspect result");
}

// Java: StarlarkSubruleTest.testSubrule_subruleContextExposesRuleLabel
#[test]
fn test_subrule_subrule_context_exposes_rule_label() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return "called in: " + str(ctx.label)

_my_subrule = subrule(implementation = _subrule_impl)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(
        string_value(&provider, "result"),
        "called in: @@//subrule_testing:foo"
    );
}

// Java: StarlarkSubruleTest.testSubrule_subruleContextExposesActionsApi
#[test]
fn test_subrule_subrule_context_exposes_actions_api() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.write(out, "subrule file content")
    return out

_my_subrule = subrule(implementation = _subrule_impl)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let artifact = artifact_value(&provider, "result");

    assert_eq!(artifact.get_filename(), "foo.out");
    let action = t
        .get_generating_action(&artifact)
        .expect("generating action");
    let file_write_action = action
        .as_any()
        .downcast_ref::<FileWriteAction>()
        .expect("ClassCastException: FileWriteAction");
    assert_eq!(
        file_write_action.get_file_contents(),
        "subrule file content"
    );
}

// Java: StarlarkSubruleTest.testSubruleActions_run_doesNotAllowSettingToolchain
#[test]
fn test_subrule_actions_run_does_not_allow_setting_toolchain() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run(toolchain = "foo", executable = "/path/to/tool", outputs = [out])

_my_subrule = subrule(implementation = _subrule_impl)

MyInfo = provider()

def _rule_impl(ctx):
    _my_subrule()

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(&error, "'toolchain' may not be specified in subrules");
}

// Java: StarlarkSubruleTest.testSubruleActions_run_doesNotAllowSettingExecGroup
#[test]
fn test_subrule_actions_run_does_not_allow_setting_exec_group() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run(exec_group = "foo", executable = "/path/to/tool", outputs = [out])

_my_subrule = subrule(implementation = _subrule_impl)

MyInfo = provider()

def _rule_impl(ctx):
    _my_subrule()

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(&error, "'exec_group' may not be specified in subrules");
}

// Java: StarlarkSubruleTest.testSubruleContext_cannotBeUsedOutsideImplementationFunction
#[test]
fn test_subrule_context_cannot_be_used_outside_implementation_function() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return ctx

_my_subrule = subrule(implementation = _subrule_impl)

def _rule_impl(ctx):
    subrule_ctx = _my_subrule()
    subrule_ctx.label

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error: cannot access field or method 'label' of subrule context outside of its own implementation function",
    );
}

// Java: StarlarkSubruleTest.testRuleContext_cannotBeUsedInSubruleImplementation
#[test]
fn test_rule_context_cannot_be_used_in_subrule_implementation() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, rule_ctx):
    rule_ctx.label

_my_subrule = subrule(implementation = _subrule_impl)

def _rule_impl(ctx):
    subrule_ctx = _my_subrule(ctx)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error: cannot access field or method 'label' of rule context for '//subrule_testing:foo' outside of its own rule implementation function",
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_publicAttributesAreNotPermitted
#[test]
fn test_subrule_attrs_public_attributes_are_not_permitted() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "illegal attribute name 'foo': subrules may only define private attributes",
        &["subrule(implementation = lambda: None, attrs = {'foo': attr.string()})"],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_computedDefaultsAreNotPermitted
#[test]
fn test_subrule_attrs_computed_defaults_are_not_permitted() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "for attribute '_foo': subrules cannot define computed defaults.",
        &[
            "subrule(",
            "  implementation = lambda: None,",
            "  attrs = {'_foo': attr.label(default = lambda: '')}",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_onlyLabelsOrLabelListsPermitted
#[test]
fn test_subrule_attrs_only_labels_or_label_lists_permitted() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "bad type for attribute '_foo': subrule attributes may only be label or lists of labels.",
        &[
            "subrule(",
            "  implementation = lambda: None,",
            "  attrs = {'_foo': attr.int()}",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_attributeMustHaveDefaultValue
#[test]
fn test_subrule_attrs_attribute_must_have_default_value() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "for attribute '_foo': no default value specified",
        &[
            "subrule(",
            "  implementation = lambda: None,",
            "  attrs = {'_foo': attr.label()}",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_cannotHaveStarlarkTransitions
#[test]
fn test_subrule_attrs_cannot_have_starlark_transitions() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "bad cfg for attribute '_foo': subrules may only have target/exec attributes.",
        &[
            "my_transition = transition(implementation = lambda: None, inputs = [], outputs = [])",
            "_my_subrule = subrule(",
            "  implementation = lambda: None,",
            "  attrs = {'_foo': attr.label(cfg = my_transition)}",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_cannotHaveNativeTransitions
#[test]
#[ignore = "needs BazelEvaluationTestCase.update: a native test transition value (the Java test's NativeTransition, a TransitionFactory<AttributeTransitionData> implementing ConfigurationTransitionApi) bound in the .bzl environment"]
fn test_subrule_attrs_cannot_have_native_transitions() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    // Java: ev.update("native_transition", new NativeTransition());
    ev.check_eval_error_contains(
        &mut t,
        "bad cfg for attribute '_foo': subrules may only have target/exec attributes.",
        &[
            "_my_subrule = subrule(",
            "  implementation = lambda: None,",
            "  attrs = {'_foo': attr.label(cfg = native_transition)}",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_notVisibleInRuleCtx
#[test]
#[ignore = "needs BuildViewTestCase.getRuleContext (getRuleContext(target).getRule().getRuleClassObject())"]
fn test_subrule_attrs_not_visible_in_rule_ctx() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "default/BUILD",
        &["genrule(name = 'default', outs = ['a'], cmd = '')"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_foo": attr.label(default = "//default")},
)
MyInfo = provider()

def _rule_impl(ctx):
    res = dir(ctx.attr)
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    // Java: ruleClassAttributes = getRuleContext(getConfiguredTarget("//subrule_testing:foo"))
    //     .getRule().getRuleClassObject().getAttributeProvider().getAttributes() names;
    //     assertThat(ruleClassAttributes).contains(ruleAttrName);
    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let attributes_visible_to_starlark = string_list_value(&provider, "result");
    let rule_attr_name = rule_attr_name("//subrule_testing:myrule.bzl", "_my_subrule", "_foo");

    assert!(!attributes_visible_to_starlark.contains(&rule_attr_name));
}

// Java: StarlarkSubruleTest.testSubruleAttrs_notVisibleInAspectCtx
#[test]
fn test_subrule_attrs_not_visible_in_aspect_ctx() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "default/BUILD",
        &["genrule(name = 'default', outs = ['a'], cmd = '')"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"_my_subrule = subrule(
    implementation = lambda: None,
    attrs = {"_foo": attr.label(default = "//default")},
)
MyInfo = provider()

def _aspect_impl(target, ctx):
    res = dir(ctx.attr)
    return MyInfo(result = res)

my_aspect = aspect(implementation = _aspect_impl, subrules = [_my_subrule])

def _rule_impl(ctx):
    return ctx.attr.dep[MyInfo]

my_rule = rule(
    implementation = _rule_impl,
    attrs = {"dep": attr.label(aspects = [my_aspect])},
)
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(
    name = "foo",
    dep = "//default",
)
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let attributes_visible_to_starlark = string_list_value(&provider, "result");
    let aspect = get_aspect(&mut t, "//subrule_testing:myrule.bzl%my_aspect");
    let aspect_attributes = aspect.get_aspect().get_definition().get_attributes();
    let rule_attr_name = rule_attr_name("//subrule_testing:myrule.bzl", "_my_subrule", "_foo");

    assert!(
        aspect_attributes.contains_key(&rule_attr_name),
        "{rule_attr_name} not in {:?}",
        aspect_attributes.keys().collect::<Vec<_>>()
    );
    assert!(!attributes_visible_to_starlark.contains(&rule_attr_name));
}

// Java: StarlarkSubruleTest.testSubruleAttrs_overridingImplicitAttributeValueFails
#[test]
fn test_subrule_attrs_overriding_implicit_attribute_value_fails() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "default/BUILD",
        &["genrule(name = 'default', outs = ['a'], cmd = '')"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _foo):
    return

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_foo": attr.label(default = "//default")},
)

def _rule_impl(ctx):
    res = _my_subrule(_foo = "//override")
    return []

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in _my_subrule: got invalid named argument: '_foo' is an implicit dependency and cannot be overridden",
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_implicitLabelDepsAreResolvedToTargets_inRule
#[test]
fn test_subrule_attrs_implicit_label_deps_are_resolved_to_targets_in_rule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "some/pkg/BUILD",
        &["genrule(name = 'tool', cmd = '', outs = ['tool.exe'])"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    return _tool

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(default = "//some/pkg:tool")},
)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(
        configured_target_label_value(&provider, "result"),
        "//some/pkg:tool"
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_implicitLabelDepsAreResolvedToTargets_inAspect
#[test]
fn test_subrule_attrs_implicit_label_deps_are_resolved_to_targets_in_aspect() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "some/pkg/BUILD",
        &["genrule(name = 'tool', cmd = '', outs = ['tool.exe'])"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    return _tool

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(default = "//some/pkg:tool")},
)

MyInfo = provider()

def _aspect_impl(ctx, target):
    res = _my_subrule()
    return [MyInfo(result = res)]

_my_aspect = aspect(implementation = _aspect_impl, subrules = [_my_subrule])

my_rule = rule(
    implementation = lambda ctx: [ctx.attr.dep[MyInfo]],
    attrs = {"dep": attr.label(mandatory = True, aspects = [_my_aspect])},
)
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")
filegroup(name = 'bar')
my_rule(name = "foo", dep = ":bar")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(
        configured_target_label_value(&provider, "result"),
        "//some/pkg:tool"
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_singleFileLabelAttributesAreResolvedToFile
#[test]
fn test_subrule_attrs_single_file_label_attributes_are_resolved_to_file() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "some/pkg/BUILD",
        &["genrule(name = 'tool', cmd = '', outs = ['tool.exe'])"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    return _tool

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(allow_single_file = True, default = "//some/pkg:tool")},
)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(
        artifact_value(&provider, "result").get_root_relative_path_string(),
        "some/pkg/tool.exe"
    );
}

// Java: StarlarkSubruleTest.testSubruleAttr_executableAttrIsPassedAsFilesToRun
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn test_subrule_attr_executable_attr_is_passed_as_files_to_run() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "my/BUILD",
        &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")
cc_binary(name = 'tool')
"#],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    return _tool

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(default = "//my:tool", executable = True, cfg = "exec")},
)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");

    // Java: assertThat(result).isInstanceOf(FilesToRunProvider.class);
    //       ((FilesToRunProvider) result).getExecutable().getRootRelativePathString()
    //           .isEqualTo("my/tool")
    let executable = with_value(&provider, "result", |v| {
        let files_to_run =
            blaze_analysis::starlark::starlark_action_factory::files_to_run_provider_from_value(v)
                .unwrap_or_else(|| panic!("ClassCastException: {v} is not a FilesToRunProvider"));
        files_to_run
            .get_executable()
            .expect("executable")
            .get_root_relative_path_string()
    });
    assert_eq!(executable, "my/tool");
}

// Java: StarlarkSubruleTest.testSubruleAction_executableMustBeFilesToRunProvider
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn test_subrule_action_executable_must_be_files_to_run_provider() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "my/BUILD",
        &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")
cc_binary(name = 'tool')
"#],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run(executable = _tool.executable, outputs = [out])
    return out

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(default = "//my:tool", executable = True, cfg = "exec")},
)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "Error in run: for 'executable', expected FilesToRunProvider, got File",
    );
}

// Java: StarlarkSubruleTest.testSubruleAttrs_lateBoundDefaultsAreResolved
#[test]
#[ignore = "needs the mock C++ toolchains (cc_binary from the mock rules_cc)"]
fn test_subrule_attrs_late_bound_defaults_are_resolved() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "my/BUILD",
        &[r#"load("@rules_cc//cc:cc_binary.bzl", "cc_binary")
cc_binary(name = 'tool')
"#],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx, _tool):
    return _tool

_my_subrule = subrule(
    implementation = _subrule_impl,
    attrs = {"_tool": attr.label(
        default = configuration_field(fragment = "coverage", name = "output_generator"),
    )},
)

MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(implementation = _rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );
    // TODO: b/293304174 - use a custom fragment instead of coverage
    t.use_configuration(&[
        "--collect_code_coverage",
        "--coverage_output_generator=//my:tool",
    ]);

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    let provider = provider.expect("provider is null");
    assert_eq!(
        configured_target_label_value(&provider, "result"),
        "//my:tool"
    );
}

// Java: StarlarkSubruleTest.testSubruleToolchains_cannotRequireMoreThanOne
#[test]
fn test_subrule_toolchains_cannot_require_more_than_one() {
    let mut t = BuildViewTestCase::new();
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "subrules may require at most 1 toolchain",
        &[
            "_my_subrule = subrule(",
            "  implementation = lambda: None,",
            "  toolchains = ['//t1', '//t2'],",
            ")",
        ],
    );
}

// Java: StarlarkSubruleTest.testSubruleToolchains_cannotAccessUnrequestedToolchain
#[test]
fn test_subrule_toolchains_cannot_access_unrequested_toolchain() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    let toolchain_line = format!("  ctx.toolchains['{JAVA_TOOLCHAIN_TYPE}']");
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            "def _subrule_impl(ctx):",
            &toolchain_line,
            "_my_subrule = subrule(",
            "  implementation = _subrule_impl,",
            ")",
            "",
            "def _rule_impl(ctx):",
            "  _my_subrule()",
            "",
            "my_rule = rule(",
            "  implementation = _rule_impl,",
            "  subrules = [_my_subrule],",
            ")",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    // Java: assertThrows(JAVA_TOOLCHAIN_TYPE + " was requested but only types [] are configured",
    //     AssertionError.class, ...): the string is the failure message, not an expectation.
    assert_throws_assertion_error(|| {
        t.get_configured_target("//subrule_testing:foo");
    });
}

// Java: StarlarkSubruleTest.testSubruleToolchains_cannotAccessToolchainFromRule
#[test]
#[ignore = "needs the mock Java toolchains (@bazel_tools//tools/jdk:toolchain_type with a registered Java toolchain)"]
fn test_subrule_toolchains_cannot_access_toolchain_from_rule() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    let toolchain_line = format!("  ctx.toolchains['{JAVA_TOOLCHAIN_TYPE}']");
    let rule_toolchains_line = format!("  toolchains = ['{JAVA_TOOLCHAIN_TYPE}'],");
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            "def _subrule_impl(ctx):",
            &toolchain_line,
            "_my_subrule = subrule(",
            "  implementation = _subrule_impl,",
            ")",
            "",
            "def _rule_impl(ctx):",
            "  _my_subrule()",
            "",
            "my_rule = rule(",
            "  implementation = _rule_impl,",
            "  subrules = [_my_subrule],",
            &rule_toolchains_line,
            ")",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    // Java: assertThrows(JAVA_TOOLCHAIN_TYPE + " was requested but only types [] are configured",
    //     AssertionError.class, ...): the string is the failure message, not an expectation.
    assert_throws_assertion_error(|| {
        t.get_configured_target("//subrule_testing:foo");
    });
}

// Java: StarlarkSubruleTest.testSubruleToolchains_requestedToolchainIsResolved_inRule
#[test]
#[ignore = "needs the mock Java toolchains (@bazel_tools//tools/jdk:toolchain_type with a registered Java toolchain, JavaToolchainProvider)"]
fn test_subrule_toolchains_requested_toolchain_is_resolved_in_rule() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    let toolchain_line = format!("  return ctx.toolchains['{JAVA_TOOLCHAIN_TYPE}']");
    let subrule_toolchains_line = format!("  toolchains = ['{JAVA_TOOLCHAIN_TYPE}'],");
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            "def _subrule_impl(ctx):",
            &toolchain_line,
            "_my_subrule = subrule(",
            "  implementation = _subrule_impl,",
            &subrule_toolchains_line,
            ")",
            "MyInfo = provider()",
            "def _rule_impl(ctx):",
            "  return [MyInfo(result = _my_subrule())]",
            "",
            "my_rule = rule(",
            "  implementation = _rule_impl,",
            "  subrules = [_my_subrule],",
            ")",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");

    // Java: toolchainInfo = provider.getValue("result", ToolchainInfo.class); not null;
    //   toolchainInfo.getValue("java", StarlarkInfo.class).getProvider().getKey()
    //     .isEqualTo(JavaToolchainProvider.PROVIDER.getKey())
    let toolchain_info_type = with_value(&provider, "result", |v| v.get_type().to_string());
    assert_eq!(toolchain_info_type, "ToolchainInfo");
}

// Java: StarlarkSubruleTest.testSubruleToolchains_requstedToolchainIsResolved_inAspect
#[test]
#[ignore = "needs the mock Java toolchains (@bazel_tools//tools/jdk:toolchain_type with a registered Java toolchain, JavaToolchainProvider)"]
fn test_subrule_toolchains_requsted_toolchain_is_resolved_in_aspect() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    t.scratch.file(
        "default/BUILD",
        &["genrule(name = 'default', outs = ['a'], cmd = '')"],
    );
    let toolchain_line = format!("  return ctx.toolchains['{JAVA_TOOLCHAIN_TYPE}']");
    let subrule_toolchains_line = format!("  toolchains = ['{JAVA_TOOLCHAIN_TYPE}'],");
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            "def _subrule_impl(ctx):",
            &toolchain_line,
            "_my_subrule = subrule(",
            "  implementation = _subrule_impl,",
            &subrule_toolchains_line,
            ")",
            "MyInfo=provider()",
            "def _aspect_impl(target, ctx):",
            "  return MyInfo(result = _my_subrule())",
            "my_aspect = aspect(implementation = _aspect_impl, subrules = [_my_subrule])",
            "def _rule_impl(ctx):",
            "  return ctx.attr.dep[MyInfo]",
            "my_rule = rule(",
            "  implementation = _rule_impl,",
            "  attrs = {'dep' : attr.label(aspects = [my_aspect])}",
            ")",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(
    name = "foo",
    dep = "//default",
)
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    );

    // Java: toolchainInfo = provider.getValue("result", ToolchainInfo.class); not null;
    //   toolchainInfo.getValue("java", StarlarkInfo.class).getProvider().getKey()
    //     .isEqualTo(JavaToolchainProvider.PROVIDER.getKey())
    assert!(provider.is_some());
}

// Java: StarlarkSubruleTest.testSubruleToolchains_noToolchainIsSuppliedToAction
#[test]
#[ignore = "needs BuildViewTestCase.getRuleContext (getRuleContext(target).getActionOwner())"]
fn test_subrule_toolchains_no_toolchain_is_supplied_to_action() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run(outputs = [out], executable = "/bin/ls", tools = [depset()])
    return out

_my_subrule = subrule(
    implementation = _subrule_impl,
)

def _rule_impl(ctx):
    return [DefaultInfo(files = depset([_my_subrule()]))]

my_rule = rule(
    implementation = _rule_impl,
    subrules = [_my_subrule],
)
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let target = t
        .get_configured_target("//subrule_testing:foo")
        .expect("configured target");
    let action = t.get_generating_action_for_output(&target, "subrule_testing/foo.out");

    assert!(action.is_some());
    // Java: assertThat(action.getOwner()).isEqualTo(getRuleContext(target).getActionOwner());
}

// Java: StarlarkSubruleTest.testSubruleToolchains_requestedToolchainIsSuppliedToAction
#[test]
#[ignore = "needs the mock Java toolchains (@bazel_tools//tools/jdk:toolchain_type with a registered Java toolchain) and BuildViewTestCase.getRuleContext (getActionOwner(toolchainType))"]
fn test_subrule_toolchains_requested_toolchain_is_supplied_to_action() {
    let mut t = BuildViewTestCase::new();
    t.use_configuration(&["--incompatible_auto_exec_groups"]);
    let subrule_toolchains_line = format!("  toolchains = ['{JAVA_TOOLCHAIN_TYPE}'],");
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[
            "def _subrule_impl(ctx):",
            "  out = ctx.actions.declare_file(ctx.label.name + '.out')",
            "  ctx.actions.run(outputs = [out], executable = '/bin/ls', tools = [depset()])",
            "  return out",
            "_my_subrule = subrule(",
            "  implementation = _subrule_impl,",
            &subrule_toolchains_line,
            ")",
            "def _rule_impl(ctx):",
            "  return [DefaultInfo(files = depset([_my_subrule()]))]",
            "",
            "my_rule = rule(",
            "  implementation = _rule_impl,",
            "  subrules = [_my_subrule],",
            ")",
        ],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let target = t
        .get_configured_target("//subrule_testing:foo")
        .expect("configured target");
    let action = t.get_generating_action_for_output(&target, "subrule_testing/foo.out");

    assert!(action.is_some());
    // Java: assertThat(action.getOwner())
    //     .isEqualTo(getRuleContext(target).getActionOwner(TestConstants.JAVA_TOOLCHAIN_TYPE));
}

// Java: StarlarkSubruleTest.testSubruleFragments_errorForInvalidFragments
#[test]
fn test_subrule_fragments_error_for_invalid_fragments() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return ctx.fragments.foobar

_my_subrule = subrule(
    implementation = _subrule_impl,
    fragments = ["java", "cpp"],
)
MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let assertion_error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &assertion_error,
        "There is no configuration fragment named 'foobar'. Available fragments: 'java', 'cpp'",
    );
}

// Java: StarlarkSubruleTest.testSubruleFragments_onlyDeclaredFragmentsAreVisible
#[test]
fn test_subrule_fragments_only_declared_fragments_are_visible() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return dir(ctx.fragments)

_my_subrule = subrule(
    implementation = _subrule_impl,
    fragments = ["cpp", "python"],
)
MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_my_subrule], fragments = ["java"])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let mut fragments = string_list_value(&provider, "result");

    // containsExactly("cpp", "python") (any order)
    fragments.sort();
    assert_eq!(fragments, ["cpp", "python"]);
}

// Java: StarlarkSubruleTest.testSubruleFragments_ruleCannotAccessSubruleFragments
#[test]
fn test_subrule_fragments_rule_cannot_access_subrule_fragments() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    pass

_my_subrule = subrule(
    implementation = _subrule_impl,
    fragments = ["cpp"],
)

def _rule_impl(ctx):
    ctx.fragments.cpp
    return []

my_rule = rule(_rule_impl, subrules = [_my_subrule])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let assertion_error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &assertion_error,
        "my_rule has to declare 'cpp' as a required fragment in order to access it",
    );
}

// Java: StarlarkSubruleTest.testSubruleFragments_canAccessDeclaredFragments
#[test]
fn test_subrule_fragments_can_access_declared_fragments() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    return ctx.fragments.cpp

_my_subrule = subrule(
    implementation = _subrule_impl,
    fragments = ["cpp"],
)
MyInfo = provider()

def _rule_impl(ctx):
    res = _my_subrule()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_my_subrule], fragments = ["java"])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");

    // Java: getValue("result", CppConfigurationApi.class) is not null: the value is the
    // Starlark type of CppConfigurationApi (`@StarlarkBuiltin(name = "cpp")`).
    let fragment_type = with_value(&provider, "result", |v| v.get_type().to_string());
    assert_eq!(fragment_type, "cpp");
}

// Java: StarlarkSubruleTest.testSubruleFragments_mustDeclareFragmentsIfAccessed
#[test]
fn test_subrule_fragments_must_declare_fragments_if_accessed() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _subrule_impl(ctx):
    ctx.fragments.java

_my_subrule = subrule(
    implementation = _subrule_impl,
    fragments = ["cpp", "python"],
)

def _rule_impl(ctx):
    res = _my_subrule()
    return []

my_rule = rule(_rule_impl, subrules = [_my_subrule], fragments = ["java"])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let assertion_error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &assertion_error,
        "_my_subrule has to declare 'java' as a required fragment in order to access it",
    );
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_subruleMustDeclareCalledSubrule
#[test]
fn test_transitive_subrules_subrule_must_declare_called_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx):
    return "from subruleA"

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    return _A()

_B = subrule(implementation = _B_impl)

def _rule_impl(ctx):
    res = _B()

my_rule = rule(_rule_impl, subrules = [_B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let assertion_error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &assertion_error,
        "Error in _A: subrule _B must declare _A in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_ruleCannotCallUndeclaredTransitiveSubrule
#[test]
fn test_transitive_subrules_rule_cannot_call_undeclared_transitive_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx):
    return "from subruleA"

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    return _A()

_B = subrule(implementation = _B_impl, subrules = [_A])

def _rule_impl(ctx):
    _A()

my_rule = rule(_rule_impl, subrules = [_B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let assertion_error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &assertion_error,
        "rule 'my_rule' must declare '_A' in 'subrules'",
    );
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_subruleCanCallDeclaredSubrule
#[test]
fn test_transitive_subrules_subrule_can_call_declared_subrule() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx):
    return "from subruleA"

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    return _A()

_B = subrule(implementation = _B_impl, subrules = [_A])

MyInfo = provider()

def _rule_impl(ctx):
    res = _B()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let result = string_value(&provider, "result");

    assert_eq!(result, "from subruleA");
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_arbitrarilyLongTransitiveChainsAreResolved
#[test]
fn test_transitive_subrules_arbitrarily_long_transitive_chains_are_resolved() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/BUILD",
        &["genrule(name = 'tool', cmd = '', outs = ['tool.out'])"],
    );
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx, _tool):
    return "tool name: " + _tool.label.name

_A = subrule(implementation = _A_impl, attrs = {"_tool": attr.label(default = "//a:tool")})
_B = subrule(implementation = lambda ctx: _A(), subrules = [_A])
_C = subrule(implementation = lambda ctx: _B(), subrules = [_B])
_D = subrule(implementation = lambda ctx: _C(), subrules = [_C])

MyInfo = provider()

def _rule_impl(ctx):
    return MyInfo(result = _D())

my_rule = rule(_rule_impl, subrules = [_D])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let result = string_value(&provider, "result");

    assert_eq!(result, "tool name: tool");
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_ruleAndSubruleCanHaveCommonSubruleDependency
#[test]
fn test_transitive_subrules_rule_and_subrule_can_have_common_subrule_dependency() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx):
    return "from subruleA"

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    _A()
    return "from subruleB"

_B = subrule(implementation = _B_impl, subrules = [_A])

MyInfo = provider()

def _rule_impl(ctx):
    resA = _A()
    resB = _B()
    return MyInfo(resA = resA, resB = resB)

my_rule = rule(_rule_impl, subrules = [_A, _B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let res_a = string_value(
        &get_provider(
            &mut t,
            "//subrule_testing:foo",
            "//subrule_testing:myrule.bzl",
            "MyInfo",
        )
        .expect("provider is null"),
        "resA",
    );
    let res_b = string_value(
        &get_provider(
            &mut t,
            "//subrule_testing:foo",
            "//subrule_testing:myrule.bzl",
            "MyInfo",
        )
        .expect("provider is null"),
        "resB",
    );

    assert_eq!(res_a, "from subruleA");
    assert_eq!(res_b, "from subruleB");
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_callerSubruleCtxIsLocked
#[test]
fn test_transitive_subrules_caller_subrule_ctx_is_locked() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx, ctxB):
    return ctxB.label

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    return _A(ctx)

_B = subrule(implementation = _B_impl, subrules = [_A])

MyInfo = provider()

def _rule_impl(ctx):
    res = _B()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let error = configured_target_error(&mut t, "//subrule_testing:foo");

    assert_message_contains(
        &error,
        "cannot access field or method 'label' of subrule context outside of its own implementation function",
    );
}

// Java: StarlarkSubruleTest.testTransitiveSubrules_callerSubruleCtxIsUnlockedUponResumption
#[test]
fn test_transitive_subrules_caller_subrule_ctx_is_unlocked_upon_resumption() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "subrule_testing/myrule.bzl",
        &[r#"def _A_impl(ctx):
    return "from A"

_A = subrule(implementation = _A_impl)

def _B_impl(ctx):
    _A()
    return "from B: " + ctx.label.name

_B = subrule(implementation = _B_impl, subrules = [_A])

MyInfo = provider()

def _rule_impl(ctx):
    res = _B()
    return MyInfo(result = res)

my_rule = rule(_rule_impl, subrules = [_B])
"#],
    );
    t.scratch.file(
        "subrule_testing/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    let provider = get_provider(
        &mut t,
        "//subrule_testing:foo",
        "//subrule_testing:myrule.bzl",
        "MyInfo",
    )
    .expect("provider is null");
    let result = string_value(&provider, "result");

    assert_eq!(result, "from B: foo");
}

// Java: StarlarkSubruleTest.testSubrulesParamForRule_isPrivateAPI
#[test]
fn test_subrules_param_for_rule_is_private_api() {
    let mut t = BuildViewTestCase::new();
    t.set_build_language_options(&["--noexperimental_rule_extension_api"]);
    t.scratch.file(
        "foo/myrule.bzl",
        &[r#"def _impl(ctx):
    pass
my_subrule = subrule(implementation = _impl)
my_rule = rule(_impl, subrules = [my_subrule])
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load("myrule.bzl", "my_rule")

my_rule(name = "foo")
"#],
    );

    t.remove_fail_fast_handler();
    // Java: reporter.addHandler(ev.getEventCollector()): the test's event collector also
    // receives the reporter's events.
    t.get_configured_target("//foo");

    t.assert_contains_event_of_kinds(
        "Non-allowlisted attempt to use subrules.",
        EventKind::ERRORS,
    );
}

// Java: StarlarkSubruleTest.testSubrulesParamForAspect_isPrivateAPI
#[test]
#[ignore = "needs BazelEvaluationTestCase labelled //foo:bar (evOutsideAllowlist): a .bzl file loaded by a package must have a label ending in .bzl, so the expected '//foo:bar' cannot be reproduced"]
fn test_subrules_param_for_aspect_is_private_api() {
    let mut t = BuildViewTestCase::new();
    // Java: evOutsideAllowlist.setSemantics("--noexperimental_rule_extension_api");
    t.set_build_language_options(&["--noexperimental_rule_extension_api"]);
    let ev = Ev::new();
    ev.check_eval_error_contains(
        &mut t,
        "'//foo:bar' cannot use private API",
        &["aspect(implementation = lambda: 0, subrules = [1])"],
    );
}
