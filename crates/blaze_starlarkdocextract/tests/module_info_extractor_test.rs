//! Port of `src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java`.

mod common;

use blaze_protos::stardoc_output::*;
use blaze_starlarkdocextract::label_renderer::LabelRenderer;
use common::{FAKE_LABEL_STRING, exec, exec_with_options, extract, extract_with};

/// `OriginKey.newBuilder().setName(name).setFile(fakeLabelString)`.
fn origin(name: &str) -> Option<OriginKey> {
    Some(OriginKey {
        name: name.to_owned(),
        file: FAKE_LABEL_STRING.to_owned(),
    })
}

/// `OriginKey.newBuilder().setFile(fakeLabelString)` (name unset).
fn origin_file_only() -> Option<OriginKey> {
    Some(OriginKey {
        name: String::new(),
        file: FAKE_LABEL_STRING.to_owned(),
    })
}

fn param(name: &str, role: FunctionParamRole) -> FunctionParamInfo {
    FunctionParamInfo {
        name: name.to_owned(),
        role: role as i32,
        ..Default::default()
    }
}

/// `.ignoringFields(StarlarkFunctionInfo.ORIGIN_KEY_FIELD_NUMBER)`.
fn funcs_ignoring_origin(info: &ModuleInfo) -> Vec<StarlarkFunctionInfo> {
    info.func_info
        .iter()
        .cloned()
        .map(|mut f| {
            f.origin_key = None;
            f
        })
        .collect()
}

/// `.ignoringFields(RuleInfo.ATTRIBUTE_FIELD_NUMBER)`.
fn rules_ignoring_attributes(info: &ModuleInfo) -> Vec<RuleInfo> {
    info.rule_info
        .iter()
        .cloned()
        .map(|mut r| {
            r.attribute.clear();
            r
        })
        .collect()
}

/// `.ignoringFields(ProviderInfo.ORIGIN_KEY_FIELD_NUMBER)`.
fn providers_ignoring_origin(info: &ModuleInfo) -> Vec<ProviderInfo> {
    info.provider_info
        .iter()
        .cloned()
        .map(|mut p| {
            p.origin_key = None;
            p
        })
        .collect()
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:106 ModuleInfoExtractorTest.moduleDocstring
#[test]
fn module_docstring() {
    let module_with_docstring = exec("'''This is my docstring'''\nfoo = 1\n");
    assert_eq!(
        extract(&module_with_docstring).unwrap().module_docstring,
        "This is my docstring"
    );
    let module_without_docstring = exec("foo = 1\n");
    assert_eq!(
        extract(&module_without_docstring).unwrap().module_docstring,
        ""
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:116 ModuleInfoExtractorTest.extractOnlyWantedLoadablePublicNames
#[test]
fn extract_only_wanted_loadable_public_names() {
    let module = exec(
        r#"
def loadable_unwanted():
    pass

def loadable_wanted():
    pass

def _nonloadable():
    pass

def _nonloadable_matches_wanted_predicate():
    pass

def _f():
    pass

def _g():
    pass

def _h():
    pass

namespace = struct(
    public_field_wanted = _f,
    public_field_unwanted = _g,
    _hidden_field_matches_wanted_predicate = _h,
)
"#,
    );
    let module_info = extract_with(
        &module,
        &|name: &str| name.contains("_wanted"),
        LabelRenderer::default_renderer(),
        false,
    )
    .unwrap();
    let names: Vec<&str> = module_info
        .func_info
        .iter()
        .map(|f| f.function_name.as_str())
        .collect();
    assert_eq!(
        sorted(names),
        sorted(vec!["loadable_wanted", "namespace.public_field_wanted"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:154 ModuleInfoExtractorTest.namespacedEntities
#[test]
fn namespaced_entities() {
    let module = exec(
        r#"
def _my_func(**kwargs):
    pass

_my_binary = rule(implementation = _my_func)
_my_aspect = aspect(implementation = _my_func)
_MyInfo = provider()
name = struct(
    spaced = struct(
        my_func = _my_func,
        my_binary = _my_binary,
        my_aspect = _my_aspect,
        MyInfo = _MyInfo,
    ),
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let names_and_origins = |v: Vec<(&str, Option<&OriginKey>)>| -> Vec<(String, String)> {
        v.into_iter()
            .map(|(n, o)| (n.to_owned(), o.unwrap().name.clone()))
            .collect()
    };
    assert_eq!(
        names_and_origins(
            module_info
                .func_info
                .iter()
                .map(|f| (f.function_name.as_str(), f.origin_key.as_ref()))
                .collect()
        ),
        vec![("name.spaced.my_func".to_owned(), "_my_func".to_owned())]
    );
    assert_eq!(
        names_and_origins(
            module_info
                .rule_info
                .iter()
                .map(|r| (r.rule_name.as_str(), r.origin_key.as_ref()))
                .collect()
        ),
        vec![("name.spaced.my_binary".to_owned(), "_my_binary".to_owned())]
    );
    assert_eq!(
        names_and_origins(
            module_info
                .aspect_info
                .iter()
                .map(|a| (a.aspect_name.as_str(), a.origin_key.as_ref()))
                .collect()
        ),
        vec![("name.spaced.my_aspect".to_owned(), "_my_aspect".to_owned())]
    );
    assert_eq!(
        names_and_origins(
            module_info
                .provider_info
                .iter()
                .map(|p| (p.provider_name.as_str(), p.origin_key.as_ref()))
                .collect()
        ),
        vec![("name.spaced.MyInfo".to_owned(), "_MyInfo".to_owned())]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:208 ModuleInfoExtractorTest.isWantedQualifiedName_appliesToQualifiedNamePrefixes
#[test]
fn is_wanted_qualified_name_applies_to_qualified_name_prefixes() {
    let module = exec(
        r#"
def _f():
    pass

def _g():
    pass

def _h():
    pass

def _i():
    pass

def _j():
    pass

foo = struct(
    bar = struct(
        f = _f,
    ),
    baz = struct(
        g = _g,
    ),
    h = _h,
)
baz = struct(
    qux = struct(
        i = _i,
    ),
    j = _j,
)
"#,
    );
    let module_info = extract_with(
        &module,
        &|name: &str| name == "foo.bar" || name == "baz",
        LabelRenderer::default_renderer(),
        false,
    )
    .unwrap();
    let names: Vec<&str> = module_info
        .func_info
        .iter()
        .map(|f| f.function_name.as_str())
        .collect();
    assert_eq!(
        sorted(names),
        sorted(vec!["foo.bar.f", "baz.qux.i", "baz.j"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:251 ModuleInfoExtractorTest.functionDocstring
#[test]
fn function_docstring() {
    let module = exec(
        r#"
def with_detailed_docstring():
    '''My function

    This function does things.
    '''
    pass

def with_one_line_docstring():
    '''My function'''
    pass

def without_docstring():
    pass
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.func_info,
        vec![
            StarlarkFunctionInfo {
                function_name: "with_detailed_docstring".to_owned(),
                doc_string: "My function\n\nThis function does things.".to_owned(),
                origin_key: origin("with_detailed_docstring"),
                ..Default::default()
            },
            StarlarkFunctionInfo {
                function_name: "with_one_line_docstring".to_owned(),
                doc_string: "My function".to_owned(),
                origin_key: origin("with_one_line_docstring"),
                ..Default::default()
            },
            StarlarkFunctionInfo {
                function_name: "without_docstring".to_owned(),
                origin_key: origin("without_docstring"),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:296 ModuleInfoExtractorTest.functionParams
#[test]
fn function_params() {
    let module = exec(
        r#"
def my_func(documented, undocumented, has_default = {"foo": "bar"}, *args, **kwargs):
    '''My function

    Args:
      documented: Documented param
    '''
    pass
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.func_info[0].parameter,
        vec![
            FunctionParamInfo {
                doc_string: "Documented param".to_owned(),
                mandatory: true,
                ..param("documented", FunctionParamRole::ParamRoleOrdinary)
            },
            FunctionParamInfo {
                mandatory: true,
                ..param("undocumented", FunctionParamRole::ParamRoleOrdinary)
            },
            FunctionParamInfo {
                default_value: "{\"foo\": \"bar\"}".to_owned(),
                ..param("has_default", FunctionParamRole::ParamRoleOrdinary)
            },
            param("args", FunctionParamRole::ParamRoleVarargs),
            param("kwargs", FunctionParamRole::ParamRoleKwargs),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:333 ModuleInfoExtractorTest.functionReturn
#[test]
fn function_return() {
    let module = exec(
        r#"
def with_return():
    '''My doc

    Returns:
      None
    '''
    return None

def without_return():
    '''My doc'''
    pass
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        funcs_ignoring_origin(&module_info),
        vec![
            StarlarkFunctionInfo {
                function_name: "with_return".to_owned(),
                doc_string: "My doc".to_owned(),
                r#return: Some(FunctionReturnInfo {
                    doc_string: "None".to_owned(),
                }),
                ..Default::default()
            },
            StarlarkFunctionInfo {
                function_name: "without_return".to_owned(),
                doc_string: "My doc".to_owned(),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:365 ModuleInfoExtractorTest.functionDeprecated
#[test]
fn function_deprecated() {
    let module = exec(
        r#"
def with_deprecated():
    '''My doc

    Deprecated:
      This is deprecated
    '''
    pass

def without_deprecated():
    '''My doc'''
    pass
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        funcs_ignoring_origin(&module_info),
        vec![
            StarlarkFunctionInfo {
                function_name: "with_deprecated".to_owned(),
                doc_string: "My doc".to_owned(),
                deprecated: Some(FunctionDeprecationInfo {
                    doc_string: "This is deprecated".to_owned(),
                }),
                ..Default::default()
            },
            StarlarkFunctionInfo {
                function_name: "without_deprecated".to_owned(),
                doc_string: "My doc".to_owned(),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:398 ModuleInfoExtractorTest.unexportedLambdaFunction
#[test]
fn unexported_lambda_function() {
    let module = exec(
        r#"
s = struct(
    lambda_function = lambda x: x * 2,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.func_info,
        vec![StarlarkFunctionInfo {
            // Note that origin key name is unset
            origin_key: origin_file_only(),
            function_name: "s.lambda_function".to_owned(),
            parameter: vec![FunctionParamInfo {
                mandatory: true,
                ..param("x", FunctionParamRole::ParamRoleOrdinary)
            }],
            ..Default::default()
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:422 ModuleInfoExtractorTest.unexportedGeneratedFunction
#[test]
fn unexported_generated_function() {
    let module = exec(
        r#"
def _multiply_by(y):
    def multiply(x):
        '''Multiplies x by constant y'''
        return x * y
    return multiply

s = struct(
    generated = _multiply_by(2),
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.func_info,
        vec![StarlarkFunctionInfo {
            // Note that origin key name is unset
            origin_key: origin_file_only(),
            function_name: "s.generated".to_owned(),
            doc_string: "Multiplies x by constant y".to_owned(),
            parameter: vec![FunctionParamInfo {
                mandatory: true,
                ..param("x", FunctionParamRole::ParamRoleOrdinary)
            }],
            ..Default::default()
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:453 ModuleInfoExtractorTest.providerDocstring
#[test]
fn provider_docstring() {
    let module = exec(
        r#"
DocumentedInfo = provider(doc = "My doc")
UndocumentedInfo = provider()
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.provider_info,
        vec![
            ProviderInfo {
                provider_name: "DocumentedInfo".to_owned(),
                doc_string: "My doc".to_owned(),
                origin_key: origin("DocumentedInfo"),
                ..Default::default()
            },
            ProviderInfo {
                provider_name: "UndocumentedInfo".to_owned(),
                origin_key: origin("UndocumentedInfo"),
                ..Default::default()
            },
        ]
    );
}

fn field(name: &str, doc: &str) -> ProviderFieldInfo {
    ProviderFieldInfo {
        name: name.to_owned(),
        doc_string: doc.to_owned(),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:477 ModuleInfoExtractorTest.providerFields
#[test]
fn provider_fields() {
    let module = exec(
        // Note fields below are not alphabetized
        r#"
DocumentedInfo = provider(fields = {"c": "C", "a": "A", "b": "B", "_hidden": "Hidden"})
UndocumentedInfo = provider(fields = ["c", "a", "b", "_hidden"])
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        providers_ignoring_origin(&module_info),
        vec![
            ProviderInfo {
                provider_name: "DocumentedInfo".to_owned(),
                field_info: vec![field("c", "C"), field("a", "A"), field("b", "B")],
                ..Default::default()
            },
            ProviderInfo {
                provider_name: "UndocumentedInfo".to_owned(),
                field_info: vec![field("c", ""), field("a", ""), field("b", "")],
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:504 ModuleInfoExtractorTest.providerInit
#[test]
fn provider_init() {
    let module = exec(
        r#"
def _my_info_init(x_value, y_value = 0):
    '''MyInfo constructor

    Args:
        x_value: my x value
        y_value: my y value
    '''
    return {"x": x_value, "y": y_value}

_MyInfo, _new_my_info = provider(
    doc = '''My provider''',
    fields = ["x", "y"],
    init = _my_info_init,
)

namespace = struct(
    MyInfo = _MyInfo,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.provider_info,
        vec![ProviderInfo {
            provider_name: "namespace.MyInfo".to_owned(),
            doc_string: "My provider".to_owned(),
            field_info: vec![field("x", ""), field("y", "")],
            init: Some(StarlarkFunctionInfo {
                function_name: "namespace.MyInfo".to_owned(),
                doc_string: "MyInfo constructor".to_owned(),
                parameter: vec![
                    FunctionParamInfo {
                        doc_string: "my x value".to_owned(),
                        mandatory: true,
                        ..param("x_value", FunctionParamRole::ParamRoleOrdinary)
                    },
                    FunctionParamInfo {
                        doc_string: "my y value".to_owned(),
                        default_value: "0".to_owned(),
                        ..param("y_value", FunctionParamRole::ParamRoleOrdinary)
                    },
                ],
                origin_key: origin("_my_info_init"),
                ..Default::default()
            }),
            origin_key: origin("_MyInfo"),
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:562 ModuleInfoExtractorTest.unexportedProvider_notDocumented
#[test]
fn unexported_provider_not_documented() {
    let module = exec(
        r#"
s = struct(
    MyUnexportedInfo = provider(),
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert!(module_info.provider_info.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:575 ModuleInfoExtractorTest.ruleDocstring
#[test]
fn rule_docstring() {
    let module = exec(
        r#"
def _my_impl(ctx):
    pass

documented_lib = rule(doc = "My doc", implementation = _my_impl)
undocumented_lib = rule(implementation = _my_impl)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        rules_ignoring_attributes(&module_info),
        vec![
            RuleInfo {
                rule_name: "documented_lib".to_owned(),
                doc_string: "My doc".to_owned(),
                origin_key: origin("documented_lib"),
                ..Default::default()
            },
            RuleInfo {
                rule_name: "undocumented_lib".to_owned(),
                origin_key: origin("undocumented_lib"),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:603 ModuleInfoExtractorTest.ruleAdvertisedProviders
#[test]
fn rule_advertised_providers() {
    let module = exec(
        r#"
MyInfo = provider()

def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    provides = [MyInfo, DefaultInfo],
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        rules_ignoring_attributes(&module_info),
        vec![RuleInfo {
            rule_name: "my_lib".to_owned(),
            origin_key: origin("my_lib"),
            advertised_providers: Some(ProviderNameGroup {
                provider_name: vec!["MyInfo".to_owned(), "DefaultInfo".to_owned()],
                origin_key: vec![
                    OriginKey {
                        name: "MyInfo".to_owned(),
                        file: FAKE_LABEL_STRING.to_owned(),
                    },
                    OriginKey {
                        name: "DefaultInfo".to_owned(),
                        file: "<native>".to_owned(),
                    },
                ],
            }),
            ..Default::default()
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:636 ModuleInfoExtractorTest.ruleTest
#[test]
fn rule_test() {
    let module = exec(
        r#"
MyInfo = provider()

def _my_impl(ctx):
    pass

my_test = rule(
    implementation = _my_impl,
    test = True,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        rules_ignoring_attributes(&module_info),
        vec![RuleInfo {
            rule_name: "my_test".to_owned(),
            origin_key: origin("my_test"),
            test: true,
            executable: true,
            ..Default::default()
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:663 ModuleInfoExtractorTest.ruleExecutable
#[test]
fn rule_executable() {
    let module = exec(
        r#"
MyInfo = provider()

def _my_impl(ctx):
    pass

my_binary = rule(
    implementation = _my_impl,
    executable = True,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        rules_ignoring_attributes(&module_info),
        vec![RuleInfo {
            rule_name: "my_binary".to_owned(),
            origin_key: origin("my_binary"),
            executable: true,
            ..Default::default()
        }]
    );
}
fn attr(name: &str, type_: AttributeType) -> AttributeInfo {
    AttributeInfo {
        name: name.to_owned(),
        r#type: type_ as i32,
        ..Default::default()
    }
}

fn with_default(name: &str, type_: AttributeType, default_value: &str) -> AttributeInfo {
    AttributeInfo {
        default_value: default_value.to_owned(),
        ..attr(name, type_)
    }
}

fn name_group(names: &[&str]) -> ProviderNameGroup {
    ProviderNameGroup {
        provider_name: names.iter().map(|n| (*n).to_owned()).collect(),
        origin_key: names
            .iter()
            .map(|n| OriginKey {
                name: (*n).to_owned(),
                file: FAKE_LABEL_STRING.to_owned(),
            })
            .collect(),
    }
}

fn implicit_rule_attributes() -> Vec<AttributeInfo> {
    blaze_starlarkdocextract::rule_info_extractor::implicit_rule_attributes()
        .into_iter()
        .map(|(_, a)| a)
        .collect()
}

fn implicit_macro_attributes() -> Vec<AttributeInfo> {
    blaze_starlarkdocextract::module_info_extractor::implicit_macro_attributes()
        .into_iter()
        .map(|(_, a)| a)
        .collect()
}

/// `containsExactlyElementsIn` (order-insensitive) for attribute lists; names are unique.
fn assert_same_attributes(actual: &[AttributeInfo], expected: Vec<AttributeInfo>) {
    let by_name = |mut v: Vec<AttributeInfo>| {
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    };
    assert_eq!(by_name(actual.to_vec()), by_name(expected));
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:689 ModuleInfoExtractorTest.ruleAttributes
#[test]
fn rule_attributes() {
    let module = exec_with_options(
        // TODO(https://github.com/bazelbuild/bazel/issues/6420): attr.license() is deprecated,
        // and will eventually be removed from Bazel.
        &["--noincompatible_no_attr_license"],
        r#"
MyInfo1 = provider()
MyInfo2 = provider()
MyInfo3 = provider()

def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    attrs = {
        "a": attr.string(doc = "My doc", default = "foo"),
        "b": attr.string(mandatory = True, values = ["foo", "bar"]),
        "c": attr.label(providers = [MyInfo1, MyInfo2]),
        "d": attr.label(providers = [[MyInfo1, MyInfo2], [MyInfo3]]),
        "_e": attr.string(doc = "Hidden attribute"),
        "deprecated_license": attr.license(),
    },
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let mut expected = implicit_rule_attributes();
    expected.extend([
        AttributeInfo {
            doc_string: "My doc".to_owned(),
            ..with_default("a", AttributeType::String, "\"foo\"")
        },
        AttributeInfo {
            mandatory: true,
            values: vec!["\"foo\"".to_owned(), "\"bar\"".to_owned()],
            ..attr("b", AttributeType::String)
        },
        AttributeInfo {
            provider_name_group: vec![name_group(&["MyInfo1", "MyInfo2"])],
            ..with_default("c", AttributeType::Label, "None")
        },
        AttributeInfo {
            provider_name_group: vec![
                name_group(&["MyInfo1", "MyInfo2"]),
                name_group(&["MyInfo3"]),
            ],
            ..with_default("d", AttributeType::Label, "None")
        },
        AttributeInfo {
            nonconfigurable: true,
            ..with_default(
                "deprecated_license",
                AttributeType::StringList,
                "[\"none\"]",
            )
        },
    ]);
    assert_same_attributes(&module_info.rule_info[0].attribute, expected);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:784 ModuleInfoExtractorTest.attributeOrder
#[test]
fn attribute_order() {
    let module = exec(
        r#"
def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    attrs = {
        "foo": attr.int(),
        "bar": attr.int(),
        "baz": attr.int(),
    },
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let names: Vec<&str> = module_info.rule_info[0]
        .attribute
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, vec!["name", "foo", "bar", "baz"]);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:809 ModuleInfoExtractorTest.attributeTypes
#[test]
fn attribute_types() {
    let module = exec(
        r#"
def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    attrs = {
        "a": attr.int(),
        "b": attr.label(),
        "c": attr.string(),
        "d": attr.string_list(),
        "e": attr.int_list(),
        "f": attr.label_list(),
        "g": attr.bool(),
        "h": attr.label_keyed_string_dict(),
        "i": attr.string_dict(),
        "j": attr.string_list_dict(),
        "k": attr.output(),
        "l": attr.output_list(),
        "m": attr.label_list_dict(),
    },
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let mut expected = implicit_rule_attributes();
    expected.extend([
        with_default("a", AttributeType::Int, "0"),
        with_default("b", AttributeType::Label, "None"),
        with_default("c", AttributeType::String, "\"\""),
        with_default("d", AttributeType::StringList, "[]"),
        with_default("e", AttributeType::IntList, "[]"),
        with_default("f", AttributeType::LabelList, "[]"),
        with_default("g", AttributeType::Boolean, "False"),
        with_default("h", AttributeType::LabelStringDict, "{}"),
        with_default("i", AttributeType::StringDict, "{}"),
        with_default("j", AttributeType::StringListDict, "{}"),
        AttributeInfo {
            nonconfigurable: true,
            ..with_default("k", AttributeType::Output, "None")
        },
        AttributeInfo {
            nonconfigurable: true,
            ..with_default("l", AttributeType::OutputList, "[]")
        },
        with_default("m", AttributeType::LabelListDict, "{}"),
    ]);
    assert_same_attributes(&module_info.rule_info[0].attribute, expected);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:912 ModuleInfoExtractorTest.unexportedRule_notDocumented
#[test]
fn unexported_rule_not_documented() {
    let module = exec(
        r#"
def _my_impl(ctx):
    pass

s = struct(
    my_rule = rule(
        doc = "Unexported rule",
        implementation = _my_impl,
    )
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert!(module_info.rule_info.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:931 ModuleInfoExtractorTest.macroDocstring
#[test]
fn macro_docstring() {
    let module = exec(
        r#"
def _my_impl(name, visibility):
    pass

documented_macro = macro(
    doc = "My doc",
    implementation = _my_impl,
)
undocumented_macro = macro(
    implementation = _my_impl,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.macro_info,
        vec![
            MacroInfo {
                macro_name: "documented_macro".to_owned(),
                doc_string: "My doc".to_owned(),
                origin_key: origin("documented_macro"),
                attribute: implicit_macro_attributes(),
                ..Default::default()
            },
            MacroInfo {
                macro_name: "undocumented_macro".to_owned(),
                origin_key: origin("undocumented_macro"),
                attribute: implicit_macro_attributes(),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:965 ModuleInfoExtractorTest.macroFinalizer
#[test]
fn macro_finalizer() {
    let module = exec(
        r#"
def _my_impl(name, visibility):
    pass

my_finalizer = macro(
    doc = "My finalizer",
    implementation = _my_impl,
    finalizer = True,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.macro_info,
        vec![MacroInfo {
            macro_name: "my_finalizer".to_owned(),
            doc_string: "My finalizer".to_owned(),
            origin_key: origin("my_finalizer"),
            attribute: implicit_macro_attributes(),
            finalizer: true,
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:992 ModuleInfoExtractorTest.macroAttributes
#[test]
fn macro_attributes() {
    let module = exec(
        r#"
def _my_impl(name):
    pass

my_macro = macro(
    attrs = {
        "some_attr": attr.label(mandatory = True),
        "another_attr": attr.int(doc = "An integer", default = 42),
        "_implicit_attr": attr.string(default = "IMPLICIT"),
    },
    implementation = _my_impl,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let mut expected = implicit_macro_attributes();
    expected.extend([
        AttributeInfo {
            mandatory: true,
            ..attr("some_attr", AttributeType::Label)
        },
        AttributeInfo {
            doc_string: "An integer".to_owned(),
            ..with_default("another_attr", AttributeType::Int, "42")
        },
    ]);
    assert_same_attributes(&module_info.macro_info[0].attribute, expected);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1029 ModuleInfoExtractorTest.macroInheritedAttributes
#[test]
fn macro_inherited_attributes() {
    let module = exec(
        r#"
def _my_rule_impl(ctx):
    pass

_my_rule = rule(
    implementation = _my_rule_impl,
    attrs = {
       "srcs": attr.label_list(doc = "My rule sources"),
    },
)

def _my_macro_impl(name, visibility, srcs, **kwargs):
    _my_rule(name = name, visibility = visibility, srcs = srcs, **kwargs)

my_macro = macro(
    inherit_attrs = _my_rule,
    implementation = _my_macro_impl,
)
"#,
    );
    let module_info = extract(&module).unwrap();
    let attributes = &module_info.macro_info[0].attribute;
    let implicit = blaze_starlarkdocextract::module_info_extractor::implicit_macro_attributes();
    let implicit_named = |name: &str| {
        implicit
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, a)| a.clone())
            .unwrap()
    };
    assert_eq!(attributes[0], implicit_named("name"));
    assert_eq!(attributes[1], implicit_named("visibility"));
    // Starlark-defined inherited attribute
    assert!(attributes.contains(&AttributeInfo {
        doc_string: "My rule sources".to_owned(),
        // Default value of inherited attributes is always None
        ..with_default("srcs", AttributeType::LabelList, "None")
    }));
    // Native inherited attributes may not be documented, so ignore doc string for them.
    let tags = AttributeInfo {
        nonconfigurable: true,
        natively_defined: true,
        // Default value of inherited attributes is always None
        ..with_default("tags", AttributeType::StringList, "None")
    };
    assert!(
        attributes
            .iter()
            .map(|a| AttributeInfo {
                doc_string: String::new(),
                ..a.clone()
            })
            .any(|a| a == tags),
        "{attributes:#?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1078 ModuleInfoExtractorTest.unexportedMacro_notDocumented
#[test]
fn unexported_macro_not_documented() {
    let module = exec(
        r#"
def _my_impl(name):
    pass

s = struct(
    my_macro = macro(
        doc = "Unexported macro",
        implementation = _my_impl,
    )
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert!(module_info.macro_info.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1097 ModuleInfoExtractorTest.providerNameGroups_useFirstDocumentableProviderName
#[test]
fn provider_name_groups_use_first_documentable_provider_name() {
    let module = exec(
        r#"
_MyInfo = provider()

def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    attrs = {
        "foo": attr.label(providers = [_MyInfo]),
    },
    provides = [_MyInfo],
)
namespace1 = struct(_MyUndocumentedInfo = _MyInfo)
namespace2 = struct(MyInfoB = _MyInfo, MyInfoA = _MyInfo)
namespace3 = struct(MyInfo = _MyInfo)
"#,
    );
    let module_info = extract(&module).unwrap();
    // Struct fields are extracted in field name alphabetical order, so namespace2.MyInfoA
    // (despite being declared after namespace2.MyInfoB) wins.
    assert_eq!(
        module_info.rule_info[0]
            .advertised_providers
            .as_ref()
            .unwrap()
            .provider_name[0],
        "namespace2.MyInfoA"
    );
    assert_eq!(
        // 0 is the implicit name attribute
        module_info.rule_info[0].attribute[1].provider_name_group[0].provider_name[0],
        "namespace2.MyInfoA"
    );
    let names: Vec<&str> = module_info
        .provider_info
        .iter()
        .map(|p| p.provider_name.as_str())
        .collect();
    assert_eq!(
        sorted(names),
        sorted(vec![
            "namespace2.MyInfoA",
            "namespace2.MyInfoB",
            "namespace3.MyInfo"
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1137 ModuleInfoExtractorTest.labelStringification
#[test]
fn label_stringification() {
    let module = exec(
        r#"
def _my_impl(ctx):
    pass

my_lib = rule(
    implementation = _my_impl,
    attrs = {
        "label": attr.label(default = "//test:foo"),
        "label_list": attr.label_list(
            default = ["//x", "@@canonical//y", "@@canonical//y:z"],
        ),
        "label_keyed_string_dict": attr.label_keyed_string_dict(
            default = {"//x": "label_in_main", "@@canonical//y": "label_in_dep"},
        ),
        "label_list_dict": attr.label_list_dict(
            default = {"a": ["//x", "@@canonical//y", "@@canonical//y:z"]},
        ),
    },
)
"#,
    );
    use blaze_cmdline::repository_mapping::RepositoryMapping;
    use blaze_cmdline::repository_name::{MAIN, RepositoryName};
    let canonical_name = RepositoryName::create("canonical").unwrap();
    let repository_mapping = RepositoryMapping::create([("local", canonical_name)], MAIN.clone());
    let module_info = extract_with(
        &module,
        &|_| true,
        LabelRenderer::new(repository_mapping, Some("my_repo".to_owned())),
        false,
    )
    .unwrap();
    let implicit = blaze_starlarkdocextract::rule_info_extractor::implicit_rule_attributes();
    let defaults: Vec<&str> = module_info.rule_info[0]
        .attribute
        .iter()
        .filter(|a| !implicit.iter().any(|(n, _)| *n == a.name))
        .map(|a| a.default_value.as_str())
        .collect();
    assert_eq!(
        sorted(defaults),
        sorted(vec![
            "\"@my_repo//test:foo\"",
            "[\"@my_repo//x\", \"@local//y\", \"@local//y:z\"]",
            "{\"@my_repo//x\": \"label_in_main\", \"@local//y\": \"label_in_dep\"}",
            "{\"a\": [\"@my_repo//x\", \"@local//y\", \"@local//y:z\"]}",
        ])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1176 ModuleInfoExtractorTest.aspectDocstring
#[test]
fn aspect_docstring() {
    let module = exec(
        r#"
def _my_impl(target, ctx):
    pass

documented_aspect = aspect(doc = "My doc", implementation = _my_impl)
undocumented_aspect = aspect(implementation = _my_impl)
"#,
    );
    let module_info = extract(&module).unwrap();
    // ignore implicit attributes
    let aspects: Vec<AspectInfo> = module_info
        .aspect_info
        .iter()
        .cloned()
        .map(|mut a| {
            a.attribute.clear();
            a
        })
        .collect();
    assert_eq!(
        aspects,
        vec![
            AspectInfo {
                aspect_name: "documented_aspect".to_owned(),
                doc_string: "My doc".to_owned(),
                origin_key: origin("documented_aspect"),
                ..Default::default()
            },
            AspectInfo {
                aspect_name: "undocumented_aspect".to_owned(),
                origin_key: origin("undocumented_aspect"),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1204 ModuleInfoExtractorTest.aspectAttributes
#[test]
fn aspect_attributes() {
    let module = exec(
        r#"
def _my_impl(target, ctx):
    pass

my_aspect = aspect(
    implementation = _my_impl,
    attr_aspects = ["deps", "srcs", "_private"],
    attrs = {
        "a": attr.string(doc = "My doc", default = "foo"),
        "b": attr.string(mandatory = True),
        "_c": attr.string(doc = "Hidden attribute"),
    },
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.aspect_info,
        vec![AspectInfo {
            aspect_name: "my_aspect".to_owned(),
            origin_key: origin("my_aspect"),
            aspect_attribute: vec!["deps".to_owned(), "srcs".to_owned()],
            attribute: vec![
                AttributeInfo {
                    doc_string: "My doc".to_owned(),
                    ..with_default("a", AttributeType::String, "\"foo\"")
                },
                AttributeInfo {
                    mandatory: true,
                    ..attr("b", AttributeType::String)
                },
            ],
            ..Default::default()
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1245 ModuleInfoExtractorTest.unexportedAspect_notDocumented
#[test]
fn unexported_aspect_not_documented() {
    let module = exec(
        r#"
def _my_impl(target, ctx):
    pass

s = struct(
    my_aspect = aspect(
        doc = "Unexported aspect",
        implementation = _my_impl,
    )
)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert!(module_info.aspect_info.is_empty());
}

fn other_symbol(name: &str, doc: &str, type_name: &str) -> StarlarkOtherSymbolInfo {
    StarlarkOtherSymbolInfo {
        name: name.to_owned(),
        doc: doc.to_owned(),
        type_name: type_name.to_owned(),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1264 ModuleInfoExtractorTest.starlarkOtherSymbols_extractedIfExportableAndDocumented
#[test]
fn starlark_other_symbols_extracted_if_exportable_and_documented() {
    let module = exec(
        r#"#: Exportable and documented
NAMES = ["foo", "bar"]

# Exportable but not documented
MORE_NAMES = ["baz", "qux"]

#: Ignored - non-exportable symbol
_PRIVATE_CONSTANT = 42

#: Struct
S = struct(answer = _PRIVATE_CONSTANT)
"#,
    );
    let module_info = extract(&module).unwrap();
    assert_eq!(
        module_info.starlark_other_symbol_info,
        vec![
            other_symbol("NAMES", "Exportable and documented", "list"),
            other_symbol("S", "Struct", "struct"),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1296 ModuleInfoExtractorTest.starlarkOtherSymbols_conflictingDocComments
fn starlark_other_symbols_conflicting_doc_comments(allow_unused_doc_comments: bool) {
    let module = exec("#: Leading doc comment\nANSWER = 42 #: Trailing doc comment\n");
    if allow_unused_doc_comments {
        assert_eq!(
            extract_with(&module, &|_| true, LabelRenderer::default_renderer(), true)
                .unwrap()
                .starlark_other_symbol_info,
            // Overrides leading doc comment.
            vec![other_symbol("ANSWER", "Trailing doc comment", "int")]
        );
    } else {
        let exception = extract(&module).unwrap_err();
        assert!(
            exception
                .to_string()
                .contains("unexpected or conflicting doc comments on line 1"),
            "{exception}"
        );
    }
}

#[test]
fn starlark_other_symbols_conflicting_doc_comments_allow_unused_doc_comments_true() {
    starlark_other_symbols_conflicting_doc_comments(true);
}

#[test]
fn starlark_other_symbols_conflicting_doc_comments_allow_unused_doc_comments_false() {
    starlark_other_symbols_conflicting_doc_comments(false);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1326 ModuleInfoExtractorTest.functions_cannotUseDocComments
fn functions_cannot_use_doc_comments(allow_unused_doc_comments: bool) {
    let module = exec(
        r#"def _my_function():
    pass

#: Unexpected doc comment
MY_FUNCTION_ALIAS = _my_function
"#,
    );
    if allow_unused_doc_comments {
        assert_eq!(
            extract_with(&module, &|_| true, LabelRenderer::default_renderer(), true)
                .unwrap()
                .func_info
                .len(),
            1
        );
    } else {
        let exception = extract(&module).unwrap_err();
        assert!(
            exception.to_string().contains(
                "unexpected doc comment for MY_FUNCTION_ALIAS on line 4; API documentation for a \
                 function must be provided in a docstring at the top of the function body"
            ),
            "{exception}"
        );
    }
}

#[test]
fn functions_cannot_use_doc_comments_allow_unused_doc_comments_true() {
    functions_cannot_use_doc_comments(true);
}

#[test]
fn functions_cannot_use_doc_comments_allow_unused_doc_comments_false() {
    functions_cannot_use_doc_comments(false);
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractorTest.java:1352 ModuleInfoExtractorTest.rules_cannotUseDocComments
fn rules_cannot_use_doc_comments(allow_unused_doc_comments: bool) {
    let module = exec(
        r#"def _impl(ctx):
    pass

#: Unexpected doc comment
my_rule = rule(implementation = _impl)
"#,
    );
    if allow_unused_doc_comments {
        assert_eq!(
            extract_with(&module, &|_| true, LabelRenderer::default_renderer(), true)
                .unwrap()
                .rule_info
                .len(),
            1
        );
    } else {
        let exception = extract(&module).unwrap_err();
        assert!(
            exception.to_string().contains(
                "unexpected doc comment for my_rule on line 4; API documentation for a rule must \
                 be provided in the doc argument to rule()"
            ),
            "{exception}"
        );
    }
}

#[test]
fn rules_cannot_use_doc_comments_allow_unused_doc_comments_true() {
    rules_cannot_use_doc_comments(true);
}

#[test]
fn rules_cannot_use_doc_comments_allow_unused_doc_comments_false() {
    rules_cannot_use_doc_comments(false);
}
