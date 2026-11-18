//! Starlark values for the native transition factories exposed to .bzl files (Java
//! `ExecutionTransitionFactory` from `config.exec()`, `NoTransition.getFactory()` from
//! `config.target()`, `NoConfigTransition.getFactory()` from `config.none()`).
//!
//! Creating transitions belongs to analysis (W4); the loading phase only needs their identity.

use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

/// Java `DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME` (the exec group of
/// `ExecutionTransitionFactory.createFactory()`).
// Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:57 DeclaredExecGroup.DEFAULT_EXEC_GROUP_NAME
pub const DEFAULT_EXEC_GROUP_NAME: &str = "default-exec-group";

/// Java `TransitionFactory.TransitionType`.
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:42 TransitionFactory.TransitionType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionType {
    Any,
    Rule,
    Attribute,
}

impl TransitionType {
    // Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:48 TransitionFactory.TransitionType.isCompatibleWith
    pub fn is_compatible_with(self, other: TransitionType) -> bool {
        if self == TransitionType::Any {
            return true;
        }
        if other == TransitionType::Any {
            return true;
        }
        self == other
    }
}

/// The value of `config.exec(exec_group)` (Java `ExecutionTransitionFactory`).
// Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:44 ExecutionTransitionFactory
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExecutionTransitionFactory {
    exec_group: String,
}

starlark_simple_value!(ExecutionTransitionFactory);

impl ExecutionTransitionFactory {
    // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:52 ExecutionTransitionFactory.createFactory
    pub fn create_factory() -> Self {
        ExecutionTransitionFactory {
            exec_group: DEFAULT_EXEC_GROUP_NAME.to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:56 ExecutionTransitionFactory.createFactory
    pub fn create_factory_for(exec_group: &str) -> Self {
        ExecutionTransitionFactory {
            exec_group: exec_group.to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java ExecutionTransitionFactory.getExecGroup
    pub fn get_exec_group(&self) -> &str {
        &self.exec_group
    }

    /// Java `transitionType()`: ATTRIBUTE.
    // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java ExecutionTransitionFactory.transitionType
    pub fn transition_type(&self) -> TransitionType {
        TransitionType::Attribute
    }
}

impl fmt::Display for ExecutionTransitionFactory {
    // PORT: Java has no repr override; Starlark's default repr of a StarlarkValue without one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<ExecTransitionFactory object>")
    }
}

#[starlark_value(type = "ExecTransitionFactory")]
impl<'v> StarlarkValue<'v> for ExecutionTransitionFactory {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ExecutionTransitionFactory>()
            .is_some_and(|o| o == self))
    }
}

/// Which `config.target()` / `config.none()` factory a [`NativeTransitionFactory`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Allocative)]
pub enum NativeTransitionKind {
    /// Java `NoTransition.getFactory()`.
    NoTransition,
    /// Java `NoConfigTransition.getFactory()`.
    NoConfigTransition,
}

/// The value of `config.target()` / `config.none()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct NativeTransitionFactory {
    pub kind: NativeTransitionKind,
}

starlark_simple_value!(NativeTransitionFactory);

impl NativeTransitionFactory {
    /// Both are `TransitionType.ANY`.
    pub fn transition_type(&self) -> TransitionType {
        TransitionType::Any
    }
}

impl fmt::Display for NativeTransitionFactory {
    // PORT: Java has no repr override; Starlark's default repr of a StarlarkValue without one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<transition object>")
    }
}

#[starlark_value(type = "transition")]
impl<'v> StarlarkValue<'v> for NativeTransitionFactory {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<NativeTransitionFactory>()
            .is_some_and(|o| o == self))
    }
}
