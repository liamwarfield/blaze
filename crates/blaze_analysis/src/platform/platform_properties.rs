//! Port of `analysis/platform/PlatformProperties.java`.

use indexmap::IndexMap;

/// Properties set on a specific `PlatformInfo`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:27 PlatformProperties
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlatformProperties {
    properties: IndexMap<String, String>,
}

impl PlatformProperties {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:28 PlatformProperties.properties
    pub fn properties(&self) -> &IndexMap<String, String> {
        &self.properties
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:30 PlatformProperties.isEmpty
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Returns a new builder for creating a fresh [`PlatformProperties`] instance.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:35 PlatformProperties.builder
    pub fn builder() -> PlatformPropertiesBuilder {
        PlatformPropertiesBuilder::default()
    }
}

/// Builder class to facilitate creating valid [`PlatformProperties`] instances.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:40 PlatformProperties.Builder
#[derive(Clone, Debug, Default)]
pub struct PlatformPropertiesBuilder {
    parent: Option<PlatformProperties>,
    properties: IndexMap<String, String>,
}

impl PlatformPropertiesBuilder {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:45 PlatformProperties.Builder.setParent
    pub fn set_parent(&mut self, parent: Option<PlatformProperties>) -> &mut Self {
        self.parent = parent;
        self
    }

    /// Returns the current properties (but not any from the parent), for validation.
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:51 PlatformProperties.Builder.getProperties
    pub(crate) fn get_properties(&self) -> &IndexMap<String, String> {
        &self.properties
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:56 PlatformProperties.Builder.setProperties
    pub fn set_properties(&mut self, properties: IndexMap<String, String>) -> &mut Self {
        self.properties = properties;
        self
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:61 PlatformProperties.Builder.build
    pub fn build(&self) -> PlatformProperties {
        let properties = Self::merge_parent(self.parent.as_ref(), &self.properties);
        PlatformProperties { properties }
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformProperties.java:68 PlatformProperties.Builder.mergeParent
    fn merge_parent(
        parent: Option<&PlatformProperties>,
        properties: &IndexMap<String, String>,
    ) -> IndexMap<String, String> {
        let Some(parent) = parent.filter(|p| !p.is_empty()) else {
            return properties.clone();
        };

        let mut result: IndexMap<String, String> = IndexMap::new();
        if !parent.properties().is_empty() {
            result.extend(
                parent
                    .properties()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
        }

        if !properties.is_empty() {
            for (key, value) in properties {
                if value.is_empty() {
                    // Java LinkedHashMap.remove.
                    result.shift_remove(key);
                } else {
                    result.insert(key.clone(), value.clone());
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformPropertiesTest.java properties_empty
    #[test]
    fn properties_empty() {
        let mut builder = PlatformProperties::builder();
        builder.set_properties(map(&[]));
        let platform_properties = builder.build();
        assert!(platform_properties.properties().is_empty());
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformPropertiesTest.java properties_one
    #[test]
    fn properties_one() {
        let mut builder = PlatformProperties::builder();
        builder.set_properties(map(&[("elem1", "value1")]));
        let platform_properties = builder.build();
        assert_eq!(
            platform_properties.properties(),
            &map(&[("elem1", "value1")])
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformPropertiesTest.java properties_parentPlatform_keep
    #[test]
    fn properties_parent_platform_keep() {
        let parent = PlatformProperties::builder()
            .set_properties(map(&[("parent", "properties")]))
            .build();
        let mut builder = PlatformProperties::builder();
        builder.set_parent(Some(parent));
        let platform_properties = builder.build();
        assert_eq!(
            platform_properties.properties(),
            &map(&[("parent", "properties")])
        );
    }

    // Java: com/google/devtools/build/lib/analysis/platform/PlatformPropertiesTest.java properties_parentPlatform_inheritance
    #[test]
    fn properties_parent_platform_inheritance() {
        let parent = PlatformProperties::builder()
            .set_properties(map(&[
                ("p1", "keep"),
                ("p2", "delete"),
                ("p3", "parent"),
                ("p4", "del2"),
            ]))
            .build();
        let mut builder = PlatformProperties::builder();
        builder.set_parent(Some(parent));
        let platform_properties = builder
            .set_properties(map(&[("p2", ""), ("p3", "child"), ("p4", "")]))
            .build();
        assert_eq!(
            platform_properties.properties(),
            &map(&[("p1", "keep"), ("p3", "child")])
        );
    }
}
