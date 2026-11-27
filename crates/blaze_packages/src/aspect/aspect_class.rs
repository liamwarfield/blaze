//! Port of `packages/AspectClass.java` (the interface; its two implementations are
//! [`super::NativeAspectClass`] and [`super::StarlarkAspectClass`]).

use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use super::NativeAspectClass;
use super::StarlarkAspectClass;

/// A class of aspects (Java `AspectClass`): a native aspect class or the location of a Starlark
/// aspect definition. See the Java class comment for the aspect pipeline (`AspectClass` +
/// `AspectParameters` = `AspectDescriptor`; + `AspectDefinition` = `Aspect`).
// Java: com/google/devtools/build/lib/packages/AspectClass.java:92 AspectClass
#[derive(Clone)]
pub enum AspectClass {
    // PORT: skipped, no class extends StarlarkNativeAspect in Bazel 9.3.2: StarlarkNativeAspect.repr, StarlarkNativeAspect.getAspectClass, StarlarkNativeAspect.getParamAttributes, StarlarkNativeAspect.getDefaultParametersExtractor
    /// Java `NativeAspectClass` (and `StarlarkNativeAspect`).
    Native(Arc<dyn NativeAspectClass>),
    /// Java `StarlarkAspectClass`.
    Starlark(StarlarkAspectClass),
}

impl AspectClass {
    /// Returns the aspect name.
    // Java: com/google/devtools/build/lib/packages/AspectClass.java:96 AspectClass.getName
    pub fn get_name(&self) -> &str {
        match self {
            AspectClass::Native(n) => n.get_name(),
            AspectClass::Starlark(s) => s.get_name(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/AspectClass.java:98 AspectClass.getKey
    pub fn get_key(&self) -> String {
        match self {
            AspectClass::Native(n) => n.get_key(),
            AspectClass::Starlark(s) => s.get_name().to_owned(),
        }
    }

    /// Java `instanceof NativeAspectClass`.
    pub fn as_native(&self) -> Option<&Arc<dyn NativeAspectClass>> {
        match self {
            AspectClass::Native(n) => Some(n),
            AspectClass::Starlark(_) => None,
        }
    }

    /// Java `instanceof StarlarkAspectClass`.
    pub fn as_starlark(&self) -> Option<&StarlarkAspectClass> {
        match self {
            AspectClass::Native(_) => None,
            AspectClass::Starlark(s) => Some(s),
        }
    }
}

impl PartialEq for AspectClass {
    fn eq(&self, other: &AspectClass) -> bool {
        match (self, other) {
            (AspectClass::Native(a), AspectClass::Native(b)) => **a == **b,
            (AspectClass::Starlark(a), AspectClass::Starlark(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for AspectClass {}

impl Hash for AspectClass {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            AspectClass::Native(n) => {
                0u8.hash(state);
                (**n).hash(state);
            }
            AspectClass::Starlark(s) => {
                1u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl fmt::Display for AspectClass {
    /// Java `toString()`: the Starlark class's name; a native class's `Object.toString()` is not
    /// reproducible, its name stands for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AspectClass::Native(n) => write!(f, "{:?}", &**n),
            AspectClass::Starlark(s) => fmt::Display::fmt(s, f),
        }
    }
}

impl fmt::Debug for AspectClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl From<StarlarkAspectClass> for AspectClass {
    fn from(c: StarlarkAspectClass) -> AspectClass {
        AspectClass::Starlark(c)
    }
}

impl From<Arc<dyn NativeAspectClass>> for AspectClass {
    fn from(c: Arc<dyn NativeAspectClass>) -> AspectClass {
        AspectClass::Native(c)
    }
}
