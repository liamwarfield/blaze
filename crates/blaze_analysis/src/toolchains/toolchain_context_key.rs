//! Port of `skyframe/toolchains/ToolchainContextKey.java`.

use std::fmt;

use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ToolchainTypeRequirement;
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName};

use crate::toolchains::unloaded_toolchain_context::UnloadedToolchainContextImpl;

/// `SkyFunctions.TOOLCHAIN_RESOLUTION`.
pub const TOOLCHAIN_RESOLUTION: SkyFunctionName = SkyFunctionName("TOOLCHAIN_RESOLUTION");

/// SkyKey implementation used for `ToolchainResolutionFunction` to produce
/// [`UnloadedToolchainContextImpl`] instances.
///
/// PORT: Java's `ImmutableSet`s are kept as insertion-ordered vectors without duplicates; two
/// keys whose sets differ only in order are distinct here (Java set equality ignores order).
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:31 ToolchainContextKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ToolchainContextKey {
    configuration_key: BuildConfigurationKey,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    exec_constraint_labels: Vec<Label>,
    force_execution_platform: Option<Label>,
    debug_target: bool,
}

impl ToolchainContextKey {
    /// Returns a new [`ToolchainContextKeyBuilder`].
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:36 ToolchainContextKey.key
    pub fn key() -> ToolchainContextKeyBuilder {
        ToolchainContextKeyBuilder {
            configuration_key: None,
            toolchain_types: Vec::new(),
            exec_constraint_labels: Vec::new(),
            force_execution_platform: None,
            debug_target: false,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:53 ToolchainContextKey.configurationKey
    pub fn configuration_key(&self) -> &BuildConfigurationKey {
        &self.configuration_key
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:55 ToolchainContextKey.toolchainTypes
    pub fn toolchain_types(&self) -> &[ToolchainTypeRequirement] {
        &self.toolchain_types
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:57 ToolchainContextKey.execConstraintLabels
    pub fn exec_constraint_labels(&self) -> &[Label] {
        &self.exec_constraint_labels
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:59 ToolchainContextKey.forceExecutionPlatform
    pub fn force_execution_platform(&self) -> Option<&Label> {
        self.force_execution_platform.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:61 ToolchainContextKey.debugTarget
    pub fn debug_target(&self) -> bool {
        self.debug_target
    }
}

/// Java record `toString` of `ToolchainTypeRequirement`.
pub fn toolchain_type_requirement_to_string(r: &ToolchainTypeRequirement) -> String {
    format!(
        "ToolchainTypeRequirement[toolchainType={}, mandatory={}, ignoreIfInvalid={}]",
        r.toolchain_type, r.mandatory, r.ignore_if_invalid
    )
}

impl fmt::Debug for ToolchainContextKey {
    // Java: AutoValue toString of ToolchainContextKey.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ToolchainContextKey{{configurationKey={:?}, toolchainTypes=[{}], execConstraintLabels=[{}], forceExecutionPlatform={}, debugTarget={}}}",
            self.configuration_key,
            self.toolchain_types
                .iter()
                .map(toolchain_type_requirement_to_string)
                .collect::<Vec<_>>()
                .join(", "),
            self.exec_constraint_labels
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            match &self.force_execution_platform {
                Some(l) => format!("Optional[{l}]"),
                None => "Optional.empty".to_owned(),
            },
            self.debug_target
        )
    }
}

impl Key for ToolchainContextKey {
    type Value = UnloadedToolchainContextImpl;
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:44 ToolchainContextKey.functionName
    // PORT: skipped, key interning (the keys are plain values here): ToolchainContextKey.getSkyKeyInterner
    const FUNCTION: SkyFunctionName = TOOLCHAIN_RESOLUTION;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:145 SkyFunctions.TOOLCHAIN_RESOLUTION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: an AutoValue (key, both platforms by `PlatformInfo.equals`, toolchain types, resolved toolchains, requested labels, error data). PORT: the toolchain types and resolved toolchains are compared in order (Java: a set and a set multimap): Rust prunes less, never more.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/UnloadedToolchainContextImpl.java:32 UnloadedToolchainContextImpl
    fn values_equal(
        old: &UnloadedToolchainContextImpl,
        new: &UnloadedToolchainContextImpl,
    ) -> bool {
        old == new
    }
}

/// Builder for [`ToolchainContextKey`].
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:65 ToolchainContextKey.Builder
#[derive(Clone, Debug)]
pub struct ToolchainContextKeyBuilder {
    configuration_key: Option<BuildConfigurationKey>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    exec_constraint_labels: Vec<Label>,
    force_execution_platform: Option<Label>,
    debug_target: bool,
}

/// Java `ImmutableSet.copyOf`: keeps the first occurrence of each element.
fn dedup<T: PartialEq + Clone>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

impl ToolchainContextKeyBuilder {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:66 ToolchainContextKey.Builder.configurationKey
    pub fn configuration_key(&mut self, key: BuildConfigurationKey) -> &mut Self {
        self.configuration_key = Some(key);
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:68 ToolchainContextKey.Builder.toolchainTypes
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:70 ToolchainContextKey.Builder.toolchainTypes
    pub fn toolchain_types(
        &mut self,
        toolchain_types: impl IntoIterator<Item = ToolchainTypeRequirement>,
    ) -> &mut Self {
        self.toolchain_types = dedup(toolchain_types);
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:72 ToolchainContextKey.Builder.execConstraintLabels
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:74 ToolchainContextKey.Builder.execConstraintLabels
    pub fn exec_constraint_labels(
        &mut self,
        exec_constraint_labels: impl IntoIterator<Item = Label>,
    ) -> &mut Self {
        self.exec_constraint_labels = dedup(exec_constraint_labels);
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:76 ToolchainContextKey.Builder.debugTarget
    pub fn debug_target(&mut self, flag: bool) -> &mut Self {
        self.debug_target = flag;
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:78 ToolchainContextKey.Builder.forceExecutionPlatform
    pub fn force_execution_platform(&mut self, exec_platform: Label) -> &mut Self {
        self.force_execution_platform = Some(exec_platform);
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:80 ToolchainContextKey.Builder.build
    pub fn build(&self) -> ToolchainContextKey {
        ToolchainContextKey {
            configuration_key: self
                .configuration_key
                .clone()
                .expect("IllegalStateException: Missing required properties: configurationKey"),
            toolchain_types: self.toolchain_types.clone(),
            exec_constraint_labels: self.exec_constraint_labels.clone(),
            force_execution_platform: self.force_execution_platform.clone(),
            debug_target: self.debug_target,
        }
    }
}
