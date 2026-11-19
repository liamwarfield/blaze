//! Port of the .bzl-only parts of `src/test/java/.../starlark/StarlarkRuleClassFunctionsTest`.
//! Expected values come from the Java test. Tests needing BUILD loading, analysis or
//! configured targets are generated as `#[ignore]` stubs in `tests_ignored.rs`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_starlark::EvalError;
use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use starlark::environment::FrozenModule;
use starlark::values::ValueLike;

use super::attr_module::as_descriptor;
use super::rule_class_functions::rule_function_state;
use super::value_cell::resolve;
use crate::attribute::Attribute;
use crate::bzl_context::BazelModuleContext;
use crate::bzl_context::BzlInitThreadContext;
use crate::rule_class::RuleClass;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::Type;

fn registry() -> NativeRegistry {
    let mut r = NativeRegistry::with_blaze_starlark_builtins();
    r.register(&[FileKind::Bzl], super::register_bzl_toplevels);
    r.register(
        &[FileKind::Bzl],
        super::register_rule_class_provider_bzl_toplevels,
    );
    r
}

/// Executes `src` as the top level of `//pkg:<file_name>` in a .bzl initialization thread.
fn exec_bzl_with(
    file_name: &str,
    src: &str,
    semantics: StarlarkSemantics,
) -> Result<FrozenModule, EvalError> {
    let file_name = file_name.to_owned();
    let src = src.to_owned();
    eval::run_on_evaluation_thread(move || {
        let label = Label::parse_canonical(&format!("//pkg:{file_name}")).unwrap();
        let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();
        let module_context = Arc::new(BazelModuleContext {
            label: label.clone(),
            is_builtins: false,
            repo_mapping: mapping.clone(),
            filename: file_name.clone(),
            loads: Vec::new(),
            bzl_transitive_digest: vec![1, 2, 3],
        });
        let ctx = ThreadContext::new(
            Phase::BzlLoad,
            Arc::new(semantics.clone()),
            Arc::new(CollectingEvents::default()),
        )
        .with_file_label(label.clone())
        .with_main_repo_mapping(mapping.clone())
        .with_file_package_context(
            file_name.clone(),
            blaze_cmdline::LabelPackageContext::of(label.get_package_identifier(), mapping.clone()),
        )
        .with_payload(BzlInitThreadContext {
            bzl_file: label,
            transitive_digest: vec![1, 2, 3],
            bzl_visibility: None,
            tools_repository: blaze_cmdline::repository_name::BAZEL_TOOLS.clone(),
            fragment_names: vec!["cpp".to_owned(), "java".to_owned(), "apple".to_owned()],
            main_repo_mapping: mapping,
        });
        let globals = registry().build(FileKind::Bzl, &semantics);
        eval::compile(FileKind::Bzl, &file_name, src).and_then(|c| {
            eval::exec_module_with_extra_value(
                c,
                &globals,
                &LoadMap::new(),
                &ctx,
                &Default::default(),
                // Java: the module's client data (BazelModuleContext).
                |module| {
                    module
                        .heap()
                        .alloc(crate::bzl_context::BazelModuleContextValue(
                            module_context.clone(),
                        ))
                },
            )
        })
    })
}

fn exec_bzl(file_name: &str, src: &str) -> FrozenModule {
    match exec_bzl_with(file_name, src, StarlarkSemantics::default()) {
        Ok(m) => m,
        Err(e) => panic!("{e}"),
    }
}

fn eval_error(file_name: &str, src: &str) -> String {
    match exec_bzl_with(file_name, src, StarlarkSemantics::default()) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.message,
    }
}

fn assert_error_contains(file_name: &str, expected: &str, src: &str) {
    let msg = eval_error(file_name, src);
    assert!(
        msg.contains(expected),
        "{msg:?} does not contain {expected:?}"
    );
}

/// Java `buildAttribute(name, lines...)`.
fn build_attribute(file_name: &str, name: &str, expr: &str) -> Attribute {
    let m = exec_bzl(file_name, &format!("{name} = {expr}\n"));
    let v = m.get(name).unwrap();
    as_descriptor(v.value().to_value()).unwrap().build(name)
}

/// Java `getRuleClass(name)`.
fn get_rule_class(m: &FrozenModule, name: &str) -> Arc<RuleClass> {
    let v = m.get(name).unwrap();
    rule_function_state(v.value().to_value())
        .unwrap()
        .rule_class()
        .unwrap()
        .clone()
}

#[test]
fn test_attr_with_only_type() {
    let attr = build_attribute("t1.bzl", "a1", "attr.string_list()");
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::StringList);
}

#[test]
fn test_output_list_attr() {
    let attr = build_attribute("t2.bzl", "a1", "attr.output_list()");
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::OutputList);
}

#[test]
fn test_int_list_attr() {
    let attr = build_attribute("t3.bzl", "a1", "attr.int_list()");
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::IntegerList);
}

#[test]
fn test_output_attr() {
    let attr = build_attribute("t4.bzl", "a1", "attr.output()");
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::Output);
}

#[test]
fn test_string_dict_attr() {
    let attr = build_attribute("t5.bzl", "a1", "attr.string_dict(default = {'a': 'b'})");
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::StringDict);
}

#[test]
fn test_string_list_dict_attr() {
    let attr = build_attribute(
        "t6.bzl",
        "a1",
        "attr.string_list_dict(default = {'a': ['b', 'c']})",
    );
    assert!(attr.starlark_defined());
    assert_eq!(attr.get_type(), Type::StringListDict);
}

#[test]
fn test_attr_allowed_file_types_any_file() {
    let attr = build_attribute("t7.bzl", "a1", "attr.label_list(allow_files = True)");
    assert!(attr.starlark_defined());
    assert_eq!(
        attr.get_allowed_file_types_predicate(),
        Some(&FileTypeSet::AnyFile)
    );
}

#[test]
fn test_attr_allowed_file_types_wrong_type() {
    assert_error_contains(
        "t8.bzl",
        "got value of type 'int', want 'bool, sequence, or NoneType'",
        "attr.label_list(allow_files = 18)\n",
    );
}

#[test]
fn test_attr_name_special_characters_are_forbidden() {
    assert_error_contains(
        "t9.bzl",
        "attribute name `ab$c` is not a valid identifier",
        "def impl(ctx): return\nr = rule(impl, attrs = {'ab$c': attr.int()})\n",
    );
}

#[test]
fn test_attr_name_cannot_start_with_digit() {
    assert_error_contains(
        "t10.bzl",
        "attribute name `2_foo` is not a valid identifier",
        "def impl(ctx): return\nr = rule(impl, attrs = {'2_foo': attr.int()})\n",
    );
}

#[test]
fn test_attr_allowed_single_file_types_wrong_type() {
    assert_error_contains(
        "t11.bzl",
        "allow_single_file should be a boolean or a string list",
        "attr.label(allow_single_file = 18)\n",
    );
}

#[test]
fn test_attr_default_value_bad_type() {
    assert_error_contains(
        "t12.bzl",
        "got value of type 'int', want 'string'",
        "attr.string(default = 1)\n",
    );
}

#[test]
fn test_attr_cfg_host_disabled() {
    let semantics = StarlarkSemantics {
        incompatible_disable_starlark_host_transitions: true,
        ..StarlarkSemantics::default()
    };
    let msg = match exec_bzl_with("t13.bzl", "attr.label(cfg = 'host')\n", semantics) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.message,
    };
    assert!(msg.contains("Please use 'cfg = \"exec\"' instead"), "{msg}");
}

#[test]
fn test_attr_mandatory() {
    let attr = build_attribute("t14.bzl", "a1", "attr.string(mandatory=True)");
    assert!(attr.is_mandatory());
    assert!(!attr.is_non_empty());
}

#[test]
fn test_attr_allow_empty() {
    let attr = build_attribute("t15.bzl", "a1", "attr.string_list(allow_empty=False)");
    assert!(attr.is_non_empty());
    assert!(!attr.is_mandatory());
}

#[test]
fn test_attr_values() {
    let attr = build_attribute("t16.bzl", "a1", "attr.string(values = ['ab', 'cd'])");
    let predicate = attr.get_allowed_values().unwrap();
    assert!(predicate.apply(&AttrValue::String("ab".to_owned())));
    assert!(!predicate.apply(&AttrValue::String("xy".to_owned())));
}

#[test]
fn test_attr_int_values() {
    let attr = build_attribute("t17.bzl", "a1", "attr.int(values = [1, 2])");
    let predicate = attr.get_allowed_values().unwrap();
    assert!(predicate.apply(&AttrValue::Int(2)));
    assert!(!predicate.apply(&AttrValue::Int(3)));
}

#[test]
fn test_rule_bad_type_for_doc() {
    assert_error_contains(
        "t18.bzl",
        "got value of type 'int', want 'string or NoneType'",
        "def impl():\n  pass\nrule(impl, doc = 1)\n",
    );
}

#[test]
fn test_rule_add_toolchain() {
    let m = exec_bzl(
        "t19.bzl",
        r#"
def impl(ctx): return None
r1 = rule(impl,
    toolchains=[
        '//test:my_toolchain_type1',
        config_common.toolchain_type('//test:my_toolchain_type2'),
        config_common.toolchain_type('//test:my_toolchain_type3', mandatory=False),
        config_common.toolchain_type('//test:my_toolchain_type4', mandatory=True),
    ],
)
"#,
    );
    let c = get_rule_class(&m, "r1");
    let mandatory = |name: &str| {
        let l = Label::parse_canonical(name).unwrap();
        c.get_toolchain_types()
            .iter()
            .find(|t| t.toolchain_type == l)
            .unwrap_or_else(|| panic!("no toolchain type {name}"))
            .mandatory
    };
    assert!(mandatory("//test:my_toolchain_type1"));
    assert!(mandatory("//test:my_toolchain_type2"));
    assert!(!mandatory("//test:my_toolchain_type3"));
    assert!(mandatory("//test:my_toolchain_type4"));
}

#[test]
fn test_create_exec_group() {
    let m = exec_bzl(
        "t20.bzl",
        r#"
group = exec_group(
  toolchains=[
    '//test:my_toolchain_type1',
    config_common.toolchain_type('//test:my_toolchain_type2'),
    config_common.toolchain_type('//test:my_toolchain_type3', mandatory=False),
    config_common.toolchain_type('//test:my_toolchain_type4', mandatory=True),
  ],
  exec_compatible_with=['//constraint:cv1', '//constraint:cv2'],
)
"#,
    );
    let v = m.get("group").unwrap();
    let group = v
        .value()
        .downcast_ref::<super::rule_class_functions::ExecGroupValue>()
        .unwrap()
        .0
        .clone();
    let t = |name: &str| {
        group
            .toolchain_type(&Label::parse_canonical(name).unwrap())
            .unwrap()
            .mandatory
    };
    assert!(t("//test:my_toolchain_type1"));
    assert!(t("//test:my_toolchain_type2"));
    assert!(!t("//test:my_toolchain_type3"));
    assert!(t("//test:my_toolchain_type4"));
    assert!(
        group
            .exec_compatible_with
            .contains(&Label::parse_canonical("//constraint:cv1").unwrap())
    );
    assert!(
        group
            .exec_compatible_with
            .contains(&Label::parse_canonical("//constraint:cv2").unwrap())
    );
}

#[test]
fn test_rule_add_execution_constraints() {
    let m = exec_bzl(
        "t21.bzl",
        r#"
def impl():
  pass
r1 = rule(
  implementation = impl,
  exec_compatible_with=['//constraint:cv1', '//constraint:cv2'],
)
"#,
    );
    let c = get_rule_class(&m, "r1");
    let got: Vec<Label> = c
        .get_execution_platform_constraints()
        .iter()
        .cloned()
        .collect();
    assert_eq!(
        got,
        vec![
            Label::parse_canonical("//constraint:cv1").unwrap(),
            Label::parse_canonical("//constraint:cv2").unwrap(),
        ]
    );
}

#[test]
fn test_export_aliased_name() {
    // When there are multiple names aliasing the same StarlarkExportable, the first one to be
    // declared should be used.
    let m = exec_bzl(
        "t22.bzl",
        r#"
def _impl(ctx): pass
d = rule(implementation = _impl)
a = d
b = d
c = d
e = d
f = d
foo = d
bar = d
baz = d
x = d
y = d
z = d
"#,
    );
    assert_eq!(get_rule_class(&m, "d").get_name(), "d");
    assert_eq!(get_rule_class(&m, "foo").get_name(), "d");
}

#[test]
fn star_the_only_aspect_arg() {
    assert_error_contains(
        "t23.bzl",
        "'*' must be the only string in 'attr_aspects' list",
        "def _impl(target, ctx):\n   pass\naspect(_impl, attr_aspects=['*', 'foo'])\n",
    );
}

#[test]
fn invalid_attr_aspects_type() {
    assert_error_contains(
        "t24.bzl",
        "'attr_aspects' got value of type 'string', want 'sequence or function'",
        "def _impl(target, ctx):\n   pass\naspect(_impl, attr_aspects='foo')\n",
    );
}

#[test]
fn aspect_attrs() {
    let m = exec_bzl(
        "t25.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl, attr_aspects=['srcs', \
         'data'])\n",
    );
    let v = m.get("my_aspect").unwrap();
    let data = super::aspect::aspect_data(v.value().to_value()).unwrap();
    match &data.attribute_aspects {
        crate::aspect::AspectPropagationEdgesSupplier::FixedList(l) => {
            let l = l.get_list();
            assert_eq!(
                l.iter().cloned().collect::<Vec<_>>(),
                vec!["srcs".to_owned(), "data".to_owned()]
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn aspect_provides_error() {
    assert_error_contains(
        "t26.bzl",
        "at index 1 of provides, got element of type int, want Provider",
        "def _impl(target, ctx):\n   pass\ny = provider()\nmy_aspect = aspect(_impl, provides \
         = [y, 1])\n",
    );
}

/// Blaze-specific: the implementation handle of an exported rule class resolves to the frozen
/// function once the module is frozen (see `value_cell`).
#[test]
fn rule_implementation_handle_resolves_after_freeze() {
    let m = exec_bzl(
        "t27.bzl",
        "def impl(ctx): return None\nrule1 = rule(impl)\n",
    );
    let c = get_rule_class(&m, "rule1");
    let f = resolve(c.get_configured_target_function().unwrap()).unwrap();
    assert_eq!(f.to_value().get_type(), "function");
    assert_eq!(
        c.get_rule_definition_environment_digest(),
        Some(&[1u8, 2, 3][..])
    );
    assert_eq!(
        c.get_rule_definition_environment_label(),
        Some(&Label::parse_canonical("//pkg:t27.bzl").unwrap())
    );
}

#[test]
fn test_rule_class_name_must_match_test_suffix() {
    // Java: StarlarkRuleFunction.export reports "Invalid rule class name ..."
    assert_error_contains(
        "t28.bzl",
        "Invalid rule class name 'foo', test rule class names must end with '_test' and other \
         rule classes must not",
        "def impl(ctx): return None\nfoo = rule(impl, test = True)\n",
    );
}

// ---- docs/reviews/2026-11-18-loading.md S2-4: typed `default` parameters of `attr.*`

// Java: StarlarkRuleClassFunctionsTest.testFunctionAsAttrDefault
#[test]
fn test_function_as_attr_default() {
    // Late-bound attributes, which are computed during analysis as a function of the
    // configuration, are only available for attributes involving labels:
    //   attr.label
    //   attr.label_list
    //   attr.label_keyed_string_dict
    //   attr.output,
    //   attr.output_list
    exec_bzl(
        "s2_4_a.bzl",
        "def f(): pass\n\
         a = attr.label(default=f)\n\
         b = attr.label_list(default=f)\n\
         c = attr.label_keyed_string_dict(default=f)\n",
    );
    // For all other attribute types, the default value may not be a function.
    //
    // (This is a regression test for github.com/bazelbuild/bazel/issues/9463. The
    // loading-phase feature of "computed attribute defaults" is not exposed to Starlark; the bug
    // was that the @StarlarkMethod annotation was more permissive than the method declaration.)
    for (expected, expr) in [
        (
            "got value of type 'function', want 'string'",
            "attr.string(default=f)",
        ),
        (
            "got value of type 'function', want 'sequence'",
            "attr.string_list(default=f)",
        ),
        (
            "got value of type 'function', want 'int'",
            "attr.int(default=f)",
        ),
        (
            "got value of type 'function', want 'sequence'",
            "attr.int_list(default=f)",
        ),
        (
            "got value of type 'function', want 'bool'",
            "attr.bool(default=f)",
        ),
        (
            "got value of type 'function', want 'dict'",
            "attr.string_dict(default=f)",
        ),
        (
            "got value of type 'function', want 'dict'",
            "attr.string_list_dict(default=f)",
        ),
    ] {
        assert_error_contains(
            "s2_4_b.bzl",
            expected,
            &format!("def f(): pass\nx = {expr}\n"),
        );
    }
}

// Java: net/starlark/java/eval/BuiltinFunction.java:516 checkParamValue ("in call to %s(),
// parameter '%s' got value of type '%s', want '%s'"), with the allowed classes of
// StarlarkAttrModuleApi's `default` parameters joined by ParamDescriptor.java:123
// getTypeErrorMessage ("a", "a or b", "a, b, or c"; NativeComputedDefault is left out).
#[test]
fn attr_default_type_errors_name_the_function_and_the_allowed_types() {
    for (expected, expr) in [
        (
            // StarlarkAttrModuleApi.java: `StarlarkInt defaultValue`.
            "in call to int(), parameter 'default' got value of type 'function', want 'int'",
            "attr.int(default = lambda: 3)",
        ),
        (
            "in call to int(), parameter 'default' got value of type 'string', want 'int'",
            "attr.int(default = '3')",
        ),
        (
            // allowedTypes = {Sequence<String>, NativeComputedDefaultApi}.
            "in call to string_list(), parameter 'default' got value of type 'dict', want \
             'sequence'",
            "attr.string_list(default = {})",
        ),
        (
            // `Dict<?, ?> defaultValue`.
            "in call to label_list_dict(), parameter 'default' got value of type 'function', \
             want 'dict'",
            "attr.label_list_dict(default = lambda: {})",
        ),
        (
            // allowedTypes = {Sequence<Label>, StarlarkFunction}.
            "in call to label_list(), parameter 'default' got value of type 'int', want \
             'sequence or function'",
            "attr.label_list(default = 1)",
        ),
        (
            // allowedTypes = {Dict, StarlarkFunction}.
            "in call to label_keyed_string_dict(), parameter 'default' got value of type \
             'list', want 'dict or function'",
            "attr.label_keyed_string_dict(default = [])",
        ),
        (
            // allowedTypes = {Label, String, LateBoundDefaultApi, NativeComputedDefaultApi,
            // StarlarkFunction, NoneType}.
            "in call to label(), parameter 'default' got value of type 'int', want 'Label, \
             string, LateBoundDefault, function, or NoneType'",
            "attr.label(default = 1)",
        ),
    ] {
        assert_error_contains("s2_4_c.bzl", expected, &format!("x = {expr}\n"));
    }
}

// ---- docs/reviews/2026-11-18-loading.md S2-1: BuiltinRestriction allowlist entries

/// `failIfLabelOutsideAllowlist(label, repoMapping, allowlist)`: `Ok(())` or the error message.
fn allowlist_check(
    label: &str,
    repo_mapping: &blaze_cmdline::RepositoryMapping,
    allowlist: &[(&str, &str)],
) -> Result<(), String> {
    super::rule_class_functions::fail_if_label_outside_allowlist(
        &Label::parse_canonical(label).unwrap(),
        repo_mapping,
        allowlist,
    )
    .map_err(|e| e.to_string())
}

// Java: BuiltinRestriction.java:129-150 AllowlistEntry.reposMatch. For a label of the main
// repository: `allowedName.equals("") || repoMapping.get(allowedName).isMain()`.
#[test]
fn allowlist_entry_matches_the_main_repo_when_it_is_the_ruleset() {
    let main = blaze_cmdline::repository_name::MAIN.clone();
    // The root module is rules_cc: its own name maps to the main repository.
    let rules_cc_is_main = blaze_cmdline::RepositoryMapping::create(
        [("", main.clone()), ("rules_cc", main.clone())],
        main.clone(),
    );
    let other_root = blaze_cmdline::repository_mapping::EMPTY.clone();
    let allowlist = [("rules_cc", "")];

    assert_eq!(
        allowlist_check(
            "//cc/private/rules_impl:x.bzl",
            &rules_cc_is_main,
            &allowlist
        ),
        Ok(())
    );
    let err = allowlist_check("//cc/private/rules_impl:x.bzl", &other_root, &allowlist)
        .expect_err("the main repository is not rules_cc");
    // Java: BuiltinRestriction.java:203 "file '%s' cannot use private API".
    assert!(
        err.contains("file '//cc/private/rules_impl:x.bzl' cannot use private API"),
        "{err}"
    );
    // `allowedName.equals(RepositoryName.MAIN.getName())`.
    assert_eq!(
        allowlist_check(
            "//initializer_testing/builtins:b.bzl",
            &other_root,
            &[("", "initializer_testing/builtins")]
        ),
        Ok(())
    );
}

// Java: BuiltinRestriction.java:144-149: `bazel_tools` only by its own name; any other
// repository by `givenName.getName().startsWith(allowedName + "+")`.
#[test]
fn allowlist_entry_matches_canonical_repo_names_by_module_prefix() {
    let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();

    assert_eq!(
        allowlist_check("@@rules_cc+//cc:x.bzl", &mapping, &[("rules_cc", "")]),
        Ok(())
    );
    assert_eq!(
        allowlist_check("@@rules_cc+1.2.3//cc:x.bzl", &mapping, &[("rules_cc", "")]),
        Ok(())
    );
    // A canonical name that is exactly the module name has no "+" after it.
    assert!(allowlist_check("@@rules_cc//cc:x.bzl", &mapping, &[("rules_cc", "")]).is_err());
    assert!(allowlist_check("@@rules_cc_extra+//cc:x.bzl", &mapping, &[("rules_cc", "")]).is_err());

    assert_eq!(
        allowlist_check(
            "@@bazel_tools//tools/build_defs/build_info:x.bzl",
            &mapping,
            &[("bazel_tools", "tools/build_defs/build_info")]
        ),
        Ok(())
    );
    // "bazel_tools+..." is not @bazel_tools, and @bazel_tools matches no other entry name.
    assert!(
        allowlist_check(
            "@@bazel_tools//tools/build_defs/build_info:x.bzl",
            &mapping,
            &[("bazel", "tools/build_defs/build_info")]
        )
        .is_err()
    );

    // Java: BuiltinRestriction.java:212 isNotAllowed: @_builtins is always allowed.
    assert_eq!(
        allowlist_check("@@_builtins//:common/cc/x.bzl", &mapping, &[]),
        Ok(())
    );
}

// Java: BuiltinRestriction.java:126 `label.getPackageFragment().startsWith(packagePrefix())`:
// PathFragment.startsWith compares whole segments.
#[test]
fn allowlist_entry_compares_packages_by_segment() {
    let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();
    let allowlist = [("rules_java", "java")];

    assert_eq!(
        allowlist_check("@@rules_java+//java:x.bzl", &mapping, &allowlist),
        Ok(())
    );
    assert_eq!(
        allowlist_check(
            "@@rules_java+//java/common/rules:x.bzl",
            &mapping,
            &allowlist
        ),
        Ok(())
    );
    let err = allowlist_check("@@rules_java+//javatests/foo:x.bzl", &mapping, &allowlist)
        .expect_err("javatests is not under java");
    assert!(
        err.contains("file '@@rules_java+//javatests/foo:x.bzl' cannot use private API"),
        "{err}"
    );
}

/// A def of a frozen .bzl module prints its module's client data: `BazelModuleContext.toString`
/// is the label's canonical form (Java StarlarkFunction.java:222 `repr`, BazelModuleContext.java:133
/// `toString`, Label.java:479 `toString`, RepositoryName.java:284: "" for the main repository).
#[test]
fn function_repr_names_its_bzl_label() {
    let m = exec_bzl("defs.bzl", "def f():\n    pass\n");
    let f = m.get("f").unwrap();
    assert_eq!(f.value().to_string(), "<function f from //pkg:defs.bzl>");
}

// ---------------------------------------------------------------------------------------------
// StarlarkDefinedAspect model (packages/StarlarkDefinedAspect.java,
// AspectPropagationEdgesSupplier.java, AspectPropagationPredicate.java). The Java tests of these
// (StarlarkDefinedAspectsTest, StarlarkAspectAttrAspectsFunctionTest,
// StarlarkAspectsPropagationPredicateTest) run whole analyses; the tests below check the
// loading-level part of those with the Java tests' expected messages. The .bzl label is
// `//pkg:<file>` here where the Java tests use `//test:defs.bzl`.
// ---------------------------------------------------------------------------------------------

fn frozen_aspect(
    m: &FrozenModule,
    name: &str,
) -> &'static super::aspect::FrozenStarlarkDefinedAspect {
    let handle = super::aspect::aspect_handle_of(&m.get(name).unwrap()).unwrap();
    super::aspect::as_frozen_aspect(handle.value()).unwrap()
}

fn top_level_params(pairs: &[(&str, &str)]) -> indexmap::IndexMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A context value for propagation functions (Java `StarlarkAspectPropagationContext`; the
/// functions below only print or ignore it).
fn none_context<'v>(_m: &starlark::environment::Module<'v>) -> starlark::values::Value<'v> {
    starlark::values::Value::new_none()
}

#[test]
fn aspect_definition_cached_per_parameters() {
    let m = exec_bzl(
        "ta1.bzl",
        "def _impl(target, ctx):\n  pass\n\
         p = provider()\n\
         base = aspect(_impl)\n\
         my_aspect = aspect(_impl, attr_aspects = ['deps'], attrs = {'p': \
         attr.string(values = ['a', 'b']), '_tool': attr.label(default = '//x:y')}, \
         required_providers = [p], provides = [p], fragments = ['cpp'], requires = [base])\n",
    );
    let a = frozen_aspect(&m, "my_aspect");
    assert_eq!(a.get_name(), "//pkg:ta1.bzl%my_aspect");
    assert_eq!(a.get_aspect_class().get_name(), "//pkg:ta1.bzl%my_aspect");

    let mut pa = crate::aspect::AspectParameters::builder();
    pa.add_attribute("p", "a");
    let pa = pa.build();
    let mut pb = crate::aspect::AspectParameters::builder();
    pb.add_attribute("p", "b");
    let pb = pb.build();
    let d1 = a.get_definition(&pa);
    let d2 = a.get_definition(&pa.clone());
    let d3 = a.get_definition(&pb);
    assert!(Arc::ptr_eq(&d1, &d2));
    assert!(!Arc::ptr_eq(&d1, &d3));
    assert_eq!(d1.get_name(), "//pkg:ta1.bzl%my_aspect");
    assert_eq!(
        d1.get_attributes().keys().cloned().collect::<Vec<_>>(),
        vec!["p".to_owned(), "$tool".to_owned()]
    );
    assert_eq!(
        d3.get_attributes()["p"].get_default_value(None),
        Some(AttrValue::String("b".to_owned()))
    );
    assert_eq!(
        d1.get_attribute_aspects().as_fixed_list().unwrap(),
        &indexmap::IndexSet::from(["deps".to_owned()])
    );
    assert!(!d1.get_required_providers().accepts_any());
    assert_eq!(
        d1.get_advertised_providers().get_starlark_providers().len(),
        1
    );
    assert_eq!(
        d1.get_required_aspect_classes()
            .iter()
            .map(|c| c.get_name().to_owned())
            .collect::<Vec<_>>(),
        vec!["//pkg:ta1.bzl%base".to_owned()]
    );
    assert!(
        d1.get_configuration_fragment_policy()
            .is_legal_configuration_fragment("CppConfiguration", Some("cpp"))
    );
}

/// `addAttrValue`: a parameter without `values` gets an allowed-values set of its own value.
#[test]
fn aspect_definition_add_attr_value() {
    let m = exec_bzl(
        "ta2.bzl",
        "def _impl(target, ctx):\n  pass\n\
         my_aspect = aspect(_impl, attrs = {'n': attr.int(values = [1, 2]), 'b': attr.bool()})\n",
    );
    let a = frozen_aspect(&m, "my_aspect");
    let mut p = crate::aspect::AspectParameters::builder();
    p.add_attribute("n", "2").add_attribute("b", "true");
    let d = a.get_definition(&p.build());
    assert_eq!(
        d.get_attributes()["n"].get_default_value(None),
        Some(AttrValue::Int(2))
    );
    let b = &d.get_attributes()["b"];
    assert_eq!(b.get_default_value(None), Some(AttrValue::Bool(true)));
    assert!(b.check_allowed_values());
}

#[test]
fn top_level_aspects_with_parameters_invalid_parameter_value() {
    let m = exec_bzl(
        "ta3.bzl",
        "def _aspect_a_impl(target, ctx):\n  pass\n\
         aspect_a = aspect(implementation = _aspect_a_impl, attr_aspects = ['dep'], attrs = { 'p' \
         : attr.string(values = ['p_v1', 'p_v2']) })\n",
    );
    let a = frozen_aspect(&m, "aspect_a");
    let e = a
        .extract_top_level_parameters(&top_level_params(&[("p", "p_v")]))
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        "//pkg:ta3.bzl%aspect_a: invalid value in 'p' attribute: has to be one of 'p_v1' or \
         'p_v2' instead of 'p_v'"
    );
    let ok = a
        .extract_top_level_parameters(&top_level_params(&[("p", "p_v2")]))
        .unwrap();
    assert_eq!(ok.get_only_value_of_attribute("p"), "p_v2");
}

#[test]
fn top_level_aspects_with_boolean_parameter_invalid_value() {
    let m = exec_bzl(
        "ta4.bzl",
        "def _aspect_a_impl(target, ctx):\n  pass\n\
         aspect_a = aspect(implementation = _aspect_a_impl, attr_aspects = ['dep'], attrs = { 'p' \
         : attr.bool() })\n",
    );
    let a = frozen_aspect(&m, "aspect_a");
    let e = a
        .extract_top_level_parameters(&top_level_params(&[("p", "x")]))
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        "//pkg:ta4.bzl%aspect_a: expected value of type 'bool' for attribute 'p' but got 'x'"
    );
    let ok = a
        .extract_top_level_parameters(&top_level_params(&[("p", "Yes")]))
        .unwrap();
    assert_eq!(ok.get_only_value_of_attribute("p"), "true");
}

#[test]
fn top_level_aspects_with_parameters_invalid_integer_parameter_value() {
    let m = exec_bzl(
        "ta5.bzl",
        "def _aspect_a_impl(target, ctx):\n  pass\n\
         aspect_a = aspect(implementation = _aspect_a_impl, attr_aspects = ['dep'], attrs = { 'p' \
         : attr.int(values = [1, 2]) })\n",
    );
    let a = frozen_aspect(&m, "aspect_a");
    let e = a
        .extract_top_level_parameters(&top_level_params(&[("p", "3")]))
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        "//pkg:ta5.bzl%aspect_a: invalid value in 'p' attribute: has to be one of '1' or '2' \
         instead of '3'"
    );
    let e = a
        .extract_top_level_parameters(&top_level_params(&[("p", "x")]))
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        "//pkg:ta5.bzl%aspect_a: expected value of type 'int' for attribute 'p' but got 'x'"
    );
}

#[test]
fn top_level_aspects_with_integer_parameter_use_default_value() {
    let m = exec_bzl(
        "ta6.bzl",
        "def _aspect_a_impl(target, ctx):\n  pass\n\
         aspect_a = aspect(implementation = _aspect_a_impl, attrs = { 'p' : attr.int(default = 1, \
         values = [1, 2]), '_x': attr.int(default = 5) })\n",
    );
    let a = frozen_aspect(&m, "aspect_a");
    let params = a
        .extract_top_level_parameters(&top_level_params(&[]))
        .unwrap();
    assert_eq!(params.to_string(), "{p=[1]}");
}

#[test]
fn wild_card_attr_returned_build_fails() {
    let m = exec_bzl(
        "ta7.bzl",
        "def _impl(target, ctx):\n  pass\n\
         def _propagation_attrs(ctx):\n    return ['*']\n\
         cmdline_aspect = aspect(_impl, attr_aspects = _propagation_attrs)\n",
    );
    let a = frozen_aspect(&m, "cmdline_aspect");
    let crate::aspect::AspectPropagationEdgesSupplier::Function(f) = &a.data().attribute_aspects
    else {
        panic!("expected a function supplier")
    };
    let events = blaze_events::EventCollector::new();
    let e = f.compute_list(&none_context, &events).unwrap_err();
    assert_eq!(e.message, "'*' is not allowed in 'attr_aspects' list");
}

#[test]
fn attr_aspects_function_compute_list() {
    let m = exec_bzl(
        "ta8.bzl",
        "def _impl(target, ctx):\n  pass\n\
         def _attrs(ctx):\n    print('ctx is', ctx)\n    return ['deps', '_tool']\n\
         def _bad(ctx):\n    return ('deps',)\n\
         a = aspect(_impl, attr_aspects = _attrs)\n\
         b = aspect(_impl, attr_aspects = _bad)\n",
    );
    let events = blaze_events::EventCollector::new();
    let crate::aspect::AspectPropagationEdgesSupplier::Function(f) =
        &frozen_aspect(&m, "a").data().attribute_aspects
    else {
        panic!("expected a function supplier")
    };
    assert_eq!(
        f.compute_list(&none_context, &events)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["deps".to_owned(), "$tool".to_owned(), ":tool".to_owned()]
    );
    let printed: Vec<String> = events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect();
    assert_eq!(printed, vec!["ctx is None".to_owned()]);
    let crate::aspect::AspectPropagationEdgesSupplier::Function(f) =
        &frozen_aspect(&m, "b").data().attribute_aspects
    else {
        panic!("expected a function supplier")
    };
    assert_eq!(
        f.compute_list(&none_context, &events).unwrap_err().message,
        "Expected a list"
    );
}

#[test]
fn toolchains_aspects_function_compute_list() {
    let m = exec_bzl(
        "ta9.bzl",
        "def _impl(target, ctx):\n  pass\n\
         def _tc(ctx):\n    return [Label('//t:t')]\n\
         def _bad(ctx):\n    return ['//t:t']\n\
         a = aspect(_impl, toolchains_aspects = _tc)\n\
         b = aspect(_impl, toolchains_aspects = _bad)\n\
         c = aspect(_impl, toolchains_aspects = ['*'])\n",
    );
    let events = blaze_events::EventCollector::new();
    let crate::aspect::AspectPropagationEdgesSupplier::Function(f) =
        &frozen_aspect(&m, "a").data().toolchains_aspects
    else {
        panic!("expected a function supplier")
    };
    assert_eq!(
        f.compute_list(&none_context, &events)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![Label::parse_canonical_unchecked("//t:t")]
    );
    let crate::aspect::AspectPropagationEdgesSupplier::Function(f) =
        &frozen_aspect(&m, "b").data().toolchains_aspects
    else {
        panic!("expected a function supplier")
    };
    assert_eq!(
        f.compute_list(&none_context, &events).unwrap_err().message,
        "at index 0 of toolchains_aspects, got element of type string, want Label"
    );
    assert_eq!(
        frozen_aspect(&m, "c")
            .data()
            .toolchains_aspects
            .as_fixed_list()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![crate::aspect::ALL_TOOLCHAINS.clone()]
    );
}

#[test]
fn propagation_predicate_evaluate() {
    let m = exec_bzl(
        "ta10.bzl",
        "def _impl(target, ctx):\n  pass\n\
         def _yes(ctx):\n    return True\n\
         def _bad(ctx):\n    return 1\n\
         a = aspect(_impl, propagation_predicate = _yes)\n\
         b = aspect(_impl, propagation_predicate = _bad)\n",
    );
    let events = blaze_events::EventCollector::new();
    let a = frozen_aspect(&m, "a");
    let pred = a.data().get_propagation_predicate().unwrap();
    assert!(pred.evaluate(&none_context, &events).unwrap());
    // The definition carries the same predicate.
    let d = a.get_definition(&crate::aspect::AspectParameters::empty());
    assert_eq!(d.get_propagation_predicate(), Some(pred));
    let b = frozen_aspect(&m, "b");
    assert_eq!(
        b.data()
            .get_propagation_predicate()
            .unwrap()
            .evaluate(&none_context, &events)
            .unwrap_err()
            .message,
        "Expected a boolean"
    );
}

/// `AspectsList.getAspects(rule)` / top-level `buildAspects` through `attr.label(aspects=...)`.
#[test]
fn aspects_list_build_aspects() {
    let m = exec_bzl(
        "ta11.bzl",
        "def _impl(target, ctx):\n  pass\n\
         base = aspect(_impl)\n\
         a = aspect(_impl, attrs = {'p': attr.string(default = 'x', values = ['x', 'y'])}, \
         requires = [base])\n",
    );
    let mut b = crate::rule_class::AspectsListBuilder::new();
    b.add_starlark_aspect(super::aspect::aspect_handle_of(&m.get("a").unwrap()).unwrap())
        .unwrap();
    let list = b.build();
    assert_eq!(
        list.get_aspect_classes()
            .iter()
            .map(|c| c.get_name().to_owned())
            .collect::<Vec<_>>(),
        vec![
            "//pkg:ta11.bzl%base".to_owned(),
            "//pkg:ta11.bzl%a".to_owned()
        ]
    );
    let aspects = list
        .build_aspects(&top_level_params(&[("p", "y")]))
        .unwrap();
    assert_eq!(
        aspects.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
        vec![
            "Aspect //pkg:ta11.bzl%base".to_owned(),
            "Aspect //pkg:ta11.bzl%a[p=\"y\"]".to_owned()
        ]
    );
    assert!(aspects[1].get_definition().requires(&aspects[0]));
    assert_eq!(
        list.build_aspects(&top_level_params(&[("p", "y")]))
            .unwrap(),
        aspects
    );
    assert_eq!(
        list.validate_top_level_aspects_parameters(&top_level_params(&[("q", "1"), ("p", "x")]))
            .unwrap_err()
            .get_message(),
        "Parameters '[q]' are not parameters of any of the top-level aspects but they are \
         specified in --aspects_parameters."
    );
}

// Java: StarlarkRuleClassFunctionsTest `declared(name)`: the provider exported as `name` from the
// evaluated file.
fn declared(file_name: &str, name: &str) -> crate::rule_class::StarlarkProviderIdentifier {
    crate::rule_class::StarlarkProviderIdentifier::for_key(
        crate::rule_class::ProviderKey::Starlark {
            extension_label: Label::parse_canonical(&format!("//pkg:{file_name}")).unwrap(),
            exported_name: name.to_owned(),
        },
    )
}

fn advertised_set(
    providers: &[crate::rule_class::StarlarkProviderIdentifier],
) -> crate::rule_class::AdvertisedProviderSet {
    let mut b = crate::rule_class::AdvertisedProviderSet::builder();
    for p in providers {
        b.add_starlark(p.clone());
    }
    b.build()
}

fn fixed_attr_aspects(m: &FrozenModule, name: &str) -> Vec<String> {
    frozen_aspect(m, name)
        .data()
        .get_attribute_aspects()
        .as_fixed_list()
        .expect("FixedListSupplier")
        .iter()
        .cloned()
        .collect()
}

fn empty_definition(m: &FrozenModule, name: &str) -> Arc<crate::aspect::AspectDefinition> {
    frozen_aspect(m, name).get_definition(&crate::aspect::AspectParameters::empty())
}

// Java: StarlarkRuleClassFunctionsTest.aspectAllAttrs
#[test]
fn aspect_all_attrs() {
    let m = exec_bzl(
        "tb1.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl, attr_aspects=['*'])\n",
    );
    assert_eq!(fixed_attr_aspects(&m, "my_aspect"), vec!["*".to_owned()]);
}

// Java: StarlarkRuleClassFunctionsTest.aspectEmptyAttrs
#[test]
fn aspect_empty_attrs() {
    let m = exec_bzl(
        "tb2.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl, attr_aspects=[])\n",
    );
    assert!(fixed_attr_aspects(&m, "my_aspect").is_empty());
}

// Java: StarlarkRuleClassFunctionsTest.aspectDefaultAttrs
#[test]
fn aspect_default_attrs() {
    let m = exec_bzl(
        "tb3.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl)\n",
    );
    assert!(fixed_attr_aspects(&m, "my_aspect").is_empty());
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersSingle
#[test]
fn aspect_required_aspect_providers_single() {
    let m = exec_bzl(
        "tb4.bzl",
        "def _impl(target, ctx):\n   pass\njava = provider()\ncc = provider()\n\
         my_aspect = aspect(_impl, required_aspect_providers=[java, cc])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers_for_aspects();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
    assert!(required_providers.is_satisfied_by(&advertised_set(&[
        declared("tb4.bzl", "cc"),
        declared("tb4.bzl", "java")
    ])));
    assert!(!required_providers.is_satisfied_by(&advertised_set(&[declared("tb4.bzl", "cc")])));
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersAlternatives
#[test]
fn aspect_required_aspect_providers_alternatives() {
    let m = exec_bzl(
        "tb5.bzl",
        "def _impl(target, ctx):\n   pass\njava = provider()\ncc = provider()\n\
         my_aspect = aspect(_impl, required_aspect_providers=[[java], [cc]])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers_for_aspects();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
    assert!(required_providers.is_satisfied_by(&advertised_set(&[declared("tb5.bzl", "java")])));
    assert!(required_providers.is_satisfied_by(&advertised_set(&[declared("tb5.bzl", "cc")])));
    assert!(!required_providers.is_satisfied_by(&advertised_set(&[declared("tb5.bzl", "prolog")])));
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersEmpty
#[test]
fn aspect_required_aspect_providers_empty() {
    let m = exec_bzl(
        "tb6.bzl",
        "def _impl(target, ctx):\n   pass\n\
         my_aspect = aspect(_impl, required_aspect_providers=[])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers_for_aspects();
    assert!(!required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredAspectProvidersDefault
#[test]
fn aspect_required_aspect_providers_default() {
    let m = exec_bzl(
        "tb7.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl)\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers_for_aspects();
    assert!(!required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersSingle
#[test]
fn aspect_required_providers_single() {
    let m = exec_bzl(
        "tb8.bzl",
        "def _impl(target, ctx):\n   pass\njava = provider()\ncc = provider()\n\
         my_aspect = aspect(_impl, required_providers=[java, cc])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
    assert!(required_providers.is_satisfied_by(&advertised_set(&[
        declared("tb8.bzl", "cc"),
        declared("tb8.bzl", "java")
    ])));
    assert!(!required_providers.is_satisfied_by(&advertised_set(&[declared("tb8.bzl", "cc")])));
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersAlternatives
#[test]
fn aspect_required_providers_alternatives() {
    let m = exec_bzl(
        "tb9.bzl",
        "def _impl(target, ctx):\n   pass\njava = provider()\ncc = provider()\n\
         my_aspect = aspect(_impl, required_providers=[[java], [cc]])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(
        !required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty())
    );
    assert!(required_providers.is_satisfied_by(&advertised_set(&[declared("tb9.bzl", "java")])));
    assert!(required_providers.is_satisfied_by(&advertised_set(&[declared("tb9.bzl", "cc")])));
    assert!(!required_providers.is_satisfied_by(&advertised_set(&[declared("tb9.bzl", "prolog")])));
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersEmpty
#[test]
fn aspect_required_providers_empty() {
    let m = exec_bzl(
        "tb10.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl, required_providers=[])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty()));
}

// Java: StarlarkRuleClassFunctionsTest.aspectRequiredProvidersDefault
#[test]
fn aspect_required_providers_default() {
    let m = exec_bzl(
        "tb11.bzl",
        "def _impl(target, ctx):\n   pass\nmy_aspect = aspect(_impl)\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let required_providers = d.get_required_providers();
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::any()));
    assert!(required_providers.is_satisfied_by(&crate::rule_class::AdvertisedProviderSet::empty()));
}

// Java: StarlarkRuleClassFunctionsTest.aspectProvides
#[test]
fn aspect_provides() {
    let m = exec_bzl(
        "tb12.bzl",
        "def _impl(target, ctx):\n   pass\ny = provider()\n\
         my_aspect = aspect(_impl, provides = [y])\n",
    );
    let d = empty_definition(&m, "my_aspect");
    let advertised_providers = d.get_advertised_providers();
    assert!(!advertised_providers.can_have_any_provider());
    assert_eq!(
        advertised_providers
            .get_starlark_providers()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![declared("tb12.bzl", "y")]
    );
}

// Java: StarlarkRuleClassFunctionsTest.aspectDoc
#[test]
fn aspect_doc() {
    let m = exec_bzl(
        "tb13.bzl",
        "def _impl(target, ctx):\n   pass\n\
         documented_aspect = aspect(_impl, doc='My doc string')\n\
         long_documented_aspect = aspect(\n    implementation = _impl,\n    doc='''\n           \
         My doc string\n           \n           With details''',\n)\n\
         undocumented_aspect = aspect(_impl)\n",
    );
    assert_eq!(
        frozen_aspect(&m, "documented_aspect")
            .data()
            .get_documentation(),
        Some("My doc string")
    );
    assert_eq!(
        frozen_aspect(&m, "long_documented_aspect")
            .data()
            .get_documentation(),
        Some("My doc string\n\nWith details")
    );
    assert_eq!(
        frozen_aspect(&m, "undocumented_aspect")
            .data()
            .get_documentation(),
        None
    );
}
