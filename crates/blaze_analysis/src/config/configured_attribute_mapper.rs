//! Port of `packages/ConfiguredAttributeMapper.java`: the attribute mapper that resolves
//! `select()` against a configuration's config conditions.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_packages::attribute::{
    AbstractAttributeMapper, Attribute, AttributeInfoProvider, AttributeMap, DependencyFilter,
};
use blaze_packages::package::PackageArgs;
use blaze_packages::target::{Rule, RuleOrMacroInstance};
use blaze_packages::types::{AttrValue, BuildTypeSelector, Type};
use indexmap::{IndexMap, IndexSet};

use crate::config::config_matching_provider::{ConfigMatchingProvider, MatchResult};

/// [`AttributeMap`] implementation that binds a rule's attribute as follows:
///
/// 1. If the attribute is selectable (i.e. its BUILD declaration is of the form
///    `attr = { config1: "value1", "config2: "value2", ... }`, returns the subset of values
///    chosen by the current configuration in accordance with Bazel's documented policy on
///    configurable attribute selection.
/// 2. If the attribute is not selectable (i.e. its value is static), returns that value with no
///    additional processing.
///
/// Example usage: `label = mapper.get("dep", BuildType.LABEL)`.
// Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:52 ConfiguredAttributeMapper
#[derive(Clone)]
pub struct ConfiguredAttributeMapper {
    rule: Arc<Rule>,
    config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
    config_hash: String,
    always_succeed: bool,
}

/// `ValidationException` indicates an error during attribute validation.
// Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:116 ConfiguredAttributeMapper.ValidationException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationException(pub String);

/// Representation of the config key and its value.
// Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:178 ConfiguredAttributeMapper.ConfigKeyAndValue
#[derive(Clone, Debug)]
pub struct ConfigKeyAndValue {
    pub config_key: Label,
    pub value: Option<AttrValue>,
    /// If `None`, this means the default condition (doesn't correspond to a config_setting).
    pub provider: Option<ConfigMatchingProvider>,
}

/// Combines all of the individual select resolution results if there are multiple selects for
/// the same attribute: either the resolved value of the attribute, or an error string.
// Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:204 ConfiguredAttributeMapper.AttributeResolutionResult
#[derive(Clone, Debug)]
pub enum AttributeResolutionResult {
    Success(Option<AttrValue>),
    Failure(String),
}

/// Either the matching (config key, value) of one select, or an error string.
// Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:235 ConfiguredAttributeMapper.SelectResolutionResult
#[derive(Clone, Debug)]
pub enum SelectResolutionResult {
    Success(ConfigKeyAndValue),
    Failure(String),
}

/// Java `Object.equals` on two attribute values.
///
/// PORT: `AttrValue` is plain data without `PartialEq`; its `Debug` form is structural, so equal
/// forms mean equal values.
fn attr_values_equal(a: Option<&AttrValue>, b: Option<&AttrValue>) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// The rule as Java's `AttributeInfoProvider` (Java passes the rule itself).
struct RuleInfoProvider<'a>(&'a Rule);

impl AttributeInfoProvider for RuleInfoProvider<'_> {
    fn is_attribute_value_explicitly_specified_for(&self, attribute: &Attribute) -> bool {
        self.0
            .is_attribute_value_explicitly_specified(attribute.get_name())
    }
}

impl ConfiguredAttributeMapper {
    /// "Manual" constructor that requires the caller to pass the set of configurability
    /// conditions that trigger this rule's configurable attributes.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:61 ConfiguredAttributeMapper.of
    pub fn of_hash(
        rule: Arc<Rule>,
        config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
        config_hash: String,
        always_succeed: bool,
    ) -> ConfiguredAttributeMapper {
        ConfiguredAttributeMapper {
            rule,
            config_conditions,
            config_hash,
            always_succeed,
        }
    }

    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:76 ConfiguredAttributeMapper.of
    pub fn of(
        rule: Arc<Rule>,
        config_conditions: Arc<IndexMap<Label, ConfigMatchingProvider>>,
        configuration: &BuildConfigurationValue,
    ) -> ConfiguredAttributeMapper {
        let always_succeed = configuration
            .get_options()
            .get::<CoreOptions>()
            .expect("CoreOptions")
            .debug_selects_always_succeed;
        Self::of_hash(
            rule,
            config_conditions,
            configuration.short_id(),
            always_succeed,
        )
    }

    pub fn get_rule(&self) -> &Arc<Rule> {
        &self.rule
    }

    pub fn get_config_conditions(&self) -> &Arc<IndexMap<Label, ConfigMatchingProvider>> {
        &self.config_conditions
    }

    fn base(&self) -> AbstractAttributeMapper<'_> {
        AbstractAttributeMapper::new(&*self.rule as &dyn RuleOrMacroInstance)
    }

    /// Checks that all attributes can be mapped to their configured values. This is useful for
    /// checking that the configuration space in a configured attribute doesn't contain
    /// unresolvable contradictions.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:109 ConfiguredAttributeMapper.validateAttributes
    pub fn validate_attributes(&self) -> Result<(), ValidationException> {
        for attr_name in self.get_attribute_names() {
            let type_ = self
                .get_attribute_type(&attr_name)
                .expect("attribute of the rule class");
            self.get_and_validate(&attr_name, type_)?;
        }
        Ok(())
    }

    /// Variation of [`AttributeMap::get`] that returns an informative error if the attribute
    /// can't be resolved due to intrinsic contradictions in the configuration.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:126 ConfiguredAttributeMapper.getAndValidate
    pub fn get_and_validate(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Result<Option<AttrValue>, ValidationException> {
        match self.get_resolved_attribute(attribute_name, type_) {
            AttributeResolutionResult::Failure(f) => Err(ValidationException(f)),
            AttributeResolutionResult::Success(v) => Ok(v),
        }
    }

    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:135 ConfiguredAttributeMapper.getResolvedAttribute
    pub fn get_resolved_attribute_of(&self, attr: &Attribute) -> AttributeResolutionResult {
        self.get_resolved_attribute(attr.get_name(), attr.get_type())
    }

    /// Variation of [`ConfiguredAttributeMapper::get_and_validate`] that does not fail; returns
    /// the resolution result instead.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:145 ConfiguredAttributeMapper.getResolvedAttribute
    pub fn get_resolved_attribute(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> AttributeResolutionResult {
        let Some(selector_list) = self.base().get_selector_list(attribute_name, type_) else {
            // This is a normal attribute.
            return AttributeResolutionResult::Success(self.super_get(attribute_name, type_));
        };

        let mut resolved_list: Vec<Option<AttrValue>> = Vec::new();
        for selector in selector_list.get_selectors() {
            let resolved_path = match self.resolve_selector(attribute_name, selector) {
                SelectResolutionResult::Failure(f) => {
                    return AttributeResolutionResult::Failure(f);
                }
                SelectResolutionResult::Success(s) => s,
            };
            if !selector.is_value_set(&resolved_path.config_key) {
                // Use the default. We don't have access to the rule here, so pass null to
                // Attribute.getValue(). This has the result of making attributes with condition
                // predicates ineligible for "None" values. But no user-facing attributes should
                // do that anyway, so that isn't a loss.
                let attr = self
                    .get_attribute_definition(attribute_name)
                    .expect("attribute definition");
                if attr.is_mandatory() {
                    return AttributeResolutionResult::Failure(format!(
                        "Mandatory attribute '{}' resolved to 'None' after evaluating 'select' expression",
                        attribute_name
                    ));
                }
                let default_value =
                    attr.get_default_value(Some(&*self.rule as &dyn RuleOrMacroInstance));
                resolved_list.push(default_value);
            } else {
                resolved_list.push(resolved_path.value);
            }
        }

        AttributeResolutionResult::Success(if resolved_list.len() == 1 {
            resolved_list.into_iter().next().unwrap()
        } else {
            // PORT: Java's `type.concat` of a list of non-null list values; list types have
            // non-null defaults, so no element is null here.
            type_.concat(resolved_list.into_iter().flatten().collect())
        })
    }

    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:258 ConfiguredAttributeMapper.resolveSelector
    fn resolve_selector(
        &self,
        attribute_name: &str,
        selector: &BuildTypeSelector,
    ) -> SelectResolutionResult {
        // Use a LinkedHashMap to guarantee a deterministic branch selection when multiple
        // branches matches but they resolve to the same value.
        let mut matching_conditions: IndexMap<Label, ConfigKeyAndValue> = IndexMap::new();
        // Use a LinkedHashSet to guarantee deterministic error message ordering. We use a
        // LinkedHashSet vs. a more general SortedSet because the latter supports insertion-order,
        // which should more closely match how users see select() structures in BUILD files.
        let mut condition_labels: IndexSet<Label> = IndexSet::new();

        let mut errors: Vec<String> = Vec::new();
        // Find the matching condition and record its value (checking for duplicates).
        selector.for_each(|selector_key, value| {
            if BuildTypeSelector::is_default_condition_label(selector_key) {
                return;
            }

            let Some(cur_condition) = self.config_conditions.get(selector_key) else {
                // This can happen if the rule is in error
                return;
            };
            condition_labels.insert(selector_key.clone());

            match cur_condition.result() {
                MatchResult::InError(messages) => {
                    // Resolving selects so last chance to actually surface these errors.
                    errors.push(format!(
                        "config_setting {} is unresolvable because: {}",
                        selector_key,
                        messages.join(", ")
                    ));
                    // Defer the throw in order to collect all possible config_setting that are in
                    // error.
                }
                MatchResult::Match => {
                    // We keep track of all matches which are more precise than any we have found
                    // so far. Therefore, we remove any previous matches which are strictly less
                    // precise than this one, and only add this one if none of the previous
                    // matches are more precise. It is an error if we do not end up with only one
                    // most-precise match.
                    let mut suppressed = false;
                    let mut to_remove: Vec<Label> = Vec::new();
                    for (key, existing) in &matching_conditions {
                        let existing_match = existing
                            .provider
                            .as_ref()
                            .expect("matching conditions have providers");
                        if cur_condition.refines(existing_match) {
                            to_remove.push(key.clone());
                        } else if existing_match.refines(cur_condition) {
                            suppressed = true;
                            break;
                        }
                    }
                    // Java removes through the iterator as it goes (`it.remove()`); entries
                    // visited before a `break` are removed either way.
                    for key in to_remove {
                        matching_conditions.shift_remove(&key);
                    }
                    if !suppressed {
                        matching_conditions.insert(
                            selector_key.clone(),
                            ConfigKeyAndValue {
                                config_key: selector_key.clone(),
                                value: value.cloned(),
                                provider: Some(cur_condition.clone()),
                            },
                        );
                    }
                }
                MatchResult::NoMatch(_) => {}
            }
        });
        if !errors.is_empty() {
            return SelectResolutionResult::Failure(format!(
                "Unresolvable config_settings for configurable attribute \"{}\" in {}:\n{}",
                attribute_name,
                self.get_label(),
                errors.join("\n")
            ));
        }

        let mut distinct: Vec<Option<&AttrValue>> = Vec::new();
        for s in matching_conditions.values() {
            if !distinct
                .iter()
                .any(|d| attr_values_equal(*d, s.value.as_ref()))
            {
                distinct.push(s.value.as_ref());
            }
        }
        if distinct.len() > 1 {
            return SelectResolutionResult::Failure(format!(
                "Illegal ambiguous match on configurable attribute \"{}\" in {}:\n{}\nMultiple matches are not allowed unless one is unambiguously more specialized or they resolve to the same value. See https://bazel.build/reference/be/functions#select.",
                attribute_name,
                self.get_label(),
                matching_conditions
                    .keys()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        } else if !matching_conditions.is_empty() {
            return SelectResolutionResult::Success(
                matching_conditions.into_values().next().unwrap(),
            );
        }

        // If nothing matched, choose the default condition.
        if selector.has_default() {
            return SelectResolutionResult::Success(ConfigKeyAndValue {
                config_key: BuildTypeSelector::default_condition_label(),
                value: selector.get_default().cloned(),
                provider: None,
            });
        }

        // If we're in a debugging mode, set a fake default using the empty value for this
        // select's type.
        if self.always_succeed {
            return SelectResolutionResult::Success(ConfigKeyAndValue {
                config_key: BuildTypeSelector::default_condition_label(),
                value: selector.get_original_type().get_default_value(),
                provider: None,
            });
        }

        SelectResolutionResult::Failure(Self::no_match_error(
            attribute_name,
            selector.get_no_match_error(),
            &condition_labels,
            self.get_label(),
            &self.config_hash,
        ))
    }

    /// Constructs a beautiful error for when no conditions in a configurable attribute match.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:357 ConfiguredAttributeMapper.noMatchError
    fn no_match_error(
        attribute: &str,
        custom_no_match_error: &str,
        condition_labels: &IndexSet<Label>,
        target_label: &Label,
        config_hash: &str,
    ) -> String {
        let mut error = format!(
            "configurable attribute \"{}\" in {} doesn't match this configuration",
            attribute, target_label
        );
        if !custom_no_match_error.is_empty() {
            error += &format!(": {}\n", custom_no_match_error);
        } else {
            error += &format!(
                ". Would a default condition help?\n\nConditions checked:\n {}\n\nTo see a condition's definition, run: bazel query --output=build <condition label>.\n",
                condition_labels
                    .iter()
                    .map(|l| l.to_string())
                    .collect::<Vec<_>>()
                    .join("\n ")
            );
        }
        // See ConfiguredTargetQueryEnvironment#shortID for the substring rationale.
        let config_short_hash = &config_hash[..7];
        error += &format!(
            "\nThis instance of {} has configuration identifier {}. To inspect its configuration, run: bazel config {}.\n",
            target_label, config_short_hash, config_short_hash
        );
        error += "\nFor more help, see https://bazel.build/docs/configurable-attributes#faq-select-choose-condition.\n\n";
        error
    }

    /// Java `super.get(attributeName, type)` (`AbstractAttributeMapper.get` with `this` as the
    /// map computed defaults read).
    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:56 AbstractAttributeMapper.get
    fn super_get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        let base = self.base();
        base.get_from_raw_attribute_value(
            self,
            self.rule.get_attr_typed(attribute_name, type_),
            attribute_name,
            type_,
        )
    }

    /// Returns the labels that appear multiple times in the same attribute value.
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:440 ConfiguredAttributeMapper.checkForDuplicateLabels
    pub fn check_for_duplicate_labels(&self, attribute: &Attribute) -> IndexSet<Label> {
        let attr_type = Type::LabelList;
        blaze_util::check_argument!(
            attribute.get_type() == attr_type,
            "Not a label list type: {}",
            attribute.get_name()
        );
        let attr_name = attribute.get_name();
        let selector_list = self.base().get_selector_list(attr_name, attr_type);
        // already checked in RuleClass via AggregatingAttributeMapper.checkForDuplicateLabels
        match &selector_list {
            None => return IndexSet::new(),
            Some(s) if s.get_selectors().len() == 1 => return IndexSet::new(),
            _ => {}
        }
        let labels = match self.get(attr_name, attr_type) {
            Some(AttrValue::LabelList(l)) => l,
            _ => Vec::new(),
        };
        // Java: CollectionUtils.duplicatedElementsOf
        let mut seen: IndexSet<&Label> = IndexSet::new();
        let mut duplicates: IndexSet<Label> = IndexSet::new();
        for label in &labels {
            if !seen.insert(label) {
                duplicates.insert(label.clone());
            }
        }
        duplicates
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:196 AbstractAttributeMapper.visitLabels
    fn visit_labels_of_attribute(
        &self,
        attribute: &Attribute,
        type_: Type,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        // null values are particularly possible for computed defaults.
        if let Some(value) = self.get(attribute.get_name(), type_) {
            let mut visitor =
                |label: &Label, _context: Option<&Attribute>| consumer(attribute, label);
            type_.visit_labels(&mut visitor, &value, Some(attribute));
        }
    }
}

impl AttributeMap for ConfiguredAttributeMapper {
    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:99 ConfiguredAttributeMapper.describeRule
    fn describe_rule(&self) -> String {
        format!(
            "{} ({})",
            self.base().describe_rule(),
            &self.config_hash[..6]
        )
    }

    fn get_label(&self) -> &Label {
        self.rule.get_label()
    }

    fn has(&self, attr_name: &str) -> bool {
        self.base().has(attr_name)
    }

    fn has_typed(&self, attr_name: &str, type_: Type) -> bool {
        self.base().has_typed(attr_name, type_)
    }

    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:400 ConfiguredAttributeMapper.get
    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        match self.get_and_validate(attribute_name, type_) {
            Ok(v) => v,
            // Callers that reach this branch should explicitly validate the attribute through an
            // appropriate call (either validateAttributes or getAndValidate) and handle the
            // exception directly. This method assumes pre-validated attributes.
            Err(e) => panic!(
                "IllegalStateException: lookup failed on attribute {}: {}",
                attribute_name, e.0
            ),
        }
    }

    fn is_configurable(&self, attribute_name: &str) -> bool {
        AbstractAttributeMapper::is_configurable_in(
            &*self.rule as &dyn RuleOrMacroInstance,
            attribute_name,
        )
    }

    fn get_attribute_names(&self) -> Vec<String> {
        self.base().get_attribute_names()
    }

    fn get_attribute_type(&self, attr_name: &str) -> Option<Type> {
        self.get_attribute_definition(attr_name)
            .map(Attribute::get_type)
    }

    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        self.rule
            .get_attribute_provider()
            .get_attribute_by_name_maybe(attr_name)
    }

    // Java: com/google/devtools/build/lib/packages/ConfiguredAttributeMapper.java:414 ConfiguredAttributeMapper.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
        let Some(type_) = self.get_attribute_type(attribute_name) else {
            return self
                .base()
                .is_attribute_value_explicitly_specified(attribute_name);
        };
        let Some(selector_list) = self.base().get_selector_list(attribute_name, type_) else {
            // This is a normal attribute.
            return self
                .base()
                .is_attribute_value_explicitly_specified(attribute_name);
        };
        for selector in selector_list.get_selectors() {
            match self.resolve_selector(attribute_name, selector) {
                SelectResolutionResult::Failure(_) => return true,
                SelectResolutionResult::Success(resolved_path) => {
                    if selector.is_value_set(&resolved_path.config_key) {
                        return true;
                    }
                }
            }
        }
        false // Every select() in this list chooses a path with value "None".
    }

    fn visit_all_labels(&self, consumer: &mut dyn FnMut(&Attribute, &Label)) {
        self.visit_labels_with_filter(&DependencyFilter::all_deps(), consumer)
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:161 AbstractAttributeMapper.visitLabels
    fn visit_labels(&self, attribute_name: &str, consumer: &mut dyn FnMut(&Label)) {
        let attribute = self
            .rule
            .get_attribute_provider()
            .get_attribute_by_name(attribute_name);
        let type_ = attribute.get_type();
        if type_ != Type::Output
            && type_ != Type::OutputList
            && type_ != Type::NodepLabel
            && type_ != Type::NodepLabelList
        {
            self.visit_labels_of_attribute(attribute, type_, &mut |_attr, label| consumer(label));
        }
    }

    // Java: com/google/devtools/build/lib/packages/AbstractAttributeMapper.java:173 AbstractAttributeMapper.visitLabels
    fn visit_labels_with_filter(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        let info = RuleInfoProvider(&self.rule);
        for attribute in self.rule.get_attribute_provider().get_attributes() {
            let type_ = attribute.get_type();
            // TODO(bazel-team): clean up the typing / visitation interface so we don't have to
            // special-case these types.
            if type_ != Type::Output
                && type_ != Type::OutputList
                && type_ != Type::NodepLabel
                && type_ != Type::NodepLabelList
                && filter.test(&info, attribute)
            {
                self.visit_labels_of_attribute(attribute, type_, consumer);
            }
        }
    }

    fn get_package_args(&self) -> PackageArgs {
        self.base().get_package_args()
    }
}

#[cfg(test)]
mod tests {
    // The BuildViewTestCase tests of ConfiguredAttributeMapperTest live in
    // crates/blaze_sky_top/src/tests/configured_attribute_mapper_test.rs.

    // Java: ConfiguredAttributeMapperTest.noMatchErrorFormat (the message format itself).
    #[test]
    fn no_match_error_format() {
        let mut conditions = indexmap::IndexSet::new();
        conditions.insert(blaze_cmdline::Label::parse_canonical_unchecked(
            "//conditions:a",
        ));
        conditions.insert(blaze_cmdline::Label::parse_canonical_unchecked(
            "//conditions:b",
        ));
        let error = super::ConfiguredAttributeMapper::no_match_error(
            "srcs",
            "",
            &conditions,
            &blaze_cmdline::Label::parse_canonical_unchecked("//a:gen"),
            "0123456789abcdef",
        );
        assert_eq!(
            error,
            "configurable attribute \"srcs\" in //a:gen doesn't match this configuration. Would a default condition help?\n\nConditions checked:\n //conditions:a\n //conditions:b\n\nTo see a condition's definition, run: bazel query --output=build <condition label>.\n\nThis instance of //a:gen has configuration identifier 0123456. To inspect its configuration, run: bazel config 0123456.\n\nFor more help, see https://bazel.build/docs/configurable-attributes#faq-select-choose-condition.\n\n"
        );
    }
}
