//! The Starlark values of configuration fragments (`ctx.fragments.<name>`), except `cpp` (agent
//! CC): agent PY, crates/blaze_analysis/DESIGN.md.
//!
//! In Java every `Fragment` is itself a Starlark value whose `@StarlarkMethod`s form its
//! Starlark API. Blaze keeps the fragment data in `blaze_build_config::fragments` and wraps each
//! fragment in a Starlark value here; [`register_fragments`] registers their factories with
//! `blaze_analysis::starlark::fragment_collection`.
//!
//! Ported: `py` (PythonConfiguration), `bazel_py` (BazelPythonConfiguration), `apple`
//! (AppleConfiguration), `objc` (ObjcConfiguration), `j2objc` (J2ObjcConfiguration), `platform`
//! (PlatformConfiguration), `coverage` (CoverageConfiguration), `proto` (ProtoConfiguration).
//! Not registered (no analyzed rule of the corpus reads them, so `ctx.fragments.<name>` fails
//! loudly in `FragmentCollection`): `java` (JavaConfiguration), `android`
//! (AndroidConfiguration), `bazel_android` (BazelAndroidConfiguration).

use std::sync::Arc;

use blaze_analysis::starlark::fragment_collection::{
    FragmentStarlarkFactory, register_fragment_starlark_factory,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::{Fragment, FragmentClass};
use starlark::values::{Heap, Value};

pub mod apple_configuration;
pub mod coverage_configuration;
pub mod objc_configuration;
pub mod platform_configuration;
pub mod proto_configuration;
pub mod python_configuration;

/// A [`FragmentStarlarkFactory`] that wraps the fragment in its Starlark value type.
struct FnFragmentFactory(for<'v> fn(&Arc<dyn Fragment>, Heap<'v>) -> Value<'v>);

impl FragmentStarlarkFactory for FnFragmentFactory {
    fn create<'v>(
        &self,
        fragment: &Arc<dyn Fragment>,
        _configuration: &Arc<BuildConfigurationValue>,
        heap: Heap<'v>,
    ) -> Value<'v> {
        (self.0)(fragment, heap)
    }
}

fn register(
    fragment_class: FragmentClass,
    create: for<'v> fn(&Arc<dyn Fragment>, Heap<'v>) -> Value<'v>,
) {
    register_fragment_starlark_factory(fragment_class, Arc::new(FnFragmentFactory(create)));
}

/// Registers the Starlark value factories of the fragments ported here. The lead calls this
/// once, before analysis.
pub fn register_fragments() {
    python_configuration::register();
    apple_configuration::register();
    objc_configuration::register();
    platform_configuration::register();
    coverage_configuration::register();
    proto_configuration::register();
}

/// Declares the Starlark value type of one fragment class: a shared handle on the fragment,
/// type `$starlark_name`, Java's default `StarlarkValue.repr`
/// (`<unknown object $java_name>`), identity equality (Java `Object.equals`) and the methods of
/// `$methods`.
macro_rules! fragment_value {
    (
        $(#[$meta:meta])*
        $value:ident, $fragment:ty, $starlark_name:tt, $java_name:literal, $methods:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, ::starlark::any::ProvidesStaticType, ::starlark::values::NoSerialize, ::allocative::Allocative)]
        pub struct $value(
            #[allocative(skip)] pub ::std::sync::Arc<dyn ::blaze_build_config::fragment::Fragment>,
        );

        ::starlark::starlark_simple_value!($value);

        impl $value {
            /// The wrapped fragment.
            pub fn fragment(&self) -> &$fragment {
                self.0
                    .downcast_ref::<$fragment>()
                    .expect(concat!("fragment of class ", $java_name))
            }

            /// Allocates the Starlark value of `fragment`.
            pub fn alloc<'v>(
                fragment: &::std::sync::Arc<dyn ::blaze_build_config::fragment::Fragment>,
                heap: ::starlark::values::Heap<'v>,
            ) -> ::starlark::values::Value<'v> {
                heap.alloc($value(fragment.clone()))
            }
        }

        impl ::std::fmt::Display for $value {
            // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(concat!("<unknown object ", $java_name, ">"))
            }
        }

        #[::starlark::values::starlark_value(type = $starlark_name)]
        impl<'v> ::starlark::values::StarlarkValue<'v> for $value {
            fn get_methods() -> Option<&'static ::starlark::environment::Methods> {
                static RES: ::starlark::environment::MethodsStatic =
                    ::starlark::environment::MethodsStatic::new($starlark_name, $methods);
                Some(RES.methods())
            }

            // Java: Object.equals (fragments do not override it).
            fn equals(&self, other: ::starlark::values::Value<'v>) -> starlark::Result<bool> {
                Ok(::starlark::values::ValueLike::downcast_ref::<$value>(other)
                    .is_some_and(|o| ::std::sync::Arc::ptr_eq(&o.0, &self.0)))
            }
        }
    };
}

pub(crate) use fragment_value;
