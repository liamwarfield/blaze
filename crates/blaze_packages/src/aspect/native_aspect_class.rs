//! Port of `packages/NativeAspectClass.java`.

use std::any::Any;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use super::AspectDefinition;
use super::AspectParameters;

/// A class of aspects that are implemented natively in Bazel (Java `NativeAspectClass`).
///
/// Java wraps the Java class implementing the aspect factory; the instances are singletons
/// registered with the rule class provider, so equality is by class, i.e. by
/// [`NativeAspectClass::get_name`] (the class simple name).
// Java: com/google/devtools/build/lib/packages/NativeAspectClass.java:34 NativeAspectClass
pub trait NativeAspectClass: Send + Sync + 'static {
    /// Java `getName()`: the implementing class's simple name.
    // Java: com/google/devtools/build/lib/packages/NativeAspectClass.java:36 NativeAspectClass.getName
    fn get_name(&self) -> &str;

    /// Java `AspectClass.getKey()`.
    // Java: com/google/devtools/build/lib/packages/AspectClass.java:98 AspectClass.getKey
    fn get_key(&self) -> String {
        self.get_name().to_owned()
    }

    /// Java `getDefinition(AspectParameters)`. Callers go through `Aspect::for_native`, which
    /// caches the result per class and parameters (Java `Aspect.definitionCache`).
    // Java: com/google/devtools/build/lib/packages/NativeAspectClass.java:40 NativeAspectClass.getDefinition
    fn get_definition(&self, aspect_parameters: &AspectParameters) -> Arc<AspectDefinition>;

    /// For downcasting to the concrete class (Java `instanceof`).
    fn as_any(&self) -> &dyn Any;
}

// PORT: skipped, Skyframe serialization (remote analysis caching) is not ported: NativeAspectClass.Codec.getEncodedClass
// PORT: skipped, Skyframe serialization (remote analysis caching) is not ported: NativeAspectClass.Codec.serialize
// PORT: skipped, Skyframe serialization (remote analysis caching) is not ported: NativeAspectClass.Codec.deserialize

impl PartialEq for dyn NativeAspectClass {
    fn eq(&self, other: &dyn NativeAspectClass) -> bool {
        self.get_name() == other.get_name()
    }
}

impl Eq for dyn NativeAspectClass {}

impl Hash for dyn NativeAspectClass {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get_name().hash(state);
    }
}

impl fmt::Debug for dyn NativeAspectClass {
    // PORT: Java's `Object.toString()` (`<class>@<identity hash>`) is not reproducible; the
    // class name stands for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}
