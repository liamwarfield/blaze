//! Port of `analysis/AspectAwareAttributeMapper.java`.

use blaze_cmdline::Label;
use blaze_packages::attribute::{Attribute, AttributeMap, DependencyFilter};
use blaze_packages::package::PackageArgs;
use blaze_packages::types::{AttrValue, Type};
use indexmap::IndexMap;

/// An `AttributeMap` that supports attribute type queries on both a rule and its aspects and
/// attribute value queries on the rule.
///
/// An attribute type query is anything accessible from `Attribute` (i.e. anything about how the
/// attribute is integrated into the `RuleClass`). An attribute value query is anything related to
/// the actual value an attribute takes.
///
/// For example, given `deps = [":adep"]`, checking that `deps` exists or that it's type is
/// `BuildType#LABEL_LIST` are type queries. Checking that its value is explicitly set in the BUILD
/// File or that its value `[":adep"]` are value queries.
///
/// Value queries on aspect attributes trigger `UnsupportedOperationException` (a panic).
// Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:45 AspectAwareAttributeMapper
pub struct AspectAwareAttributeMapper<M: RuleAttributes> {
    rule_attributes: M,
    aspect_attributes: IndexMap<String, Attribute>,
}

/// How an [`AspectAwareAttributeMapper`] holds the rule's `AttributeMap` (Java: a reference):
/// borrowed (`&M`) or shared (`Arc<M>`, for a mapper that an `AspectContext` keeps).
pub trait RuleAttributes {
    fn as_attribute_map(&self) -> &dyn AttributeMap;
}

impl<T: AttributeMap> RuleAttributes for &T {
    fn as_attribute_map(&self) -> &dyn AttributeMap {
        *self
    }
}

impl RuleAttributes for &dyn AttributeMap {
    fn as_attribute_map(&self) -> &dyn AttributeMap {
        *self
    }
}

impl<T: AttributeMap> RuleAttributes for std::sync::Arc<T> {
    fn as_attribute_map(&self) -> &dyn AttributeMap {
        &**self
    }
}

impl<M: RuleAttributes> AspectAwareAttributeMapper<M> {
    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:49 AspectAwareAttributeMapper.<init>
    pub fn new(
        rule_attributes: M,
        aspect_attributes: IndexMap<String, Attribute>,
    ) -> AspectAwareAttributeMapper<M> {
        AspectAwareAttributeMapper {
            rule_attributes,
            aspect_attributes,
        }
    }

    fn rule_attributes(&self) -> &dyn AttributeMap {
        self.rule_attributes.as_attribute_map()
    }
}

impl<M: RuleAttributes> AttributeMap for AspectAwareAttributeMapper<M> {
    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:56 AspectAwareAttributeMapper.getLabel
    fn get_label(&self) -> &Label {
        self.rule_attributes().get_label()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:61 AspectAwareAttributeMapper.get
    fn get(&self, attribute_name: &str, type_: Type) -> Option<AttrValue> {
        if self.rule_attributes().has_typed(attribute_name, type_) {
            self.rule_attributes().get(attribute_name, type_)
        } else {
            match self.aspect_attributes.get(attribute_name) {
                None => panic!(
                    "IllegalArgumentException: no attribute '{}' in either {} or its aspects",
                    attribute_name,
                    self.rule_attributes().describe_rule()
                ),
                Some(attribute) if attribute.get_type() != type_ => panic!(
                    "IllegalArgumentException: attribute {} has type {}, not expected type {}",
                    attribute_name,
                    attribute.get_type(),
                    type_
                ),
                Some(attribute) if attribute.is_implicit() => {
                    attribute.get_default_value(/* rule= */ None)
                }
                Some(_) => panic!(
                    "UnsupportedOperationException: Attribute '{attribute_name}' comes from an \
                     aspect. Value retrieval for aspect attributes is not supported."
                ),
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:86 AspectAwareAttributeMapper.isConfigurable
    fn is_configurable(&self, attribute_name: &str) -> bool {
        self.rule_attributes().is_configurable(attribute_name)
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:91 AspectAwareAttributeMapper.getAttributeNames
    fn get_attribute_names(&self) -> Vec<String> {
        let mut names = self.rule_attributes().get_attribute_names();
        names.extend(self.aspect_attributes.keys().cloned());
        names
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:96 AspectAwareAttributeMapper.getAttributeType
    fn get_attribute_type(&self, attr_name: &str) -> Option<Type> {
        let type_ = self.rule_attributes().get_attribute_type(attr_name);
        if type_.is_some() {
            type_
        } else {
            self.aspect_attributes
                .get(attr_name)
                .map(|attribute| attribute.get_type())
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:108 AspectAwareAttributeMapper.getAttributeDefinition
    fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        let attribute = self.rule_attributes().get_attribute_definition(attr_name);
        if attribute.is_some() {
            attribute
        } else {
            self.aspect_attributes.get(attr_name)
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:118 AspectAwareAttributeMapper.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified(&self, attribute_name: &str) -> bool {
        self.rule_attributes()
            .is_attribute_value_explicitly_specified(attribute_name)
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:123 AspectAwareAttributeMapper.visitAllLabels
    fn visit_all_labels(&self, _consumer: &mut dyn FnMut(&Attribute, &Label)) {
        panic!("UnsupportedOperationException: rule + aspects label visition is not supported");
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:128 AspectAwareAttributeMapper.visitLabels
    fn visit_labels(&self, _attribute_name: &str, _consumer: &mut dyn FnMut(&Label)) {
        panic!("UnsupportedOperationException: rule + aspects label visition is not supported");
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:133 AspectAwareAttributeMapper.visitLabels
    fn visit_labels_with_filter(
        &self,
        _filter: &DependencyFilter,
        _consumer: &mut dyn FnMut(&Attribute, &Label),
    ) {
        panic!("UnsupportedOperationException: rule + aspects label visition is not supported");
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:138 AspectAwareAttributeMapper.getPackageArgs
    fn get_package_args(&self) -> PackageArgs {
        self.rule_attributes().get_package_args()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:143 AspectAwareAttributeMapper.has
    fn has(&self, attr_name: &str) -> bool {
        if self.rule_attributes().has(attr_name) {
            true
        } else {
            self.aspect_attributes.contains_key(attr_name)
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectAwareAttributeMapper.java:152 AspectAwareAttributeMapper.has
    fn has_typed(&self, attr_name: &str, type_: Type) -> bool {
        if self.rule_attributes().has_typed(attr_name, type_) {
            true
        } else {
            self.aspect_attributes
                .get(attr_name)
                .is_some_and(|attribute| attribute.get_type() == type_)
        }
    }
}

// The JUnit tests (AspectAwareAttributeMapperTest) are in
// crates/blaze_sky_top/src/tests/aspect_aware_attribute_mapper_test.rs: they need the analysis
// harness, which lives downstream of this crate.
