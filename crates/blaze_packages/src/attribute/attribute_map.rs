//! Port of `packages/AttributeMap.java`: the interface for looking up attribute values of a
//! rule or macro instance under different policies (raw, aggregated, configured, ...).

use blaze_cmdline::Label;

use crate::attribute::Attribute;
use crate::attribute::DependencyFilter;
use crate::package::PackageArgs;
use crate::types::AttrValue;
use crate::types::Type;

/// Java `AttributeMap`.
// Java: com/google/devtools/build/lib/packages/AttributeMap.java:30 AttributeMap
pub trait AttributeMap {
    /// Describes the underlying rule, for use in messages.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:32 AttributeMap.describeRule
    fn describe_rule(&self) -> String {
        self.get_label().to_string()
    }

    /// The label of the rule.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:39 AttributeMap.getLabel
    fn get_label(&self) -> &Label;

    /// True if an attribute with the given name exists.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:44 AttributeMap.has
    fn has(&self, attr_name: &str) -> bool;

    /// True if an attribute with the given name exists with the given type (Java
    /// `has(String, Type)`).
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:51 AttributeMap.has
    fn has_typed(&self, attr_name: &str, type_: Type) -> bool;

    /// The value of the named attribute, which must be of the given type; `None` is Java null.
    /// Panics (Java IllegalArgumentException) if the rule has no such attribute of that type.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:62 AttributeMap.get
    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue>;

    /// The value of the named attribute if it exists, otherwise `default_value`.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:69 AttributeMap.getOrDefault
    fn get_or_default(
        &self,
        attribute_name: &str,
        type_: Type,
        default_value: Option<AttrValue>,
    ) -> Option<AttrValue> {
        if self.has(attribute_name) {
            return self.get(attribute_name, type_);
        }
        default_value
    }

    /// True if the attribute is configurable for this instance (or attributes its computed
    /// default requires are); false if it does not exist.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:81 AttributeMap.isConfigurable
    fn is_configurable(&self, attribute_name: &str) -> bool;

    /// The names of all attributes, in definition order.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:84 AttributeMap.getAttributeNames
    fn get_attribute_names(&self) -> Vec<String>;

    /// The type of the named attribute, or `None` if it does not exist.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:90 AttributeMap.getAttributeType
    fn get_attribute_type(&self, attr_name: &str) -> Option<Type>;

    /// The definition of the named attribute, or `None` if it does not exist.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:96 AttributeMap.getAttributeDefinition
    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute>;

    /// True iff the value of the attribute is explicitly set in the BUILD file or macro.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:114 AttributeMap.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool;

    /// Calls `consumer` for every label of every attribute.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:124 AttributeMap.visitAllLabels
    fn visit_all_labels(&self, consumer: &mut dyn FnMut(&Attribute, &Label));

    /// Calls `consumer` for every label of the named attribute.
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:127 AttributeMap.visitLabels
    fn visit_labels(&self, attribute_name: &str, consumer: &mut dyn FnMut(&Label));

    /// Calls `consumer` for every label of every attribute `filter` accepts (Java
    /// `visitLabels(DependencyFilter, BiConsumer)`).
    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:130 AttributeMap.visitLabels
    fn visit_labels_with_filter(
        &self,
        filter: &DependencyFilter,
        consumer: &mut dyn FnMut(&Attribute, &Label),
    );

    // Java: com/google/devtools/build/lib/packages/AttributeMap.java:132 AttributeMap.getPackageArgs
    fn get_package_args(&self) -> PackageArgs;
}
