//! Port of `analysis/AspectValue.java` (with `AspectValueWithTransitivePackages`,
//! `AspectValueForAlias` and `AspectValueWithTransitivePackagesForAlias`).

use std::fmt;

use blaze_actions::ActionRef;
use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata;
use blaze_collect::NestedSet;
use blaze_packages::aspect::Aspect;

use crate::aspect_key_creator::AspectKey;
use crate::configured_aspect::ConfiguredAspect;
use crate::provider_map::TransitiveInfoProviderMap;

/// An aspect in the context of the Skyframe graph (the value of `ASPECT`).
///
/// PORT: Java's four classes are one struct: `key` and `transitive_packages` are set for the
/// `...WithTransitivePackages` classes, `for_alias` for the `...ForAlias` ones. `clear()`
/// (`--discard_analysis_cache`) is skipped (incremental).
// Java: com/google/devtools/build/lib/analysis/AspectValue.java:34 AspectValue
pub struct AspectValue {
    /// Java `BasicActionLookupValue.actions`.
    actions: Vec<ActionRef>,
    aspect: Aspect,
    providers: TransitiveInfoProviderMap,
    /// We store this in a boolean because the aspect variable from which it comes may be cleared
    /// to save memory.
    writes_output_to_master_log: bool,
    /// Java `AspectValueWithTransitivePackages.key`.
    key: Option<AspectKey>,
    /// Java `AspectValueWithTransitivePackages.transitivePackages`.
    transitive_packages: Option<NestedSet<RepoMappingPackageMetadata>>,
    /// Java `instanceof AspectValueForAlias || instanceof AspectValueWithTransitivePackagesForAlias`.
    for_alias: bool,
}

impl AspectValue {
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:37 AspectValue.create
    pub fn create(
        key: AspectKey,
        aspect: Aspect,
        configured_aspect: &ConfiguredAspect,
        transitive_packages: Option<NestedSet<RepoMappingPackageMetadata>>,
    ) -> AspectValue {
        Self::new(
            key,
            aspect,
            configured_aspect,
            transitive_packages,
            /* for_alias= */ false,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:47 AspectValue.createForAlias
    pub fn create_for_alias(
        key: AspectKey,
        aspect: Aspect,
        configured_aspect: &ConfiguredAspect,
        transitive_packages: Option<NestedSet<RepoMappingPackageMetadata>>,
    ) -> AspectValue {
        Self::new(
            key,
            aspect,
            configured_aspect,
            transitive_packages,
            /* for_alias= */ true,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:67 AspectValue.<init>
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:149 AspectValue.AspectValueWithTransitivePackages.<init>
    fn new(
        key: AspectKey,
        aspect: Aspect,
        configured_aspect: &ConfiguredAspect,
        transitive_packages: Option<NestedSet<RepoMappingPackageMetadata>>,
        for_alias: bool,
    ) -> AspectValue {
        let writes_output_to_master_log = aspect
            .get_definition()
            .get_attributes()
            .contains_key("$print_to_master_log");
        // Java: `transitivePackages == null ? new AspectValue(...) : new
        // AspectValueWithTransitivePackages(key, ...)`.
        let key = transitive_packages.as_ref().map(|_| key);
        AspectValue {
            actions: configured_aspect.get_actions().to_vec(),
            aspect,
            providers: configured_aspect.get_providers().clone(),
            writes_output_to_master_log,
            key,
            transitive_packages,
            for_alias,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:87 AspectValue.getKeyForTransitivePackageTracking
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:165 AspectValue.AspectValueWithTransitivePackages.getKeyForTransitivePackageTracking
    pub fn get_key_for_transitive_package_tracking(&self) -> &AspectKey {
        match &self.key {
            Some(key) => key,
            None => panic!(
                "UnsupportedOperationException: Only supported if transitive packages are tracked."
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:91 AspectValue.getAspect
    pub fn get_aspect(&self) -> &Aspect {
        &self.aspect
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:96 AspectValue.getProviders
    pub fn get_providers(&self) -> &TransitiveInfoProviderMap {
        &self.providers
    }

    /// Java `BasicActionLookupValue.getActions()`.
    // Java: com/google/devtools/build/lib/actions/BasicActionLookupValue.java:33 BasicActionLookupValue.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:100 AspectValue.getWritesOutputToMasterLog
    pub fn get_writes_output_to_master_log(&self) -> bool {
        self.writes_output_to_master_log
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:124 AspectValue.getTransitivePackages
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:160 AspectValue.AspectValueWithTransitivePackages.getTransitivePackages
    pub fn get_transitive_packages(&self) -> Option<&NestedSet<RepoMappingPackageMetadata>> {
        self.transitive_packages.as_ref()
    }

    /// Java `getConfiguredObject()`: the value itself, as a `ProviderCollection` (a
    /// `ConfiguredAspect`).
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:129 AspectValue.getConfiguredObject
    pub fn get_configured_object(self: &std::sync::Arc<Self>) -> ConfiguredAspect {
        ConfiguredAspect::Value(self.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:210 AspectValue.isForAliasTarget
    pub fn is_for_alias_target(aspect_value: &AspectValue) -> bool {
        aspect_value.for_alias
    }

    /// Java's class simple name (`MoreObjects.toStringHelper(this)`).
    fn java_class_simple_name(&self) -> &'static str {
        match (self.transitive_packages.is_some(), self.for_alias) {
            (false, false) => "AspectValue",
            (true, false) => "AspectValueWithTransitivePackages",
            (false, true) => "AspectValueForAlias",
            (true, true) => "AspectValueWithTransitivePackagesForAlias",
        }
    }
}

impl fmt::Debug for AspectValue {
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:134 AspectValue.getStringHelper
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:177 AspectValue.AspectValueWithTransitivePackages.getStringHelper
    // PORT: the actions and providers print with their Rust `Debug` forms.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{{actions={:?}, aspect=Aspect {}, providers={:?}",
            self.java_class_simple_name(),
            self.actions,
            self.aspect.get_descriptor(),
            self.providers
        )?;
        if let Some(transitive_packages) = &self.transitive_packages {
            write!(
                f,
                ", key={}, transitivePackages={:?}",
                self.key
                    .as_ref()
                    .map(|k| format!("{k:?}"))
                    .unwrap_or_else(|| "null".to_owned()),
                transitive_packages
            )?;
        }
        f.write_str("}")
    }
}

impl fmt::Display for AspectValue {
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:139 AspectValue.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Port of `src/test/java/.../analysis/AspectValueTest.java`. Expected values come from the Java
/// test.
#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use blaze_build_config::build_configuration_key::BuildConfigurationKey;
    use blaze_cmdline::Label;
    use blaze_packages::aspect::{
        AspectClass, AspectDefinition, AspectDescriptor, AspectParameters, NativeAspectClass,
    };

    use crate::aspect_key_creator::{AspectKey, AspectKeyCreator};
    use crate::configured_target_key::ConfiguredTargetKey;

    /// Java `TestAspects.ATTRIBUTE_ASPECT` / `EXTRA_ATTRIBUTE_ASPECT` (singleton native aspect
    /// classes; key creation never reads their definitions).
    struct TestAspectClass(&'static str);

    impl NativeAspectClass for TestAspectClass {
        fn get_name(&self) -> &str {
            self.0
        }

        fn get_definition(&self, _aspect_parameters: &AspectParameters) -> Arc<AspectDefinition> {
            blaze_util::unported!("TestAspects.getDefinition")
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn create_key(
        label: &Label,
        base_configuration: &Option<BuildConfigurationKey>,
        aspect_class: &AspectClass,
        parameters: &AspectParameters,
    ) -> AspectKey {
        AspectKeyCreator::create_aspect_key_no_base_keys(
            AspectDescriptor::of(aspect_class.clone(), parameters.clone()),
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(base_configuration.clone())
                .build(),
        )
    }

    fn create_derived_key(
        label: &Label,
        base_configuration: &Option<BuildConfigurationKey>,
        aspect_class1: &AspectClass,
        parameters1: &AspectParameters,
        aspect_class2: &AspectClass,
        parameters2: &AspectParameters,
    ) -> AspectKey {
        let base_key = create_key(label, base_configuration, aspect_class1, parameters1);
        AspectKeyCreator::create_aspect_key(
            AspectDescriptor::of(aspect_class2.clone(), parameters2.clone()),
            vec![base_key],
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(base_configuration.clone())
                .build(),
        )
    }

    fn hash_of(k: &AspectKey) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        k.hash(&mut h);
        h.finish()
    }

    /// Guava `EqualsTester`: members of a group are equal (with equal hashes), members of
    /// different groups are not.
    fn test_equals(groups: &[Vec<AspectKey>]) {
        for (i, group) in groups.iter().enumerate() {
            for a in group {
                for b in group {
                    assert_eq!(a, b);
                    assert_eq!(hash_of(a), hash_of(b));
                }
                for (j, other) in groups.iter().enumerate() {
                    if i != j {
                        for b in other {
                            assert_ne!(a, b, "{a:?} vs {b:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn key_equality() {
        // PORT: Java uses the target and exec configurations of an AnalysisTestCase build; any
        // two distinct configuration keys exercise the same equality.
        let c1: Option<BuildConfigurationKey> = Some(BuildConfigurationKey::create(Arc::new(
            blaze_build_config::transitions::EMPTY_OPTIONS.clone(),
        )));
        let c2: Option<BuildConfigurationKey> = None;
        let l1 = Label::parse_canonical("//a:l1").unwrap();
        let l1b = Label::parse_canonical("//a:l1").unwrap();
        let l2 = Label::parse_canonical("//a:l2").unwrap();
        let mut b = AspectParameters::builder();
        b.add_attribute("foo", "bar");
        let i1 = b.build();
        let mut b = AspectParameters::builder();
        b.add_attribute("foo", "bar");
        let i1b = b.build();
        let mut b = AspectParameters::builder();
        b.add_attribute("foo", "baz");
        let i2 = b.build();
        let attribute_aspect: Arc<dyn NativeAspectClass> =
            Arc::new(TestAspectClass("AttributeAspect"));
        let a1 = AspectClass::Native(attribute_aspect.clone());
        let a1b = AspectClass::Native(attribute_aspect);
        let a2 = AspectClass::Native(Arc::new(TestAspectClass("ExtraAttributeAspect")));

        // label: //a:l1 or //a:l2
        // baseConfiguration: target or exec
        // aspect: Attribute or ExtraAttribute
        // parameters: bar or baz

        let k = create_key;
        test_equals(&[
            vec![
                k(&l1, &c1, &a1, &i1),
                k(&l1, &c1, &a1, &i1b),
                k(&l1, &c1, &a1b, &i1),
                k(&l1, &c1, &a1b, &i1b),
                k(&l1b, &c1, &a1, &i1),
                k(&l1b, &c1, &a1, &i1b),
                k(&l1b, &c1, &a1b, &i1),
                k(&l1b, &c1, &a1b, &i1b),
            ],
            vec![
                k(&l1, &c1, &a1, &i2),
                k(&l1, &c1, &a1b, &i2),
                k(&l1b, &c1, &a1, &i2),
                k(&l1b, &c1, &a1b, &i2),
            ],
            vec![
                k(&l1, &c1, &a2, &i1),
                k(&l1, &c1, &a2, &i1b),
                k(&l1b, &c1, &a2, &i1),
                k(&l1b, &c1, &a2, &i1b),
            ],
            vec![k(&l1, &c1, &a2, &i2), k(&l1b, &c1, &a2, &i2)],
            vec![
                k(&l1, &c2, &a1, &i1),
                k(&l1, &c2, &a1, &i1b),
                k(&l1, &c2, &a1b, &i1),
                k(&l1, &c2, &a1b, &i1b),
                k(&l1b, &c2, &a1, &i1),
                k(&l1b, &c2, &a1, &i1b),
                k(&l1b, &c2, &a1b, &i1),
                k(&l1b, &c2, &a1b, &i1b),
            ],
            vec![
                k(&l1, &c2, &a1, &i2),
                k(&l1, &c2, &a1b, &i2),
                k(&l1b, &c2, &a1, &i2),
                k(&l1b, &c2, &a1b, &i2),
            ],
            vec![
                k(&l1, &c2, &a2, &i1),
                k(&l1, &c2, &a2, &i1b),
                k(&l1b, &c2, &a2, &i1),
                k(&l1b, &c2, &a2, &i1b),
            ],
            vec![k(&l1, &c2, &a2, &i2), k(&l1b, &c2, &a2, &i2)],
            vec![
                k(&l2, &c1, &a1, &i1),
                k(&l2, &c1, &a1, &i1b),
                k(&l2, &c1, &a1b, &i1),
                k(&l2, &c1, &a1b, &i1b),
            ],
            vec![k(&l2, &c1, &a1, &i2), k(&l2, &c1, &a1b, &i2)],
            vec![k(&l2, &c1, &a2, &i1), k(&l2, &c1, &a2, &i1b)],
            vec![k(&l2, &c1, &a2, &i2)],
            vec![
                k(&l2, &c2, &a1, &i1),
                k(&l2, &c2, &a1, &i1b),
                k(&l2, &c2, &a1b, &i1),
                k(&l2, &c2, &a1b, &i1b),
            ],
            vec![k(&l2, &c2, &a1, &i2), k(&l2, &c2, &a1b, &i2)],
            vec![k(&l2, &c2, &a2, &i1), k(&l2, &c2, &a2, &i1b)],
            vec![k(&l2, &c2, &a2, &i2)],
            vec![
                create_derived_key(&l1, &c1, &a1, &i1, &a2, &i2),
                create_derived_key(&l1, &c1, &a1, &i1b, &a2, &i2),
            ],
            vec![
                create_derived_key(&l1, &c1, &a2, &i1, &a1, &i2),
                create_derived_key(&l1, &c1, &a2, &i1b, &a1, &i2),
            ],
        ]);
    }

    #[test]
    #[ignore = "needs AutoCodec serialization (roundTripMemoized), not ported"]
    fn round_tripping_empty_aspect_parameters_outputs_single_instance() {}

    #[test]
    #[ignore = "needs AutoCodec serialization (roundTripMemoized), not ported"]
    fn round_tripping_merges_equivalent_aspect_parameters() {}
}
