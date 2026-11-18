//! Port of `packages/AspectsList.java` (the loading-phase parts). Aspect classes and
//! parameter extraction belong to analysis; the loading phase keeps each aspect's identity,
//! name, required parameters and attributes.

use blaze_cmdline::EvalException;
use indexmap::IndexMap;

use crate::attribute::Attribute;
use crate::rule_class::RuleClass;
use crate::rule_class::StarlarkCallable;
use crate::types::Type;

/// What `AspectsList` needs to know about a Starlark aspect value (Java
/// `StarlarkDefinedAspect`): the .bzl builtins pass this for `attr.label(aspects = [...])`.
#[derive(Debug, Clone, PartialEq)]
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
}

/// Java `AspectsList.AspectDetails` (its native and Starlark subclasses).
// Java: com/google/devtools/build/lib/packages/AspectsList.java:180 AspectsList.AspectDetails
#[derive(Debug, Clone, PartialEq)]
pub enum AspectDetails {
    /// Java `NativeAspectDetails`: a native aspect class, by name.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:222 AspectsList.NativeAspectDetails
    Native {
        aspect_class_name: String,
        required_by_aspect: Option<String>,
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

    /// Java `addAspect(NativeAspectClass)`.
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:338 AspectsList.Builder.addAspect
    pub fn add_native_aspect(&mut self, aspect_class_name: &str) {
        self.aspects.insert(
            aspect_class_name.to_owned(),
            AspectDetails::Native {
                aspect_class_name: aspect_class_name.to_owned(),
                required_by_aspect: None,
            },
        );
    }

    /// Java `addAspect(StarlarkAspect)` given only the aspect value. Use
    /// [`AspectsListBuilder::add_starlark_aspect_info`].
    // Java: com/google/devtools/build/lib/packages/AspectsList.java:343 AspectsList.Builder.addAspect
    pub fn add_starlark_aspect(
        &mut self,
        starlark_aspect: StarlarkCallable,
    ) -> Result<(), EvalException> {
        let _ = starlark_aspect;
        blaze_util::unported!(
            "AspectsList.Builder.addAspect(StarlarkAspect) without StarlarkAspectInfo"
        )
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
