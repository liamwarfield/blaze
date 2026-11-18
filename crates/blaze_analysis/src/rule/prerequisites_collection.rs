//! Port of `analysis/PrerequisitesCollection.java`.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_packages::attribute::{Attribute, AttributeMap};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::target::Rule;
use blaze_packages::types::LabelClass;
use blaze_util::check_state;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::provider_map::TransitiveInfoProvider;
use crate::providers::file_provider::{FileProvider, FilesToRunProvider};
use crate::rule::rule_error_consumer::{RuleErrorConsumer, RuleReporter};

/// Represents the prerequisites of a rule (and of an aspect's attributes) with the lookup
/// methods `RuleContext` delegates to.
// Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:47 PrerequisitesCollection
pub struct PrerequisitesCollection {
    /// Java `ImmutableSortedKeyListMultimap<String, ConfiguredTargetAndData>` (keys sorted).
    attribute_to_prerequisites_map: BTreeMap<String, Vec<ConfiguredTargetAndData>>,
    attributes: Arc<ConfiguredAttributeMapper>,
    rule_error_consumer: Arc<RuleReporter>,
    rule: Arc<Rule>,
    rule_class_name_for_logging: String,
}

static EMPTY: Vec<ConfiguredTargetAndData> = Vec::new();

impl PrerequisitesCollection {
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:81 PrerequisitesCollection.<init>
    pub fn new(
        attribute_to_prerequisites_map: BTreeMap<String, Vec<ConfiguredTargetAndData>>,
        attributes: Arc<ConfiguredAttributeMapper>,
        rule_error_consumer: Arc<RuleReporter>,
        rule: Arc<Rule>,
        rule_class_name_for_logging: String,
    ) -> PrerequisitesCollection {
        PrerequisitesCollection {
            attribute_to_prerequisites_map,
            attributes,
            rule_error_consumer,
            rule,
            rule_class_name_for_logging,
        }
    }

    fn get_attribute(&self, attribute_name: &str) -> Option<&Attribute> {
        self.attributes.get_attribute_definition(attribute_name)
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:113 PrerequisitesCollection.has
    pub fn has(&self, attribute_name: &str) -> bool {
        self.get_attribute(attribute_name).is_some()
    }

    /// Returns all prerequisites, in sorted attribute-name order.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:118 PrerequisitesCollection.getAllPrerequisites
    pub fn get_all_prerequisites(&self) -> Vec<ConfiguredTarget> {
        self.attribute_to_prerequisites_map
            .values()
            .flatten()
            .map(|c| c.get_configured_target().clone())
            .collect()
    }

    /// Returns the `ConfiguredTargetAndData` the given attribute.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:125 PrerequisitesCollection.getPrerequisiteConfiguredTargets
    pub fn get_prerequisite_configured_targets(
        &self,
        attribute_name: &str,
    ) -> &[ConfiguredTargetAndData] {
        self.attribute_to_prerequisites_map
            .get(attribute_name)
            .map(Vec::as_slice)
            .unwrap_or(&EMPTY)
    }

    /// Returns the list of transitive info collections that feed into this target through the
    /// specified attribute.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:133 PrerequisitesCollection.getPrerequisites
    pub fn get_prerequisites(&self, attribute_name: &str) -> Vec<ConfiguredTarget> {
        let Some(attribute) = self.get_attribute(attribute_name) else {
            return Vec::new();
        };
        let rule_class = self.rule.get_rule_class();
        // android_binary, android_test, and android_binary_internal override deps to use a split
        // transition.
        let prerequisite_configured_targets: Vec<ConfiguredTargetAndData> = if (rule_class
            == "android_binary"
            || rule_class == "android_test"
            || rule_class == "android_binary_internal")
            && attribute_name == "deps"
            && attribute.get_transition_factory().is_split()
        {
            // TODO(b/168038145): Restore legacy behavior of returning the prerequisites from the
            // first portion of the split transition.
            let map = self.get_split_prerequisites(attribute_name);
            map.into_iter().next().map(|(_, v)| v).unwrap_or_default()
        } else {
            self.get_prerequisite_configured_targets(attribute_name)
                .to_vec()
        };
        prerequisite_configured_targets
            .iter()
            .map(|c| c.get_configured_target().clone())
            .collect()
    }

    /// Returns all the native providers of the given class under the attribute.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:162 PrerequisitesCollection.getPrerequisites
    pub fn get_prerequisites_providers<P: TransitiveInfoProvider + Clone>(
        &self,
        attribute_name: &str,
    ) -> Vec<P> {
        // Java: AnalysisUtils.getProviders(getPrerequisites(attributeName), classType).
        self.get_prerequisites(attribute_name)
            .iter()
            .filter_map(|p| crate::configuredtargets::get_provider::<P>(p).cloned())
            .collect()
    }

    /// Returns all the declared providers (native and Starlark) for the given key under the
    /// attribute.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:180 PrerequisitesCollection.getPrerequisites
    pub fn get_prerequisites_infos(
        &self,
        attribute_name: &str,
        key: &ProviderKey,
    ) -> Vec<OwnedFrozenValue> {
        // Java: AnalysisUtils.getProviders(getPrerequisites(attributeName), builtinProvider).
        self.get_prerequisites(attribute_name)
            .iter()
            .filter_map(|p| crate::configuredtargets::get_info(p, key))
            .collect()
    }

    /// Returns the a prerequisites keyed by the CPU of their configurations (split transition
    /// keys), preserving order.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:194 PrerequisitesCollection.getSplitPrerequisites
    pub fn get_split_prerequisites(
        &self,
        attribute_name: &str,
    ) -> IndexMap<Option<String>, Vec<ConfiguredTargetAndData>> {
        self.check_attribute_is_dependency(attribute_name);
        // Use an ordered multimap here to preserve ordering.
        let mut result: IndexMap<Option<String>, Vec<ConfiguredTargetAndData>> = IndexMap::new();
        let deps = self.get_prerequisite_configured_targets(attribute_name);
        for t in deps {
            let transition_keys = t.get_transition_keys();
            if transition_keys.is_empty() {
                // The split transition is not active, i.e. does not change build configurations.
                // TODO(jungjw): Investigate if we need to do a check here.
                let mut m = IndexMap::new();
                m.insert(None, deps.to_vec());
                return m;
            }
            for key in transition_keys {
                result.entry(Some(key.clone())).or_default().push(t.clone());
            }
        }
        result
    }

    /// Returns the transitive info collection that feeds into this target through the specified
    /// attribute. Returns `None` if the attribute is empty.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:222 PrerequisitesCollection.getPrerequisite
    pub fn get_prerequisite(&self, attribute_name: &str) -> Option<ConfiguredTarget> {
        self.check_attribute_is_dependency(attribute_name);
        let elements = self.get_prerequisite_configured_targets(attribute_name);
        check_state!(
            elements.len() <= 1,
            "{} attribute {} produces more than one prerequisite",
            self.rule_class_name_for_logging,
            attribute_name
        );
        elements.first().map(|e| e.get_configured_target().clone())
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:235 PrerequisitesCollection.getPrerequisite
    pub fn get_prerequisite_info(
        &self,
        attribute_name: &str,
        key: &ProviderKey,
    ) -> Option<OwnedFrozenValue> {
        let prerequisite = self.get_prerequisite(attribute_name)?;
        crate::configuredtargets::get_info(&prerequisite, key)
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:242 PrerequisitesCollection.getPrerequisite
    pub fn get_prerequisite_provider<P: TransitiveInfoProvider + Clone>(
        &self,
        attribute_name: &str,
    ) -> Option<P> {
        let prerequisite = self.get_prerequisite(attribute_name)?;
        crate::configuredtargets::get_provider::<P>(&prerequisite).cloned()
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:256 PrerequisitesCollection.getPrerequisiteArtifact
    pub fn get_prerequisite_artifact(&self, attribute_name: &str) -> Option<Artifact> {
        let target = self.get_prerequisite(attribute_name);
        self.transitive_info_collection_to_artifact(attribute_name, target.as_ref())
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:262 PrerequisitesCollection.transitiveInfoCollectionToArtifact
    fn transitive_info_collection_to_artifact(
        &self,
        attribute_name: &str,
        target: Option<&ConfiguredTarget>,
    ) -> Option<Artifact> {
        let target = target?;
        let artifacts = crate::configuredtargets::get_provider::<FileProvider>(target)
            .expect("FileProvider")
            .get_files_to_build()
            .clone();
        if artifacts.is_singleton() {
            return Some(artifacts.get_singleton().clone());
        }
        self.rule_error_consumer.attribute_error(
            attribute_name,
            &format!("{} expected a single artifact", target.get_label()),
        );
        None
    }

    /// Returns the prerequisite referred to by the specified attribute. Also checks whether the
    /// attribute is marked as executable and that the target referred to can actually be
    /// executed.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:283 PrerequisitesCollection.getExecutablePrerequisite
    pub fn get_executable_prerequisite(
        &self,
        attribute_name: &str,
    ) -> Option<Arc<FilesToRunProvider>> {
        let rule_definition = self.get_attribute(attribute_name).unwrap_or_else(|| {
            panic!(
                "NullPointerException: {} attribute {} is not defined",
                self.rule_class_name_for_logging, attribute_name
            )
        });
        check_state!(
            rule_definition.is_executable(),
            "{} attribute {} is not configured to be executable",
            self.rule_class_name_for_logging,
            attribute_name
        );
        let prerequisite = self.get_prerequisite(attribute_name)?;
        let result = crate::configuredtargets::get_files_to_run_provider(&prerequisite);
        if result.as_ref().is_none_or(|r| r.get_executable().is_none()) {
            self.rule_error_consumer.attribute_error(
                attribute_name,
                &format!(
                    "{} does not refer to a valid executable target",
                    prerequisite.get_label()
                ),
            );
        }
        result
    }

    /// Returns the prerequisites of the attribute that have the native provider `P`.
    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:310 PrerequisitesCollection.getPrerequisitesIf
    pub fn get_prerequisites_if<P: TransitiveInfoProvider>(
        &self,
        attribute_name: &str,
    ) -> Vec<ConfiguredTarget> {
        self.get_prerequisites(attribute_name)
            .into_iter()
            .filter(|p| crate::configuredtargets::get_provider::<P>(p).is_some())
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java:317 PrerequisitesCollection.checkAttributeIsDependency
    fn check_attribute_is_dependency(&self, attribute_name: &str) {
        let attribute_definition = self.get_attribute(attribute_name).unwrap_or_else(|| {
            panic!(
                "NullPointerException: {}: {} attribute {} is not defined",
                self.rule.get_location(),
                self.rule_class_name_for_logging,
                attribute_name
            )
        });
        check_state!(
            attribute_definition.get_type().get_label_class() == LabelClass::Dependency,
            "{} attribute {} is not a label type attribute",
            self.rule_class_name_for_logging,
            attribute_name
        );
    }
}
