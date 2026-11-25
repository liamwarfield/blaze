//! Port of `packages/ImplicitOutputsFunction.java`: computes the implicit outputs of a rule.
//!
//! In the case that attribute placeholders are configurable attributes, errors are reported as
//! output templates are expanded before configurable attributes are resolved. In the case that
//! attribute placeholders are invalid, the template string is left unexpanded.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::EvalException;
use blaze_events::EventHandler;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::attribute::AttributeMap;
use crate::rule_class::StarlarkCallable;
use crate::types::AttrValue;
use crate::types::Type;

/// A native safe implicit outputs function body (Java anonymous `SafeImplicitOutputsFunction`).
pub type NativeImplicitOutputsFn = Arc<dyn Fn(&dyn AttributeMap) -> Vec<String> + Send + Sync>;

/// Java `ImplicitOutputsFunction.AttributeValueGetter`: the value(s) of an attribute as strings,
/// or the empty set if the attribute is unknown. Java passes a nullable rule.
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:186 ImplicitOutputsFunction.AttributeValueGetter
pub type AttributeValueGetter<'a> =
    dyn Fn(Option<&dyn AttributeMap>, &str) -> Result<IndexSet<String>, EvalException> + 'a;

/// Java `ImplicitOutputsFunction` and its subclasses.
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:55 ImplicitOutputsFunction
#[derive(Clone)]
pub enum ImplicitOutputsFunction {
    /// Java `SafeImplicitOutputsFunction.NONE`.
    None,
    /// Java `fromTemplates(...)`: `%{attr}` placeholder templates.
    Templates(Vec<String>),
    /// Java `fromFunctions(...)`: the union of several safe functions.
    Functions(Vec<ImplicitOutputsFunction>),
    /// A native safe function.
    Native(NativeImplicitOutputsFn),
    /// Java `StarlarkImplicitOutputsFunctionWithCallback`: `outputs = <function>`, with the
    /// semantics of the thread that defined it (Java `StarlarkCallbackHelper.starlarkSemantics`).
    StarlarkWithCallback {
        callback: StarlarkCallable,
        semantics: Option<Arc<blaze_starlark::StarlarkSemantics>>,
    },
    /// Java `StarlarkImplicitOutputsFunctionWithMap`: `outputs = {"key": "template"}`.
    StarlarkWithMap(Vec<(String, String)>),
}

impl fmt::Debug for ImplicitOutputsFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImplicitOutputsFunction::None => f.write_str("None"),
            ImplicitOutputsFunction::Templates(t) => f.debug_tuple("Templates").field(t).finish(),
            ImplicitOutputsFunction::Functions(v) => f.debug_tuple("Functions").field(v).finish(),
            ImplicitOutputsFunction::Native(_) => f.write_str("Native(..)"),
            ImplicitOutputsFunction::StarlarkWithCallback { callback, .. } => f
                .debug_struct("StarlarkWithCallback")
                .field("callback", callback)
                .finish(),
            ImplicitOutputsFunction::StarlarkWithMap(m) => {
                f.debug_tuple("StarlarkWithMap").field(m).finish()
            }
        }
    }
}

impl ImplicitOutputsFunction {
    /// Java `instanceof StarlarkImplicitOutputsFunction`.
    pub fn is_starlark(&self) -> bool {
        matches!(
            self,
            ImplicitOutputsFunction::StarlarkWithCallback { .. }
                | ImplicitOutputsFunction::StarlarkWithMap(_)
        )
    }

    /// Java `instanceof SafeImplicitOutputsFunction`.
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            ImplicitOutputsFunction::None
                | ImplicitOutputsFunction::Templates(_)
                | ImplicitOutputsFunction::Functions(_)
                | ImplicitOutputsFunction::Native(_)
        )
    }

    /// Java `== SafeImplicitOutputsFunction.NONE`.
    pub fn is_none(&self) -> bool {
        matches!(self, ImplicitOutputsFunction::None)
    }

    /// The output files this rule produces implicitly, given its populated attributes.
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:201 ImplicitOutputsFunction.getImplicitOutputs
    pub fn get_implicit_outputs(
        &self,
        event_handler: &dyn EventHandler,
        rule: &dyn AttributeMap,
    ) -> Result<Vec<String>, EvalException> {
        match self {
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:170 ImplicitOutputsFunction.SafeImplicitOutputsFunction.NONE.getImplicitOutputs
            ImplicitOutputsFunction::None => Ok(Vec::new()),
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:236 ImplicitOutputsFunction.TemplateImplicitOutputsFunction.getImplicitOutputs
            ImplicitOutputsFunction::Templates(templates) => {
                let mut result = IndexSet::new();
                for template in templates {
                    let substitutions = Self::substitute_placeholder_into_template(template, rule)?;
                    if substitutions.is_empty() {
                        continue;
                    }
                    result.extend(substitutions);
                }
                Ok(result.into_iter().collect())
            }
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:334 ImplicitOutputsFunction.FunctionCombinationImplicitOutputsFunction.getImplicitOutputs
            ImplicitOutputsFunction::Functions(functions) => {
                let mut result = IndexSet::new();
                for function in functions {
                    result.extend(function.get_implicit_outputs(event_handler, rule)?);
                }
                Ok(result.into_iter().collect())
            }
            ImplicitOutputsFunction::Native(f) => Ok(f(rule)),
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:69 ImplicitOutputsFunction.StarlarkImplicitOutputsFunction.getImplicitOutputs
            ImplicitOutputsFunction::StarlarkWithCallback { .. }
            | ImplicitOutputsFunction::StarlarkWithMap(_) => Ok(self
                .calculate_outputs(event_handler, rule)?
                .into_iter()
                .map(|(_, v)| v)
                .collect()),
        }
    }

    /// Java `StarlarkImplicitOutputsFunction.calculateOutputs`: output key to file name, in
    /// order.
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:66 ImplicitOutputsFunction.StarlarkImplicitOutputsFunction.calculateOutputs
    pub fn calculate_outputs(
        &self,
        event_handler: &dyn EventHandler,
        map: &dyn AttributeMap,
    ) -> Result<Vec<(String, String)>, EvalException> {
        match self {
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:88 ImplicitOutputsFunction.StarlarkImplicitOutputsFunctionWithCallback.calculateOutputs
            ImplicitOutputsFunction::StarlarkWithCallback {
                callback,
                semantics,
            } => starlark_callback_outputs(callback, semantics.as_ref(), event_handler, map),
            // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:142 ImplicitOutputsFunction.StarlarkImplicitOutputsFunctionWithMap.calculateOutputs
            ImplicitOutputsFunction::StarlarkWithMap(output_map) => {
                let mut builder: Vec<(String, String)> = Vec::new();
                for (key, value) in output_map {
                    // Empty iff invalid placeholders present.
                    let substitutions =
                        unsafe_templates_get_implicit_outputs(std::slice::from_ref(value), map)?;
                    if substitutions.is_empty() {
                        return Err(EvalException::new(format!(
                            "For attribute '{key}' in outputs: Invalid placeholder(s) in template"
                        )));
                    }
                    // Java: Iterables.getOnlyElement.
                    if substitutions.len() != 1 {
                        panic!(
                            "IllegalArgumentException: expected one element but was: <{}>",
                            substitutions.join(", ")
                        );
                    }
                    // Java: ImmutableMap.Builder.buildOrThrow (keys are distinct map keys).
                    builder.push((key.clone(), substitutions.into_iter().next().unwrap()));
                }
                Ok(builder)
            }
            _ => panic!("ClassCastException: not a StarlarkImplicitOutputsFunction"),
        }
    }

    /// Java `fromTemplates(String...)`.
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:207 ImplicitOutputsFunction.fromTemplates
    pub fn from_templates(templates: &[&str]) -> ImplicitOutputsFunction {
        ImplicitOutputsFunction::Templates(templates.iter().map(|s| (*s).to_owned()).collect())
    }

    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:306 ImplicitOutputsFunction.fromFunctions
    pub fn from_functions(functions: Vec<ImplicitOutputsFunction>) -> ImplicitOutputsFunction {
        ImplicitOutputsFunction::Functions(functions)
    }

    /// Collects all named placeholders from the template while replacing them with `%s`;
    /// incomplete placeholders are treated like text (`%` is escaped as `%%`).
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:416 ImplicitOutputsFunction.createPlaceholderSubstitutionFormatString
    pub fn create_placeholder_substitution_format_string(
        template: &str,
        placeholders: &mut Vec<String>,
    ) -> String {
        let mut format_builder = String::new();
        create_placeholder_substitution_format_string_recursive(
            template,
            placeholders,
            &mut format_builder,
        );
        format_builder
    }

    /// Replaces all `%{...}` placeholders of `template` with the values of `rule`'s attributes;
    /// with several placeholders, the result is the cross product of substitutions.
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:445 ImplicitOutputsFunction.substitutePlaceholderIntoTemplate
    pub fn substitute_placeholder_into_template(
        template: &str,
        rule: &dyn AttributeMap,
    ) -> Result<Vec<String>, EvalException> {
        Self::substitute_placeholder_into_template_with(template, Some(rule), &|rule, attr| {
            attribute_values(rule.expect("NullPointerException: rule"), attr)
        })
    }

    /// Substitutes attribute placeholders in a template string, producing all possible
    /// combinations; empty if any of the placeholders expands to no values.
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:461 ImplicitOutputsFunction.substitutePlaceholderIntoTemplate
    pub fn substitute_placeholder_into_template_with(
        template: &str,
        rule: Option<&dyn AttributeMap>,
        attribute_getter: &AttributeValueGetter<'_>,
    ) -> Result<Vec<String>, EvalException> {
        // Parse the template to get the attribute names and format string.
        let parsed_template = ParsedTemplate::parse(template);

        // Return the substituted strings.
        parsed_template.substitute_attributes(rule, attribute_getter)
    }
}

// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:422 ImplicitOutputsFunction.createPlaceholderSubstitutionFormatStringRecursive
fn create_placeholder_substitution_format_string_recursive(
    template: &str,
    placeholders: &mut Vec<String>,
    format_builder: &mut String,
) {
    let Some(start) = template.find("%{") else {
        format_builder.push_str(&percent_escape(template));
        return;
    };

    let Some(end) = template[start + 2..].find('}').map(|e| e + start + 2) else {
        format_builder.push_str(&percent_escape(template));
        return;
    };

    format_builder.push_str(&percent_escape(&template[..start]));
    format_builder.push_str("%s");
    placeholders.push(template[start + 2..end].to_owned());
    create_placeholder_substitution_format_string_recursive(
        &template[end + 1..],
        placeholders,
        format_builder,
    );
}

/// Java `PERCENT_ESCAPER`: `%` to `%%`.
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:194 ImplicitOutputsFunction.PERCENT_ESCAPER
fn percent_escape(s: &str) -> String {
    s.replace('%', "%%")
}

/// Java `String.format(formatStr, args)` for the format strings this file creates (only `%s`
/// and `%%` directives).
fn format_with_args(format_str: &str, args: &[&String]) -> String {
    let mut out = String::with_capacity(format_str.len());
    let mut args = args.iter();
    let mut chars = format_str.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push_str(args.next().expect("MissingFormatArgumentException")),
            Some('%') => out.push('%'),
            other => panic!("UnknownFormatConversionException: {other:?}"),
        }
    }
    out
}

/// Java `FileSystemUtils.removeExtension(String)`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:128 FileSystemUtils.removeExtension
fn remove_extension(filename: &str) -> String {
    let Some(last_dot_index) = filename.rfind('.') else {
        return filename.to_owned();
    };
    if let Some(last_slash_index) = filename.rfind('/')
        && last_slash_index > last_dot_index
    {
        return filename.to_owned();
    }
    filename[..last_dot_index].to_owned()
}

/// Coerces attribute `attr_name` of `rule` into a set of strings.
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:356 ImplicitOutputsFunction.attributeValues
fn attribute_values(
    rule: &dyn AttributeMap,
    attr_name: &str,
) -> Result<IndexSet<String>, EvalException> {
    if attr_name == "dirname" {
        let dir = PathFragment::create(rule.get_label().get_name()).get_parent_directory();
        let dir = dir.expect("NullPointerException: parent directory");
        return Ok(if dir.is_empty() {
            IndexSet::from([String::new()])
        } else {
            IndexSet::from([format!("{}/", dir.get_path_string())])
        });
    } else if attr_name == "basename" {
        return Ok(IndexSet::from([PathFragment::create(
            rule.get_label().get_name(),
        )
        .get_base_name()
        .to_owned()]));
    }

    let Some(attr_type) = rule.get_attribute_type(attr_name) else {
        return Ok(IndexSet::new());
    };
    let value = || {
        rule.get(attr_name, attr_type)
            .expect("NullPointerException: attribute value")
    };
    // String attributes and lists are easy.
    match attr_type {
        Type::String | Type::StringNoIntern => match value() {
            AttrValue::String(s) => Ok(IndexSet::from([s])),
            other => panic!("ClassCastException: {other:?}"),
        },
        Type::StringList => match value() {
            AttrValue::StringList(l) => Ok(l.into_iter().collect()),
            other => panic!("ClassCastException: {other:?}"),
        },
        // Labels are most often used to change the extension, e.g. %.foo -> %.java, so we
        // return the basename w/o extension.
        Type::Label => match value() {
            AttrValue::Label(label) => Ok(IndexSet::from([remove_extension(label.get_name())])),
            other => panic!("ClassCastException: {other:?}"),
        },
        Type::LabelList => match value() {
            AttrValue::LabelList(labels) => Ok(labels
                .iter()
                .map(|label| remove_extension(label.get_name()))
                .collect()),
            other => panic!("ClassCastException: {other:?}"),
        },
        Type::Output => match value() {
            AttrValue::Label(out) => Ok(IndexSet::from([out.get_name().to_owned()])),
            other => panic!("ClassCastException: {other:?}"),
        },
        Type::OutputList => match value() {
            AttrValue::LabelList(outs) => {
                Ok(outs.iter().map(|l| l.get_name().to_owned()).collect())
            }
            other => panic!("ClassCastException: {other:?}"),
        },
        _ => Err(EvalException::new(format!(
            "For attribute '{attr_name}' in outputs: Attributes of type {attr_type} cannot be \
             used in an outputs substitution template"
        ))),
    }
}

/// Java `UnsafeTemplatesImplicitOutputsFunction.getImplicitOutputs` (the function
/// `fromUnsafeTemplates` creates, used only by the Starlark output map).
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:265 ImplicitOutputsFunction.UnsafeTemplatesImplicitOutputsFunction.getImplicitOutputs
fn unsafe_templates_get_implicit_outputs(
    templates: &[String],
    rule: &dyn AttributeMap,
) -> Result<Vec<String>, EvalException> {
    let mut result = IndexSet::new();
    for template in templates {
        let substitutions =
            substitute_placeholder_into_unsafe_template(template, rule, &|r, a| {
                attribute_values(r.expect("NullPointerException: rule"), a)
            })?;
        if substitutions.is_empty() {
            continue;
        }
        result.extend(substitutions);
    }
    Ok(result.into_iter().collect())
}

// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:509 ImplicitOutputsFunction.substitutePlaceholderIntoUnsafeTemplate
fn substitute_placeholder_into_unsafe_template(
    unsafe_template: &str,
    rule: &dyn AttributeMap,
    attribute_getter: &AttributeValueGetter<'_>,
) -> Result<Vec<String>, EvalException> {
    // Parse the template to get the attribute names and format string.
    let parsed_template = ParsedTemplate::parse(unsafe_template);

    // Make sure all attributes are valid.
    for placeholder in &parsed_template.attribute_names {
        if rule.is_configurable(placeholder) {
            return Err(EvalException::new(format!(
                "Attribute {placeholder} is configurable and cannot be used in outputs"
            )));
        }
    }

    // Return the substituted strings.
    parsed_template.substitute_attributes(Some(rule), attribute_getter)
}

/// A template parsed into a format string and its placeholders (Java
/// `ImplicitOutputsFunction.ParsedTemplate`).
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:471 ImplicitOutputsFunction.ParsedTemplate
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTemplate {
    pub template: String,
    pub format_str: String,
    pub attribute_names: Vec<String>,
}

impl ParsedTemplate {
    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:478 ImplicitOutputsFunction.ParsedTemplate.parse
    pub fn parse(raw_template: &str) -> ParsedTemplate {
        let mut placeholders = Vec::new();
        let format_str = ImplicitOutputsFunction::create_placeholder_substitution_format_string(
            raw_template,
            &mut placeholders,
        );
        ParsedTemplate {
            template: raw_template.to_owned(),
            format_str,
            attribute_names: placeholders,
        }
    }

    // Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:487 ImplicitOutputsFunction.ParsedTemplate.substituteAttributes
    pub fn substitute_attributes(
        &self,
        attribute_map: Option<&dyn AttributeMap>,
        attribute_getter: &AttributeValueGetter<'_>,
    ) -> Result<Vec<String>, EvalException> {
        if self.attribute_names.is_empty() {
            return Ok(vec![self.template.clone()]);
        }

        let mut values: Vec<Vec<String>> = Vec::with_capacity(self.attribute_names.len());
        for placeholder in &self.attribute_names {
            let attr_values = attribute_getter(attribute_map, placeholder)?;
            if attr_values.is_empty() {
                return Ok(Vec::new());
            }
            values.push(attr_values.into_iter().collect());
        }
        // Java: Sets.cartesianProduct (lexicographic, first set most significant).
        let mut out = Vec::new();
        let mut indices = vec![0usize; values.len()];
        loop {
            let combination: Vec<&String> =
                indices.iter().zip(&values).map(|(&i, v)| &v[i]).collect();
            out.push(format_with_args(&self.format_str, &combination));
            // Advance the odometer from the last position.
            let mut pos = values.len();
            loop {
                if pos == 0 {
                    return Ok(out);
                }
                pos -= 1;
                indices[pos] += 1;
                if indices[pos] < values[pos].len() {
                    break;
                }
                indices[pos] = 0;
            }
        }
    }
}

/// Java `StarlarkImplicitOutputsFunctionWithCallback.calculateOutputs`: calls the callback with
/// the values of the rule's non-configurable attributes (by Starlark name), then expands each
/// returned template.
// Java: com/google/devtools/build/lib/packages/ImplicitOutputsFunction.java:87 ImplicitOutputsFunction.StarlarkImplicitOutputsFunctionWithCallback.calculateOutputs
fn starlark_callback_outputs(
    callback: &StarlarkCallable,
    semantics: Option<&Arc<blaze_starlark::StarlarkSemantics>>,
    event_handler: &dyn EventHandler,
    map: &dyn AttributeMap,
) -> Result<Vec<(String, String)>, EvalException> {
    use crate::attribute::Attribute;
    let Some(semantics) = semantics else {
        blaze_util::unported!(
            "StarlarkImplicitOutputsFunctionWithCallback without semantics (set by rule())"
        )
    };
    // Java: HashMap<String, Object> attrValues; configurable attributes are left out (we don't
    // know which value they might take since we don't yet have a build configuration).
    let mut attr_values: Vec<(String, Option<AttrValue>)> = Vec::new();
    for attr_name in map.get_attribute_names() {
        let Some(attr_type) = map.get_attribute_type(&attr_name) else {
            continue;
        };
        if !map.is_configurable(&attr_name) {
            let value = map.get(&attr_name, attr_type);
            attr_values.push((Attribute::get_starlark_name(&attr_name), value));
        }
    }
    // Java: StarlarkCallbackHelper.buildArgumentList: one positional argument per parameter of
    // the callback, looked up in the `attrs` struct; a missing one is the struct's unknown-field
    // error (StarlarkInfoWithMessage.getErrorMessageForUnknownField: the message, then
    // allAttributesSuffix), thrown as IllegalArgumentException and wrapped in EvalException.
    let names = callback.get_parameter_names();
    for name in &names {
        if !attr_values.iter().any(|(k, _)| k == name) {
            let mut sorted: Vec<&str> = attr_values.iter().map(|(k, _)| k.as_str()).collect();
            sorted.sort();
            return Err(EvalException::new(format!(
                "Attribute '{name}' either doesn't exist or uses a select() (i.e. could have \
                 multiple values)\nAvailable attributes: {}",
                sorted.join(", ")
            )));
        }
    }
    let events = Arc::new(blaze_starlark::thread::CollectingEvents::default());
    let ctx = blaze_starlark::ThreadContext::new(
        blaze_starlark::Phase::Loading,
        semantics.clone(),
        events.clone(),
    );
    let result = blaze_starlark::eval::call_frozen(
        &callback.to_owned_frozen_value(),
        &ctx,
        &blaze_starlark::eval::EvalOptions::default(),
        |module| {
            let heap = module.heap();
            let positional = names
                .iter()
                .map(|n| {
                    let v = attr_values
                        .iter()
                        .find(|(k, _)| k == n)
                        .and_then(|(_, v)| v.as_ref());
                    Attribute::value_to_starlark(v, heap)
                })
                .collect();
            (positional, Vec::new())
        },
        |res, _module, _eval| {
            // Java: Dict.cast(result, String.class, String.class, "implicit outputs function
            // return value").
            Ok(crate::starlark_defs::util::dict_cast_string_string(
                res,
                "implicit outputs function return value",
            )
            .map_err(|e| blaze_starlark::error::bare_message(&e)))
        },
    );
    // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(eventHandler))
    for e in events.take() {
        let location = e
            .location
            .as_ref()
            .map(|l| blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32));
        let event = if e.kind == "DEBUG" {
            blaze_events::Event::debug_at(location, e.message)
        } else {
            blaze_events::Event::warn_at(location, e.message)
        };
        event_handler.handle(&event);
    }
    let entries = match result {
        Ok(Ok(entries)) => entries,
        Ok(Err(message)) => return Err(EvalException::new(message)),
        Err(e) => return Err(EvalException::new(e.message_with_stack())),
    };
    let mut builder: Vec<(String, String)> = Vec::new();
    for (key, value) in entries {
        // Returns empty string only in case of invalid templates
        let substitutions = ImplicitOutputsFunction::from_templates(&[value.as_str()])
            .get_implicit_outputs(event_handler, map)?;
        if substitutions.is_empty() {
            return Err(EvalException::new(format!(
                "For attribute '{key}' in outputs: Invalid placeholder(s) in template"
            )));
        }
        // Java: Iterables.getOnlyElement.
        if substitutions.len() != 1 {
            panic!(
                "IllegalArgumentException: expected one element but was: <{}>",
                substitutions.join(", ")
            );
        }
        // Java: ImmutableMap.Builder.buildOrThrow; the keys are distinct dict keys.
        builder.push((key, substitutions.into_iter().next().unwrap()));
    }
    Ok(builder)
}
