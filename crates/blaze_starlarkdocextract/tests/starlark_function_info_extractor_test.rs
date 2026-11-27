//! Port of `src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java`.

mod common;

use blaze_protos::stardoc_output::{
    FunctionDeprecationInfo, FunctionParamInfo, FunctionParamRole, FunctionReturnInfo, OriginKey,
    StarlarkFunctionInfo,
};
use blaze_starlarkdocextract::extraction_exception::ExtractionException;
use blaze_starlarkdocextract::label_renderer::LabelRenderer;
use blaze_starlarkdocextract::starlark_function_info_extractor::from_name_and_function;
use common::{FAKE_LABEL_STRING, exec};

/// Java's `exec(lines...)` returns the first global, which must be a function, and the tests pass
/// it to `fromNameAndFunction`; here the function is looked up by its name in the loaded module.
fn extract_function(
    source: &str,
    global: &str,
    function_name: &str,
) -> Result<StarlarkFunctionInfo, ExtractionException> {
    let module = exec(source);
    let value = module
        .value
        .get_module()
        .get(global)
        .unwrap_or_else(|e| panic!("no global {global}: {e}"));
    from_name_and_function(
        function_name,
        value.value(),
        &LabelRenderer::default_renderer(),
    )
}

fn param(
    name: &str,
    role: FunctionParamRole,
    doc: &str,
    mandatory: bool,
    default_value: &str,
) -> FunctionParamInfo {
    FunctionParamInfo {
        name: name.to_owned(),
        role: role as i32,
        doc_string: doc.to_owned(),
        mandatory,
        default_value: default_value.to_owned(),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:67 StarlarkFunctionInfoExtractorTest.basicFunctionality
#[test]
fn basic_functionality() {
    let info = extract_function("def fn(x):\n    pass\n", "fn", "namespace.fn").unwrap();
    assert_eq!(
        info,
        StarlarkFunctionInfo {
            function_name: "namespace.fn".to_owned(),
            parameter: vec![param(
                "x",
                FunctionParamRole::ParamRoleOrdinary,
                "",
                true,
                ""
            )],
            origin_key: Some(OriginKey {
                name: "fn".to_owned(),
                file: FAKE_LABEL_STRING.to_owned(),
            }),
            ..Default::default()
        }
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:93 StarlarkFunctionInfoExtractorTest.summary_canStartOnFirstOrSecondLine
#[test]
fn summary_can_start_on_first_or_second_line() {
    let info1 = extract_function(
        r#"
def fn1(x):
    "Summary."
    pass
"#,
        "fn1",
        "fn",
    )
    .unwrap();
    let info2 = extract_function(
        r#"
def fn2(x):
    '''
    Summary.
    '''
    pass
"#,
        "fn2",
        "fn",
    )
    .unwrap();
    assert_eq!(info1.doc_string, "Summary.");
    assert_eq!(info2.doc_string, "Summary.");
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:120 StarlarkFunctionInfoExtractorTest.summary_mustBeFollowedByBlankLine
#[test]
fn summary_must_be_followed_by_blank_line() {
    let good = extract_function(
        r#"
def good(x):
    '''
    Summary.

    Details.'''
    pass
"#,
        "good",
        "good",
    )
    .unwrap();
    assert_eq!(good.doc_string, "Summary.\n\nDetails.");
    let no_blank_line_exception = extract_function(
        r#"
def bad_no_blank_line(x):
    '''
    Summary.
    Details.
    '''
    pass
"#,
        "bad_no_blank_line",
        "bad_no_blank_line",
    )
    .unwrap_err();
    assert!(
        no_blank_line_exception
            .to_string()
            .contains("the one-line summary should be followed by a blank line"),
        "{no_blank_line_exception}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:158 StarlarkFunctionInfoExtractorTest.keywordOnly
#[test]
fn keyword_only() {
    let info = extract_function(
        r#"
def fn(a, b=1, *, c, d=2):
    '''This function does stuff.

    Args:
      a: A value.
      b: B value
      c: C value.
      d: D value.
    '''
    pass
"#,
        "fn",
        "fn",
    )
    .unwrap();
    assert_eq!(
        info.parameter,
        vec![
            param(
                "a",
                FunctionParamRole::ParamRoleOrdinary,
                "A value.",
                true,
                ""
            ),
            param(
                "b",
                FunctionParamRole::ParamRoleOrdinary,
                "B value",
                false,
                "1"
            ),
            param(
                "c",
                FunctionParamRole::ParamRoleKeywordOnly,
                "C value.",
                true,
                ""
            ),
            param(
                "d",
                FunctionParamRole::ParamRoleKeywordOnly,
                "D value.",
                false,
                "2"
            ),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:207 StarlarkFunctionInfoExtractorTest.keywordOnly_withVarargs
#[test]
fn keyword_only_with_varargs() {
    let info = extract_function(
        r#"
def fn(a, b=1, *args, c, d=2):
    '''This function does stuff.

    Args:
      a: A value.
      b: B value
      c: C value.
      d: D value.
      *args: Remaining positional arguments.
    '''
    pass
"#,
        "fn",
        "fn",
    )
    .unwrap();
    assert_eq!(
        info.parameter,
        vec![
            param(
                "a",
                FunctionParamRole::ParamRoleOrdinary,
                "A value.",
                true,
                ""
            ),
            param(
                "b",
                FunctionParamRole::ParamRoleOrdinary,
                "B value",
                false,
                "1"
            ),
            param(
                "c",
                FunctionParamRole::ParamRoleKeywordOnly,
                "C value.",
                true,
                ""
            ),
            param(
                "d",
                FunctionParamRole::ParamRoleKeywordOnly,
                "D value.",
                false,
                "2"
            ),
            param(
                "args",
                FunctionParamRole::ParamRoleVarargs,
                "Remaining positional arguments.",
                false,
                ""
            ),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:263 StarlarkFunctionInfoExtractorTest.keywordOnly_withVarargsAndKwargs
#[test]
fn keyword_only_with_varargs_and_kwargs() {
    let info = extract_function(
        r#"
def fn(a, b=1, *args, c, d=2, **kwargs):
    '''This function does stuff.

    Args:
      a: A value.
      b: B value
      c: C value.
      d: D value.
    '''
    pass
"#,
        "fn",
        "fn",
    )
    .unwrap();
    assert_eq!(
        info.parameter,
        vec![
            param(
                "a",
                FunctionParamRole::ParamRoleOrdinary,
                "A value.",
                true,
                ""
            ),
            param(
                "b",
                FunctionParamRole::ParamRoleOrdinary,
                "B value",
                false,
                "1"
            ),
            param(
                "c",
                FunctionParamRole::ParamRoleKeywordOnly,
                "C value.",
                true,
                ""
            ),
            param(
                "d",
                FunctionParamRole::ParamRoleKeywordOnly,
                "D value.",
                false,
                "2"
            ),
            param("args", FunctionParamRole::ParamRoleVarargs, "", false, ""),
            param("kwargs", FunctionParamRole::ParamRoleKwargs, "", false, ""),
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:322 StarlarkFunctionInfoExtractorTest.returns
#[test]
fn returns() {
    let info = extract_function(
        r#"
def fn(x):
    '''
    My function.

    Returns:
      The value of x.
    '''
    return x
"#,
        "fn",
        "fn",
    )
    .unwrap();
    assert_eq!(
        info.r#return,
        Some(FunctionReturnInfo {
            doc_string: "The value of x.".to_owned(),
        })
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:342 StarlarkFunctionInfoExtractorTest.deprecation
#[test]
fn deprecation() {
    let info = extract_function(
        r#"
def fn(x):
    '''
    My function.

    Deprecated:
      Do not use.
      Use something else instead.
    '''
    pass
"#,
        "fn",
        "fn",
    )
    .unwrap();
    assert_eq!(
        info.deprecated,
        Some(FunctionDeprecationInfo {
            doc_string: "Do not use.\nUse something else instead.".to_owned(),
        })
    );
}

// Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractorTest.java:366 StarlarkFunctionInfoExtractorTest.specialSections_canBeSeparatedByAnyNumberOfBlankLines
#[test]
fn special_sections_can_be_separated_by_any_number_of_blank_lines() {
    let mut extra_blank_lines = String::new();
    for i in 0..2 {
        let source = format!(
            "def fn{i}(x):\n    '''\n    My function.\n\n    Args:\n      x: X value.{e}\n    \
             Returns:\n      The value of x.{e}\n    Deprecated:\n      Do not use.\n    '''\n    \
             return x\n",
            e = extra_blank_lines
        );
        let name = format!("fn{i}");
        let info = extract_function(&source, &name, &name).unwrap();
        assert_eq!(
            info,
            StarlarkFunctionInfo {
                function_name: name.clone(),
                doc_string: "My function.".to_owned(),
                parameter: vec![param(
                    "x",
                    FunctionParamRole::ParamRoleOrdinary,
                    "X value.",
                    true,
                    ""
                )],
                r#return: Some(FunctionReturnInfo {
                    doc_string: "The value of x.".to_owned(),
                }),
                deprecated: Some(FunctionDeprecationInfo {
                    doc_string: "Do not use.".to_owned(),
                }),
                origin_key: Some(OriginKey {
                    name: name.clone(),
                    file: FAKE_LABEL_STRING.to_owned(),
                }),
            }
        );
        extra_blank_lines.push('\n');
    }
}
