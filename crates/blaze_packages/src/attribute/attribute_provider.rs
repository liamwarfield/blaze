//! Port of `packages/AttributeProvider.java`: the attribute definitions of a rule class or
//! macro class, and attribute population for new instances.

use std::collections::HashMap;
use std::fmt;

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use indexmap::IndexSet;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::CannotPrecomputeDefaultsException;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::attribute::RawAttributeMapper;
use crate::attribute::string_util::did_you_mean;
use crate::package::BuildLangTypedAttributeValuesMap;
use crate::package::RuleVisibility;
use crate::package::TargetDefinitionContext;
use crate::rule_class::rule_class::APPLICABLE_METADATA_ATTR;
use crate::rule_class::rule_class::APPLICABLE_METADATA_ATTR_ALT;
use crate::rule_class::rule_class::CONFIG_SETTING_DEPS_ATTRIBUTE;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::LabelConverter;
use crate::types::License;
use crate::types::Type;
use crate::types::build_type::convert_from_build_lang_type;

/// Java `AttributeProvider`.
// Java: com/google/devtools/build/lib/packages/AttributeProvider.java:30 AttributeProvider
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeProvider {
    attribute_index: HashMap<String, usize>,
    attributes: Vec<Attribute>,
    non_configurable_attributes: Option<Vec<String>>,
    /// The name of the rule or macro class that owns these attributes.
    owner: String,
    ignore_licenses: bool,
    /// Java `((Rule) ruleOrMacroInstance).getRuleClassObject().isPackageMetadataRule()`.
    // PORT: the rule class sets this, since a `dyn RuleOrMacroInstance` cannot be downcast.
    package_metadata_rule: bool,
}

impl AttributeProvider {
    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:43 AttributeProvider.<init>
    pub(crate) fn new(
        attributes: Vec<Attribute>,
        attribute_index: HashMap<String, usize>,
        non_configurable_attributes: Option<Vec<String>>,
        owner: String,
        ignore_licenses: bool,
    ) -> Self {
        AttributeProvider {
            attribute_index,
            attributes,
            non_configurable_attributes,
            owner,
            ignore_licenses,
            package_metadata_rule: false,
        }
    }

    /// Records whether the owning rule class is a package metadata rule (see the field).
    pub(crate) fn with_package_metadata_rule(mut self, package_metadata_rule: bool) -> Self {
        self.package_metadata_rule = package_metadata_rule;
        self
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:86 AttributeProvider.ignoreLicenses
    pub fn ignore_licenses(&self) -> bool {
        self.ignore_licenses
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:94 AttributeProvider.hasAttr
    pub fn has_attr(&self, attr_name: &str, type_: Type) -> bool {
        match self.get_attribute_index(attr_name) {
            Some(index) => self.get_attribute(index).get_type() == type_,
            None => false,
        }
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:107 AttributeProvider.getAttributeIndex
    pub fn get_attribute_index(&self, attr_name: &str) -> Option<usize> {
        self.attribute_index.get(attr_name).copied()
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:112 AttributeProvider.getAttribute
    pub fn get_attribute(&self, attr_index: usize) -> &Attribute {
        &self.attributes[attr_index]
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:119 AttributeProvider.getAttributeByName
    pub fn get_attribute_by_name(&self, attr_name: &str) -> &Attribute {
        match self.get_attribute_index(attr_name) {
            Some(i) => &self.attributes[i],
            None => panic!("NullPointerException: Attribute {attr_name} does not exist"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:128 AttributeProvider.getAttributeByNameMaybe
    pub fn get_attribute_by_name_maybe(&self, attr_name: &str) -> Option<&Attribute> {
        self.get_attribute_index(attr_name)
            .map(|i| &self.attributes[i])
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:134 AttributeProvider.getAttributeCount
    pub fn get_attribute_count(&self) -> usize {
        self.attribute_index.len()
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:142 AttributeProvider.getAttributes
    pub fn get_attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:151 AttributeProvider.getNonConfigurableAttributes
    pub fn get_non_configurable_attributes(&self) -> Option<&[String]> {
        self.non_configurable_attributes.as_deref()
    }

    /// Populates the attributes of a new instance from `attribute_values` (converting,
    /// checking and reporting errors), then defaults, then the config-dependencies attribute.
    // PORT: Java's `AttributeValues<T>` interface has one implementation in use
    // (`BuildLangTypedAttributeValuesMap`), so it is taken directly.
    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:162 AttributeProvider.populateRuleAttributeValues
    pub fn populate_rule_attribute_values<'v>(
        &self,
        rule_or_macro_instance: &mut dyn RuleOrMacroInstance,
        target_definition_context: &TargetDefinitionContext,
        attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
        fail_on_unknown_attributes: bool,
        is_starlark: bool,
    ) -> Result<(), CannotPrecomputeDefaultsException> {
        let defined_attr_indices = self.populate_defined_rule_attribute_values(
            rule_or_macro_instance,
            target_definition_context.get_label_converter(),
            attribute_values,
            fail_on_unknown_attributes,
            target_definition_context.get_local_event_handler(),
            target_definition_context.simplify_unconditional_selects_in_rule_attrs(),
        );
        self.populate_default_rule_attribute_values(
            rule_or_macro_instance,
            target_definition_context,
            &defined_attr_indices,
            is_starlark,
        )?;
        // Now that all attributes are bound to values, collect and store configurable attribute
        // keys.
        AttributeProvider::populate_config_dependencies_attribute(rule_or_macro_instance);
        Ok(())
    }

    // PORT: Java's list interner is dropped (memory only).
    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:196 AttributeProvider.populateDefinedRuleAttributeValues
    fn populate_defined_rule_attribute_values<'v>(
        &self,
        rule_or_macro_instance: &mut dyn RuleOrMacroInstance,
        label_converter: &LabelConverter,
        attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
        fail_on_unknown_attributes: bool,
        event_handler: &dyn EventHandler,
        simplify_unconditional_selects: bool,
    ) -> Vec<bool> {
        let mut defined_attr_indices = vec![false; self.attributes.len()];
        for (name, attribute_value) in attribute_values.get_attribute_accessors() {
            let mut attribute_name = name;
            // Ignore all None values.
            if attribute_value.is_none() && !fail_on_unknown_attributes {
                continue;
            }

            // If the user sets "applicable_liceneses", change it to the correct name.
            // TODO(aiuto): In the time frame of Bazel 9, remove this alternate spelling.
            if attribute_name == APPLICABLE_METADATA_ATTR_ALT {
                attribute_name = APPLICABLE_METADATA_ATTR;
            }

            // Check that the attribute's name belongs to a valid attribute for this rule or macro
            // class.
            let Some(attr_index) = self.get_attribute_index(attribute_name) else {
                let documented: Vec<&str> = rule_or_macro_instance
                    .get_attributes()
                    .iter()
                    .filter(|a| a.is_documented())
                    .map(|a| a.get_name())
                    .collect();
                let message = format!(
                    "{}: no such attribute '{}' in '{}' {}{}",
                    rule_or_macro_instance.get_label(),
                    attribute_name,
                    self.owner,
                    if rule_or_macro_instance.is_rule_instance() {
                        "rule"
                    } else {
                        "macro"
                    },
                    did_you_mean(attribute_name, &documented)
                );
                rule_or_macro_instance.report_error(&message, event_handler);
                continue;
            };
            // Ignore all None values (after reporting an error)
            if attribute_value.is_none() {
                continue;
            }

            let attr = self.get_attribute(attr_index);

            if attribute_name == "licenses" && self.ignore_licenses {
                rule_or_macro_instance.set_attribute_value(
                    attr,
                    Some(AttrValue::License(License::no_license())),
                    false,
                );
                defined_attr_indices[attr_index] = true;
                continue;
            }

            // Convert the build-lang value to a native value, if necessary.
            // PORT: valuesAreBuildLanguageTyped() is always true for the one AttributeValues
            // implementation in use.
            let native_attribute_value = match convert_from_build_lang_type(
                &rule_or_macro_instance.get_attribute_provider().to_string(),
                attr,
                attribute_value,
                Some(label_converter),
                simplify_unconditional_selects,
            ) {
                Ok(v) => v,
                Err(e) => {
                    let message = format!(
                        "{}: {}",
                        rule_or_macro_instance.get_label(),
                        e.get_message()
                    );
                    rule_or_macro_instance.report_error(&message, event_handler);
                    continue;
                }
            };
            // Ignore select({"//conditions:default": None}) values for attr types with null
            // default.
            let Some(mut native_attribute_value) = native_attribute_value else {
                continue;
            };

            if attr.get_name() == "visibility" {
                let vis = match &native_attribute_value {
                    AttrValue::LabelList(l) => l.clone(),
                    other => panic!("ClassCastException: visibility value {other:?}"),
                };
                match RuleVisibility::validate_and_simplify(&vis) {
                    Ok(v) => native_attribute_value = AttrValue::LabelList(v),
                    Err(e) => {
                        let message =
                            format!("{} {}", rule_or_macro_instance.get_label(), e.get_message());
                        rule_or_macro_instance.report_error(&message, event_handler);
                    }
                }
            }

            let explicit = attribute_values.is_explicitly_specified(attribute_name);
            rule_or_macro_instance.set_attribute_value(
                attr,
                Some(native_attribute_value),
                explicit,
            );
            AttributeProvider::check_allowed_values(rule_or_macro_instance, attr, event_handler);
            defined_attr_indices[attr_index] = true;
        }
        defined_attr_indices
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:304 AttributeProvider.populateDefaultRuleAttributeValues
    fn populate_default_rule_attribute_values(
        &self,
        rule_or_macro_instance: &mut dyn RuleOrMacroInstance,
        target_definition_context: &TargetDefinitionContext,
        defined_attr_indices: &[bool],
        is_starlark: bool,
    ) -> Result<(), CannotPrecomputeDefaultsException> {
        // Set defaults; ensure that every mandatory attribute has a value. Use the default if
        // none is specified.
        let mut attrs_with_computed_defaults: Vec<&Attribute> = Vec::new();
        let num_attributes = self.get_attribute_count();
        // PORT: Java indexes the BitSet and the attribute list by attrIndex.
        #[allow(clippy::needless_range_loop)]
        for attr_index in 0..num_attributes {
            if defined_attr_indices[attr_index] {
                continue;
            }
            let attr = self.get_attribute(attr_index);
            if attr.is_mandatory() {
                let message = format!(
                    "{}: missing value for mandatory attribute '{}' in '{}' {}",
                    rule_or_macro_instance.get_label(),
                    attr.get_name(),
                    self.owner,
                    if rule_or_macro_instance.is_rule_instance() {
                        "rule"
                    } else {
                        "macro"
                    }
                );
                rule_or_macro_instance.report_error(
                    &message,
                    target_definition_context.get_local_event_handler(),
                );
            }

            // Macros don't have computed defaults or special logic for licenses or
            // distributions.
            if rule_or_macro_instance.is_rule_instance() {
                // We must check both the name and the type of each attribute below in case a
                // Starlark rule defines a licenses or distributions attribute of another type.
                if attr.has_computed_default() {
                    // Note that it is necessary to set all non-computed default values before
                    // calling Attribute#getDefaultValue for computed default attributes.
                    attrs_with_computed_defaults.push(attr);
                } else if attr.is_late_bound() {
                    let value = attr
                        .get_late_bound_default()
                        .map(|l| AttrValue::LateBound(l.clone()));
                    rule_or_macro_instance.set_attribute_value(attr, value, false);
                } else if attr.is_materializing() {
                    let value = AttrValue::Materializer(attr.get_materializer().clone());
                    rule_or_macro_instance.set_attribute_value(attr, Some(value), false);
                } else if attr.get_name() == APPLICABLE_METADATA_ATTR
                    && attr.get_type() == Type::LabelList
                {
                    // The check here is preventing against a corner case where the
                    // license()/package_info() rule can get itself as applicable_metadata. This
                    // breaks the graph because there is now a self-edge.
                    if self.package_metadata_rule {
                        rule_or_macro_instance.set_attribute_value(
                            attr,
                            Some(AttrValue::LabelList(Vec::new())),
                            false,
                        );
                    }
                } else if attr.get_name() == "licenses" && attr.get_type() == Type::License {
                    let value = if self.ignore_licenses {
                        Some(License::no_license())
                    } else {
                        target_definition_context
                            .get_partial_package_args()
                            .license()
                            .cloned()
                    };
                    rule_or_macro_instance.set_attribute_value(
                        attr,
                        value.map(AttrValue::License),
                        false,
                    );
                }
                // Don't store default values, querying materializes them at read time.
            }
        }
        // An instance of the built-in 'test_suite' rule with an undefined or empty 'tests'
        // attribute gets an '$implicit_tests' attribute, whose value is a shared per-package
        // list of all test labels, populated later.
        if self.owner == "test_suite" && !is_starlark {
            let implicit_tests = self.get_attribute_by_name_maybe("$implicit_tests");
            if let Some(implicit_tests) = implicit_tests {
                let tags = {
                    let attribute_mapper =
                        NonconfigurableAttributeMapper::of(&*rule_or_macro_instance);
                    let tests_empty = match attribute_mapper.get("tests", Type::LabelList) {
                        Some(AttrValue::LabelList(l)) => l.is_empty(),
                        other => panic!("ClassCastException: tests value {other:?}"),
                    };
                    if tests_empty {
                        match attribute_mapper.get("tags", Type::StringList) {
                            Some(AttrValue::StringList(t)) => Some(t),
                            other => panic!("ClassCastException: tags value {other:?}"),
                        }
                    } else {
                        None
                    }
                };
                if let Some(tags) = tags {
                    let explicit = true; // so that it appears in query output
                    rule_or_macro_instance.set_implicit_tests_ref(
                        implicit_tests,
                        target_definition_context.get_test_suite_implicit_tests_ref(&tags),
                        explicit,
                    );
                }
            }
        }
        // Set computed default attribute values now that all other (i.e. non-computed) default
        // values have been set. Macros won't hit this because they don't have attrs with
        // computed defaults.
        for attr in attrs_with_computed_defaults {
            // Note that we cannot determine the exact value of the computed default function
            // here because it may depend on other attribute values that are configurable;
            // instead the attribute is set to the computed default (Starlark ones precompute all
            // possible values, reporting errors here).
            let default_value = attr.get_default_value(None);
            let value_to_set = match &default_value {
                Some(AttrValue::StarlarkComputedDefaultTemplate(template)) => Some(
                    AttrValue::ComputedDefault(Arc::new(template.compute_possible_values(
                        attr,
                        &*rule_or_macro_instance,
                        target_definition_context.get_local_event_handler(),
                    )?)),
                ),
                Some(AttrValue::ComputedDefault(computed_default)) => {
                    // Compute all possible values to verify that the ComputedDefault is
                    // well-defined.
                    let _unused = computed_default
                        .get_possible_values(attr.get_type(), &*rule_or_macro_instance);
                    default_value.clone()
                }
                _ => default_value.clone(),
            };
            rule_or_macro_instance.set_attribute_value(attr, value_to_set, false);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:447 AttributeProvider.populateConfigDependenciesAttribute
    fn populate_config_dependencies_attribute(
        rule_or_macro_instance: &mut dyn RuleOrMacroInstance,
    ) {
        let (config_deps_attribute, config_labels) = {
            let attributes = RawAttributeMapper::of(&*rule_or_macro_instance);
            let Some(config_deps_attribute) =
                attributes.get_attribute_definition(CONFIG_SETTING_DEPS_ATTRIBUTE)
            else {
                return;
            };
            let mut config_labels: IndexSet<Label> = IndexSet::new();
            for attr in rule_or_macro_instance
                .get_attribute_provider()
                .get_attributes()
            {
                if let Some(selector_list) = attributes
                    .base()
                    .get_selector_list(attr.get_name(), attr.get_type())
                {
                    config_labels.extend(selector_list.get_key_labels());
                }
            }
            (config_deps_attribute.clone(), config_labels)
        };
        rule_or_macro_instance.set_attribute_value(
            &config_deps_attribute,
            Some(AttrValue::LabelList(config_labels.into_iter().collect())),
            false,
        );
    }

    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:478 AttributeProvider.checkAllowedValues
    fn check_allowed_values(
        rule_or_macro_instance: &dyn RuleOrMacroInstance,
        attribute: &Attribute,
        event_handler: &dyn EventHandler,
    ) {
        if attribute.check_allowed_values() {
            let allowed_values = attribute
                .get_allowed_values()
                .expect("allowed values of a checked attribute");
            let values = AggregatingAttributeMapper::of(rule_or_macro_instance)
                .visit_attribute(attribute.get_name(), attribute.get_type());
            for value in values {
                // PORT: a Java null value fails apply() of a set of non-null values.
                let ok = value.as_ref().is_some_and(|v| allowed_values.apply(v));
                if !ok {
                    let reason = match &value {
                        Some(v) => allowed_values.get_error_reason(v),
                        None => blaze_util::unported!(
                            "AttributeProvider.checkAllowedValues of a null value"
                        ),
                    };
                    let message = format!(
                        "{}: invalid value in '{}' attribute: {}",
                        rule_or_macro_instance.get_label(),
                        attribute.get_name(),
                        reason
                    );
                    rule_or_macro_instance.report_error(&message, event_handler);
                }
            }
        }
    }
}

impl fmt::Display for AttributeProvider {
    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:76 AttributeProvider.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.owner)
    }
}
