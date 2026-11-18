//! Ports `packages/DeclaredExecGroup.java` and (as data) `analysis/config/ToolchainTypeRequirement`.

use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;

/// A toolchain type a rule or exec group requires (Java
/// `analysis/config/ToolchainTypeRequirement`).
// PORT: move to the analysis crate (W4).
// Java: com/google/devtools/build/lib/analysis/config/ToolchainTypeRequirement.java:35 ToolchainTypeRequirement
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolchainTypeRequirement {
    pub toolchain_type: Label,
    pub mandatory: bool,
    pub ignore_if_invalid: bool,
}

impl ToolchainTypeRequirement {
    /// Java `ToolchainTypeRequirement.create(label)`: mandatory.
    // Java: com/google/devtools/build/lib/analysis/config/ToolchainTypeRequirement.java:43 ToolchainTypeRequirement.create
    pub fn create(toolchain_type: Label) -> Self {
        ToolchainTypeRequirement {
            toolchain_type,
            mandatory: true,
            ignore_if_invalid: false,
        }
    }
}

/// Java `DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME`.
// Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:57 DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME
pub const DEFAULT_EXEC_GROUP_NAME: &str = "default-exec-group";

/// An execution group declared by a rule (Java `DeclaredExecGroup`).
// Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:43 DeclaredExecGroup
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredExecGroup {
    pub toolchain_types_map: IndexMap<Label, ToolchainTypeRequirement>,
    pub exec_compatible_with: IndexSet<Label>,
    pub copy_from_default: bool,
}

impl DeclaredExecGroup {
    /// Java `DeclaredExecGroup.COPY_FROM_DEFAULT`.
    // Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:60 DeclaredExecGroup.COPY_FROM_DEFAULT
    pub fn copy_from_default_group() -> Self {
        DeclaredExecGroup {
            toolchain_types_map: IndexMap::new(),
            exec_compatible_with: IndexSet::new(),
            copy_from_default: true,
        }
    }

    // Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:71 DeclaredExecGroup.isAutomatic
    pub fn is_automatic(exec_group_name: &str) -> bool {
        !is_valid_identifier(exec_group_name) && exec_group_name != DEFAULT_EXEC_GROUP_NAME
    }

    // Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:76 DeclaredExecGroup.toolchainTypes
    pub fn toolchain_types(&self) -> Vec<ToolchainTypeRequirement> {
        self.toolchain_types_map.values().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:81 DeclaredExecGroup.toolchainType
    pub fn toolchain_type(&self, label: &Label) -> Option<&ToolchainTypeRequirement> {
        self.toolchain_types_map.get(label)
    }
}

/// Java `Identifier.isValid(name)`.
// Java: net/starlark/java/syntax/Identifier.java:82 Identifier.isValid
pub fn is_valid_identifier(name: &str) -> bool {
    // Keep consistent with Lexer.scanIdentifier.
    for (i, c) in name.chars().enumerate() {
        if !(c.is_ascii_lowercase()
            || c.is_ascii_uppercase()
            || (i > 0 && c.is_ascii_digit())
            || c == '_')
        {
            return false;
        }
    }
    !name.is_empty()
}
