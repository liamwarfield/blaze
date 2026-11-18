//! The loading-phase aspect helpers query needs: `Attribute.getAspects(rule)` and
//! `AspectDefinition.forEachLabelDepFromAllAttributesOfAspect` / `addAllAttributesOfAspect`.
//!
//! PORT: aspects are not instantiated during loading in Blaze (blaze_packages keeps an
//! attribute's `AspectsList` of `AspectDetails`). A Starlark aspect's definition attributes are
//! its declared attributes with their defaults (Java substitutes the rule's values for public
//! parameter attributes, which are never label-typed in practice); a native aspect's definition
//! needs its Java class and is `unported!`. `AspectDefinition.satisfies` (required providers) is
//! not ported either: callers only ask it for aspects that have label dependencies.

use blaze_cmdline::Label;
use blaze_packages::attribute::Attribute;
use blaze_packages::attribute::dependency_filter::{AttributeInfoProvider, DependencyFilter};
use blaze_packages::rule_class::AspectDetails;
use blaze_packages::types::{LabelClass, Type};

/// Java `Aspect` as an `AttributeInfoProvider`: "All aspect attributes are implicit."
// Java: com/google/devtools/build/lib/packages/Aspect.java:108 Aspect.isAttributeValueExplicitlySpecified
struct AspectInfoProvider;

impl AttributeInfoProvider for AspectInfoProvider {
    fn is_attribute_value_explicitly_specified_for(&self, _attribute: &Attribute) -> bool {
        false
    }
}

/// Java `Attribute.getAspects(rule)`: the aspects attached to `attribute`, in order.
// Java: com/google/devtools/build/lib/packages/Attribute.java:2380 Attribute.getAspects
pub fn get_aspects(attribute: &Attribute) -> &[AspectDetails] {
    attribute.get_aspects_list().get_aspect_details()
}

/// Java `AspectDefinition.forEachLabelDepFromAllAttributesOfAspect(aspect, dependencyFilter,
/// consumer)`: the (attribute, label) dependencies of the aspect's definition attributes.
// Java: com/google/devtools/build/lib/packages/AspectDefinition.java:280 AspectDefinition.forEachLabelDepFromAllAttributesOfAspect
pub fn label_deps_from_all_attributes_of_aspect(
    aspect: &AspectDetails,
    dependency_filter: &DependencyFilter,
) -> Vec<(Attribute, Label)> {
    let attributes = match aspect {
        AspectDetails::Starlark { info, .. } => &info.aspect_attributes,
        AspectDetails::Native {
            aspect_class_name, ..
        } => blaze_util::unported!(
            "AspectDefinition.forEachLabelDepFromAllAttributesOfAspect",
            "definition of native aspect {aspect_class_name}"
        ),
    };
    let mut result = Vec::new();
    for aspect_attribute in attributes {
        if !dependency_filter.test(&AspectInfoProvider, aspect_attribute) {
            continue;
        }
        let type_: Type = aspect_attribute.get_type();
        if type_.get_label_class() != LabelClass::Dependency {
            continue;
        }
        if let Some(default_value) = aspect_attribute.get_default_value(None) {
            type_.visit_labels(
                &mut |label: &Label, _attr: Option<&Attribute>| {
                    result.push((aspect_attribute.clone(), label.clone()));
                },
                &default_value,
                Some(aspect_attribute),
            );
        }
    }
    result
}

/// Java `AspectDefinition.satisfies(aspect, advertisedProviders)`.
// Java: com/google/devtools/build/lib/packages/AspectDefinition.java:233 AspectDefinition.satisfies
pub fn satisfies_unported(aspect: &AspectDetails) -> bool {
    blaze_util::unported!(
        "AspectDefinition.satisfies",
        "required providers of aspect {}",
        aspect.get_name()
    )
}
