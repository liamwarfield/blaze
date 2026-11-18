//! Ports of `skyframe/toolchains/UnloadedToolchainContext.java` (the interface) and
//! `UnloadedToolchainContextImpl.java` (its only implementation).

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::rule_class::ToolchainTypeRequirement;
use indexmap::IndexMap;

use crate::platform::{PlatformInfo, ToolchainTypeInfo};
use crate::toolchains::no_matching_platform_data::NoMatchingPlatformData;
use crate::toolchains::toolchain_context::ToolchainContext;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;

/// Java `UnloadedToolchainContext` (an interface with one implementation).
pub type UnloadedToolchainContext = UnloadedToolchainContextImpl;

/// Represents the state of toolchain resolution once the specific required toolchains have been
/// determined, but before the toolchain dependencies have been resolved.
// Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContextImpl.java:32 UnloadedToolchainContextImpl
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnloadedToolchainContextImpl {
    key: ToolchainContextKey,
    execution_platform: Arc<PlatformInfo>,
    target_platform: Arc<PlatformInfo>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    /// Java `ImmutableSetMultimap<ToolchainTypeInfo, Label>`: distinct entries in insertion
    /// order.
    toolchain_type_to_resolved: Vec<(ToolchainTypeInfo, Label)>,
    requested_label_to_toolchain_type: IndexMap<Label, ToolchainTypeInfo>,
    error_data: Option<NoMatchingPlatformData>,
}

impl UnloadedToolchainContextImpl {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContextImpl.java:34 UnloadedToolchainContextImpl.builder
    pub fn builder(key: ToolchainContextKey) -> UnloadedToolchainContextImplBuilder {
        UnloadedToolchainContextImplBuilder {
            key,
            execution_platform: None,
            target_platform: None,
            toolchain_types: Vec::new(),
            toolchain_type_to_resolved: Vec::new(),
            requested_label_to_toolchain_type: IndexMap::new(),
            error_data: None,
        }
    }

    /// The map of toolchain type to resolved toolchain to be used.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContext.java:32 UnloadedToolchainContext.toolchainTypeToResolved
    pub fn toolchain_type_to_resolved(&self) -> &[(ToolchainTypeInfo, Label)] {
        &self.toolchain_type_to_resolved
    }

    /// Java `toolchainTypeToResolved().inverse().get(label)`.
    pub fn toolchain_types_resolved_to(&self, label: &Label) -> Vec<ToolchainTypeInfo> {
        self.toolchain_type_to_resolved
            .iter()
            .filter(|(_, l)| l == label)
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// Maps from the actual requested label to the discovered [`ToolchainTypeInfo`].
    ///
    /// Note that the key may be different from [`ToolchainTypeInfo::type_label`] if the
    /// requested label is an `alias`. In this case, there will be two labels for the same
    /// [`ToolchainTypeInfo`].
    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContext.java:41 UnloadedToolchainContext.requestedLabelToToolchainType
    pub fn requested_label_to_toolchain_type(&self) -> &IndexMap<Label, ToolchainTypeInfo> {
        &self.requested_label_to_toolchain_type
    }

    /// Returns the error, if any, that occurred in resolving this toolchain context.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContext.java:48 UnloadedToolchainContext.errorData
    pub fn error_data(&self) -> Option<&NoMatchingPlatformData> {
        self.error_data.as_ref()
    }
}

impl ToolchainContext for UnloadedToolchainContextImpl {
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

    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContextImpl.java:81 UnloadedToolchainContextImpl.resolvedToolchainLabels
    fn resolved_toolchain_labels(&self) -> Vec<Label> {
        // Java: toolchainTypeToResolved().inverse().keySet().
        let mut labels: Vec<Label> = Vec::new();
        for (_, l) in &self.toolchain_type_to_resolved {
            if !labels.contains(l) {
                labels.push(l.clone());
            }
        }
        labels
    }
}

/// Builder class to help create the [`UnloadedToolchainContextImpl`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContextImpl.java:45 UnloadedToolchainContextImpl.Builder
#[derive(Clone, Debug)]
pub struct UnloadedToolchainContextImplBuilder {
    key: ToolchainContextKey,
    execution_platform: Option<Arc<PlatformInfo>>,
    target_platform: Option<Arc<PlatformInfo>>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    toolchain_type_to_resolved: Vec<(ToolchainTypeInfo, Label)>,
    requested_label_to_toolchain_type: IndexMap<Label, ToolchainTypeInfo>,
    error_data: Option<NoMatchingPlatformData>,
}

impl UnloadedToolchainContextImplBuilder {
    /// Sets the key that identifies this context.
    pub fn set_key(&mut self, key: ToolchainContextKey) -> &mut Self {
        self.key = key;
        self
    }

    /// Sets the selected execution platform that these toolchains use.
    pub fn set_execution_platform(&mut self, execution_platform: Arc<PlatformInfo>) -> &mut Self {
        self.execution_platform = Some(execution_platform);
        self
    }

    /// Sets the target platform that these toolchains generate output for.
    pub fn set_target_platform(&mut self, target_platform: Arc<PlatformInfo>) -> &mut Self {
        self.target_platform = Some(target_platform);
        self
    }

    /// Sets the toolchain types that were requested.
    pub fn set_toolchain_types(
        &mut self,
        toolchain_types: Vec<ToolchainTypeRequirement>,
    ) -> &mut Self {
        self.toolchain_types = toolchain_types;
        self
    }

    /// Maps from the actual toolchain type to the resolved toolchain implementation that should
    /// be used (Java `ImmutableSetMultimap.copyOf`: duplicate entries are dropped).
    pub fn set_toolchain_type_to_resolved(
        &mut self,
        toolchain_type_to_resolved: Vec<(ToolchainTypeInfo, Label)>,
    ) -> &mut Self {
        let mut entries: Vec<(ToolchainTypeInfo, Label)> = Vec::new();
        for e in toolchain_type_to_resolved {
            if !entries.contains(&e) {
                entries.push(e);
            }
        }
        self.toolchain_type_to_resolved = entries;
        self
    }

    /// Maps from the actual requested label to the discovered [`ToolchainTypeInfo`].
    pub fn set_requested_label_to_toolchain_type(
        &mut self,
        requested_label_to_toolchain_type: IndexMap<Label, ToolchainTypeInfo>,
    ) -> &mut Self {
        self.requested_label_to_toolchain_type = requested_label_to_toolchain_type;
        self
    }

    /// Stores an exception that occurred during resolution of this toolchain.
    pub fn set_error_data(&mut self, error_data: Option<NoMatchingPlatformData>) -> &mut Self {
        self.error_data = error_data;
        self
    }

    pub fn build(&self) -> UnloadedToolchainContextImpl {
        // Java: AutoValue "Missing required properties".
        let mut missing = Vec::new();
        if self.execution_platform.is_none() {
            missing.push("executionPlatform");
        }
        if self.target_platform.is_none() {
            missing.push("targetPlatform");
        }
        if !missing.is_empty() {
            panic!(
                "IllegalStateException: Missing required properties: {}",
                missing.join(" ")
            );
        }
        UnloadedToolchainContextImpl {
            key: self.key.clone(),
            execution_platform: self.execution_platform.clone().unwrap(),
            target_platform: self.target_platform.clone().unwrap(),
            toolchain_types: self.toolchain_types.clone(),
            toolchain_type_to_resolved: self.toolchain_type_to_resolved.clone(),
            requested_label_to_toolchain_type: self.requested_label_to_toolchain_type.clone(),
            error_data: self.error_data.clone(),
        }
    }
}
