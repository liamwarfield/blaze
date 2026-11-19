//! Port of `analysis/starlark/StarlarkAspectPropagationContext.java`: the `propagation_ctx`
//! value passed to an aspect's `propagation_predicate`, `attr_aspects` and
//! `toolchains_aspects` functions (`starlarkbuildapi/StarlarkAspectPropagationContextApi.java`).
//!
//! The functions run on a transient thread owned by the packages layer
//! (`AspectPropagationPredicate.evaluate`, `AspectPropagationEdgesSupplier.computeList`), so the
//! context is built as Rust data ([`StarlarkAspectPropagationContext`]) and allocated on that
//! thread's heap with [`StarlarkAspectPropagationContext::to_value`]; the conversion of attribute
//! values to Starlark (`StructProvider.STRUCT.create`, `Attribute.valueToStarlark`,
//! `convertAttributeValueForAspectPropagationFunc`) happens there, with the same results Java
//! computes eagerly.

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::aspect::Aspect;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::attribute::Attribute;
use blaze_packages::rule_class::rule_class::RuleClass;
use blaze_packages::target::Rule;
use blaze_packages::types::AttrValue;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::starlark_complex_value;
use starlark::starlark_simple_value;
use starlark::values::{
    Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, StarlarkValue, Trace, Value,
    ValueLifetimeless, ValueLike, starlark_value,
};

use super::starlark_attributes_collection::{
    alloc_struct, convert_attribute_value_for_aspect_propagation_func,
};
use crate::dependency_kind::DependencyKind;

/// What a `propagation_ctx` holds, before it is allocated on the evaluating thread's heap
/// (Java `StarlarkAspectPropagationContext(aspectPublicParams, rule)`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:37 StarlarkAspectPropagationContext
#[derive(Debug, Clone)]
pub struct StarlarkAspectPropagationContext {
    /// `createAspectPublicParams(aspect)`: (public name, default value) of each public
    /// parameter, in `aspect.getParameters().getAttributes().keySet()` order.
    aspect_public_params: Vec<(String, Option<AttrValue>)>,
    /// `StarlarkAspectPropagationRule.label`.
    label: Label,
    /// `createQualifiedRuleKind(ruleClass)`: the rule definition environment label (null for
    /// native rules) and the rule class name.
    file_label: Option<Label>,
    rule_name: String,
    /// The fields of `StarlarkAspectPropagationRule.attr`.
    rule_attributes: PropagationRuleAttributes,
}

/// The source of `ctx.rule.attr`.
#[derive(Debug, Clone)]
enum PropagationRuleAttributes {
    /// `createForPropagationPredicate`: only `tags`.
    Tags(Vec<String>),
    /// `createForPropagationEdges`: each rule attribute with its configured value and its
    /// dependency labels (`dependencyLabels.get(AttributeDependencyKind.forRule(attr))`).
    Attributes {
        attributes: Vec<(Attribute, Option<AttrValue>, Vec<Label>)>,
        /// `rule.toString()` for the unknown-field message.
        rule_description: String,
    },
}

impl StarlarkAspectPropagationContext {
    /// Creates a `StarlarkAspectPropagationContext` for the propagation predicate.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:43 StarlarkAspectPropagationContext.createForPropagationPredicate
    pub fn create_for_propagation_predicate(
        aspect: &Aspect,
        label: &Label,
        rule_class: &RuleClass,
        tags: &[String],
    ) -> StarlarkAspectPropagationContext {
        StarlarkAspectPropagationContext {
            aspect_public_params: Self::create_aspect_public_params(aspect),
            label: label.clone(),
            file_label: rule_class.get_rule_definition_environment_label().cloned(),
            rule_name: rule_class.get_name().to_owned(),
            rule_attributes: PropagationRuleAttributes::Tags(tags.to_vec()),
        }
    }

    /// Creates a `StarlarkAspectPropagationContext` for `attr_aspects` and
    /// `toolchains_aspects`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:62 StarlarkAspectPropagationContext.createForPropagationEdges
    pub fn create_for_propagation_edges(
        aspect: &Aspect,
        rule: &Rule,
        attribute_map: &dyn AttributeMap,
        dependency_labels: &IndexMap<DependencyKind, Vec<Label>>,
    ) -> StarlarkAspectPropagationContext {
        let mut attributes = Vec::new();
        for attr in rule
            .get_rule_class_object()
            .get_attribute_provider()
            .get_attributes()
            .iter()
        {
            let labels = dependency_labels
                .get(&DependencyKind::for_rule(std::sync::Arc::new(attr.clone())))
                .cloned()
                .unwrap_or_default();
            let value = attribute_map.get(attr.get_name(), attr.get_type());
            attributes.push((attr.clone(), value, labels));
        }
        StarlarkAspectPropagationContext {
            aspect_public_params: Self::create_aspect_public_params(aspect),
            label: rule.get_label().clone(),
            file_label: rule
                .get_rule_class_object()
                .get_rule_definition_environment_label()
                .cloned(),
            rule_name: rule.get_rule_class_object().get_name().to_owned(),
            rule_attributes: PropagationRuleAttributes::Attributes {
                attributes,
                rule_description: rule.to_string(),
            },
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:88 StarlarkAspectPropagationContext.createAspectPublicParams
    fn create_aspect_public_params(aspect: &Aspect) -> Vec<(String, Option<AttrValue>)> {
        let definition_attributes = aspect.get_definition().get_attributes();
        let mut params: Vec<(String, Option<AttrValue>)> = Vec::new();
        for name in aspect.get_parameters().get_attributes().keys() {
            let attr = definition_attributes
                .get(name)
                .expect("NullPointerException: aspect parameter without a definition attribute");
            let public_name = attr.get_public_name();
            // Java: toImmutableMap rejects duplicate keys.
            if params.iter().any(|(k, _)| *k == public_name) {
                panic!("IllegalArgumentException: Multiple entries with same key: {public_name}");
            }
            params.push((public_name, attr.get_default_value(None)));
        }
        params
    }

    /// Allocates the `propagation_ctx` value on `heap` (the thread that calls the propagation
    /// function).
    pub fn to_value<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        // Java: StructProvider.STRUCT.create(..., " '%s' is not a public parameter of the aspect.")
        let aspect_public_params = alloc_struct(
            heap,
            self.aspect_public_params
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Attribute::value_to_starlark(value.as_ref(), heap),
                    )
                })
                .collect(),
            " '%s' is not a public parameter of the aspect.",
        );
        let rule_attributes = match &self.rule_attributes {
            PropagationRuleAttributes::Tags(tags) => {
                // Java: new RuleAttribute(StarlarkList.immutableCopyOf(tags), false)
                let tags_value = super::util::alloc_immutable_list(
                    heap,
                    tags.iter().map(|t| heap.alloc_str(t).to_value()).collect(),
                );
                alloc_struct(
                    heap,
                    vec![(
                        "tags".to_owned(),
                        heap.alloc(RuleAttributeGen {
                            value: tags_value,
                            is_tool: false,
                        }),
                    )],
                    "Only rule's \"tags\" are available in propagation_predicate function.",
                )
            }
            PropagationRuleAttributes::Attributes {
                attributes,
                rule_description,
            } => {
                let mut fields: Vec<(String, Value<'v>)> = Vec::new();
                for (attr, value, labels) in attributes {
                    let starlark_value = convert_attribute_value_for_aspect_propagation_func(
                        &|| labels.clone(),
                        attr,
                        value.as_ref(),
                        heap,
                    );
                    if let Some(starlark_value) = starlark_value {
                        let public_name = attr.get_public_name();
                        // Java: ImmutableMap.Builder.buildOrThrow rejects duplicate keys.
                        if fields.iter().any(|(k, _)| *k == public_name) {
                            panic!(
                                "IllegalArgumentException: Multiple entries with same key: \
                                 {public_name}"
                            );
                        }
                        fields.push((
                            public_name,
                            heap.alloc(RuleAttributeGen {
                                value: starlark_value,
                                is_tool: attr.is_tool_dependency(),
                            }),
                        ));
                    }
                }
                // Java: "'%s' is not an attribute of target: " + rule (the rule's toString may
                // itself contain '%'; Java formats the whole string).
                alloc_struct(
                    heap,
                    fields,
                    &format!("'%s' is not an attribute of target: {rule_description}"),
                )
            }
        };
        let qualified_kind = heap.alloc(QualifiedRuleKind {
            file_label: self.file_label.clone(),
            rule_name: self.rule_name.clone(),
        });
        let rule = heap.alloc(StarlarkAspectPropagationRuleGen {
            label: heap.alloc(LabelValue(self.label.clone())),
            qualified_rule_kind: qualified_kind,
            attr: rule_attributes,
        });
        heap.alloc(StarlarkAspectPropagationContextValueGen {
            aspect_public_params,
            rule,
        })
    }
}

// -------------------------------------------------------------------------------------------
// Starlark values

/// The `propagation_ctx` value.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:37 StarlarkAspectPropagationContext
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkAspectPropagationContextValueGen<V: ValueLifetimeless> {
    aspect_public_params: V,
    rule: V,
}

starlark_complex_value!(pub StarlarkAspectPropagationContextValue);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkAspectPropagationContextValueGen<Value<'v>>>
    for StarlarkAspectPropagationContextValueGen<FrozenValue>
{
}

impl<'v> Freeze for StarlarkAspectPropagationContextValue<'v> {
    type Frozen = FrozenStarlarkAspectPropagationContextValue;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(StarlarkAspectPropagationContextValueGen {
            aspect_public_params: self.aspect_public_params.freeze(freezer)?,
            rule: self.rule.freeze(freezer)?,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkAspectPropagationContextValueGen<V> {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (no override)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object \
             com.google.devtools.build.lib.analysis.starlark.StarlarkAspectPropagationContext>",
        )
    }
}

#[starlark_value(type = "propagation_ctx")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkAspectPropagationContextValueGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:38 StarlarkAspectPropagationContextApi.getAttr
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:46 StarlarkAspectPropagationContextApi.getRule
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "attr" => Some(self.aspect_public_params.to_value()),
            "rule" => Some(self.rule.to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "attr" | "rule")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["attr".to_owned(), "rule".to_owned()]
    }
}

/// `propagation_ctx.rule` (Java `StarlarkAspectPropagationRule`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:115 StarlarkAspectPropagationContext.StarlarkAspectPropagationRule
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkAspectPropagationRuleGen<V: ValueLifetimeless> {
    label: V,
    qualified_rule_kind: V,
    attr: V,
}

starlark_complex_value!(pub StarlarkAspectPropagationRule);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkAspectPropagationRuleGen<Value<'v>>>
    for StarlarkAspectPropagationRuleGen<FrozenValue>
{
}

impl<'v> Freeze for StarlarkAspectPropagationRule<'v> {
    type Frozen = FrozenStarlarkAspectPropagationRule;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(StarlarkAspectPropagationRuleGen {
            label: self.label.freeze(freezer)?,
            qualified_rule_kind: self.qualified_rule_kind.freeze(freezer)?,
            attr: self.attr.freeze(freezer)?,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkAspectPropagationRuleGen<V> {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (no override)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object \
             com.google.devtools.build.lib.analysis.starlark.StarlarkAspectPropagationContext$StarlarkAspectPropagationRule>",
        )
    }
}

// PORT: the Java record's interface has no @StarlarkBuiltin, so `type()` is the class's simple
// name (net/starlark/java/eval/Starlark.java:337 Starlark.classType).
#[starlark_value(type = "StarlarkAspectPropagationRule")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkAspectPropagationRuleGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:54 StarlarkAspectPropagationRuleApi.getLabel
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:57 StarlarkAspectPropagationRuleApi.getAttr
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:68 StarlarkAspectPropagationRuleApi.getQualifiedKind
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "label" => Some(self.label.to_value()),
            "attr" => Some(self.attr.to_value()),
            "qualified_kind" => Some(self.qualified_rule_kind.to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "label" | "attr" | "qualified_kind")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "attr".to_owned(),
            "label".to_owned(),
            "qualified_kind".to_owned(),
        ]
    }
}

/// `propagation_ctx.rule.qualified_kind` (Java `QualifiedRuleKind`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:135 StarlarkAspectPropagationContext.QualifiedRuleKind
#[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct QualifiedRuleKind {
    #[allocative(skip)]
    file_label: Option<Label>,
    rule_name: String,
}

starlark_simple_value!(QualifiedRuleKind);

impl fmt::Display for QualifiedRuleKind {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (no override)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object \
             com.google.devtools.build.lib.analysis.starlark.StarlarkAspectPropagationContext$QualifiedRuleKind>",
        )
    }
}

// PORT: `type()` is the class's simple name (see StarlarkAspectPropagationRuleGen).
#[starlark_value(type = "QualifiedRuleKind")]
impl<'v> StarlarkValue<'v> for QualifiedRuleKind {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:80 QualifiedRuleKindApi.getFileLabel
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:88 QualifiedRuleKindApi.getRuleName
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // allowReturnNones: null (native rules) is None.
            "file_label" => Some(match &self.file_label {
                Some(l) => heap.alloc(LabelValue(l.clone())),
                None => Value::new_none(),
            }),
            "rule_name" => Some(heap.alloc_str(&self.rule_name).to_value()),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "file_label" | "rule_name")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["file_label".to_owned(), "rule_name".to_owned()]
    }
}

/// A field of `propagation_ctx.rule.attr` (Java `RuleAttribute`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAspectPropagationContext.java:148 StarlarkAspectPropagationContext.RuleAttribute
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct RuleAttributeGen<V: ValueLifetimeless> {
    value: V,
    is_tool: bool,
}

starlark_complex_value!(pub RuleAttribute);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<RuleAttributeGen<Value<'v>>> for RuleAttributeGen<FrozenValue> {}

impl<'v> Freeze for RuleAttribute<'v> {
    type Frozen = FrozenRuleAttribute;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<Self::Frozen> {
        Ok(RuleAttributeGen {
            value: self.value.freeze(freezer)?,
            is_tool: self.is_tool,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Display for RuleAttributeGen<V> {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (no override)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object \
             com.google.devtools.build.lib.analysis.starlark.StarlarkAspectPropagationContext$RuleAttribute>",
        )
    }
}

// PORT: `type()` is the class's simple name (see StarlarkAspectPropagationRuleGen).
#[starlark_value(type = "RuleAttribute")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for RuleAttributeGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:97 RuleAttributeApi.getValue
    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkAspectPropagationContextApi.java:100 RuleAttributeApi.isTool
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "value" => Some(self.value.to_value()),
            "is_tool" => Some(Value::new_bool(self.is_tool)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "value" | "is_tool")
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["is_tool".to_owned(), "value".to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starlark::environment::Module;

    fn tags_rule_attr<'v>(heap: Heap<'v>, tags: &[&str]) -> Value<'v> {
        let tags_value = super::super::util::alloc_immutable_list(
            heap,
            tags.iter().map(|t| heap.alloc_str(t).to_value()).collect(),
        );
        heap.alloc(RuleAttributeGen {
            value: tags_value,
            is_tool: false,
        })
    }

    #[test]
    fn rule_attribute_fields() {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let v = tags_rule_attr(heap, &["a", "b"]);
            assert_eq!(v.get_type(), "RuleAttribute");
            assert_eq!(
                v.get_attr("value", heap).unwrap().unwrap().to_repr(),
                "[\"a\", \"b\"]"
            );
            assert!(!v.get_attr("is_tool", heap).unwrap().unwrap().to_bool());
            assert!(v.get_attr("nope", heap).unwrap().is_none());
        });
    }

    #[test]
    fn qualified_rule_kind_native_rule_has_no_file_label() {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let v = heap.alloc(QualifiedRuleKind {
                file_label: None,
                rule_name: "cc_library".to_owned(),
            });
            assert_eq!(v.get_type(), "QualifiedRuleKind");
            assert!(v.get_attr("file_label", heap).unwrap().unwrap().is_none());
            assert_eq!(
                v.get_attr("rule_name", heap).unwrap().unwrap().unpack_str(),
                Some("cc_library")
            );
        });
    }

    #[test]
    fn propagation_context_freezes() {
        let frozen = Module::with_temp_heap(build_and_freeze);
        let v = frozen.owned_extra_value().unwrap();
        assert_eq!(v.value().get_type(), "propagation_ctx");
    }

    fn build_and_freeze(module: Module) -> starlark::environment::FrozenModule {
        let heap = module.heap();
        let attr = alloc_struct(
            heap,
            Vec::new(),
            " '%s' is not a public parameter of the aspect.",
        );
        let rule = heap.alloc(StarlarkAspectPropagationRuleGen {
            label: heap.alloc(LabelValue(Label::parse_canonical_unchecked("//a:b"))),
            qualified_rule_kind: heap.alloc(QualifiedRuleKind {
                file_label: None,
                rule_name: "r".to_owned(),
            }),
            attr: alloc_struct(heap, Vec::new(), "x"),
        });
        let ctx = heap.alloc(StarlarkAspectPropagationContextValueGen {
            aspect_public_params: attr,
            rule,
        });
        assert_eq!(ctx.get_type(), "propagation_ctx");
        module.set_extra_value(ctx);
        module.freeze().unwrap()
    }
}
