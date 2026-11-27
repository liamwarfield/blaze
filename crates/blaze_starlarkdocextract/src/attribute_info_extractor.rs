//! Port of `starlarkdocextract/AttributeInfoExtractor.java`.

use blaze_packages::attribute::Attribute;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::stardoc_output::{AttributeInfo, AttributeType};
use starlark::values::Heap;

use crate::extractor_context::ExtractorContext;
use crate::provider_name_group_extractor::build_provider_name_group;

// Java: com/google/devtools/build/lib/starlarkdocextract/AttributeInfoExtractor.java:35 AttributeInfoExtractor.UNREPRESENTABLE_VALUE
pub const UNREPRESENTABLE_VALUE: &str = "<unrepresentable value>";

// Java: com/google/devtools/build/lib/starlarkdocextract/AttributeInfoExtractor.java:37 AttributeInfoExtractor.buildAttributeInfo
pub fn build_attribute_info(context: &ExtractorContext, attribute: &Attribute) -> AttributeInfo {
    let mut builder = AttributeInfo {
        name: attribute.get_public_name(),
        r#type: get_attribute_type(attribute.get_type(), &attribute.get_public_name()) as i32,
        mandatory: attribute.is_mandatory(),
        ..AttributeInfo::default()
    };
    if let Some(doc) = attribute.get_doc() {
        builder.doc_string = doc.to_owned();
    }
    if !attribute.is_configurable() {
        builder.nonconfigurable = true;
    }
    if !attribute.starlark_defined() {
        builder.natively_defined = true;
    }
    for provider_group in attribute.get_required_providers().get_starlark_providers() {
        // TODO(b/290788853): it is meaningless to require a provider on an attribute of a
        // repository rule or of a module extension tag.
        builder
            .provider_name_group
            .push(build_provider_name_group(context, provider_group));
    }

    if !attribute.is_mandatory() {
        builder.default_value = Heap::temp(|heap| {
            // Attribute.valueToStarlark(attribute.getDefaultValueUnchecked()); a computed or
            // late-bound default is no Starlark value (InvalidStarlarkValueException).
            match attribute.get_default_value_unchecked() {
                Some(
                    AttrValue::ComputedDefault(_)
                    | AttrValue::StarlarkComputedDefaultTemplate(_)
                    | AttrValue::LateBound(_),
                ) => UNREPRESENTABLE_VALUE.to_owned(),
                default_value => {
                    let value = Attribute::value_to_starlark(default_value, heap);
                    context.label_renderer.repr_without_label_constructor(value)
                }
            }
        });
    }
    if let Some(allowed_value_set) = attribute
        .get_allowed_values()
        .and_then(|allowed_values| allowed_values.allowed_value_set())
    {
        for value in allowed_value_set.get_allowed_values() {
            let repr = Heap::temp(|heap| {
                let value = Attribute::value_to_starlark(Some(value), heap);
                context.label_renderer.repr_without_label_constructor(value)
            });
            builder.values.push(repr);
        }
    }
    builder
}

/// Adds `implicit_attribute_infos`, followed by documentable attributes from `attributes`.
// Java: com/google/devtools/build/lib/starlarkdocextract/AttributeInfoExtractor.java:81 AttributeInfoExtractor.addDocumentableAttributes
pub fn add_documentable_attributes<'a>(
    context: &ExtractorContext,
    implicit_attribute_infos: &[(&str, AttributeInfo)],
    attributes: impl IntoIterator<Item = &'a Attribute>,
    builder: &mut Vec<AttributeInfo>,
) {
    // Inject implicit attributes first.
    for (_, implicit_attribute_info) in implicit_attribute_infos {
        builder.push(implicit_attribute_info.clone());
    }
    for attribute in attributes {
        if implicit_attribute_infos
            .iter()
            .any(|(name, _)| *name == attribute.get_name())
        {
            continue;
        }
        if (attribute.starlark_defined() || context.extract_natively_defined_attrs)
            && attribute.is_documented()
            && ExtractorContext::is_public_name(&attribute.get_public_name())
        {
            builder.push(build_attribute_info(context, attribute));
        }
    }
}

// Java: com/google/devtools/build/lib/starlarkdocextract/AttributeInfoExtractor.java:102 AttributeInfoExtractor.getAttributeType
pub fn get_attribute_type(type_: Type, attribute_public_name: &str) -> AttributeType {
    match type_ {
        Type::Integer => AttributeType::Int,
        Type::Label | Type::NodepLabel | Type::GenqueryScopeType | Type::DormantLabel => {
            AttributeType::Label
        }
        Type::String | Type::StringNoIntern => {
            if attribute_public_name == "name" {
                AttributeType::Name
            } else {
                AttributeType::String
            }
        }
        // Since STRING_SET is not exposed to Starlark attr API, we can treat it as STRING_LIST.
        Type::StringList | Type::StringSet => AttributeType::StringList,
        Type::IntegerList => AttributeType::IntList,
        Type::LabelList
        | Type::NodepLabelList
        | Type::GenqueryScopeTypeList
        | Type::DormantLabelList => AttributeType::LabelList,
        Type::Boolean => AttributeType::Boolean,
        Type::LabelKeyedStringDict => AttributeType::LabelStringDict,
        Type::StringDict => AttributeType::StringDict,
        Type::StringListDict => AttributeType::StringListDict,
        Type::LabelListDict => AttributeType::LabelListDict,
        Type::LabelDictUnary => AttributeType::LabelDictUnary,
        Type::Output => AttributeType::Output,
        Type::OutputList => AttributeType::OutputList,
        // TODO(https://github.com/bazelbuild/bazel/issues/6420): deprecated, disabled in Bazel
        // by default, broken and with almost no remaining users, so we don't have an
        // AttributeType for it. Until this type is removed, following the example of legacy
        // Stardoc, pretend it's a list of strings.
        Type::License => AttributeType::StringList,
        // Given that the native TRISTATE type is not exposed to Starlark attr API, let's treat
        // it as an integer.
        Type::Tristate => AttributeType::Int,
        #[allow(unreachable_patterns)]
        _ => AttributeType::Unknown,
    }
}
