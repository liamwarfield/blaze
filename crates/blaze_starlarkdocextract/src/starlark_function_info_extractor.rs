//! Port of `starlarkdocextract/StarlarkFunctionInfoExtractor.java`.

use blaze_packages::bzl_context::BazelModuleContext;
use blaze_protos::stardoc_output::{
    FunctionDeprecationInfo, FunctionParamInfo, FunctionParamRole, FunctionReturnInfo, OriginKey,
    StarlarkFunctionInfo,
};
use blaze_starlark::Location;
use starlark::eval::{DefModule, DefParameterRole};
use starlark::values::Value;

use crate::docstring_utils::{DocstringParseError, parse_docstring};
use crate::extraction_exception::ExtractionException;
use crate::label_renderer::LabelRenderer;

/// Create and return a `StarlarkFunctionInfo` object encapsulating information obtained from
/// the given function and from its parsed docstring.
///
/// - `function_name`: the name of the function in the target scope. (Note this is not
///   necessarily the original exported function name; the function may have been renamed in the
///   target Starlark file's scope)
/// - `function`: the function object (a `def` or `lambda`)
/// - `label_renderer`: a string renderer for `Label` values in argument defaults and for the
///   `OriginKey`'s file
// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:59 StarlarkFunctionInfoExtractor.fromNameAndFunction
pub fn from_name_and_function(
    function_name: &str,
    function: Value<'_>,
    label_renderer: &LabelRenderer,
) -> Result<StarlarkFunctionInfo, ExtractionException> {
    let mut param_name_to_doc_map: Vec<(String, String)> = Vec::new();
    let mut function_info = StarlarkFunctionInfo {
        function_name: function_name.to_owned(),
        origin_key: Some(get_function_origin_key(function, label_renderer)),
        ..StarlarkFunctionInfo::default()
    };
    // fn.getDocumentation(): the trimmed docstring.
    // Java: net/starlark/java/eval/StarlarkFunction.java:190 StarlarkFunction.getDocumentation
    let doc = DefModule::def_docstring(function)
        .expect("a StarlarkFunction")
        .map(|d| blaze_packages::starlark_defs::util::trim_doc_string(&d));
    if let Some(doc) = doc {
        let mut parse_errors: Vec<DocstringParseError> = Vec::new();
        let docstring_info = parse_docstring(&doc, &mut parse_errors);
        if !parse_errors.is_empty() {
            return Err(make_docstring_extraction_exception(
                function_name,
                &function_location(function),
                &parse_errors,
            ));
        }
        let mut function_description = docstring_info.summary.clone();
        if !docstring_info.summary.is_empty() && !docstring_info.long_description.is_empty() {
            function_description.push_str("\n\n");
        }
        function_description.push_str(&docstring_info.long_description);
        function_info.doc_string = function_description;
        for param_doc in &docstring_info.parameters {
            // Maps.newLinkedHashMap().put: a later entry replaces the value in place.
            match param_name_to_doc_map
                .iter_mut()
                .find(|(name, _)| *name == param_doc.parameter_name)
            {
                Some(entry) => entry.1 = param_doc.description.clone(),
                None => param_name_to_doc_map.push((
                    param_doc.parameter_name.clone(),
                    param_doc.description.clone(),
                )),
            }
        }
        if !docstring_info.returns.is_empty() {
            function_info.r#return = Some(FunctionReturnInfo {
                doc_string: docstring_info.returns.clone(),
            });
        }
        if !docstring_info.deprecated.is_empty() {
            function_info.deprecated = Some(FunctionDeprecationInfo {
                doc_string: docstring_info.deprecated.clone(),
            });
        }
    }
    function_info.parameter = parameter_infos(function, &param_name_to_doc_map, label_renderer);
    Ok(function_info)
}

/// `fn.getLocation()`: where the function is declared.
// Java: net/starlark/java/eval/StarlarkFunction.java:172 StarlarkFunction.getLocation
fn function_location(function: Value<'_>) -> String {
    match DefModule::declaration_span(function) {
        Some(span) => Location::of_span_start(&span).to_string(),
        None => Location::builtin().to_string(),
    }
}

// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:104 StarlarkFunctionInfoExtractor.makeDocstringExtractionException
fn make_docstring_extraction_exception(
    function_name: &str,
    defined_location: &str,
    parse_errors: &[DocstringParseError],
) -> ExtractionException {
    let mut message = format!(
        "Unable to generate documentation for function {function_name} (defined at \
         {defined_location}) due to malformed docstring. Parse errors:\n"
    );
    for parse_error in parse_errors {
        message.push_str(&format!(
            "  {} line {}: {}\n",
            defined_location,
            parse_error.line_number,
            parse_error.message.replace('\n', " ")
        ));
    }
    ExtractionException::new(message)
}

// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:122 StarlarkFunctionInfoExtractor.forParam
fn for_param(
    name: &str,
    doc_string: Option<&str>,
    default_value: Option<Value<'_>>,
    role: FunctionParamRole,
    label_renderer: &LabelRenderer,
) -> FunctionParamInfo {
    let mut param = FunctionParamInfo {
        name: name.to_owned(),
        role: role as i32,
        ..FunctionParamInfo::default()
    };
    if let Some(doc) = doc_string {
        param.doc_string = doc.to_owned();
    }
    match default_value {
        None => param.mandatory = true,
        Some(default_value) => {
            param.default_value = label_renderer.repr(default_value);
            param.mandatory = false;
        }
    }
    param
}

/// Constructor to be used for *args or **kwargs.
// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:147 StarlarkFunctionInfoExtractor.forSpecialParam
fn for_special_param(
    name: &str,
    doc_string: Option<&str>,
    role: FunctionParamRole,
) -> FunctionParamInfo {
    let mut param = FunctionParamInfo {
        name: name.to_owned(),
        role: role as i32,
        mandatory: false,
        ..FunctionParamInfo::default()
    };
    if let Some(doc) = doc_string {
        param.doc_string = doc.to_owned();
    }
    param
}

// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:160 StarlarkFunctionInfoExtractor.parameterInfos
fn parameter_infos(
    function: Value<'_>,
    parameter_doc: &[(String, String)],
    label_renderer: &LabelRenderer,
) -> Vec<FunctionParamInfo> {
    let doc_of = |name: &str| {
        parameter_doc
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_str())
    };
    DefModule::def_parameters(function)
        .expect("a StarlarkFunction")
        .into_iter()
        .map(|param| match param.role {
            // *args
            DefParameterRole::Varargs => for_special_param(
                &param.name,
                doc_of(&format!("*{}", param.name)),
                FunctionParamRole::ParamRoleVarargs,
            ),
            // **kwargs
            DefParameterRole::Kwargs => for_special_param(
                &param.name,
                doc_of(&format!("**{}", param.name)),
                FunctionParamRole::ParamRoleKwargs,
            ),
            // regular parameter
            DefParameterRole::Ordinary => for_param(
                &param.name,
                doc_of(&param.name),
                param.default_value,
                FunctionParamRole::ParamRoleOrdinary,
                label_renderer,
            ),
            DefParameterRole::KeywordOnly => for_param(
                &param.name,
                doc_of(&param.name),
                param.default_value,
                FunctionParamRole::ParamRoleKeywordOnly,
                label_renderer,
            ),
        })
        .collect()
}

// Java: com/google/devtools/build/lib/starlarkdocextract/StarlarkFunctionInfoExtractor.java:197 StarlarkFunctionInfoExtractor.getFunctionOriginKey
fn get_function_origin_key(function: Value<'_>, label_renderer: &LabelRenderer) -> OriginKey {
    let mut origin_key = OriginKey::default();
    // We can't just `builder.setName(fn.getName())` - fn could be a nested function or a lambda,
    // so fn.getName() may not be a unique name in fn's module. Instead, we look for fn in the
    // module's globals, and if we fail to find it, we leave OriginKey.name unset.
    let module = DefModule::of_function(function).expect("a StarlarkFunction");
    if let Some(globals) = module.frozen_globals() {
        for (name, value) in globals {
            // fn.equals(entry.getValue()): identity.
            if value.to_value().ptr_eq(function) {
                origin_key.name = name;
                break;
            }
        }
    }
    // TODO(arostovtsev): also recurse into global structs/dicts/lists

    if let Some(module_context) = BazelModuleContext::of_filename(module.file_name()) {
        origin_key.file = label_renderer.render(module_context.label());
    }
    origin_key
}
