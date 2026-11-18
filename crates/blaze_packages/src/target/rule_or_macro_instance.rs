//! Port of `packages/RuleOrMacroInstance.java`: the attribute storage shared by rules and
//! symbolic macro instances ([`AttributeContainer`]) and the abstract base class as the trait
//! [`RuleOrMacroInstance`].

use std::sync::Arc;
use std::sync::OnceLock;

use blaze_cmdline::Label;
use blaze_events::EventHandler;

use crate::attribute::Attribute;
use crate::attribute::AttributeInfoProvider;
use crate::attribute::AttributeProvider;
use crate::macros::MacroInstance;
use crate::package::PackageArgs;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::types::AttrValue;
use crate::types::BuildTypeSelectorList;
use crate::types::Type;

// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:40 RuleOrMacroInstance.NAME
pub const NAME: &str = "name";
// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:41 RuleOrMacroInstance.GENERATOR_NAME
pub const GENERATOR_NAME: &str = "generator_name";
// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:43 RuleOrMacroInstance.GENERATOR_FUNCTION
pub const GENERATOR_FUNCTION: &str = "generator_function";
// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:44 RuleOrMacroInstance.GENERATOR_LOCATION
pub const GENERATOR_LOCATION: &str = "generator_location";

/// A shared, later-populated label list stored in an attribute (Java: the `List<Label>`
/// reference `TestSuiteImplicitTestsAccumulator` hands out for `$implicit_tests`).
pub type ImplicitTestsRef = Arc<parking_lot::RwLock<Vec<Label>>>;

/// The compact (frozen) state of an [`AttributeContainer`] (Java `AttrState.FROZEN_*`).
// PORT: Java compacts `attrValues`/`attrBytes` on freeze, omitting non-explicit values equal to
// the attribute default. Here the slots stay and `omitted` records which ones Java drops.
#[derive(Debug)]
struct FrozenAttrState {
    omitted: Vec<bool>,
    /// The `$implicit_tests` list as populated when the instance was frozen.
    implicit_tests_value: Option<AttrValue>,
}

/// The attribute values of a rule or macro instance: one slot per attribute of its provider,
/// with explicit-set bits (Java fields `attrValues`, `attrBytes`, `label`,
/// `generatorNamePrefixLength` of `RuleOrMacroInstance`).
// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:96 RuleOrMacroInstance.attrValues
#[derive(Debug)]
pub struct AttributeContainer {
    pub(crate) label: Label,
    /// Length in bytes of the generator name when it is a prefix of the name.
    pub(crate) generator_name_prefix_length: i32,
    pub(crate) attr_values: Vec<Option<AttrValue>>,
    pub(crate) explicit: Vec<bool>,
    /// Index and shared list of a `test_suite`'s `$implicit_tests` value.
    pub(crate) implicit_tests: Option<(usize, ImplicitTestsRef)>,
    frozen_state: OnceLock<FrozenAttrState>,
}

impl AttributeContainer {
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:90 RuleOrMacroInstance.<init>
    pub fn new(label: Label, attr_count: usize) -> Self {
        AttributeContainer {
            label,
            generator_name_prefix_length: 0,
            attr_values: vec![None; attr_count],
            explicit: vec![false; attr_count],
            implicit_tests: None,
            frozen_state: OnceLock::new(),
        }
    }

    pub fn get_label(&self) -> &Label {
        &self.label
    }

    pub fn generator_name_prefix_length(&self) -> i32 {
        self.generator_name_prefix_length
    }

    /// Java `isFrozen()`.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:405 RuleOrMacroInstance.isFrozen
    pub fn is_frozen(&self) -> bool {
        self.frozen_state.get().is_some()
    }

    /// Java `freeze()`: omits non-explicit values that match the attribute's default.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:410 RuleOrMacroInstance.freeze
    pub fn freeze(&self, attribute_provider: &AttributeProvider) {
        if self.is_frozen() {
            return;
        }
        let mut omitted = vec![false; self.attr_values.len()];
        for (i, value) in self.attr_values.iter().enumerate() {
            let Some(value) = value else {
                continue;
            };
            if !self.explicit[i] {
                let attr = attribute_provider.get_attribute(i);
                if Some(value) == attr.get_default_value_unchecked() {
                    // Non-explicit value matches the attribute's default. Save space by omitting
                    // storage.
                    omitted[i] = true;
                }
            }
        }
        let implicit_tests_value = self
            .implicit_tests
            .as_ref()
            .map(|(_, list)| AttrValue::LabelList(list.read().clone()));
        let _ = self.frozen_state.set(FrozenAttrState {
            omitted,
            implicit_tests_value,
        });
    }

    /// The stored value at `attr_index`, if any (Java `getAttrIfStored`).
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:280 RuleOrMacroInstance.getAttrIfStored
    pub fn get_attr_if_stored(&self, attr_index: usize) -> Option<&AttrValue> {
        blaze_util::check_argument!(
            attr_index < self.attr_values.len(),
            "index ({}) must not be greater than size ({})",
            attr_index,
            self.attr_values.len() as i64 - 1
        );
        match self.frozen_state.get() {
            None => self.attr_values[attr_index].as_ref(),
            Some(frozen) => {
                if let Some((index, _)) = &self.implicit_tests
                    && *index == attr_index
                {
                    return frozen.implicit_tests_value.as_ref();
                }
                if frozen.omitted[attr_index] {
                    None
                } else {
                    self.attr_values[attr_index].as_ref()
                }
            }
        }
    }

    /// Java `getRawAttrValues()`.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:310 RuleOrMacroInstance.getRawAttrValues
    pub fn get_raw_attr_values(&self) -> Vec<Option<&AttrValue>> {
        (0..self.attr_values.len())
            .map(|i| self.get_attr_if_stored(i))
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:501 RuleOrMacroInstance.getExplicitBit
    pub fn get_explicit_bit(&self, attr_index: usize) -> bool {
        self.explicit[attr_index]
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:508 RuleOrMacroInstance.setExplicitBit
    pub fn set_explicit_bit(&mut self, attr_index: usize) {
        self.explicit[attr_index] = true;
    }
}

/// A rule or symbolic macro instance (Java abstract class `RuleOrMacroInstance`).
/// Implemented by [`crate::target::Rule`] and [`MacroInstance`].
// Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:38 RuleOrMacroInstance
pub trait RuleOrMacroInstance: Send + Sync {
    /// The attribute storage.
    fn attribute_container(&self) -> &AttributeContainer;

    fn attribute_container_mut(&mut self) -> &mut AttributeContainer;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:145 RuleOrMacroInstance.getAttributeProvider
    fn get_attribute_provider(&self) -> &AttributeProvider;

    /// True for rules, false for macro instances.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:350 RuleOrMacroInstance.isRuleInstance
    fn is_rule_instance(&self) -> bool;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:356 RuleOrMacroInstance.isRuleCreatedInMacro
    fn is_rule_created_in_macro(&self) -> bool;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:548 RuleOrMacroInstance.getDefaultVisibility
    fn get_default_visibility(&self) -> RuleVisibility;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:573 RuleOrMacroInstance.getPackageMetadata
    fn get_package_metadata(&self) -> &Arc<PackageMetadata>;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:575 RuleOrMacroInstance.getPackageDeclarations
    fn get_package_declarations(&self) -> &Arc<PackageDeclarations>;

    /// The innermost symbolic macro that declared this instance, if any.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:583 RuleOrMacroInstance.getDeclaringMacro
    fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>>;

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:590 RuleOrMacroInstance.reportError
    fn report_error(&self, message: &str, event_handler: &dyn EventHandler);

    /// The value at `attr_index`: the stored value, else the attribute's raw default (Rule
    /// overrides this).
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:264 RuleOrMacroInstance.getAttrWithIndex
    fn get_attr_with_index(&self, attr_index: usize) -> Option<AttrValue> {
        if let Some(value) = self.attribute_container().get_attr_if_stored(attr_index) {
            return Some(value.clone());
        }
        let attr = self.get_attribute_provider().get_attribute(attr_index);
        attr.get_default_value_unchecked().cloned()
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:140 RuleOrMacroInstance.getLabel
    fn get_label(&self) -> &Label {
        &self.attribute_container().label
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:152 RuleOrMacroInstance.getName
    fn get_name(&self) -> &str {
        self.get_label().get_name()
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:161 RuleOrMacroInstance.getAttributes
    fn get_attributes(&self) -> &[Attribute] {
        self.get_attribute_provider().get_attributes()
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:171 RuleOrMacroInstance.isAttrDefined
    fn is_attr_defined(&self, attr_name: &str, type_: Type) -> bool {
        self.get_attribute_provider().has_attr(attr_name, type_)
    }

    /// Sets an attribute value (`None` is Java null), marking it explicit if requested.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:191 RuleOrMacroInstance.setAttributeValue
    fn set_attribute_value(
        &mut self,
        attribute: &Attribute,
        value: Option<AttrValue>,
        explicit: bool,
    ) {
        blaze_util::check_state!(!self.is_frozen(), "Already frozen: {}", self.get_label());
        let attr_name = attribute.get_name();
        if attr_name == NAME {
            // Avoid unnecessarily storing the name in attrValues - it's stored in the label.
            return;
        }
        if attr_name == GENERATOR_NAME {
            let generator_name = value
                .as_ref()
                .and_then(AttrValue::as_str)
                .expect("ClassCastException: generator_name is not a string");
            if self.get_name().starts_with(generator_name) {
                let len = generator_name.len() as i32;
                self.attribute_container_mut().generator_name_prefix_length = len;
                return;
            }
        }
        let attr_index = self.get_attribute_provider().get_attribute_index(attr_name);
        let is_rule = self.is_rule_instance();
        let Some(attr_index) = attr_index else {
            panic!(
                "IllegalArgumentException: Attribute {} is not valid for this {}",
                attr_name,
                if is_rule { "rule" } else { "macro" }
            );
        };
        if explicit {
            blaze_util::check_state!(
                !self.attribute_container().get_explicit_bit(attr_index),
                "Attribute {} already explicitly set",
                attr_name
            );
            self.attribute_container_mut().set_explicit_bit(attr_index);
        }
        self.attribute_container_mut().attr_values[attr_index] = value;
    }

    /// Sets `$implicit_tests` to a shared list populated when the package is built (Java
    /// `setAttributeValue(implicitTests, testSuiteImplicitTestsRef, explicit)`).
    // Java: com/google/devtools/build/lib/packages/AttributeProvider.java:394 AttributeProvider.populateRuleAttributeValues
    fn set_implicit_tests_ref(
        &mut self,
        attribute: &Attribute,
        tests: ImplicitTestsRef,
        explicit: bool,
    ) {
        let initial = AttrValue::LabelList(tests.read().clone());
        self.set_attribute_value(attribute, Some(initial), explicit);
        let attr_index = self
            .get_attribute_provider()
            .get_attribute_index(attribute.get_name())
            .expect("attribute index");
        self.attribute_container_mut().implicit_tests = Some((attr_index, tests));
    }

    /// The raw value of the named attribute (may be a select or a computed default); `None` if
    /// there is no such attribute or the value is Java null.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:225 RuleOrMacroInstance.getAttr
    fn get_attr(&self, attr_name: &str) -> Option<AttrValue> {
        if attr_name == NAME {
            return Some(AttrValue::String(self.get_name().to_owned()));
        }
        let attr_index = self
            .get_attribute_provider()
            .get_attribute_index(attr_name)?;
        self.get_attr_with_index(attr_index)
    }

    /// Like [`RuleOrMacroInstance::get_attr`], checking the attribute's type; panics (Java
    /// IllegalArgumentException) if there is no such attribute.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:239 RuleOrMacroInstance.getAttr
    fn get_attr_typed(&self, attr_name: &str, type_: Type) -> Option<AttrValue> {
        if attr_name == NAME {
            self.check_attr_type(attr_name, type_, &crate::rule_class::NAME_ATTRIBUTE);
            return Some(AttrValue::String(self.get_name().to_owned()));
        }
        let Some(index) = self.get_attribute_provider().get_attribute_index(attr_name) else {
            panic!(
                "IllegalArgumentException: No such attribute {} in {}{}{}",
                attr_name,
                self.get_attribute_provider(),
                if self.is_rule_instance() {
                    " rule "
                } else {
                    " macro "
                },
                self.get_label()
            );
        };
        self.check_attr_type(
            attr_name,
            type_,
            self.get_attribute_provider().get_attribute(index),
        );
        self.get_attr_with_index(index)
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:379 RuleOrMacroInstance.checkAttrType
    fn check_attr_type(&self, attr_name: &str, requested_type: Type, attr: &Attribute) {
        if requested_type != attr.get_type() {
            panic!(
                "IllegalArgumentException: Attribute {} is of type {} and not of type {} in {}{}{}",
                attr_name,
                attr.get_type(),
                requested_type,
                self.get_attribute_provider(),
                if self.is_rule_instance() {
                    " rule "
                } else {
                    " macro "
                },
                self.get_label()
            );
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:326 RuleOrMacroInstance.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attr_name: &str) -> bool {
        if attr_name == NAME {
            return true;
        }
        if (attr_name == GENERATOR_FUNCTION
            || attr_name == GENERATOR_LOCATION
            || attr_name == GENERATOR_NAME)
            && self.is_rule_instance()
        {
            return self.is_rule_created_in_macro();
        }
        let Some(attr_index) = self.get_attribute_provider().get_attribute_index(attr_name) else {
            return false;
        };
        // PORT: Java's FROZEN_SMALL form keeps the explicit bit only for stored values; explicit
        // values are always stored, so the bit is the same in every state.
        self.attribute_container().get_explicit_bit(attr_index)
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:405 RuleOrMacroInstance.isFrozen
    fn is_frozen(&self) -> bool {
        self.attribute_container().is_frozen()
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:410 RuleOrMacroInstance.freeze
    fn freeze(&self) {
        self.attribute_container()
            .freeze(self.get_attribute_provider())
    }

    /// The select of a configurable attribute, or `None`; panics if the attribute is missing or
    /// has another type.
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:521 RuleOrMacroInstance.getSelectorList
    fn get_selector_list(
        &self,
        attribute_name: &str,
        type_: Type,
    ) -> Option<Arc<BuildTypeSelectorList>> {
        let index = self
            .get_attribute_provider()
            .get_attribute_index(attribute_name)?;
        let Some(AttrValue::SelectorList(selector_list)) =
            self.attribute_container().get_attr_if_stored(index)
        else {
            return None;
        };
        if selector_list.get_original_type() != type_ {
            panic!(
                "IllegalArgumentException: Attribute {} is not of type {} in {} rule {}",
                attribute_name,
                type_,
                self.get_attribute_provider(),
                self.get_label()
            );
        }
        Some(selector_list.clone())
    }

    /// The stored `visibility` labels, or `None` (Java `getRawVisibilityLabels`).
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:554 RuleOrMacroInstance.getRawVisibilityLabels
    fn get_raw_visibility_labels(&self) -> Option<Vec<Label>> {
        let visibility_index = self
            .get_attribute_provider()
            .get_attribute_index("visibility")?;
        match self
            .attribute_container()
            .get_attr_if_stored(visibility_index)
        {
            None => None,
            Some(AttrValue::LabelList(labels)) => Some(labels.clone()),
            Some(other) => panic!("ClassCastException: visibility value {other:?}"),
        }
    }

    /// The labels of the `visibility` attribute as declared (default visibility if unset).
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:567 RuleOrMacroInstance.getVisibilityDeclaredLabels
    fn get_visibility_declared_labels(&self) -> Vec<Label> {
        match self.get_raw_visibility_labels() {
            Some(raw_labels) => raw_labels,
            None => self.get_default_visibility().get_declared_labels(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:586 RuleOrMacroInstance.getPackageArgs
    fn get_package_args(&self) -> PackageArgs {
        self.get_package_declarations().get_package_args()
    }
}

impl<T: RuleOrMacroInstance + ?Sized> AttributeInfoProvider for T {
    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:316 RuleOrMacroInstance.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified_for(&self, attribute: &Attribute) -> bool {
        RuleOrMacroInstance::is_attribute_value_explicitly_specified(self, attribute.get_name())
    }
}
