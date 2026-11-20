//! Port of `analysis/AspectBaseTargetResolvedToolchainContext.java` (with its
//! `ToolchainAspectsProviders` Starlark value).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::rule_class::{ProviderKey, ToolchainTypeRequirement};
use blaze_packages::starlark_defs::builtin_providers::builtin_provider_key;
use blaze_packages::starlark_defs::platform_common::TEMPLATE_VARIABLE_INFO;
use blaze_packages::starlark_defs::provider::provider_view;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::{DeeplyImmutable, ProviderInfo};
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value,
};

use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::configuredtargets::MergingException;
use crate::platform::{PlatformInfo, ToolchainTypeInfo};
use crate::provider_map::TransitiveInfoProviderMap;
use crate::toolchains::toolchain_context::ToolchainContext;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;
use crate::toolchains::unloaded_toolchain_context::UnloadedToolchainContext;

fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Java `MergedConfiguredTarget.LABEL_FIELD` (`TransitiveInfoCollectionApi.LABEL_FIELD`).
const LABEL_FIELD: &str = "label";

/// A toolchain context for the aspect's base target toolchains. It is used to represent the
/// result of applying the aspects propagation to the base target toolchains.
// Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:46 AspectBaseTargetResolvedToolchainContext
#[derive(Clone, Debug)]
pub struct AspectBaseTargetResolvedToolchainContext {
    // Fields from ToolchainContext.
    key: ToolchainContextKey,
    execution_platform: Arc<PlatformInfo>,
    target_platform: Arc<PlatformInfo>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    resolved_toolchain_labels: Vec<Label>,
    // Fields from ResolvedToolchainsDataInterface.
    target_description: String,
    requested_toolchain_type_labels: IndexMap<Label, ToolchainTypeInfo>,
    // Fields from this class.
    toolchains: IndexMap<ToolchainTypeInfo, ToolchainAspectsProviders>,
}

impl AspectBaseTargetResolvedToolchainContext {
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:50 AspectBaseTargetResolvedToolchainContext.toolchains
    pub fn toolchains(&self) -> &IndexMap<ToolchainTypeInfo, ToolchainAspectsProviders> {
        &self.toolchains
    }

    /// Returns a description of the target being used, for error messaging.
    pub fn target_description(&self) -> &str {
        &self.target_description
    }

    /// The map from requested `Label` to toolchain type provider.
    pub fn requested_toolchain_type_labels(&self) -> &IndexMap<Label, ToolchainTypeInfo> {
        &self.requested_toolchain_type_labels
    }

    /// `toolchain_targets` is Java's `ImmutableMultimap<ToolchainTypeInfo,
    /// ConfiguredTargetAndData>`: the (possibly merged) toolchain targets by type.
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:52 AspectBaseTargetResolvedToolchainContext.load
    pub fn load(
        unloaded_toolchain_context: &UnloadedToolchainContext,
        target_description: &str,
        toolchain_targets: &IndexMap<ToolchainTypeInfo, Vec<ConfiguredTargetAndData>>,
    ) -> Result<AspectBaseTargetResolvedToolchainContext, MergingException> {
        let mut toolchains_builder: IndexMap<ToolchainTypeInfo, ToolchainAspectsProviders> =
            IndexMap::new();

        // Java: unloadedToolchainContext.toolchainTypeToResolved().keySet().
        let mut toolchain_types: Vec<&ToolchainTypeInfo> = Vec::new();
        for (toolchain_type, _) in unloaded_toolchain_context.toolchain_type_to_resolved() {
            if !toolchain_types.contains(&toolchain_type) {
                toolchain_types.push(toolchain_type);
            }
        }
        for toolchain_type in toolchain_types {
            let targets: &[ConfiguredTargetAndData] = toolchain_targets
                .get(toolchain_type)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            blaze_util::check_argument!(targets.len() == 1);

            let toolchain_target = targets[0].get_configured_target();

            if let ConfiguredTarget::Merged(merged_configured_target) = toolchain_target {
                // Only add the aspects providers from the toolchains that the aspects applied to.
                let template_variable_info = merged_configured_target
                    .get_base_configured_target()
                    .get(&TEMPLATE_VARIABLE_INFO.key());
                toolchains_builder.insert(
                    toolchain_type.clone(),
                    ToolchainAspectsProviders::new(
                        merged_configured_target.get_aspects_providers()?,
                        template_variable_info,
                        toolchain_target.get_label(),
                    ),
                );
            } else {
                // Add empty providers for the toolchains that the aspects did not apply to.
                let template_variable_info = toolchain_target.get(&TEMPLATE_VARIABLE_INFO.key());
                toolchains_builder.insert(
                    toolchain_type.clone(),
                    ToolchainAspectsProviders::new(
                        TransitiveInfoProviderMap::default(),
                        template_variable_info,
                        toolchain_target.get_label(),
                    ),
                );
            }
        }

        Ok(AspectBaseTargetResolvedToolchainContext {
            // ToolchainContext:
            key: unloaded_toolchain_context.key().clone(),
            execution_platform: unloaded_toolchain_context.execution_platform().clone(),
            target_platform: unloaded_toolchain_context.target_platform().clone(),
            toolchain_types: unloaded_toolchain_context.toolchain_types().to_vec(),
            resolved_toolchain_labels: unloaded_toolchain_context.resolved_toolchain_labels(),
            // ResolvedToolchainsDataInterface:
            target_description: target_description.to_owned(),
            requested_toolchain_type_labels: unloaded_toolchain_context
                .requested_label_to_toolchain_type()
                .clone(),
            // this:
            toolchains: toolchains_builder,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:111 AspectBaseTargetResolvedToolchainContext.forToolchainType
    pub fn for_toolchain_type(
        &self,
        toolchain_type_label: &Label,
    ) -> Option<&ToolchainAspectsProviders> {
        if let Some(toolchain_type) = self
            .requested_toolchain_type_labels
            .get(toolchain_type_label)
        {
            return self.toolchains.get(toolchain_type);
        }

        None
    }

    /// The frozen `TemplateVariableInfo`s of the toolchains.
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:119 AspectBaseTargetResolvedToolchainContext.templateVariableProviders
    pub fn template_variable_providers(&self) -> Vec<OwnedFrozenValue> {
        self.toolchains
            .values()
            .filter_map(|p| p.template_variable_provider().cloned())
            .collect()
    }
}

impl ToolchainContext for AspectBaseTargetResolvedToolchainContext {
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

/// A Starlark-indexable wrapper used to represent the providers of the aspects applied on the
/// base target toolchains.
// Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:130 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ToolchainAspectsProviders {
    #[allocative(skip)]
    aspects_providers: TransitiveInfoProviderMap,
    /// A frozen `TemplateVariableInfo`, if the toolchain target has one.
    #[allocative(skip)]
    template_variable_info: Option<OwnedFrozenValue>,
    #[allocative(skip)]
    label: Label,
}

starlark_simple_value!(ToolchainAspectsProviders);

impl ToolchainAspectsProviders {
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:137 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.<init>
    fn new(
        aspects_providers: TransitiveInfoProviderMap,
        template_variable_info: Option<OwnedFrozenValue>,
        label: Label,
    ) -> ToolchainAspectsProviders {
        ToolchainAspectsProviders {
            aspects_providers,
            template_variable_info,
            label,
        }
    }

    /// The providers of the aspects applied on the toolchain target.
    pub fn get_aspects_providers(&self) -> &TransitiveInfoProviderMap {
        &self.aspects_providers
    }

    /// The toolchain target's label.
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:209 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.templateVariableProvider
    pub fn template_variable_provider(&self) -> Option<&OwnedFrozenValue> {
        self.template_variable_info.as_ref()
    }

    /// Selects the provider identified by `key`, throwing a Starlark error if the key is not a
    /// provider or not exported.
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:167 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.selectExportedProvider
    fn select_exported_provider(
        &self,
        key: Value,
        operation: &str,
    ) -> starlark::Result<(ProviderKey, String)> {
        let Some(view) = provider_view(key) else {
            return Err(errorf(format!(
                "This type only supports {operation}ing by object constructors, got {} instead",
                key.get_type()
            )));
        };
        if !view.exported {
            return Err(errorf(format!(
                "{self} only supports {operation}ing by exported providers. Assign the provider a \
                 name in a top-level assignment statement."
            )));
        }
        let provider_key = view
            .key
            .or_else(|| builtin_provider_key(key))
            .expect("exported provider has a key");
        let printable_name = key
            .request_value::<ProviderInfo>()
            .map(|p| p.printable_name)
            .unwrap_or_else(|| provider_key.to_string());
        Ok((provider_key, printable_name))
    }

    /// PORT: keep-alive, as `configuredtargets::target_value`: the providers' frozen heaps are
    /// held by the configured targets of the graph the evaluation runs in, which outlives the
    /// evaluation and every value made from the result.
    fn owned_to_value<'v>(v: &OwnedFrozenValue) -> Value<'v> {
        // SAFETY: see the comment above.
        unsafe { v.unchecked_frozen_value() }.to_value()
    }
}

impl fmt::Display for ToolchainAspectsProviders {
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:183 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<ToolchainAspectsProviders for toolchain target: {}>",
            self.label
        )
    }
}

#[starlark_value(type = "ToolchainAspectsProviders")]
impl<'v> StarlarkValue<'v> for ToolchainAspectsProviders {
    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:147 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.getIndex
    fn at(&self, index: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let (key, printable_name) = self.select_exported_provider(index, "index")?;
        if let Some(declared_provider) = self.aspects_providers.get(&key) {
            return Ok(Self::owned_to_value(declared_provider));
        }
        Err(errorf(format!(
            "{self} doesn't contain declared provider '{printable_name}'"
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:159 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        let (key, _) = self.select_exported_provider(other, "query")?;
        Ok(self.aspects_providers.get(&key).is_some())
    }

    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:189 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.getValue
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if attribute == LABEL_FIELD {
            return Some(heap.alloc(LabelValue(self.label.clone())));
        }
        None
    }

    // Java: com/google/devtools/build/lib/analysis/AspectBaseTargetResolvedToolchainContext.java:197 AspectBaseTargetResolvedToolchainContext.ToolchainAspectsProviders.getFieldNames
    fn dir_attr(&self) -> Vec<String> {
        vec![LABEL_FIELD.to_owned()]
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}
