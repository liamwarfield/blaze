//! Port of `analysis/TargetContext.java`.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_cmdline::label::LabelPackageContext;
use blaze_packages::target::Target;
use indexmap::IndexMap;

use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::providers::package_specification_provider::PackageSpecificationProvider;
use crate::providers::visibility::Visibility;
use crate::rule::analysis_environment::CachingAnalysisEnvironment;

/// A helper class for building `ConfiguredTarget` instances, in particular for non-rule ones.
/// For `RuleConfiguredTarget` instances, use `RuleContext` instead, which embeds this.
///
/// `ConfiguredTarget` implementations can use this class to access the necessary state.
// Java: com/google/devtools/build/lib/analysis/TargetContext.java:43 TargetContext
pub struct TargetContext {
    env: Arc<CachingAnalysisEnvironment>,
    target: Target,
    configuration: Option<Arc<BuildConfigurationValue>>,
    /// This list only contains prerequisites that are not declared in rule attributes, with the
    /// exception of visibility (i.e., visibility is represented here, even though it is a rule
    /// attribute in case of a rule). Rule attributes are handled by the `RuleContext` subclass.
    direct_prerequisites: IndexMap<Label, Vec<ConfiguredTargetAndData>>,
    visibility: Visibility,
    transitive_visibility_imposed_by_this_package: Option<Arc<PackageSpecificationProvider>>,
}

impl TargetContext {
    /// The constructor is intentionally package private.
    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:66 TargetContext.<init>
    pub fn new(
        env: Arc<CachingAnalysisEnvironment>,
        target: Target,
        configuration: Option<Arc<BuildConfigurationValue>>,
        direct_prerequisites: &[ConfiguredTargetAndData],
        visibility: Visibility,
        transitive_visibility: Option<Arc<PackageSpecificationProvider>>,
    ) -> TargetContext {
        // Java: Multimaps.index(directPrerequisites, ConfiguredTargetAndData::getTargetLabel).
        let mut index: IndexMap<Label, Vec<ConfiguredTargetAndData>> = IndexMap::new();
        for p in direct_prerequisites {
            index
                .entry(p.get_target_label())
                .or_default()
                .push(p.clone());
        }
        TargetContext {
            env,
            target,
            configuration,
            direct_prerequisites: index,
            visibility,
            transitive_visibility_imposed_by_this_package: transitive_visibility,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:82 TargetContext.getAnalysisEnvironment
    pub fn get_analysis_environment(&self) -> &Arc<CachingAnalysisEnvironment> {
        &self.env
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:86 TargetContext.getActionKeyContext
    pub fn get_action_key_context(&self) -> &Arc<blaze_actions::ActionKeyContext> {
        self.env.get_action_key_context()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:90 TargetContext.getTarget
    pub fn get_target(&self) -> &Target {
        &self.target
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:94 TargetContext.getLabel
    pub fn get_label(&self) -> &Label {
        self.target.get_label()
    }

    /// Returns the `PackageContext` of this target (package identifier and repository mapping).
    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:99 TargetContext.getPackageContext
    pub fn get_package_context(&self) -> LabelPackageContext {
        LabelPackageContext::of(
            self.get_label().get_package_identifier(),
            self.target
                .get_package_metadata()
                .repository_mapping
                .clone(),
        )
    }

    /// Returns the configuration for this target. This may return `None` if the target is
    /// supposed to be configuration-independent (like an input file, or a visibility rule).
    /// However, this is guaranteed to be non-null for rules and for output files.
    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:109 TargetContext.getConfiguration
    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:113 TargetContext.getVisibility
    pub fn get_visibility(&self) -> &Visibility {
        &self.visibility
    }

    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:118 TargetContext.getTransitiveVisibilityImposedByThisPackage
    pub fn get_transitive_visibility_imposed_by_this_package(
        &self,
    ) -> Option<&Arc<PackageSpecificationProvider>> {
        self.transitive_visibility_imposed_by_this_package.as_ref()
    }

    /// Returns the prerequisite with the given label and configuration, or `None` if no such
    /// prerequisite exists. If `config` is `None` (Java `Optional.empty()`), it's interpreted as
    /// "don't care".
    // Java: com/google/devtools/build/lib/analysis/TargetContext.java:128 TargetContext.findDirectPrerequisite
    pub fn find_direct_prerequisite(
        &self,
        label: &Label,
        config: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<ConfiguredTarget> {
        let prerequisites = self.direct_prerequisites.get(label)?;
        match config {
            None => prerequisites
                .first()
                .map(|p| p.get_configured_target().clone()),
            Some(config) => {
                for prerequisite in prerequisites {
                    // Java: Objects.equal(prerequisite.getConfiguration(), config.get()).
                    if prerequisite
                        .get_configuration()
                        .is_some_and(|c| Arc::ptr_eq(c, config) || **c == **config)
                    {
                        return Some(prerequisite.get_configured_target().clone());
                    }
                }
                None
            }
        }
    }
}
