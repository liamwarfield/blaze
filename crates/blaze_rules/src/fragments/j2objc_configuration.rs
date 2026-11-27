//! The Starlark value of the `j2objc` fragment (`rules/objc/J2ObjcConfiguration.java`,
//! `starlarkbuildapi/apple/J2ObjcConfigurationApi.java`).

use blaze_build_config::fragments::j2objc_configuration::{
    J2OBJC_CONFIGURATION, J2ObjcConfiguration,
};
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;
use starlark::values::{Heap, Value};

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.j2objc`: a configuration fragment for j2Objc.
    // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:38 J2ObjcConfiguration
    J2ObjcConfigurationValue,
    J2ObjcConfiguration,
    "j2objc",
    "com.google.devtools.build.lib.rules.objc.J2ObjcConfiguration",
    j2objc_configuration_methods
);

pub(super) fn register() {
    super::register(&J2OBJC_CONFIGURATION, J2ObjcConfigurationValue::alloc);
}

#[starlark_module]
fn j2objc_configuration_methods(builder: &mut MethodsBuilder) {
    /// The list of flags to be used when the j2objc compiler is invoked.
    // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:89 J2ObjcConfiguration.getTranslationFlags
    #[starlark(attribute)]
    fn translation_flags<'v>(
        this: &J2ObjcConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let elems: Vec<Value<'v>> = this
            .fragment()
            .get_translation_flags()
            .iter()
            .map(|s| heap.alloc(s.as_str()))
            .collect();
        Ok(starlark::values::list::alloc_immutable_list(heap, &elems))
    }
}
