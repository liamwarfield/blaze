//! Port of `analysis/producers/UnloadedToolchainContextsInputs.java`.
//!
//! PORT: lives with the toolchain context types (not with the producers in
//! `blaze_sky_analysis`) because `RuleContext` and the configured target function share it.

use crate::toolchains::exec_group_collection::ExecGroupCollectionBuilder;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;
use blaze_packages::rule_class::DeclaredExecGroup;
use indexmap::IndexMap;

/// Collates inputs for the `UnloadedToolchainContextsProducer`.
// Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsInputs.java:25 UnloadedToolchainContextsInputs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnloadedToolchainContextsInputs {
    /// Java: the `ExecGroupCollection.Builder` superclass.
    exec_group_collection_builder: ExecGroupCollectionBuilder,
    /// `None` if no toolchain resolution is required.
    target_toolchain_context_key: Option<ToolchainContextKey>,
}

impl UnloadedToolchainContextsInputs {
    // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsInputs.java:29 UnloadedToolchainContextsInputs.create
    pub fn create(
        processed_exec_groups: IndexMap<String, DeclaredExecGroup>,
        target_toolchain_context_key: Option<ToolchainContextKey>,
    ) -> Self {
        UnloadedToolchainContextsInputs {
            exec_group_collection_builder: ExecGroupCollectionBuilder {
                exec_groups: processed_exec_groups,
            },
            target_toolchain_context_key,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsInputs.java:36 UnloadedToolchainContextsInputs.empty
    pub fn empty() -> Self {
        Self::create(IndexMap::new(), None)
    }

    // Java: com/google/devtools/build/lib/analysis/producers/UnloadedToolchainContextsInputs.java:27 UnloadedToolchainContextsInputs.targetToolchainContextKey
    pub fn target_toolchain_context_key(&self) -> Option<&ToolchainContextKey> {
        self.target_toolchain_context_key.as_ref()
    }

    /// Java `execGroups()` (from `ExecGroupCollection.Builder`).
    pub fn exec_groups(&self) -> &IndexMap<String, DeclaredExecGroup> {
        self.exec_group_collection_builder.exec_groups()
    }

    /// The `ExecGroupCollection.Builder` view of these inputs.
    pub fn as_exec_group_collection_builder(&self) -> &ExecGroupCollectionBuilder {
        &self.exec_group_collection_builder
    }
}
