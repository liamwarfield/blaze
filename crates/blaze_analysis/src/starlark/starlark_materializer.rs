//! The analysis half of `StarlarkAttrModule.StarlarkMaterializer` (the materializer of a
//! dormant-dependency attribute) and its `StarlarkMaterializerContext`. The loading half (the
//! function and its semantics) is `blaze_packages::attribute::computed_default::StarlarkMaterializer`.

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_packages::aspect::aspect_propagation_edges_supplier::{
    eval_exception_message, run_transient_call,
};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::MaterializingDefault;
use blaze_packages::starlark_defs::util::sequence_cast;
use blaze_packages::target::Rule;
use blaze_packages::types::{LabelClass, Type};
use blaze_starlark::EvalError;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::starlark_complex_value;
use starlark::values::{
    Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, StarlarkValue, Trace, Value,
    ValueLifetimeless, ValueLike, starlark_value,
};

use super::starlark_attributes_collection::{alloc_struct, convert_attribute_value};
use crate::configured_target::ConfiguredTarget;
use crate::dormant_dependency::DormantDependency;

/// Resolves an attribute with a materializer: calls the materializer function with the rule's
/// attributes and the prerequisites of its `for_dependency_resolution` attributes, and returns
/// the labels of the dormant dependencies it chose (Java `MaterializingDefault.resolve` with a
/// `StarlarkMaterializer`; for a `LABEL` attribute at most one, `None` being none).
///
/// `prerequisite_map` is Java's `ImmutableMap<String, List<ConfiguredTarget>>` (attribute name
/// to prerequisites).
// Java: com/google/devtools/build/lib/packages/MaterializingDefault.java:80 MaterializingDefault.resolve
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:263 StarlarkAttrModule.StarlarkMaterializer.resolve
pub fn resolve(
    materializing_default: &MaterializingDefault,
    rule: &Rule,
    attributes: &dyn AttributeMap,
    prerequisite_map: &IndexMap<String, Vec<ConfiguredTarget>>,
    event_handler: &dyn EventHandler,
) -> Result<Vec<Label>, EvalError> {
    let materializer = materializing_default.get_resolver();
    let type_ = materializer.type_;
    let semantics = materializer.semantics.clone();
    // First compute the attributes for the materializer by merging the attribute map with the
    // prerequisite map...
    fn context_of<'v>(
        module: &starlark::environment::Module<'v>,
        rule: &Rule,
        attributes: &dyn AttributeMap,
        prerequisite_map: &IndexMap<String, Vec<ConfiguredTarget>>,
        semantics: &blaze_starlark::StarlarkSemantics,
    ) -> Value<'v> {
        compute_attributes_for_materializer(
            module.heap(),
            rule,
            attributes,
            prerequisite_map,
            semantics,
        )
    }
    fn coerce<F: for<'v> Fn(&starlark::environment::Module<'v>) -> Value<'v>>(f: F) -> F {
        f
    }
    let context =
        coerce(|module| context_of(module, rule, attributes, prerequisite_map, &semantics));
    // ...then call the implementation...
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:297 StarlarkAttrModule.StarlarkMaterializer.runMaterializer
    // (a transient thread with a debug print handler and a MaterializationContext, which no
    // builtin reads).
    run_transient_call(
        materializer.implementation,
        &materializer.semantics,
        &context,
        event_handler,
        // ...finally, convert the result to the appropriate type.
        |starlark_result| {
            if type_ == Type::Label {
                if starlark_result.is_none() {
                    return Ok(Vec::new());
                }
                match DormantDependency::of(starlark_result) {
                    Some(d) => Ok(vec![d.get_label().clone()]),
                    None => Err(eval_exception_message(
                        "Expected a single dormant dependency or None",
                    )),
                }
            } else if type_ == Type::LabelList {
                let sequence = sequence_cast(
                    starlark_result,
                    "DormantDependency",
                    |e| DormantDependency::of(e).is_some(),
                    "return value of materializer",
                )
                .map_err(|e| eval_exception_message(&blaze_starlark::error::bare_message(&e)))?;
                Ok(sequence
                    .into_iter()
                    .map(|d| {
                        DormantDependency::of(d)
                            .expect("checked above")
                            .get_label()
                            .clone()
                    })
                    .collect())
            } else {
                panic!("IllegalStateException");
            }
        },
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:234 StarlarkAttrModule.StarlarkMaterializer.computeAttributesForMaterializer
fn compute_attributes_for_materializer<'v>(
    heap: Heap<'v>,
    rule: &Rule,
    attribute_map: &dyn AttributeMap,
    prerequisite_map: &IndexMap<String, Vec<ConfiguredTarget>>,
    semantics: &blaze_starlark::StarlarkSemantics,
) -> Value<'v> {
    // Java's TreeMap: the struct sorts its fields.
    let mut result: Vec<(String, Value<'v>)> = Vec::new();

    for attribute in rule
        .get_rule_class_object()
        .get_attribute_provider()
        .get_attributes()
        .iter()
    {
        if attribute.get_type().get_label_class() == LabelClass::Dependency
            && !attribute.is_for_dependency_resolution()
        {
            continue;
        }

        let value = attribute_map.get(attribute.get_name(), attribute.get_type());
        let starlark_value = convert_attribute_value(
            // (List<ConfiguredTarget>) prerequisiteMap.get(attribute.getName())
            &|| {
                prerequisite_map
                    .get(attribute.get_name())
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!(
                            "NullPointerException: no prerequisites of attribute '{}'",
                            attribute.get_name()
                        )
                    })
            },
            attribute,
            value.as_ref(),
            heap,
            semantics,
        );
        let Some(starlark_value) = starlark_value else {
            continue;
        };

        result.push((attribute.get_public_name(), starlark_value));
    }

    let attrs = alloc_struct(
        heap,
        result,
        "attribute '%s' not available in materializer (it's not an attribute of the rule or it's \
         not marked with 'for_dependency_resolution')",
    );
    heap.alloc(StarlarkMaterializerContextGen {
        label: heap.alloc(LabelValue(rule.get_label().clone())),
        attrs,
    })
}

/// The object available as the `ctx` argument of materializers.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:189 StarlarkAttrModule.StarlarkMaterializerContext
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkMaterializerContextGen<V: ValueLifetimeless> {
    label: V,
    attrs: V,
}

starlark_complex_value!(pub StarlarkMaterializerContext);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkMaterializerContextGen<Value<'v>>>
    for StarlarkMaterializerContextGen<FrozenValue>
{
}

impl<'v> Freeze for StarlarkMaterializerContext<'v> {
    type Frozen = FrozenStarlarkMaterializerContext;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(StarlarkMaterializerContextGen {
            label: self.label.freeze(freezer)?,
            attrs: self.attrs.freeze(freezer)?,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkMaterializerContextGen<V> {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (no override)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object \
             com.google.devtools.build.lib.analysis.starlark.StarlarkAttrModule$StarlarkMaterializerContext>",
        )
    }
}

// Java: a class without @StarlarkBuiltin: Starlark.type is its simple name.
#[starlark_value(type = "StarlarkMaterializerContext")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkMaterializerContextGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:206 StarlarkAttrModule.StarlarkMaterializerContext.getAttr
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:214 StarlarkAttrModule.StarlarkMaterializerContext.getLabel
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "attr" => Some(self.attrs.to_value()),
            "label" => Some(self.label.to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "attr" | "label")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["attr".to_owned(), "label".to_owned()]
    }
}
