//! Port of `analysis/DependencyKind.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_config::transition_factories::DEFAULT_EXEC_GROUP_NAME;
use blaze_packages::attribute::Attribute;

/// The name of an aspect class (Java `AspectClass.getName()`): a native aspect's class name or
/// a Starlark aspect's `<bzl label>%<exported name>`.
pub type AspectClassName = String;

/// The kind of a dependency edge: an attribute (of the rule, or of an aspect), a toolchain of an
/// exec group, or one of the non-attribute kinds.
// Java: com/google/devtools/build/lib/analysis/DependencyKind.java:29 DependencyKind
#[derive(Clone)]
pub enum DependencyKind {
    /// Java `NonAttributeDependencyKind` (`VISIBILITY`, `TRANSITIVE_VISIBILITY`, `OUTPUT_FILE`).
    NonAttribute(&'static str),
    /// Java `ToolchainDependencyKindImpl`.
    Toolchain {
        exec_group_name: String,
        is_default_exec_group: bool,
    },
    /// Java `BaseTargetToolchainDependencyKind`.
    BaseTargetToolchain {
        exec_group_name: String,
        is_default_exec_group: bool,
        toolchain_type: Label,
    },
    /// Java `AttributeDependencyKind`.
    Attribute {
        attribute: Arc<Attribute>,
        owning_aspect: Option<AspectClassName>,
    },
}

impl DependencyKind {
    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:35 DependencyKind.VISIBILITY_DEPENDENCY
    pub const VISIBILITY_DEPENDENCY: DependencyKind = DependencyKind::NonAttribute("VISIBILITY");
    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:41 DependencyKind.TRANSITIVE_VISIBILITY_DEPENDENCY
    pub const TRANSITIVE_VISIBILITY_DEPENDENCY: DependencyKind =
        DependencyKind::NonAttribute("TRANSITIVE_VISIBILITY");
    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:45 DependencyKind.OUTPUT_FILE_RULE_DEPENDENCY
    pub const OUTPUT_FILE_RULE_DEPENDENCY: DependencyKind =
        DependencyKind::NonAttribute("OUTPUT_FILE");

    /// Java `getAttribute()`.
    pub fn get_attribute(&self) -> Option<&Arc<Attribute>> {
        match self {
            DependencyKind::Attribute { attribute, .. } => Some(attribute),
            _ => None,
        }
    }

    /// Java `getOwningAspect()` (panics like Java for non-attribute kinds).
    pub fn get_owning_aspect(&self) -> Option<&AspectClassName> {
        match self {
            DependencyKind::Attribute { owning_aspect, .. } => owning_aspect.as_ref(),
            _ => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:109 DependencyKind.forExecGroup
    pub fn for_exec_group(exec_group_name: &str) -> DependencyKind {
        if DEFAULT_EXEC_GROUP_NAME == exec_group_name {
            return Self::default_exec_group_toolchain();
        }
        DependencyKind::Toolchain {
            exec_group_name: exec_group_name.to_owned(),
            is_default_exec_group: false,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:117 DependencyKind.defaultExecGroupToolchain
    pub fn default_exec_group_toolchain() -> DependencyKind {
        DependencyKind::Toolchain {
            exec_group_name: DEFAULT_EXEC_GROUP_NAME.to_owned(),
            is_default_exec_group: true,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:122 DependencyKind.forBaseTargetExecGroup
    pub fn for_base_target_exec_group(
        exec_group_name: &str,
        toolchain_type: Label,
    ) -> DependencyKind {
        DependencyKind::BaseTargetToolchain {
            exec_group_name: exec_group_name.to_owned(),
            is_default_exec_group: exec_group_name == DEFAULT_EXEC_GROUP_NAME,
            toolchain_type,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:129 DependencyKind.isBaseTargetToolchain
    pub fn is_base_target_toolchain(&self) -> bool {
        matches!(self, DependencyKind::BaseTargetToolchain { .. })
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:134 DependencyKind.isToolchain
    pub fn is_toolchain(&self) -> bool {
        matches!(
            self,
            DependencyKind::Toolchain { .. } | DependencyKind::BaseTargetToolchain { .. }
        )
    }

    // Java: com/google/devtools/build/lib/analysis/DependencyKind.java:139 DependencyKind.isAttribute
    pub fn is_attribute(&self) -> bool {
        matches!(self, DependencyKind::Attribute { .. })
    }

    /// Java `AttributeDependencyKind.forRule(attribute)`.
    pub fn for_rule(attribute: Arc<Attribute>) -> DependencyKind {
        DependencyKind::Attribute {
            attribute,
            owning_aspect: None,
        }
    }

    /// Java `AttributeDependencyKind.forAspect(attribute, owningAspect)`.
    pub fn for_aspect(attribute: Arc<Attribute>, owning_aspect: AspectClassName) -> DependencyKind {
        DependencyKind::Attribute {
            attribute,
            owning_aspect: Some(owning_aspect),
        }
    }

    /// Java `ToolchainDependencyKind.getExecGroupName()`.
    pub fn get_exec_group_name(&self) -> Option<&str> {
        match self {
            DependencyKind::Toolchain {
                exec_group_name, ..
            }
            | DependencyKind::BaseTargetToolchain {
                exec_group_name, ..
            } => Some(exec_group_name),
            _ => None,
        }
    }
}

impl PartialEq for DependencyKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DependencyKind::NonAttribute(a), DependencyKind::NonAttribute(b)) => a == b,
            (
                DependencyKind::Toolchain {
                    exec_group_name: a,
                    is_default_exec_group: da,
                },
                DependencyKind::Toolchain {
                    exec_group_name: b,
                    is_default_exec_group: db,
                },
            ) => a == b && da == db,
            (
                DependencyKind::BaseTargetToolchain {
                    exec_group_name: a,
                    is_default_exec_group: da,
                    toolchain_type: ta,
                },
                DependencyKind::BaseTargetToolchain {
                    exec_group_name: b,
                    is_default_exec_group: db,
                    toolchain_type: tb,
                },
            ) => a == b && da == db && ta == tb,
            (
                DependencyKind::Attribute {
                    attribute: a,
                    owning_aspect: oa,
                },
                DependencyKind::Attribute {
                    attribute: b,
                    owning_aspect: ob,
                },
            ) => (Arc::ptr_eq(a, b) || **a == **b) && oa == ob,
            _ => false,
        }
    }
}

impl Eq for DependencyKind {}

impl Hash for DependencyKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            DependencyKind::NonAttribute(n) => n.hash(state),
            DependencyKind::Toolchain {
                exec_group_name, ..
            } => exec_group_name.hash(state),
            DependencyKind::BaseTargetToolchain {
                exec_group_name,
                toolchain_type,
                ..
            } => {
                exec_group_name.hash(state);
                toolchain_type.hash(state);
            }
            DependencyKind::Attribute {
                attribute,
                owning_aspect,
            } => {
                attribute.get_name().hash(state);
                owning_aspect.hash(state);
            }
        }
    }
}

impl fmt::Debug for DependencyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: NonAttributeDependencyKind.toString
            DependencyKind::NonAttribute(n) => write!(f, "NonAttributeDependencyKind({n})"),
            DependencyKind::Toolchain {
                exec_group_name,
                is_default_exec_group,
            } => write!(
                f,
                "ToolchainDependencyKindImpl{{execGroupName={exec_group_name}, isDefaultExecGroup={is_default_exec_group}}}"
            ),
            DependencyKind::BaseTargetToolchain {
                exec_group_name,
                is_default_exec_group,
                toolchain_type,
            } => write!(
                f,
                "BaseTargetToolchainDependencyKind{{execGroupName={exec_group_name}, isDefaultExecGroup={is_default_exec_group}, toolchainType={toolchain_type}}}"
            ),
            DependencyKind::Attribute {
                attribute,
                owning_aspect,
            } => write!(
                f,
                "AttributeDependencyKind{{attribute={}, owningAspect={}}}",
                attribute.get_name(),
                owning_aspect.as_deref().unwrap_or("null")
            ),
        }
    }
}
