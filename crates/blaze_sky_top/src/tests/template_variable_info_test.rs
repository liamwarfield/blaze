//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/platform/TemplateVariableInfoTest.java`.

use blaze_analysis::info::downcast_info;
use blaze_analysis::platform::template_variable_info::TemplateVariableInfo;
use blaze_packages::starlark_defs::platform_common::TEMPLATE_VARIABLE_INFO;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::dict::DictRef;
use starlark::values::{OwnedFrozenValue, Value};

use crate::testing::BuildViewTestCase;

/// `info` as a value (the frozen heap is kept alive by `info`).
fn frozen<'v>(info: &'v OwnedFrozenValue) -> Value<'v> {
    // SAFETY: the value is used only while `info` (which keeps its frozen heap alive) is
    // borrowed.
    unsafe { info.unchecked_frozen_value() }.to_value()
}

// Java: TemplateVariableInfoTest.proxyTemplateVariableInfo
#[test]
#[ignore = "needs the mock C++ toolchain (cc_toolchain_alias from the mock rules_cc)"]
fn proxy_template_variable_info() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/rule.bzl",
        &[r#"def _impl(ctx):
    return [ctx.attr._cc_toolchain[platform_common.TemplateVariableInfo]]

crule = rule(_impl, attrs = {"_cc_toolchain": attr.label(default = Label("//a:a"))})
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[
            r#"load("@rules_cc//cc/toolchains:cc_toolchain_alias.bzl", "cc_toolchain_alias")
load(":rule.bzl", "crule")

cc_toolchain_alias(name = "a")

crule(name = "r")

genrule(
    name = "g",
    srcs = [],
    outs = ["go"],
    cmd = "VAR $(CC)",
    toolchains = [":r"],
)
"#,
        ],
    );

    let g = t.get_configured_target("//a:g").unwrap();
    let action = t
        .get_generating_action_for_output(&g, "a/go")
        .expect("action");
    blaze_util::unported!(
        "TemplateVariableInfoTest.proxyTemplateVariableInfo",
        "SpawnAction.getArguments of {action:?}"
    );
}

// Java: TemplateVariableInfoTest.templateVariableInfo
#[test]
#[ignore = "needs the mock C++ toolchain (cc_toolchain_alias from the mock rules_cc)"]
fn template_variable_info() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/rule.bzl",
        &[r#"Info = provider()
def _impl(ctx):
    return Info(
        variables = ctx.attr._cc_toolchain[platform_common.TemplateVariableInfo].variables,
    )

crule = rule(_impl, attrs = {"_cc_toolchain": attr.label(default = Label("//a:a"))})
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[
            r#"load("@rules_cc//cc/toolchains:cc_toolchain_alias.bzl", "cc_toolchain_alias")
load(":rule.bzl", "crule")

cc_toolchain_alias(name = "a")

crule(name = "r")
"#,
        ],
    );
    let ct = t.get_configured_target("//a:r").unwrap();

    let info = BuildViewTestCase::get_starlark_provider(&ct, "//a:rule.bzl", "Info").expect("Info");
    let info = StarlarkInfo::from_value(frozen(&info)).expect("ClassCastException: StarlarkInfo");
    let make_variables = info.get_field("variables").expect("variables");
    let make_variables = DictRef::from_value(make_variables).expect("ClassCastException: Map");
    assert!(make_variables.keys().any(|k| k.unpack_str() == Some("CC")));
}

// Java: TemplateVariableInfoTest.templateVariableInfoConstructor
#[test]
fn template_variable_info_constructor() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "a/rule.bzl",
        &[r#"Info = provider()
def _consumer_impl(ctx):
    return Info(
        var = ctx.attr.supplier[platform_common.TemplateVariableInfo]
            .variables[ctx.attr.var],
    )

def _supplier_impl(ctx):
    return [platform_common.TemplateVariableInfo({ctx.attr.var: ctx.attr.value})]

consumer = rule(
    _consumer_impl,
    attrs = {"var": attr.string(), "supplier": attr.label()},
)
supplier = rule(
    _supplier_impl,
    attrs = {"var": attr.string(), "value": attr.string()},
)
"#],
    );

    t.scratch.file(
        "a/BUILD",
        &[r#"load(":rule.bzl", "consumer", "supplier")

consumer(
    name = "consumer",
    supplier = ":supplier",
    var = "cherry",
)

supplier(
    name = "supplier",
    value = "ontop",
    var = "cherry",
)
"#],
    );

    let consumer = t.get_configured_target("//a:consumer").unwrap();
    let info =
        BuildViewTestCase::get_starlark_provider(&consumer, "//a:rule.bzl", "Info").expect("Info");
    let info = StarlarkInfo::from_value(frozen(&info)).expect("ClassCastException: StarlarkInfo");
    let value = info
        .get_field("var")
        .and_then(|v| v.unpack_str())
        .expect("ClassCastException: String");
    assert_eq!(value, "ontop");

    let supplier = t.get_configured_target("//a:supplier").unwrap();
    let provider = supplier
        .get(&TEMPLATE_VARIABLE_INFO.key())
        .expect("TemplateVariableInfo");
    let variables = downcast_info::<TemplateVariableInfo>(&provider)
        .expect("ClassCastException: TemplateVariableInfo")
        .get_variables();
    let actual: Vec<(&str, &str)> = variables
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(actual, vec![("cherry", "ontop")]);
}
