//! Port of `analysis/ResolvedToolchainContext.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::rule_class::ToolchainTypeRequirement;
use blaze_packages::starlark_defs::platform_common::TEMPLATE_VARIABLE_INFO;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::info::downcast_info;
use crate::platform::platform_provider_utils;
use crate::platform::template_variable_info::TemplateVariableInfo;
use crate::platform::{PlatformInfo, ToolchainTypeInfo};
use crate::toolchains::toolchain_context::ToolchainContext;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;
use crate::toolchains::toolchain_exception::{ToolchainException, ToolchainExceptionKind};
use crate::toolchains::unloaded_toolchain_context::UnloadedToolchainContext;

/// Represents the data needed for a specific target's use of toolchains and platforms, including
/// specific `ToolchainInfo` providers for each required toolchain type.
// Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:41 ResolvedToolchainContext
#[derive(Clone, Debug)]
pub struct ResolvedToolchainContext {
    // Fields from ToolchainContext.
    key: ToolchainContextKey,
    execution_platform: Arc<PlatformInfo>,
    target_platform: Arc<PlatformInfo>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    resolved_toolchain_labels: Vec<Label>,
    // Fields from ResolvedToolchainContext.
    target_description: String,
    requested_toolchain_type_labels: IndexMap<Label, ToolchainTypeInfo>,
    /// Frozen `ToolchainInfo`s by toolchain type.
    toolchains: IndexMap<ToolchainTypeInfo, OwnedFrozenValue>,
    /// Frozen `TemplateVariableInfo`s.
    template_variable_providers: Vec<OwnedFrozenValue>,
    prerequisite_targets: Vec<ConfiguredTargetAndData>,
}

impl ResolvedToolchainContext {
    /// Finishes preparing the [`ResolvedToolchainContext`] by finding the specific toolchain
    /// providers to be used for each toolchain type.
    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:48 ResolvedToolchainContext.load
    pub fn load(
        unloaded_toolchain_context: &UnloadedToolchainContext,
        target_description: &str,
        toolchain_targets: &[ConfiguredTargetAndData],
    ) -> Result<ResolvedToolchainContext, ToolchainException> {
        let mut toolchains: IndexMap<ToolchainTypeInfo, OwnedFrozenValue> = IndexMap::new();
        let mut template_variable_providers: Vec<OwnedFrozenValue> = Vec::new();

        for target in toolchain_targets {
            // The unloadedToolchainContext.requestedToolchainTypeLabels() and target.getLabel()
            // both use the original label that was requested, rather than the post-alias label.
            let discovered_label = target.get_configured_target().get_original_label();
            let toolchain_info =
                platform_provider_utils::toolchain(Some(target.get_configured_target()));

            for toolchain_type in
                unloaded_toolchain_context.toolchain_types_resolved_to(&discovered_label)
            {
                // If the toolchainType hadn't been resolved to an actual target, resolution
                // would have failed with an error much earlier. However, the target might still
                // not be an actual toolchain.
                match &toolchain_info {
                    Some(info) => {
                        if toolchains.contains_key(&toolchain_type) {
                            // Java: ImmutableMap.Builder.buildOrThrow on a duplicate key.
                            panic!(
                                "IllegalArgumentException: Multiple entries with same key: {}",
                                toolchain_type
                            );
                        }
                        toolchains.insert(toolchain_type.clone(), info.clone());
                    }
                    None => {
                        return Err(target_not_toolchain_exception(
                            &toolchain_type,
                            &discovered_label,
                        ));
                    }
                }

                // Find any template variables present for this toolchain.
                if let Some(template_variable_info) = target
                    .get_configured_target()
                    .get(&TEMPLATE_VARIABLE_INFO.key())
                    .filter(|v| downcast_info::<TemplateVariableInfo>(v).is_some())
                {
                    template_variable_providers.push(template_variable_info.clone());
                }
            }
        }

        // Verify that all mandatory toolchain type requirements are present.
        for toolchain_type_requirement in unloaded_toolchain_context.toolchain_types() {
            if toolchain_type_requirement.mandatory {
                let toolchain_type_label = &toolchain_type_requirement.toolchain_type;
                let toolchain_type_info = unloaded_toolchain_context
                    .requested_label_to_toolchain_type()
                    .get(toolchain_type_label);
                if !toolchain_type_info.is_some_and(|t| toolchains.contains_key(t)) {
                    return Err(missing_toolchain_type_requirement_exception(
                        toolchain_type_requirement,
                    ));
                }
            }
        }

        // Java: ImmutableSet.copyOf(toolchainTargets).
        let mut prerequisite_targets: Vec<ConfiguredTargetAndData> = Vec::new();
        for t in toolchain_targets {
            prerequisite_targets.push(t.clone());
        }

        Ok(ResolvedToolchainContext {
            // ToolchainContext:
            key: unloaded_toolchain_context.key().clone(),
            execution_platform: unloaded_toolchain_context.execution_platform().clone(),
            target_platform: unloaded_toolchain_context.target_platform().clone(),
            toolchain_types: unloaded_toolchain_context.toolchain_types().to_vec(),
            resolved_toolchain_labels: unloaded_toolchain_context.resolved_toolchain_labels(),
            // ResolvedToolchainContext:
            target_description: target_description.to_owned(),
            requested_toolchain_type_labels: unloaded_toolchain_context
                .requested_label_to_toolchain_type()
                .clone(),
            toolchains,
            template_variable_providers,
            prerequisite_targets,
        })
    }

    /// Returns a description of the target being used, for error messaging.
    pub fn target_description(&self) -> &str {
        &self.target_description
    }

    /// Sets the map from requested `Label` to toolchain type provider.
    pub fn requested_toolchain_type_labels(&self) -> &IndexMap<Label, ToolchainTypeInfo> {
        &self.requested_toolchain_type_labels
    }

    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:118 ResolvedToolchainContext.toolchains
    pub fn toolchains(&self) -> &IndexMap<ToolchainTypeInfo, OwnedFrozenValue> {
        &self.toolchains
    }

    /// Returns the template variables that these toolchains provide.
    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:121 ResolvedToolchainContext.templateVariableProviders
    pub fn template_variable_providers(&self) -> &[OwnedFrozenValue] {
        &self.template_variable_providers
    }

    /// Returns the actual prerequisites for this context, for use in validation.
    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:124 ResolvedToolchainContext.prerequisiteTargets
    pub fn prerequisite_targets(&self) -> &[ConfiguredTargetAndData] {
        &self.prerequisite_targets
    }

    /// Returns the toolchain for the given type, or `None` if the toolchain type was not
    /// required in this context.
    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:135 ResolvedToolchainContext.forToolchainType
    pub fn for_toolchain_type(&self, toolchain_type_label: &Label) -> Option<OwnedFrozenValue> {
        let toolchain_type_info = self
            .requested_toolchain_type_labels
            .get(toolchain_type_label)?;
        self.toolchains.get(toolchain_type_info).cloned()
    }

    // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:144 ResolvedToolchainContext.forToolchainType
    pub fn for_toolchain_type_info(
        &self,
        toolchain_type: &ToolchainTypeInfo,
    ) -> Option<OwnedFrozenValue> {
        self.toolchains.get(toolchain_type).cloned()
    }
}

impl ToolchainContext for ResolvedToolchainContext {
    fn key(&self) -> &ToolchainContextKey {
        &self.key
    }

    fn execution_platform(&self) -> &Arc<PlatformInfo> {
        &self.execution_platform
    }

    fn target_platform(&self) -> &Arc<PlatformInfo> {
        &self.target_platform
    }

    fn toolchain_types(&self) -> &[ToolchainTypeRequirement] {
        &self.toolchain_types
    }

    fn resolved_toolchain_labels(&self) -> Vec<Label> {
        self.resolved_toolchain_labels.clone()
    }
}

/// Exception used when a toolchain type is requested but the resolved target does not have
/// ToolchainInfo.
// Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:153 ResolvedToolchainContext.TargetNotToolchainException.<init>
fn target_not_toolchain_exception(
    toolchain_type: &ToolchainTypeInfo,
    resolved_target_label: &Label,
) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::TargetNotToolchain,
        format!(
            "toolchain type {} resolved to target {}, but that target does not provide {}",
            toolchain_type.type_label(),
            resolved_target_label,
            crate::platform::toolchain_info::STARLARK_NAME
        ),
    )
}

/// Exception used when a toolchain type is required but no implementation was found.
// Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:171 ResolvedToolchainContext.MissingToolchainTypeRequirementException.<init>
fn missing_toolchain_type_requirement_exception(
    toolchain_type_requirement: &ToolchainTypeRequirement,
) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::MissingToolchainTypeRequirement,
        format!(
            "toolchain type {} was mandatory but is not present",
            toolchain_type_requirement.toolchain_type
        ),
    )
}
