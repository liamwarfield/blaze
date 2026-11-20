//! Port of the `aspect()` builtin (`StarlarkRuleClassFunctions.aspect`) and of
//! `packages/StarlarkDefinedAspect.java` (with the `StarlarkAspect` interface).

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::attr_module::DescriptorRef;
use super::attr_module::build_provider_predicate;
use super::attr_module::get_starlark_provider_identifiers;
use super::attr_module::is_starlark_function;
use super::rule_class_functions::attr_object_to_attributes_list;
use super::rule_class_functions::parse_exec_groups;
use super::rule_class_functions::parse_labels;
use super::rule_class_functions::parse_toolchain_types;
use super::subrule;
use super::util::errorf;
use super::util::eval_exception;
use super::util::label_converter_for_bzl;
use super::util::sequence_cast;
use super::util::sequence_cast_strings;
use super::util::starlark_type;
use super::util::trim_doc_string;
use super::value_cell::PendingValue;
use crate::aspect::ALL_TOOLCHAINS;
use crate::aspect::AspectClass;
use crate::aspect::AspectDefinition;
use crate::aspect::AspectDefinitionBuilder;
use crate::aspect::AspectParameters;
use crate::aspect::AspectPropagationEdgesSupplier;
use crate::aspect::AspectPropagationPredicate;
use crate::aspect::StarlarkAspectClass;
use crate::aspect::aspect_propagation_edges_supplier::create_for_attr_aspects;
use crate::aspect::aspect_propagation_edges_supplier::create_for_toolchains_aspects;
use crate::attribute::AllowedValueSet;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::RawAttributeMapper;
use crate::bzl_context::BzlInitThreadContext;
use crate::rule_class::AspectParametersExtractor;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::StarlarkAspectInfo;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::target::Rule;
use crate::target::TargetData;
use crate::types::AttrValue;
use crate::types::Type;

/// Java `AspectPropagationEdgesSupplier.ALL_TOOLCHAINS` (see [`crate::aspect::ALL_TOOLCHAINS`]).
// Java: com/google/devtools/build/lib/packages/AspectPropagationEdgesSupplier.java:48 AspectPropagationEdgesSupplier.ALL_TOOLCHAINS
pub fn all_toolchains() -> Label {
    ALL_TOOLCHAINS.clone()
}

// Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:79 StarlarkDefinedAspect.TRUE_REPS
const TRUE_REPS: [&str; 5] = ["true", "1", "yes", "t", "y"];

// Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:82 StarlarkDefinedAspect.FALSE_REPS
const FALSE_REPS: [&str; 5] = ["false", "0", "no", "f", "n"];

/// The non-Starlark part of an aspect (Java `StarlarkDefinedAspect`'s fields). Shared by the
/// unfrozen and frozen Starlark values; Starlark objects are held as keep-alive
/// [`StarlarkCallable`] handles (cells filled when the defining module freezes).
pub struct AspectData {
    /// Java `implementation`.
    pub implementation: StarlarkCallable,
    pub documentation: Option<String>,
    /// Supplier of the attributes to which the aspect will propagate.
    pub attribute_aspects: AspectPropagationEdgesSupplier<String>,
    /// Supplier of the toolchains types for which the aspect will propagate to matching
    /// resolved toolchains.
    pub toolchains_aspects: AspectPropagationEdgesSupplier<Label>,
    pub attributes: Vec<Attribute>,
    pub required_providers: Vec<Vec<StarlarkProviderIdentifier>>,
    pub required_aspect_providers: Vec<Vec<StarlarkProviderIdentifier>>,
    pub provides: Vec<StarlarkProviderIdentifier>,
    /// Aspect attributes that are required to be specified by rules propagating this aspect
    /// (Java `paramAttributes`).
    pub param_attributes: IndexSet<String>,
    /// Java `requiredAspects` (`ImmutableSet<StarlarkAspect>`): the data of each required
    /// aspect, deduplicated by aspect identity.
    pub required_aspects: Vec<Arc<AspectData>>,
    pub propagation_predicate: Option<AspectPropagationPredicate>,
    pub fragments: IndexSet<String>,
    pub toolchain_types: IndexSet<ToolchainTypeRequirement>,
    pub apply_to_generating_rules: bool,
    pub exec_compatible_with: IndexSet<Label>,
    pub exec_groups: IndexMap<String, DeclaredExecGroup>,
    /// Java `subrules`: handles to the subrule values.
    pub subrules: Vec<StarlarkCallable>,
    /// Identity of an unexported aspect (Java identity token).
    symbol: u64,
    /// Java `StarlarkAspectClass(extensionKey, exportedName)` once exported.
    exported: OnceLock<(Label, String)>,
    /// The `AspectDefinition` is a function of the aspect's parameters, so we can cache that
    /// (Java `definitionCache`). Parameters of Starlark aspects are combinatorially limited
    /// (only bool, int and enum types).
    definition_cache: Mutex<HashMap<AspectParameters, Arc<AspectDefinition>>>,
}

impl fmt::Debug for AspectData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AspectData")
            .field("exported", &self.exported.get())
            .field("symbol", &self.symbol)
            .field("attribute_aspects", &self.attribute_aspects)
            .field("toolchains_aspects", &self.toolchains_aspects)
            .field("param_attributes", &self.param_attributes)
            .finish_non_exhaustive()
    }
}

impl AspectData {
    /// Java `getName()` / `StarlarkAspectClass.getName()`: `<bzl label>%<exported name>`, `None`
    /// before export.
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:167 StarlarkDefinedAspect.getName
    pub fn get_name(&self) -> Option<String> {
        self.exported
            .get()
            .map(|(label, name)| format!("{label}%{name}"))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:295 StarlarkDefinedAspect.isExported
    pub fn is_exported(&self) -> bool {
        self.exported.get().is_some()
    }

    /// The `(extension label, exported name)` of an exported aspect.
    pub fn exported(&self) -> Option<&(Label, String)> {
        self.exported.get()
    }

    /// Java `getName()` of an exported aspect (`checkState(isExported())`).
    fn exported_name(&self) -> String {
        match self.get_name() {
            Some(n) => n,
            None => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:126 StarlarkDefinedAspect.getImplementation
    pub fn get_implementation(&self) -> StarlarkCallable {
        self.implementation
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:134 StarlarkDefinedAspect.getDocumentation
    pub fn get_documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:139 StarlarkDefinedAspect.getAttributeAspects
    pub fn get_attribute_aspects(&self) -> &AspectPropagationEdgesSupplier<String> {
        &self.attribute_aspects
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:148 StarlarkDefinedAspect.getToolchainsAspects
    pub fn get_toolchains_aspects(&self) -> &AspectPropagationEdgesSupplier<Label> {
        &self.toolchains_aspects
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:152 StarlarkDefinedAspect.getAttributes
    pub fn get_attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:172 StarlarkDefinedAspect.getAspectClass
    pub fn get_aspect_class(&self) -> StarlarkAspectClass {
        blaze_util::check_state!(self.is_exported());
        let (label, name) = self.exported.get().expect("checked above");
        StarlarkAspectClass::new(label.clone(), name)
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:178 StarlarkDefinedAspect.getParamAttributes
    pub fn get_param_attributes(&self) -> &IndexSet<String> {
        &self.param_attributes
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:417 StarlarkDefinedAspect.getToolchainTypes
    pub fn get_toolchain_types(&self) -> &IndexSet<ToolchainTypeRequirement> {
        &self.toolchain_types
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:421 StarlarkDefinedAspect.getRequiredProviders
    pub fn get_required_providers(&self) -> &[Vec<StarlarkProviderIdentifier>] {
        &self.required_providers
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:425 StarlarkDefinedAspect.getRequiredAspects
    pub fn get_required_aspects(&self) -> &[Arc<AspectData>] {
        &self.required_aspects
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:430 StarlarkDefinedAspect.getPropagationPredicate
    pub fn get_propagation_predicate(&self) -> Option<&AspectPropagationPredicate> {
        self.propagation_predicate.as_ref()
    }

    /// Java `subrules`.
    pub fn get_subrules(&self) -> &[StarlarkCallable] {
        &self.subrules
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:210 StarlarkDefinedAspect.getDefinition
    pub fn get_definition(&self, aspect_params: &AspectParameters) -> Arc<AspectDefinition> {
        if let Some(d) = self.definition_cache.lock().get(aspect_params) {
            return d.clone();
        }
        // PORT: not built under the lock; the first stored definition wins so its identity is
        // unique per parameters (Caffeine's LoadingCache semantics).
        let built = self.build_definition(aspect_params);
        self.definition_cache
            .lock()
            .entry(aspect_params.clone())
            .or_insert(built)
            .clone()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:217 StarlarkDefinedAspect.buildDefinition
    fn build_definition(&self, aspect_params: &AspectParameters) -> Arc<AspectDefinition> {
        // Java: `(StarlarkAspectClass) aspectClassOrIdentityToken` (a ClassCastException before
        // export).
        let aspect_class = match self.exported.get() {
            Some((label, name)) => StarlarkAspectClass::new(label.clone(), name),
            None => panic!("ClassCastException: aspect is not exported"),
        };
        let mut builder = AspectDefinitionBuilder::new(AspectClass::Starlark(aspect_class));
        builder.propagate_to_attributes(self.attribute_aspects.clone());
        builder.propagate_to_toolchains_types(self.toolchains_aspects.clone());

        for attribute in &self.attributes {
            let mut attr = attribute.clone(); // Might be reassigned.
            if !aspect_params.get_attribute(attr.get_name()).is_empty() {
                let attr_type = attr.get_type();
                let attr_name = attr.get_name().to_owned();
                let attr_value = aspect_params
                    .get_only_value_of_attribute(&attr_name)
                    .to_owned();
                blaze_util::check_state!(!Attribute::is_implicit_name(&attr_name));
                blaze_util::check_state!(
                    attr_type == Type::String
                        || attr_type == Type::Integer
                        || attr_type == Type::Boolean
                );
                blaze_util::check_argument!(
                    aspect_params.get_attribute(&attr_name).len() == 1,
                    "Aspect {} parameter {} has {} values (must have exactly 1).",
                    self.exported_name(),
                    attr_name,
                    aspect_params.get_attribute(&attr_name).len()
                );

                attr = add_attr_value(&attr, &attr_value);
            }
            builder.add(attr);
        }
        builder.require_starlark_provider_sets(self.required_providers.iter().cloned());
        builder.require_aspects_with_providers(self.required_aspect_providers.iter().cloned());
        builder.advertise_provider(&self.provides);
        let fragments: Vec<String> = self.fragments.iter().cloned().collect();
        builder.requires_configuration_fragments_by_starlark_builtin_name(&fragments);
        builder.add_toolchain_types(self.toolchain_types.iter().cloned());
        builder.apply_to_generating_rules(self.apply_to_generating_rules);
        let mut required_aspects_classes: IndexSet<AspectClass> = IndexSet::new();
        for required_aspect in &self.required_aspects {
            required_aspects_classes
                .insert(AspectClass::Starlark(required_aspect.get_aspect_class()));
        }
        builder.required_aspect_classes(required_aspects_classes);
        builder.propagation_predicate(self.propagation_predicate.clone());
        builder.exec_compatible_with(self.exec_compatible_with.clone());
        builder.exec_groups(self.exec_groups.clone());
        builder.subrules(self.subrules.clone());
        builder.build()
    }

    /// Java `getDefaultParametersExtractor()`: a function extracting the aspect parameter values
    /// from its base rule.
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:300 StarlarkDefinedAspect.getDefaultParametersExtractor
    pub fn get_default_parameters_extractor(self: &Arc<Self>) -> AspectParametersExtractor {
        let this = self.clone();
        AspectParametersExtractor::new(move |rule| Some(this.extract_default_parameters(rule)))
    }

    /// The body of the [`AspectData::get_default_parameters_extractor`] function.
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:302 StarlarkDefinedAspect.getDefaultParametersExtractor
    pub fn extract_default_parameters(&self, rule: &Rule) -> AspectParameters {
        let rule_attrs = RawAttributeMapper::of(rule);
        let mut builder = AspectParameters::builder();
        for aspect_attr in &self.attributes {
            let param = aspect_attr.get_name();
            if Attribute::is_implicit_name(param) || Attribute::is_analysis_dependent(param) {
                // These attributes are the private matters of the aspect
                continue;
            }

            let rule_attr = rule_attrs.get_attribute_definition(param);
            if self.param_attributes.contains(aspect_attr.get_name()) {
                // These are preconditions because if they are false, RuleFunction.call() should
                // already have generated an error.
                blaze_util::check_argument!(
                    rule_attr.is_some(),
                    "Cannot apply aspect {} to {} that does not define attribute '{}'.",
                    self.exported_name(),
                    rule.get_target_kind(),
                    param
                );
                let rule_attr_type = rule_attr.expect("checked above").get_type();
                blaze_util::check_argument!(
                    rule_attr_type == Type::String
                        || rule_attr_type == Type::Integer
                        || rule_attr_type == Type::Boolean,
                    "Cannot apply aspect {} to {} since attribute '{}' is not boolean, integer, \
                     nor string.",
                    self.exported_name(),
                    rule.get_target_kind(),
                    param
                );
            }

            if let Some(rule_attr) = rule_attr
                && rule_attr.get_type() == aspect_attr.get_type()
            {
                // If the attribute has a select() (which aspect attributes don't yet support),
                // the error gets reported in RuleClass.checkAspectAllowedValues.
                if !rule_attrs.is_configurable(param) {
                    // Java: `ruleAttrs.get(param, ruleAttr.getType()).toString()`.
                    let value = rule_attrs
                        .get(param, rule_attr.get_type())
                        .expect("NullPointerException: attribute value");
                    builder.add_attribute(param, &attr_value_to_string(&value));
                }
            }
        }
        builder.build()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:345 StarlarkDefinedAspect.extractTopLevelParameters
    pub fn extract_top_level_parameters(
        &self,
        parameters_values: &IndexMap<String, String>,
    ) -> Result<AspectParameters, EvalException> {
        let mut builder = AspectParameters::builder();
        for aspect_parameter in &self.attributes {
            let parameter_name = aspect_parameter.get_name();
            let parameter_type = aspect_parameter.get_type();

            if Attribute::is_implicit_name(parameter_name)
                || Attribute::is_analysis_dependent(parameter_name)
            {
                // These attributes are the private matters of the aspect
                continue;
            }

            blaze_util::check_argument!(
                parameter_type == Type::String
                    || parameter_type == Type::Integer
                    || parameter_type == Type::Boolean,
                "Aspect {}: Cannot pass value of attribute '{}' of type {}, only 'boolean', \
                 'int' and 'string' attributes are allowed.",
                self.exported_name(),
                parameter_name,
                parameter_type
            );

            // Java: `parametersValues.getOrDefault(parameterName,
            // parameterType.cast(aspectParameter.getDefaultValue(null)).toString())`; the
            // default is evaluated eagerly.
            let default_value = aspect_parameter
                .get_default_value(None)
                .expect("NullPointerException: aspect parameter default");
            let default_string = attr_value_to_string(&default_value);
            let parameter_value = parameters_values
                .get(parameter_name)
                .cloned()
                .unwrap_or(default_string);

            // Validate integer and boolean parameters values
            let casted_parameter_value = if parameter_type == Type::Integer {
                CastedParameter::Int(self.parse_int_parameter(parameter_name, &parameter_value)?)
            } else if parameter_type == Type::Boolean {
                CastedParameter::Bool(
                    self.parse_boolean_parameter(parameter_name, &parameter_value)?,
                )
            } else {
                CastedParameter::String(parameter_value)
            };

            if aspect_parameter.check_allowed_values() {
                let allowed_values = aspect_parameter
                    .get_allowed_values()
                    .expect("allowed values of a checked attribute");
                let value = casted_parameter_value.to_attr_value();
                if !allowed_values.apply(&value) {
                    return Err(EvalException::new(format!(
                        "{}: invalid value in '{}' attribute: {}",
                        self.exported_name(),
                        parameter_name,
                        allowed_values.get_error_reason(&value)
                    )));
                }
            }
            builder.add_attribute(parameter_name, &casted_parameter_value.to_string());
        }
        Ok(builder.build())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:392 StarlarkDefinedAspect.parseIntParameter
    fn parse_int_parameter(&self, name: &str, value: &str) -> Result<i64, EvalException> {
        match starlark_int_parse(value, /* base= */ 0) {
            Ok(i) => Ok(i),
            Err(_) => Err(EvalException::new(format!(
                "{}: expected value of type 'int' for attribute '{}' but got '{}'",
                self.exported_name(),
                name,
                value
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:404 StarlarkDefinedAspect.parseBooleanParameter
    fn parse_boolean_parameter(&self, name: &str, value: &str) -> Result<bool, EvalException> {
        let value = value.to_ascii_lowercase();
        if TRUE_REPS.contains(&value.as_str()) {
            return Ok(true);
        }
        if FALSE_REPS.contains(&value.as_str()) {
            return Ok(false);
        }
        Err(EvalException::new(format!(
            "{}: expected value of type 'bool' for attribute '{}' but got '{}'",
            self.exported_name(),
            name,
            value
        )))
    }
}

/// A top-level parameter value after validation (Java `castedParameterValue`: a String,
/// StarlarkInt or Boolean).
enum CastedParameter {
    String(String),
    Int(i64),
    Bool(bool),
}

impl CastedParameter {
    fn to_attr_value(&self) -> AttrValue {
        match self {
            CastedParameter::String(s) => AttrValue::String(s.clone()),
            CastedParameter::Int(i) => match i32::try_from(*i) {
                Ok(i) => AttrValue::Int(i),
                Err(_) => blaze_util::unported!(
                    "StarlarkDefinedAspect.extractTopLevelParameters",
                    "allowed values check of an integer beyond 32 bits: {}",
                    i
                ),
            },
            CastedParameter::Bool(b) => AttrValue::Bool(*b),
        }
    }
}

impl fmt::Display for CastedParameter {
    // Java: `castedParameterValue.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CastedParameter::String(s) => f.write_str(s),
            CastedParameter::Int(i) => write!(f, "{i}"),
            CastedParameter::Bool(b) => write!(f, "{b}"),
        }
    }
}

/// Java `toString()` of a string, StarlarkInt or Boolean attribute value.
fn attr_value_to_string(value: &AttrValue) -> String {
    match value {
        AttrValue::String(s) => s.clone(),
        AttrValue::Int(i) => i.to_string(),
        AttrValue::Bool(b) => b.to_string(),
        other => blaze_util::unported!(
            "StarlarkDefinedAspect.getDefaultParametersExtractor",
            "toString() of {:?}",
            other
        ),
    }
}

// Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:266 StarlarkDefinedAspect.addAttrValue
pub fn add_attr_value(attr: &Attribute, attr_value: &str) -> Attribute {
    let attr_type = attr.get_type();
    let mut attr_builder = attr.clone_builder();
    let casted_value = if attr_type == Type::Integer {
        let parsed = starlark_int_parse(attr_value, /* base= */ 0).unwrap_or_else(|e| {
            panic!("NumberFormatException: {e}");
        });
        match i32::try_from(parsed) {
            Ok(i) => AttrValue::Int(i),
            Err(_) => blaze_util::unported!(
                "StarlarkDefinedAspect.addAttrValue",
                "integer beyond 32 bits: {}",
                attr_value
            ),
        }
    } else if attr_type == Type::Boolean {
        // Java: Boolean.parseBoolean(attrValue)
        AttrValue::Bool(attr_value.eq_ignore_ascii_case("true"))
    } else {
        AttrValue::String(attr_value.to_owned())
    };
    attr_builder.value(Some(casted_value.clone()));

    if !attr.check_allowed_values() {
        // The aspect attribute can have no allowed values constraint if the aspect is used from
        // command-line. However, AspectDefinition.Builder$add requires the existence of allowed
        // values in all aspects string attributes for both native and starlark aspects.
        // Therefore, allowedValues list is added here with only the current value of the
        // attribute.
        attr_builder
            .allowed_values(Arc::new(AllowedValueSet::new(vec![casted_value])))
            .build_named(attr.get_name())
    } else {
        attr_builder.build_named(attr.get_name())
    }
}

/// Java `StarlarkInt.parse(s, base)` (net/starlark/java/eval/StarlarkInt.java:99).
///
/// PORT: the same port as `blaze_build_config::core_option_converters::starlark_int_parse`,
/// which this crate cannot depend on. Values beyond 64 bits fail loudly.
// Java: net/starlark/java/eval/StarlarkInt.java:99 StarlarkInt.parse
fn starlark_int_parse(s: &str, base: i32) -> Result<i64, String> {
    let string_for_errors = s;
    let mut base = base;
    if s.is_empty() {
        return Err("empty string".to_owned());
    }
    // +/- prefix?
    let mut is_negative = false;
    let mut s = s;
    if let Some(rest) = s.strip_prefix('+') {
        s = rest;
    } else if let Some(rest) = s.strip_prefix('-') {
        s = rest;
        is_negative = true;
    }
    let mut digits = s;
    let mut prefix_stripped = false;
    // 0b 0o 0x prefix?
    let mut chars = s.chars();
    if let (Some('0'), Some(c)) = (chars.next(), chars.next()) {
        let prefix_base = match c {
            'b' | 'B' => 2,
            'o' | 'O' => 8,
            'x' | 'X' => 16,
            _ => 0,
        };
        if prefix_base != 0 && (base == 0 || base == prefix_base) {
            base = prefix_base;
            digits = &s[2..]; // strip prefix
            prefix_stripped = true;
        }
    }
    // No prefix, no base? Use decimal.
    if !prefix_stripped && base == 0 {
        // Don't infer base when input starts with '0' due to octal/decimal ambiguity.
        if s.chars().count() > 1 && s.starts_with('0') {
            return Err(format!(
                "cannot infer base when string begins with a 0: {}",
                blaze_starlark::printer::quote(string_for_errors)
            ));
        }
        base = 10;
    }
    if !(2..=36).contains(&base) {
        return Err(format!("invalid base {base} (want 2 <= base <= 36)"));
    }
    let invalid_literal = || {
        format!(
            "invalid base-{base} literal: {}",
            blaze_starlark::printer::quote(string_for_errors)
        )
    };
    // Do not allow Long.parseLong and new BigInteger to accept another +/- sign.
    if digits.starts_with('+') || digits.starts_with('-') || digits.is_empty() {
        return Err(invalid_literal());
    }
    let mut magnitude: i128 = 0;
    for c in digits.chars() {
        // `Character.digit(c, base)`: the string is internal, so `c` is a Latin-1 char, and no
        // char of U+0080..U+00FF is a Java digit (`²`, U+00B2, is `No`, not `Nd`; the
        // full-width letters Character.digit accepts are above U+00FF).
        let digit = match c.to_digit(base as u32) {
            Some(d) if c.is_ascii() => d,
            _ => return Err(invalid_literal()),
        };
        magnitude = magnitude * i128::from(base) + i128::from(digit);
        if magnitude > i128::from(u64::MAX) {
            blaze_util::unported!(
                "StarlarkInt.parse",
                "integer beyond 64 bits: {}",
                string_for_errors
            );
        }
    }
    let value = if is_negative { -magnitude } else { magnitude };
    if value < i128::from(i64::MIN) || value > i128::from(i64::MAX) {
        blaze_util::unported!(
            "StarlarkInt.parse",
            "integer beyond 64 bits: {}",
            string_for_errors
        );
    }
    Ok(value as i64)
}

/// A Starlark-defined aspect (Java `StarlarkDefinedAspect`).
// Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:48 StarlarkDefinedAspect
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkDefinedAspectGen<V: ValueLifetimeless> {
    implementation: V,
    attr_aspects_function: Option<V>,
    toolchains_aspects_function: Option<V>,
    propagation_predicate: Option<V>,
    required_aspects: Vec<V>,
    subrules: Vec<V>,
    /// Values referred to by handles in `data.attributes` (empty once frozen).
    pending: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    handles: Vec<crate::rule_class::StarlarkCallable>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<AspectData>,
}

starlark_complex_value!(pub StarlarkDefinedAspect);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkDefinedAspectGen<Value<'v>>>
    for StarlarkDefinedAspectGen<FrozenValue>
{
}

fn freeze_opt(v: Option<Value>, freezer: &Freezer) -> FreezeResult<Option<FrozenValue>> {
    match v {
        Some(v) => Ok(Some(v.freeze(freezer)?)),
        None => Ok(None),
    }
}

impl<'v> Freeze for StarlarkDefinedAspect<'v> {
    type Frozen = FrozenStarlarkDefinedAspect;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkDefinedAspect> {
        for (value, handle) in self.pending.into_iter().zip(self.handles) {
            PendingValue { value, handle }.freeze(freezer)?;
        }
        Ok(StarlarkDefinedAspectGen {
            implementation: self.implementation.freeze(freezer)?,
            attr_aspects_function: freeze_opt(self.attr_aspects_function, freezer)?,
            toolchains_aspects_function: freeze_opt(self.toolchains_aspects_function, freezer)?,
            propagation_predicate: freeze_opt(self.propagation_predicate, freezer)?,
            required_aspects: self.required_aspects.freeze(freezer)?,
            subrules: self.subrules.freeze(freezer)?,
            pending: Vec::new(),
            handles: Vec::new(),
            data: self.data,
        })
    }
}

impl<'v, V: ValueLike<'v>> StarlarkDefinedAspectGen<V> {
    pub fn data(&self) -> &Arc<AspectData> {
        &self.data
    }

    /// Java `getImplementation()`.
    pub fn get_implementation(&self) -> Value<'v> {
        self.implementation.to_value()
    }

    /// Java `getRequiredAspects()`.
    pub fn get_required_aspects(&self) -> Vec<Value<'v>> {
        self.required_aspects.iter().map(|v| v.to_value()).collect()
    }

    /// Java `getImplementation()` as a keep-alive handle (see [`AspectData::implementation`]).
    pub fn get_implementation_callable(&self) -> StarlarkCallable {
        self.data.implementation
    }

    /// Java `subrules` as keep-alive handles.
    pub fn get_subrules(&self) -> &[StarlarkCallable] {
        &self.data.subrules
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:167 StarlarkDefinedAspect.getName
    pub fn get_name(&self) -> String {
        self.data.exported_name()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:295 StarlarkDefinedAspect.isExported
    pub fn is_exported(&self) -> bool {
        self.data.is_exported()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:172 StarlarkDefinedAspect.getAspectClass
    pub fn get_aspect_class(&self) -> StarlarkAspectClass {
        self.data.get_aspect_class()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:178 StarlarkDefinedAspect.getParamAttributes
    pub fn get_param_attributes(&self) -> &IndexSet<String> {
        self.data.get_param_attributes()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:152 StarlarkDefinedAspect.getAttributes
    pub fn get_attributes(&self) -> &[Attribute] {
        self.data.get_attributes()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:210 StarlarkDefinedAspect.getDefinition
    pub fn get_definition(&self, aspect_params: &AspectParameters) -> Arc<AspectDefinition> {
        self.data.get_definition(aspect_params)
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:300 StarlarkDefinedAspect.getDefaultParametersExtractor
    pub fn get_default_parameters_extractor(&self) -> AspectParametersExtractor {
        self.data.get_default_parameters_extractor()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:345 StarlarkDefinedAspect.extractTopLevelParameters
    pub fn extract_top_level_parameters(
        &self,
        parameters_values: &IndexMap<String, String>,
    ) -> Result<AspectParameters, EvalException> {
        self.data.extract_top_level_parameters(parameters_values)
    }
}

/// The frozen aspect a value denotes, if it is one (resolving a value-cell handle). For Java
/// `AspectFunction.loadAspectFromBzl`: look the exported name up in the frozen .bzl module and
/// pass the value here (`None` is Java's "is not an aspect" case).
///
/// The reference is `'static` because .bzl heaps are kept in the build arena
/// (`blaze_util::build_arena`), which outlives every evaluator and so every caller (see
/// `StarlarkCallable`; PORT: keep-alive). The reference must not be kept past the evaluation
/// or graph value that holds it.
pub fn as_frozen_aspect(v: FrozenValue) -> Option<&'static FrozenStarlarkDefinedAspect> {
    StarlarkCallable::new(v)
        .value()
        .downcast_ref::<FrozenStarlarkDefinedAspect>()
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkDefinedAspectGen<V> {
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:162 StarlarkDefinedAspect.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<aspect>")
    }
}

#[starlark_value(type = "Aspect")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkDefinedAspectGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:435 StarlarkDefinedAspect.equals
    // PORT: Java compares all fields and the identity token; the identity token is unique per
    // aspect() call (or the exported aspect class), so identity decides.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match aspect_data(other) {
            Some(o) => same_aspect(&self.data, &o),
            None => false,
        })
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.data.exported.get() {
            Some(e) => e.hash(hasher),
            None => self.data.symbol.hash(hasher),
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkDefinedAspect.java:184 StarlarkDefinedAspect.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if self.data.exported.get().is_some() {
            return Ok(());
        }
        let Some(ctx) = BzlInitThreadContext::from_or_null(eval) else {
            return Ok(());
        };
        let label = ctx.bzl_file.clone();
        drop(ctx);
        let _ = self.data.exported.set((label, variable_name.to_owned()));
        Ok(())
    }
}

fn same_aspect(a: &Arc<AspectData>, b: &Arc<AspectData>) -> bool {
    match (a.exported.get(), b.exported.get()) {
        (Some(x), Some(y)) => x == y,
        _ => Arc::ptr_eq(a, b),
    }
}

/// The shared data of an aspect value (frozen or not).
pub fn aspect_data(v: Value) -> Option<Arc<AspectData>> {
    if let Some(a) = StarlarkDefinedAspect::from_value(v) {
        return Some(a.data.clone());
    }
    v.downcast_ref::<FrozenStarlarkDefinedAspect>()
        .map(|a| a.data.clone())
}

/// Java `x instanceof StarlarkAspect`.
// PORT: native aspects exposed to Starlark (StarlarkNativeAspect) are not created by any ported
// builtin.
pub fn is_starlark_aspect(v: Value) -> bool {
    aspect_data(v).is_some()
}

fn required_aspects_of<'v>(v: Value<'v>) -> Vec<Value<'v>> {
    if let Some(a) = StarlarkDefinedAspect::from_value(v) {
        return a.get_required_aspects();
    }
    match v.downcast_ref::<FrozenStarlarkDefinedAspect>() {
        Some(a) => a.required_aspects.iter().map(|v| v.to_value()).collect(),
        None => Vec::new(),
    }
}

/// What `AspectsList` needs to know about aspect value `v` (see A1's [`StarlarkAspectInfo`]), and
/// the values its handles refer to.
pub fn aspect_info_of<'v>(
    v: Value<'v>,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<(StarlarkAspectInfo, Vec<PendingValue<'v>>)> {
    let Some(data) = aspect_data(v) else {
        return Err(errorf(format!(
            "got value of type '{}', want 'Aspect'",
            starlark_type(v)
        )));
    };
    let handle = PendingValue::new(v, eval);
    let mut pending = vec![handle];
    let mut required_aspects = Vec::new();
    for required in required_aspects_of(v) {
        let (info, p) = aspect_info_of(required, eval)?;
        required_aspects.push(info);
        pending.extend(p);
    }
    Ok((
        StarlarkAspectInfo {
            aspect: handle.handle,
            name: data.get_name().unwrap_or_default(),
            exported: data.is_exported(),
            required_aspects,
            required_parameters: data.param_attributes.iter().cloned().collect(),
            aspect_attributes: data.attributes.clone(),
            data,
        },
        pending,
    ))
}

/// The keep-alive handle of an aspect global of a frozen .bzl module (`None` if the value is
/// not an aspect). With [`as_frozen_aspect`] on the handle's value this is Java
/// `AspectFunction.loadAspectFromBzl`'s `instanceof StarlarkDefinedAspect` check.
// PORT: keep-alive (the module's heap is kept alive by BZL_LOAD).
pub fn aspect_handle_of(v: &starlark::values::OwnedFrozenValue) -> Option<StarlarkCallable> {
    let fv = v.value().unpack_frozen()?;
    as_frozen_aspect(fv)?;
    Some(StarlarkCallable::new(fv))
}

/// [`aspect_info_of`] for a frozen aspect value (Java `AspectsList.Builder.addAspect` with a
/// `StarlarkAspect` loaded from a .bzl, e.g. a top-level aspect). `None` if `v` is not an
/// aspect.
pub fn aspect_info_of_frozen(v: FrozenValue) -> Option<StarlarkAspectInfo> {
    let handle = StarlarkCallable::new(v);
    let aspect = as_frozen_aspect(v)?;
    let data = aspect.data.clone();
    let mut required_aspects = Vec::new();
    for required in aspect.required_aspects.iter() {
        required_aspects.push(aspect_info_of_frozen(*required)?);
    }
    Some(StarlarkAspectInfo {
        aspect: handle,
        name: data.get_name().unwrap_or_default(),
        exported: data.is_exported(),
        required_aspects,
        required_parameters: data.param_attributes.iter().cloned().collect(),
        aspect_attributes: data.attributes.clone(),
        data,
    })
}

/// Java `Attribute.getDefaultValue(null)` compared with Java `Objects.equals`.
fn default_is(attribute: &Attribute, v: &AttrValue) -> bool {
    attribute.get_default_value(None).as_ref() == Some(v)
}

#[starlark_module]
pub fn register_aspect(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1323 StarlarkRuleClassFunctions.aspect
    fn aspect<'v>(
        implementation: Value<'v>,
        attr_aspects: Option<Value<'v>>,
        toolchains_aspects: Option<Value<'v>>,
        attrs: Option<Value<'v>>,
        required_providers: Option<Value<'v>>,
        required_aspect_providers: Option<Value<'v>>,
        provides: Option<Value<'v>>,
        requires: Option<Value<'v>>,
        propagation_predicate: Option<Value<'v>>,
        fragments: Option<Value<'v>>,
        host_fragments: Option<Value<'v>>,
        toolchains: Option<Value<'v>>,
        doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] apply_to_generating_rules: bool,
        #[starlark(require = named)] exec_compatible_with: Option<Value<'v>>,
        #[starlark(require = named)] exec_groups: Option<Value<'v>>,
        #[starlark(require = named)] subrules: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkDefinedAspect<'v>> {
        // Java: the interpreter checks `@Param(name = "propagation_predicate", allowedTypes =
        // {StarlarkFunction, NoneType})` before the method body
        // (lib/starlarkbuildapi/StarlarkRuleFunctionsApi.java:913; text:
        // net/starlark/java/eval/BuiltinFunction.java:498 BuiltinFunction.checkParamValue).
        if let Some(raw) = propagation_predicate
            && !raw.is_none()
            && !is_starlark_function(raw)
        {
            return Err(errorf(format!(
                "in call to aspect(), parameter 'propagation_predicate' got value of type '{}', \
                 want 'function or NoneType'",
                starlark_type(raw)
            )));
        }
        let empty = || eval.heap().alloc(Vec::<Value>::new());
        let attr_aspects = attr_aspects.unwrap_or_else(empty);
        let toolchains_aspects = toolchains_aspects.unwrap_or_else(empty);
        let attrs = attrs.unwrap_or_else(|| {
            eval.heap()
                .alloc(starlark::values::dict::AllocDict(std::iter::empty::<(
                    Value,
                    Value,
                )>()))
        });
        let required_providers_arg = required_providers.unwrap_or_else(empty);
        let required_aspect_providers_arg = required_aspect_providers.unwrap_or_else(empty);
        let provides_arg = provides.unwrap_or_else(empty);
        let requires = requires.unwrap_or_else(empty);
        let fragments = fragments.unwrap_or_else(empty);
        let _host_fragments = host_fragments.unwrap_or_else(empty);
        let toolchains = toolchains.unwrap_or_else(empty);
        let raw_exec_compatible_with = exec_compatible_with.unwrap_or_else(empty);
        let subrules_unchecked = subrules.unwrap_or_else(empty);

        for (name, v) in [
            ("attr_aspects", attr_aspects),
            ("toolchains_aspects", toolchains_aspects),
        ] {
            if super::util::sequence_elements(v).is_none() && !is_starlark_function(v) {
                return Err(super::util::param_type_error(
                    "aspect",
                    name,
                    v,
                    "sequence or function",
                ));
            }
        }
        if !is_starlark_function(implementation) {
            return Err(super::util::param_type_error(
                "aspect",
                "implementation",
                implementation,
                "function",
            ));
        }
        let doc = match doc {
            Some(d) if !d.is_none() => match d.unpack_str() {
                Some(s) => Some(s),
                None => {
                    return Err(super::util::param_type_error(
                        "aspect",
                        "doc",
                        d,
                        "string or NoneType",
                    ));
                }
            },
            _ => None,
        };

        // Ensure we're initializing a .bzl file.
        BzlInitThreadContext::from_or_fail(eval, "aspect()").map_err(eval_exception)?;
        let label_converter = label_converter_for_bzl(eval)?;

        let mut descriptors: Vec<(String, DescriptorRef<'v>)> =
            attr_object_to_attributes_list(attrs)?;

        let subrule_values = sequence_cast(
            subrules_unchecked,
            "Subrule",
            |s| subrule::is_subrule(s),
            "subrules",
        )?;
        if !subrule_values.is_empty() {
            let semantics = blaze_starlark::ThreadContext::get(eval)?.semantics.clone();
            if !semantics.experimental_rule_extension_api {
                super::rule_class_functions::fail_if_called_outside_allowlist(
                    eval,
                    super::rule_class_functions::ALLOWLIST_RULE_EXTENSION_API,
                )?;
            }
        }
        let subrule_attributes = subrule::discover_attributes(&subrule_values)?;
        if !subrule_attributes.is_empty() {
            descriptors.extend(subrule_attributes);
        }

        let mut attributes: Vec<Attribute> = Vec::new();
        let mut required_params: IndexSet<String> = IndexSet::new();
        let mut pending: Vec<PendingValue<'v>> = Vec::new();
        for (native_name, descriptor) in &descriptors {
            let mut has_default = descriptor.has_default();
            let attribute = descriptor.build(native_name);
            pending.extend(descriptor.pending_values().iter().copied());

            // "configurable" may only be user-set for symbolic macros, not aspects.
            if attribute.configurable_attr_was_user_set() {
                return Err(errorf(format!(
                    "attribute '{native_name}' has the 'configurable' argument set, which is not \
                     allowed in aspect definitions"
                )));
            }

            if attribute.is_materializing() {
                return Err(errorf(format!(
                    "attribute '{native_name}' has a materializer, which is not allowed on aspects"
                )));
            }

            if attribute.get_type() == Type::DormantLabel
                || attribute.get_type() == Type::DormantLabelList
            {
                return Err(errorf(format!(
                    "attribute '{}' has a dormant label type, which is not allowed on aspects",
                    attribute.get_public_name()
                )));
            }

            if !Attribute::is_implicit_name(native_name)
                && !Attribute::is_analysis_dependent(native_name)
            {
                if attribute.get_type() == Type::String {
                    // isValueSet() is always true for attr.string as default value is "" by
                    // default.
                    has_default = !default_is(&attribute, &AttrValue::String(String::new()));
                } else if attribute.get_type() == Type::Integer {
                    // isValueSet() is always true for attr.int as default value is 0 by default.
                    has_default = !default_is(&attribute, &AttrValue::Int(0));
                } else if attribute.get_type() == Type::Boolean {
                    has_default = !default_is(&attribute, &AttrValue::Bool(false));
                } else {
                    return Err(errorf(format!(
                        "Aspect parameter attribute '{native_name}' must have type 'bool', 'int' \
                         or 'string'."
                    )));
                }

                if has_default && attribute.check_allowed_values() {
                    let allowed = attribute.get_allowed_values().unwrap();
                    let default_val = attribute.get_default_value(None);
                    if let Some(default_val) = &default_val
                        && !allowed.apply(default_val)
                    {
                        return Err(errorf(format!(
                            "Aspect parameter attribute '{native_name}' has a bad default value: \
                             {}",
                            allowed.get_error_reason(default_val)
                        )));
                    }
                }
                if !has_default || attribute.is_mandatory() {
                    required_params.insert(native_name.clone());
                }
            } else if !has_default {
                // Implicit or late bound attribute
                let starlark_name = format!("_{}", &native_name[1..]);
                if attribute.is_late_bound()
                    && !attribute
                        .get_late_bound_default()
                        .is_some_and(|l| l.get_fragment_class().is_some())
                {
                    // Code elsewhere assumes that a late-bound attribute of a Starlark-defined
                    // aspects can exist in Java-land only as a StarlarkLateBoundDefault.
                    return Err(errorf(format!(
                        "Starlark aspect attribute '{starlark_name}' is late-bound but somehow is \
                         not defined in Starlark. This violates an invariant inside of Bazel. \
                         Please file a bug with instructions for reproducing this. Thanks!"
                    )));
                }
                return Err(errorf(format!(
                    "Aspect attribute '{starlark_name}' has no default value."
                )));
            }
            if matches!(
                attribute.get_default_value_unchecked(),
                Some(AttrValue::StarlarkComputedDefaultTemplate(_))
            ) {
                // Attributes specifying dependencies using computed value are currently not
                // supported.
                let starlark_name = format!("_{}", &native_name[1..]);
                return Err(errorf(format!(
                    "Aspect attribute '{starlark_name}' ({}) with computed default value is \
                     unsupported.",
                    attribute.get_type()
                )));
            }
            attributes.push(attribute);
        }

        if apply_to_generating_rules
            && !sequence_cast(
                required_providers_arg,
                "Object",
                |_| true,
                "required_providers",
            )?
            .is_empty()
        {
            return Err(errorf(
                "An aspect cannot simultaneously have required providers and apply to generating \
                 rules.",
            ));
        }

        let mut propagation_predicate_value = None;
        if let Some(raw) = propagation_predicate
            && !raw.is_none()
        {
            if !is_starlark_function(raw) {
                return Err(errorf(format!(
                    "Expected a function in 'propagation_predicate' parameter, got '{}'.",
                    // Java passes the (null) predicate here, whose type is "NoneType".
                    "NoneType"
                )));
            }
            propagation_predicate_value = Some(raw);
        }

        if apply_to_generating_rules && propagation_predicate_value.is_some() {
            return Err(errorf(
                "An aspect cannot simultaneously have a propagation predicate and apply to \
                 generating rules.",
            ));
        }

        let exec_compatible_with = parse_labels(
            raw_exec_compatible_with,
            &label_converter,
            "exec_compatible_with",
        )?;

        let exec_groups = match exec_groups {
            Some(raw) if !raw.is_none() => parse_exec_groups(raw)?,
            _ => IndexMap::new(),
        };

        let mut toolchain_types = parse_toolchain_types(toolchains, &label_converter)?;
        toolchain_types.extend(subrule::discover_toolchains(&subrule_values));

        let semantics = blaze_starlark::ThreadContext::get(eval)?.semantics.clone();
        // Java: AspectPropagationEdgesSupplier.createForAttrAspects / createForToolchainsAspects
        let attr_aspects_function = if is_starlark_function(attr_aspects) {
            Some(attr_aspects)
        } else {
            None
        };
        let attr_aspects_handle = attr_aspects_function.map(|f| {
            let p = PendingValue::new(f, eval);
            pending.push(p);
            p.handle
        });
        let attribute_aspects =
            create_for_attr_aspects(attr_aspects, attr_aspects_handle, &semantics)?;
        let toolchains_aspects_function = if is_starlark_function(toolchains_aspects) {
            Some(toolchains_aspects)
        } else {
            None
        };
        let toolchains_aspects_handle = toolchains_aspects_function.map(|f| {
            let p = PendingValue::new(f, eval);
            pending.push(p);
            p.handle
        });
        let toolchains_aspects_edges = create_for_toolchains_aspects(
            toolchains_aspects,
            toolchains_aspects_handle,
            &semantics,
            &label_converter,
        )?;

        let required_providers = build_provider_predicate(
            &sequence_cast(
                required_providers_arg,
                "Object",
                |_| true,
                "required_providers",
            )?,
            "required_providers",
        )?;
        let required_aspect_providers = build_provider_predicate(
            &sequence_cast(
                required_aspect_providers_arg,
                "Object",
                |_| true,
                "required_aspect_providers",
            )?,
            "required_aspect_providers",
        )?;
        let provides = get_starlark_provider_identifiers(
            &sequence_cast(provides_arg, "Provider", |_| true, "provides")?,
            "provides",
        )?;
        // Java: ImmutableSet.copyOf(Sequence.cast(requiredAspects, StarlarkAspect.class,
        // "requires")).
        let mut required_aspects: Vec<Value<'v>> = Vec::new();
        let mut required_aspects_data: Vec<Arc<AspectData>> = Vec::new();
        for a in sequence_cast(requires, "Aspect", |a| is_starlark_aspect(a), "requires")? {
            let data = aspect_data(a).expect("checked by sequence_cast");
            if !required_aspects_data.iter().any(|d| same_aspect(d, &data)) {
                required_aspects.push(a);
                required_aspects_data.push(data);
            }
        }
        let fragments: IndexSet<String> = sequence_cast_strings(fragments, "fragments")?
            .into_iter()
            .collect();
        let symbol = blaze_starlark::ThreadContext::get(eval)?.next_symbol();

        // Java: `new AspectPropagationPredicate((StarlarkFunction) propagationPredicate,
        // thread.getSemantics())`.
        let propagation_predicate_supplier = propagation_predicate_value.map(|f| {
            let p = PendingValue::new(f, eval);
            pending.push(p);
            AspectPropagationPredicate::new(p.handle, semantics.clone())
        });
        let implementation_handle = PendingValue::new(implementation, eval);
        pending.push(implementation_handle);
        let mut subrule_handles = Vec::new();
        for s in &subrule_values {
            let p = PendingValue::new(*s, eval);
            pending.push(p);
            subrule_handles.push(p.handle);
        }

        Ok(StarlarkDefinedAspectGen {
            implementation,
            attr_aspects_function,
            toolchains_aspects_function,
            propagation_predicate: propagation_predicate_value,
            required_aspects,
            subrules: subrule_values,
            handles: pending.iter().map(|p| p.handle).collect(),
            pending: pending.iter().map(|p| p.value).collect(),
            data: Arc::new(AspectData {
                implementation: implementation_handle.handle,
                documentation: doc.map(trim_doc_string),
                attribute_aspects,
                toolchains_aspects: toolchains_aspects_edges,
                attributes,
                required_providers,
                required_aspect_providers,
                provides,
                param_attributes: required_params,
                required_aspects: required_aspects_data,
                propagation_predicate: propagation_predicate_supplier,
                fragments,
                toolchain_types,
                apply_to_generating_rules,
                exec_compatible_with,
                exec_groups,
                subrules: subrule_handles,
                symbol,
                exported: OnceLock::new(),
                definition_cache: Mutex::new(HashMap::new()),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Not a Java test: StarlarkInt.parse over an internal string. `²` is C2 B2 and the byte FF
    // is U+00FF; neither is a Java digit, so both are invalid literals (not a crash).
    #[test]
    fn starlark_int_parse_rejects_latin1_non_digits() {
        assert_eq!(starlark_int_parse("12", 0), Ok(12));
        for s in ["1\u{c2}\u{b2}", "\u{ff}", "0x\u{c3}\u{a9}"] {
            let e = starlark_int_parse(s, 0).unwrap_err();
            assert!(e.starts_with("invalid base-"), "{s:?}: {e}");
        }
    }
}
