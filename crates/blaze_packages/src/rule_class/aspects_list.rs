//! Port of `packages/AspectsList.java`.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::EvalException;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::aspect::Aspect;
use crate::aspect::AspectClass;
use crate::aspect::AspectParameters;
use crate::aspect::NativeAspectClass;
use crate::attribute::Attribute;
use crate::rule_class::RuleClass;
use crate::rule_class::StarlarkCallable;
use crate::starlark_defs::aspect::AspectData;
use crate::target::Rule;
use crate::types::Type;

/// What `AspectsList` needs to know about a Starlark aspect value (Java
/// `StarlarkDefinedAspect`): the .bzl builtins pass this for `attr.label(aspects = [...])`.
#[derive(Debug, Clone)]
pub struct StarlarkAspectInfo {
    /// The frozen `StarlarkDefinedAspect` value.
    pub aspect: StarlarkCallable,
    /// Java `StarlarkAspect.getName()` (`<bzl label>%<exported name>`).
    pub name: String,
    /// Java `isExported()`.
    pub exported: bool,
    /// Java `getRequiredAspects()`.
    pub required_aspects: Vec<StarlarkAspectInfo>,
    /// Java `StarlarkDefinedAspect.getParamAttributes()` names (public names).
    pub required_parameters: Vec<String>,
    /// Java `getAttributes()` of the aspect definition.
    pub aspect_attributes: Vec<Attribute>,
    /// The aspect's data (shared with the Starlark value): definitions, parameter extraction.
    pub data: Arc<AspectData>,
}

impl PartialEq for StarlarkAspectInfo {
    fn eq(&self, other: &StarlarkAspectInfo) -> bool {
        self.aspect == other.aspect
            && self.name == other.name
            && self.exported == other.exported
            && self.required_aspects == other.required_aspects
            && self.required_parameters == other.required_parameters
            && self.aspect_attributes == other.aspect_attributes
            && Arc::ptr_eq(&self.data, &other.data)
    }
}

/// Java `Function<Rule, AspectParameters>`: extracts an aspect's parameters from the rule it is
/// applied to (`None` is Java's null: the aspect is not applied). Equality is identity.
#[derive(Clone)]
pub struct AspectParametersExtractor(Arc<dyn Fn(&Rule) -> Option<AspectParameters> + Send + Sync>);

impl AspectParametersExtractor {
    pub fn new(f: impl Fn(&Rule) -> Option<AspectParameters> + Send + Sync + 'static) -> Self {
        AspectParametersExtractor(Arc::new(f))
    }

    /// Java `Function.apply`.
    pub fn apply(&self, rule: &Rule) -> Option<AspectParameters> {
        (self.0)(rule)
    }
}

impl PartialEq for AspectParametersExtractor {
    fn eq(&self, other: &AspectParametersExtractor) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for AspectParametersExtractor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AspectParametersExtractor(..)")
    }
}

/// Java `AspectsList.EMPTY_FUNCTION`.
// Java: com/google/devtools/build/lib/packages/AspectsList.java:304 AspectsList.EMPTY_FUNCTION
pub fn empty_function() -> AspectParametersExtractor {
    AspectParametersExtractor::new(|_| Some(AspectParameters::empty()))
}

/// Java `AspectsList.AspectDetails` (its native and Starlark subclasses).
// Java: com/google/devtools/build/lib/packages/AspectsList.java:180 AspectsList.AspectDetails
#[derive(Debug, Clone, PartialEq)]
pub enum AspectDetails {
    /// Java `NativeAspectDetails`: a native aspect class, by name.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:222 AspectsList.NativeAspectDetails
    Native {
        /// Java `aspectClass.getName()`.
        aspect_class_name: String,
        required_by_aspect: Option<String>,
        aspect_class: Arc<dyn NativeAspectClass>,
        parameters_extractor: AspectParametersExtractor,
    },
    /// Java `StarlarkAspectDetails`.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:250 AspectsList.StarlarkAspectDetails
    Starlark {
        info: Box<StarlarkAspectInfo>,
        required_by_aspect: Option<String>,
    },
}

impl AspectDetails {
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:200 AspectsList.AspectDetails.getName
    pub fn get_name(&self) -> &str {
        match self {
            AspectDetails::Native {
                aspect_class_name, ..
            } => aspect_class_name,
            AspectDetails::Starlark { info, .. } => &info.name,
        }
    }

    pub fn required_by_aspect(&self) -> Option<&str> {
        match self {
            AspectDetails::Native {
                required_by_aspect, ..
            }
            | AspectDetails::Starlark {
                required_by_aspect, ..
            } => required_by_aspect.as_deref(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:204 AspectsList.AspectDetails.getRequiredParameters
    pub fn get_required_parameters(&self) -> &[String] {
        match self {
            AspectDetails::Native { .. } => &[],
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:259 AspectsList.StarlarkAspectDetails.getRequiredParameters
            AspectDetails::Starlark { info, .. } => &info.required_parameters,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:217 AspectsList.AspectDetails.getAspectClass
    pub fn get_aspect_class(&self) -> AspectClass {
        match self {
            AspectDetails::Native { aspect_class, .. } => AspectClass::Native(aspect_class.clone()),
            AspectDetails::Starlark { info, .. } => {
                AspectClass::Starlark(info.data.get_aspect_class())
            }
        }
    }

    /// Java `parametersExtractor.apply(rule)`.
    pub fn get_parameters(&self, rule: &Rule) -> Option<AspectParameters> {
        match self {
            AspectDetails::Native {
                parameters_extractor,
                ..
            } => parameters_extractor.apply(rule),
            // Java: StarlarkAspectDetails gets `aspect.getDefaultParametersExtractor()`.
            AspectDetails::Starlark { info, .. } => {
                Some(info.data.extract_default_parameters(rule))
            }
        }
    }

    /// Java `getAspect(Rule)` (`None`: a native extractor returned null).
    pub fn get_aspect(&self, rule: &Rule) -> Option<Aspect> {
        match self {
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:237 AspectsList.NativeAspectDetails.getAspect
            AspectDetails::Native {
                aspect_class,
                parameters_extractor,
                ..
            } => parameters_extractor
                .apply(rule)
                .map(|params| Aspect::for_native(aspect_class, params)),
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:269 AspectsList.StarlarkAspectDetails.getAspect
            AspectDetails::Starlark { info, .. } => {
                let params = info.data.extract_default_parameters(rule);
                Some(Aspect::for_starlark(
                    info.data.get_aspect_class(),
                    info.data.get_definition(&params),
                    params,
                ))
            }
        }
    }

    /// Java `getTopLevelAspect(ImmutableMap<String, String>)`.
    pub fn get_top_level_aspect(
        &self,
        aspect_parameters: &IndexMap<String, String>,
    ) -> Result<Aspect, EvalException> {
        match self {
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:243 AspectsList.NativeAspectDetails.getTopLevelAspect
            AspectDetails::Native { aspect_class, .. } => {
                let _ = aspect_parameters;
                // Native aspects ignore their top-level parameters values for now.
                Ok(Aspect::for_native(aspect_class, AspectParameters::empty()))
            }
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:275 AspectsList.StarlarkAspectDetails.getTopLevelAspect
            AspectDetails::Starlark { info, .. } => {
                let params = info.data.extract_top_level_parameters(aspect_parameters)?;
                Ok(Aspect::for_starlark(
                    info.data.get_aspect_class(),
                    info.data.get_definition(&params),
                    params,
                ))
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:208 AspectsList.AspectDetails.getAspectAttributes
    pub fn get_aspect_attributes(&self) -> &[Attribute] {
        match self {
            AspectDetails::Native { .. } => &[],
            // Java: com/google/devtools/build/lib/packages/AspectsList.java:264 AspectsList.StarlarkAspectDetails.getAspectAttributes
            AspectDetails::Starlark { info, .. } => &info.aspect_attributes,
        }
    }
}

/// Java `AspectsList`.
// Java: com/google/devtools/build/lib/packages/AspectsList.java:38 AspectsList
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AspectsList {
    aspects: Vec<AspectDetails>,
}

impl AspectsList {
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:45 AspectsList.hasAspects
    pub fn has_aspects(&self) -> bool {
        !self.aspects.is_empty()
    }

    pub fn get_aspect_details(&self) -> &[AspectDetails] {
        &self.aspects
    }

    /// Returns the list of aspects required for dependencies through this attribute.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:50 AspectsList.getAspects
    pub fn get_aspects(&self, rule: &Rule) -> Vec<Aspect> {
        let mut builder = Vec::new();
        for aspect in &self.aspects {
            if let Some(a) = aspect.get_aspect(rule) {
                builder.push(a);
            }
        }
        builder
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:67 AspectsList.getAspectClasses
    pub fn get_aspect_classes(&self) -> Vec<AspectClass> {
        self.aspects.iter().map(|a| a.get_aspect_class()).collect()
    }

    /// Returns a list of Aspect objects for top level aspects.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:76 AspectsList.buildAspects
    pub fn build_aspects(
        &self,
        aspects_parameters: &IndexMap<String, String>,
    ) -> Result<Vec<Aspect>, EvalException> {
        let mut aspects_list = Vec::new();
        for aspect in &self.aspects {
            aspects_list.push(aspect.get_top_level_aspect(aspects_parameters)?);
        }
        Ok(aspects_list)
    }

    /// Validates top-level aspects parameters and reports an error if a parameter name is
    /// specified on the command line but no aspect has a parameter with that name, or if a
    /// mandatory aspect attribute is not given a value in the top-level parameters list.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:125 AspectsList.validateTopLevelAspectsParameters
    pub fn validate_top_level_aspects_parameters(
        &self,
        aspects_parameters: &IndexMap<String, String>,
    ) -> Result<(), EvalException> {
        let mut used_parameters: IndexSet<String> = IndexSet::new();
        for aspect_details in &self.aspects {
            if let AspectDetails::Starlark { info, .. } = aspect_details {
                for attr in info.data.get_attributes() {
                    if attr.is_implicit() || attr.is_late_bound() {
                        continue;
                    }
                    let attr_name = attr.get_name();
                    if aspects_parameters.contains_key(attr_name) {
                        used_parameters.insert(attr_name.to_owned());
                    } else if attr.is_mandatory() {
                        return Err(EvalException::new(format!(
                            "Missing mandatory attribute '{}' for aspect '{}'.",
                            attr_name,
                            aspect_details.get_name()
                        )));
                    }
                }
            }
        }
        let unused_parameters: Vec<&str> = aspects_parameters
            .keys()
            .filter(|p| !used_parameters.contains(*p))
            .map(String::as_str)
            .collect();
        if !unused_parameters.is_empty() {
            return Err(EvalException::new(format!(
                "Parameters '[{}]' are not parameters of any of the top-level aspects but they \
                 are specified in --aspects_parameters.",
                unused_parameters.join(", ")
            )));
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:87 AspectsList.validateRulePropagatedAspectsParameters
    pub fn validate_rule_propagated_aspects_parameters(
        &self,
        rule_class: &RuleClass,
    ) -> Result<(), EvalException> {
        for aspect in &self.aspects {
            let required_aspect_parameters = aspect.get_required_parameters();
            for aspect_attribute in aspect.get_aspect_attributes() {
                let aspect_attr_name = aspect_attribute.get_public_name();
                let aspect_attr_type = aspect_attribute.get_type();

                // When propagated from a rule, explicit aspect attributes must be of type
                // boolean, int or string. Integer and string attributes must have the `values`
                // restriction.
                if !aspect_attribute.is_implicit()
                    && !aspect_attribute.is_late_bound()
                    && aspect_attr_type != Type::Boolean
                    && !aspect_attribute.check_allowed_values()
                {
                    return Err(EvalException::new(format!(
                        "Aspect {}: Aspect parameter attribute '{}' must use the 'values' \
                         restriction.",
                        aspect.get_name(),
                        aspect_attr_name
                    )));
                }

                // Required aspect parameters must be specified by the rule propagating the
                // aspect with the same parameter type.
                if required_aspect_parameters.contains(&aspect_attr_name)
                    && !rule_class
                        .get_attribute_provider()
                        .has_attr(&aspect_attr_name, aspect_attr_type)
                {
                    return Err(EvalException::new(format!(
                        "Aspect {} requires rule {} to specify attribute '{}' with type {}.",
                        aspect.get_name(),
                        rule_class.get_name(),
                        aspect_attr_name,
                        aspect_attr_type
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Java `AspectsList.Builder`: aspects by name, in insertion order.
// Java: com/google/devtools/build/lib/packages/AspectsList.java:307 AspectsList.Builder
#[derive(Debug, Clone, Default)]
pub struct AspectsListBuilder {
    aspects: IndexMap<String, AspectDetails>,
}

impl AspectsListBuilder {
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:310 AspectsList.Builder.<init>
    pub fn new() -> Self {
        AspectsListBuilder::default()
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:312 AspectsList.Builder.<init>
    pub fn from_aspects_list(aspects_list: &AspectsList) -> Self {
        let mut b = AspectsListBuilder::new();
        for aspect in &aspects_list.aspects {
            b.aspects
                .insert(aspect.get_name().to_owned(), aspect.clone());
        }
        b
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:318 AspectsList.Builder.build
    pub fn build(&self) -> AspectsList {
        AspectsList {
            aspects: self.aspects.values().cloned().collect(),
        }
    }

    /// Java `addAspect(NativeAspectClass, Function<Rule, AspectParameters>)`: a native aspect
    /// with its parameters extraction function.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:328 AspectsList.Builder.addAspect
    pub fn add_native_aspect_with_extractor(
        &mut self,
        aspect: Arc<dyn NativeAspectClass>,
        evaluator: AspectParametersExtractor,
    ) {
        let native_aspect_details = AspectDetails::Native {
            aspect_class_name: aspect.get_name().to_owned(),
            required_by_aspect: None,
            aspect_class: aspect,
            parameters_extractor: evaluator,
        };
        self.aspects.insert(
            native_aspect_details.get_name().to_owned(),
            native_aspect_details,
        );
    }

    /// Java `addAspect(NativeAspectClass)`: a native aspect that does not need a parameters
    /// extractor.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:338 AspectsList.Builder.addAspect
    pub fn add_native_aspect(&mut self, aspect: Arc<dyn NativeAspectClass>) {
        self.add_native_aspect_with_extractor(aspect, empty_function());
    }

    /// Java `addAspect(StarlarkAspect)` given the aspect value (a frozen `StarlarkDefinedAspect`,
    /// e.g. loaded from a .bzl for a top-level aspect).
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:343 AspectsList.Builder.addAspect
    pub fn add_starlark_aspect(
        &mut self,
        starlark_aspect: StarlarkCallable,
    ) -> Result<(), EvalException> {
        let Some(info) =
            crate::starlark_defs::aspect::aspect_info_of_frozen(starlark_aspect.value())
        else {
            // Java: `throw new IllegalArgumentException()` for an unknown StarlarkAspect.
            panic!("IllegalArgumentException: not a StarlarkDefinedAspect");
        };
        self.add_starlark_aspect_info(&info)
    }

    /// Java `addAspect(StarlarkAspect)`.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:343 AspectsList.Builder.addAspect
    pub fn add_starlark_aspect_info(
        &mut self,
        starlark_aspect: &StarlarkAspectInfo,
    ) -> Result<(), EvalException> {
        self.add_starlark_aspect_required_by(starlark_aspect, None)
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:347 AspectsList.Builder.addAspect
    fn add_starlark_aspect_required_by(
        &mut self,
        starlark_aspect: &StarlarkAspectInfo,
        required_by_aspect: Option<&str>,
    ) -> Result<(), EvalException> {
        if !starlark_aspect.exported {
            return Err(EvalException::new(
                "Aspects should be top-level values in extension files that define them.",
            ));
        }
        for required_aspect in &starlark_aspect.required_aspects {
            self.add_starlark_aspect_required_by(required_aspect, Some(&starlark_aspect.name))?;
        }
        let needs_to_add = self.needs_to_be_added(&starlark_aspect.name, required_by_aspect)?;
        if needs_to_add {
            self.aspects.insert(
                starlark_aspect.name.clone(),
                AspectDetails::Starlark {
                    info: Box::new(starlark_aspect.clone()),
                    required_by_aspect: required_by_aspect.map(str::to_owned),
                },
            );
        }
        Ok(())
    }

    // PORT: Java's `addAspect(Aspect)` / `PredefinedAspectDetails`
    // (AspectsList.java:283, :381) exist only for deserialization, which Blaze does not do.

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:392 AspectsList.Builder.addAspects
    pub fn add_aspects(&mut self, aspects_list: &AspectsList) -> Result<(), EvalException> {
        for aspect in &aspects_list.aspects {
            let needs_to_add =
                self.needs_to_be_added(aspect.get_name(), aspect.required_by_aspect())?;
            if needs_to_add {
                self.aspects
                    .insert(aspect.get_name().to_owned(), aspect.clone());
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/AspectsList.java:401 AspectsList.Builder.needsToBeAdded
    fn needs_to_be_added(
        &self,
        aspect_name: &str,
        required_by_aspect: Option<&str>,
    ) -> Result<bool, EvalException> {
        if let Some(old_aspect) = self.aspects.get(aspect_name) {
            if required_by_aspect.is_some() {
                // If the aspect to be added already exists and it is required by another aspect,
                // no need to add it again.
                return Ok(false);
            }
            // If the aspect to be added is not required by another aspect, then we should throw
            // error
            if let Some(old_aspect_base_aspect_name) = old_aspect.required_by_aspect() {
                return Err(EvalException::new(format!(
                    "aspect {} was added before as a required aspect of aspect {}",
                    old_aspect.get_name(),
                    old_aspect_base_aspect_name
                )));
            }
            return Err(EvalException::new(format!(
                "aspect {} added more than once",
                old_aspect.get_name()
            )));
        }
        Ok(true) // we need to add the new aspect
    }
}
