//! The `_builtins` object of `@_builtins` .bzl files.
//!
//! Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java.

use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{FrozenValue, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

/// A module accessible only to `@_builtins` .bzls, that permits access to the original
/// (uninjected) native builtins, as well as internal-only symbols not accessible to users.
// Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:40 BuiltinsInternalModule
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsInternalModule {
    /// `_builtins.native`.
    #[allocative(skip)]
    uninjected_native_object: FrozenValue,
    /// `_builtins.toplevel`.
    #[allocative(skip)]
    uninjected_toplevel_object: FrozenValue,
    /// `_builtins.internal`.
    #[allocative(skip)]
    internal_object: FrozenValue,
}

starlark_simple_value!(BuiltinsInternalModule);

impl BuiltinsInternalModule {
    /// The three objects must live in heaps referenced by the heap this value is allocated in.
    // Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:49 BuiltinsInternalModule.<init>
    pub fn new(
        uninjected_native_object: FrozenValue,
        uninjected_toplevel_object: FrozenValue,
        internal_object: FrozenValue,
    ) -> Self {
        BuiltinsInternalModule {
            uninjected_native_object,
            uninjected_toplevel_object,
            internal_object,
        }
    }
}

// Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:57 BuiltinsInternalModule.repr
impl fmt::Display for BuiltinsInternalModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<_builtins module>")
    }
}

#[starlark_value(type = "_builtins")]
impl<'v> StarlarkValue<'v> for BuiltinsInternalModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("_builtins", builtins_internal_module_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:80 BuiltinsInternalModule.getUninjectedNativeObject
            "native" => Some(self.uninjected_native_object.to_value()),
            // Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:97 BuiltinsInternalModule.getUninjectedToplevelObject
            "toplevel" => Some(self.uninjected_toplevel_object.to_value()),
            // Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:109 BuiltinsInternalModule.getInternalObject
            "internal" => Some(self.internal_object.to_value()),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "get_flag".to_owned(),
            "internal".to_owned(),
            "native".to_owned(),
            "toplevel".to_owned(),
        ]
    }
}

#[starlark_module]
fn builtins_internal_module_methods(builder: &mut MethodsBuilder) {
    /// Returns the value of a `StarlarkSemantics` flag, or a default value if it could not be
    /// retrieved.
    // Java: com/google/devtools/build/lib/packages/BuiltinsInternalModule.java:137 BuiltinsInternalModule.getFlag
    fn get_flag<'v>(this: Value<'v>, name: &str, default: Value<'v>) -> anyhow::Result<Value<'v>> {
        let _ = (this, default);
        // PORT: Java returns the flag's value only if it was set explicitly
        // (StarlarkSemantics.getGeneric); blaze_starlark's typed semantics do not record that.
        blaze_util::unported!("StarlarkSemantics.getGeneric", "flag {}", name)
    }
}
