//! `info build-language`: the build language definition protobuffer
//! (`BuildLanguageInfoItem`) and `RuleClassUtils.getBuiltinRuleClasses`.

use std::sync::Arc;

use blaze_packages::attribute::Attribute;
use blaze_packages::native_rules::RuleClassProvider;
use blaze_packages::rule_class::RuleClass;
use blaze_packages::rule_class::rule_class::RuleClassNamePredicateType;
use blaze_packages::types::Type;
use blaze_packages::types::attr_value::AttrValue;
use blaze_packages::types::type_::LabelClass;
use blaze_protos::blaze_query::{
    AllowedRuleClassInfo, AttributeDefinition, AttributeValue, BuildLanguage, RuleDefinition,
    allowed_rule_class_info::AllowedRuleClasses, attribute_value::DictEntry,
};
use blaze_sky_loading::starlark_builtins::StarlarkBuiltinsValue;
use blaze_util::string_encoding::internal_to_unicode;
use prost::Message;
use starlark::values::{FrozenValue, Value, ValueLike};

/// The rule class of a `RuleFunction` value (native rule functions and the value `rule()`
/// returns), or `None` for any other value (Java `instanceof RuleFunction`).
fn rule_function_rule_class(value: Value) -> Option<Arc<RuleClass>> {
    if let Some(state) =
        blaze_packages::starlark_defs::rule_class_functions::rule_function_state(value)
    {
        // RuleFunction.getRuleClass (StarlarkRuleFunction.getRuleClass).
        return Some(
            state
                .rule_class()
                .cloned()
                .expect("IllegalStateException: rule class not exported"),
        );
    }
    value
        .downcast_ref::<blaze_packages::native_rules::builtin_rule_function::BuiltinRuleFunctionValue>()
        .map(|f| f.get_rule_class())
}

/// `entry.getValue() instanceof StarlarkFunction || entry.getValue() instanceof
/// MacroFunctionApi`.
fn is_starlark_function_or_macro(value: Value) -> bool {
    // A `def` function is starlark-rust's "function" type.
    value.get_type() == "function"
        || value
            .downcast_ref::<blaze_packages::starlark_defs::macro_function::MacroFunctionGen<FrozenValue>>()
            .is_some()
}

/// Returns the sorted list of all builtin rule classes.
///
/// Unlike `RuleClassProvider.getRuleClassMap`, this method returns real Starlark builtins
/// instead of stub overridden native rules.
///
/// `include_macro_wrapped_rules`: if true, include rule classes for rules wrapped in macros.
// PORT: skipped, the private constructor of a static-only class: RuleClassUtils.<init>
// Java: com/google/devtools/build/lib/packages/RuleClassUtils.java:38 RuleClassUtils.getBuiltinRuleClasses
pub fn get_builtin_rule_classes(
    builtins: &StarlarkBuiltinsValue,
    rule_class_provider: &RuleClassProvider,
    include_macro_wrapped_rules: bool,
) -> Vec<Arc<RuleClass>> {
    let native_rule_classes = rule_class_provider.get_rule_class_map();
    // The conditional for selecting whether or not to load symbols from @_builtins is the same
    // as in PackageFunction.compileBuildFile
    let mut rule_classes: Vec<Arc<RuleClass>>;
    if builtins
        .starlark_semantics
        .experimental_builtins_bzl_path
        .is_empty()
    {
        rule_classes = native_rule_classes.values().cloned().collect();
    } else {
        rule_classes = Vec::with_capacity(builtins.predeclared_for_build.len());
        for (key, entry) in &builtins.predeclared_for_build {
            let value = entry.value.value();
            if let Some(rule_class) = rule_function_rule_class(value) {
                rule_classes.push(rule_class);
            } else if is_starlark_function_or_macro(value) && include_macro_wrapped_rules {
                // entry.getValue() is a macro in @_builtins which overrides a native rule and
                // wraps a instantiation of a rule target. We cannot get at that main target's
                // rule class directly, so we attempt heuristics.
                // Note that we do not rely on the StarlarkFunction or MacroFunction object's name
                // because the name under which the macro was defined may not match the name
                // under which @_builtins re-exported it.
                if let Some(exported) = builtins
                    .exported_to_java
                    .get(&format!("{key}_rule_function"))
                {
                    // (RuleFunction) builtins.exportedToJava.get(...)
                    let rule_class = rule_function_rule_class(exported.value.value())
                        .expect("ClassCastException: not a RuleFunction");
                    rule_classes.push(rule_class);
                } else if let Some(native) = native_rule_classes.get(key) {
                    rule_classes.push(native.clone());
                }
            }
        }
    }
    // ImmutableList.sortedCopyOf(Comparator.comparing(RuleClass::getName), ...): a stable sort.
    rule_classes.sort_by(|a, b| a.get_name().cmp(b.get_name()));
    rule_classes
}

/// Returns a byte array containing a proto-buffer describing the build language.
///
/// `rule_classes`: a sorted list of rule classes.
// Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:109 BuildLanguageInfoItem.getBuildLanguageDefinition
pub fn get_build_language_definition(rule_classes: &[Arc<RuleClass>]) -> Vec<u8> {
    let mut result_pb = BuildLanguage::default();
    for rule_class in rule_classes {
        if is_abstract_rule(rule_class) {
            continue;
        }

        let mut rule_pb = RuleDefinition {
            name: rule_class.get_name().to_string(),
            ..Default::default()
        };

        // ImmutableList.sortedCopyOf(Comparator.comparing(Attribute::getName), ...)
        let mut sorted_attribute_definitions: Vec<&Attribute> = rule_class
            .get_attribute_provider()
            .get_attributes()
            .iter()
            .collect();
        sorted_attribute_definitions.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        for attr in sorted_attribute_definitions {
            let t = attr.get_type();
            let mut attr_pb = AttributeDefinition {
                name: attr.get_name().to_string(),
                r#type: blaze_query::compat::proto_utils::get_discriminator_from_type(t) as i32,
                mandatory: Some(attr.is_mandatory()),
                allow_empty: Some(!attr.is_non_empty()),
                allow_single_file: Some(attr.is_single_artifact()),
                configurable: Some(attr.is_configurable()),
                ..Default::default()
            };

            // Encode default value, if simple.
            // PORT: Java compares `v == t.getDefaultValue()` by identity; the type defaults are
            // canonical instances (Boolean.FALSE, cached StarlarkInt 0, empty immutable
            // collections, NO_LICENSE, TriState.AUTO), so equality is used here.
            let v = attr.get_default_value_unchecked();
            let simple = match v {
                None
                | Some(AttrValue::ComputedDefault(_))
                | Some(AttrValue::StarlarkComputedDefaultTemplate(_))
                | Some(AttrValue::LateBound(_)) => false,
                Some(v) => t.get_default_value().as_ref() != Some(v),
            };
            if simple {
                attr_pb.default = Some(convert_attr_value(t, v.expect("checked above")));
            }
            attr_pb.executable = Some(attr.is_executable());
            if t.is_label_type() {
                attr_pb.allowed_rule_classes = Some(get_allowed_rule_classes(rule_classes, attr));
                attr_pb.nodep = Some(t.get_label_class() == LabelClass::NondepReference);
            }
            rule_pb.attribute.push(attr_pb);
        }

        result_pb.rule.push(rule_pb);
    }

    result_pb.encode_to_vec()
}

/// Converts attribute value `v` of type `t` to an `AttributeValue` message.
// Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:157 BuildLanguageInfoItem.convertAttrValue
fn convert_attr_value(t: Type, v: &AttrValue) -> AttributeValue {
    let mut b = AttributeValue::default();
    // `v instanceof Map`: the dictionary values.
    let dict: Option<Vec<(String, AttributeValue)>> = match v {
        AttrValue::StringDict(m) => Some(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        convert_attr_value(Type::String, &AttrValue::String(v.clone())),
                    )
                })
                .collect(),
        ),
        AttrValue::StringListDict(m) => Some(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        convert_attr_value(Type::StringList, &AttrValue::StringList(v.clone())),
                    )
                })
                .collect(),
        ),
        AttrValue::LabelDictUnary(m) => Some(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        convert_attr_value(Type::Label, &AttrValue::Label(v.clone())),
                    )
                })
                .collect(),
        ),
        AttrValue::LabelListDict(m) => Some(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        convert_attr_value(Type::LabelList, &AttrValue::LabelList(v.clone())),
                    )
                })
                .collect(),
        ),
        AttrValue::LabelKeyedStringDict(m) => Some(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        convert_attr_value(Type::String, &AttrValue::String(v.clone())),
                    )
                })
                .collect(),
        ),
        _ => None,
    };
    // `v instanceof List`: the elements, converted with the list's element type.
    let list: Option<Vec<AttrValue>> = match v {
        AttrValue::StringList(l) => Some(l.iter().cloned().map(AttrValue::String).collect()),
        AttrValue::IntList(l) => Some(l.iter().copied().map(AttrValue::Int).collect()),
        AttrValue::LabelList(l) => Some(l.iter().cloned().map(AttrValue::Label).collect()),
        _ => None,
    };
    if let Some(entries) = dict {
        // Type.DictType: the value type is the dict type's.
        for (key, value) in entries {
            b.dict.push(DictEntry { key, value });
        }
    } else if let Some(elements) = list {
        let element_type = t
            .get_list_element_type()
            .unwrap_or_else(|| panic!("ClassCastException: {t:?} is not a list type"));
        for elem in &elements {
            b.list.push(convert_attr_value(element_type, elem));
        }
    } else if t == Type::License {
        // TODO(adonovan): need dual function of parseLicense.
        // Treat as empty list for now.
    } else if t == Type::String {
        let AttrValue::String(s) = v else {
            panic!("ClassCastException: {v:?} is not a String");
        };
        b.string = Some(internal_to_unicode(s).into_owned());
    } else if t == Type::Integer {
        b.int = Some(v.as_int().expect("ClassCastException: not a StarlarkInt"));
    } else if t == Type::Boolean {
        let AttrValue::Bool(x) = v else {
            panic!("ClassCastException: {v:?} is not a Boolean");
        };
        b.bool = Some(*x);
    } else if t == Type::Tristate {
        let AttrValue::TriState(x) = v else {
            panic!("ClassCastException: {v:?} is not a TriState");
        };
        b.int = Some(x.to_int());
    } else if t.is_label_type() {
        // case order matters!
        let AttrValue::Label(l) = v else {
            panic!("ClassCastException: {v:?} is not a Label");
        };
        b.string = Some(internal_to_unicode(&l.to_string()).into_owned());
    } else {
        // No native rule attribute of this type (FilesetEntry?) has a default value.
        panic!("IllegalStateException: unexpected type of attribute default value: {t:?}");
    }
    b
}

// Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:190 BuildLanguageInfoItem.getAllowedRuleClasses
fn get_allowed_rule_classes(
    rule_classes: &[Arc<RuleClass>],
    attr: &Attribute,
) -> AllowedRuleClassInfo {
    let mut info = AllowedRuleClassInfo {
        policy: AllowedRuleClasses::Any as i32,
        ..Default::default()
    };

    // `filter != Predicates.alwaysTrue()`: only the unspecified predicate is `alwaysTrue`.
    if attr.is_strict_label_checking_enabled()
        && attr
            .get_allowed_rule_classes_for_labels()
            .get_predicate_type()
            != RuleClassNamePredicateType::Unspecified
    {
        info.policy = AllowedRuleClasses::Specified as i32;
        for other_class in rule_classes {
            if attr.get_allowed_rule_class_object_predicate(other_class)
                && !is_abstract_rule(other_class)
            {
                info.allowed_rule_class
                    .push(other_class.get_name().to_string());
            }
        }
    }

    info
}

// Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:210 BuildLanguageInfoItem.isAbstractRule
fn is_abstract_rule(c: &RuleClass) -> bool {
    c.get_name().starts_with('$')
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_packages::types::tri_state::TriState;

    #[test]
    fn convert_attr_value_scalars() {
        assert_eq!(
            convert_attr_value(Type::String, &AttrValue::String("x".into())).string,
            Some("x".into())
        );
        assert_eq!(
            convert_attr_value(Type::Integer, &AttrValue::Int(7)).int,
            Some(7)
        );
        assert_eq!(
            convert_attr_value(Type::Boolean, &AttrValue::Bool(true)).bool,
            Some(true)
        );
        assert_eq!(
            convert_attr_value(Type::Tristate, &AttrValue::TriState(TriState::No)).int,
            Some(TriState::No.to_int())
        );
        // LICENSE: treated as an empty value.
        assert_eq!(
            convert_attr_value(
                Type::License,
                &AttrValue::License(blaze_packages::types::license::License::no_license())
            ),
            AttributeValue::default()
        );
    }

    #[test]
    fn convert_attr_value_collections() {
        let list = convert_attr_value(
            Type::StringList,
            &AttrValue::StringList(vec!["a".into(), "b".into()]),
        );
        assert_eq!(
            list.list
                .iter()
                .map(|v| v.string.clone().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let mut m = indexmap::IndexMap::new();
        m.insert("k".to_string(), vec!["v".to_string()]);
        let dict = convert_attr_value(Type::StringListDict, &AttrValue::StringListDict(m));
        assert_eq!(dict.dict.len(), 1);
        assert_eq!(dict.dict[0].key, "k");
        assert_eq!(dict.dict[0].value.list[0].string.as_deref(), Some("v"));
    }
}
