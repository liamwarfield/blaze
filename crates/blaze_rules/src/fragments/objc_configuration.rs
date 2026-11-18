//! The Starlark values of the `objc` fragment (`rules/objc/ObjcConfiguration.java`,
//! `starlarkbuildapi/apple/ObjcConfigurationApi.java`) and the `j2objc` fragment
//! (`rules/objc/J2ObjcConfiguration.java`, `starlarkbuildapi/apple/J2ObjcConfigurationApi.java`).

use blaze_build_config::fragments::j2objc_configuration::{
    J2OBJC_CONFIGURATION, J2ObjcConfiguration,
};
use blaze_build_config::fragments::objc_configuration::{OBJC_CONFIGURATION, ObjcConfiguration};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::builtins::java_common::fail_if_called_outside_default_allowlist;
use blaze_packages::types::{AttrValue, Type};
use starlark::environment::MethodsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::none::NoneOr;

use super::fragment_value;
use crate::apple::StarlarkDottedVersion;
use crate::python::py_builtins::ctx_param;

fragment_value!(
    /// `ctx.fragments.objc`: a configuration fragment for Objective-C.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:39 ObjcConfiguration
    ObjcConfigurationValue,
    ObjcConfiguration,
    "objc",
    "com.google.devtools.build.lib.rules.objc.ObjcConfiguration",
    objc_configuration_methods
);

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
    super::register(&OBJC_CONFIGURATION, ObjcConfigurationValue::alloc);
    super::register(&J2OBJC_CONFIGURATION, J2ObjcConfigurationValue::alloc);
}

#[starlark_module]
fn objc_configuration_methods(builder: &mut MethodsBuilder) {
    /// The type of device (e.g. 'iPhone 6') to use when running on the simulator.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:92 ObjcConfiguration.getIosSimulatorDevice
    #[starlark(attribute)]
    fn ios_simulator_device(this: &ObjcConfigurationValue) -> starlark::Result<NoneOr<String>> {
        Ok(match this.fragment().get_ios_simulator_device() {
            Some(d) => NoneOr::Other(d.to_owned()),
            None => NoneOr::None,
        })
    }

    /// The SDK version of the iOS simulator to use when running on the simulator.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:98 ObjcConfiguration.getIosSimulatorVersion
    #[starlark(attribute)]
    fn ios_simulator_version(
        this: &ObjcConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(match this.fragment().get_ios_simulator_version() {
            Some(v) => NoneOr::Other(StarlarkDottedVersion(v.clone())),
            None => NoneOr::None,
        })
    }

    /// Returns a boolean indicating whether memleaks should be run during tests or not.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:104 ObjcConfiguration.runMemleaks
    #[starlark(attribute)]
    fn run_memleaks(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().run_memleaks())
    }

    /// Returns a list of default options to use for compiling Objective-C in the current mode.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:119 ObjcConfiguration.getCoptsForCompilationMode
    #[starlark(attribute)]
    fn copts_for_current_compilation_mode(
        this: &ObjcConfigurationValue,
    ) -> starlark::Result<Vec<String>> {
        Ok(this.fragment().get_copts_for_compilation_mode())
    }

    /// Returns the flag-supplied certificate name to be used in signing, or None if no such
    /// certificate was specified.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:144 ObjcConfiguration.getSigningCertName
    #[starlark(attribute)]
    fn signing_certificate_name(this: &ObjcConfigurationValue) -> starlark::Result<NoneOr<String>> {
        Ok(match this.fragment().get_signing_cert_name() {
            Some(n) => NoneOr::Other(n.to_owned()),
            None => NoneOr::None,
        })
    }

    /// Returns whether device debug entitlements should be included when signing an
    /// application.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:155 ObjcConfiguration.useDeviceDebugEntitlements
    #[starlark(attribute)]
    fn uses_device_debug_entitlements(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_device_debug_entitlements())
    }

    /// Returns whether sdk_frameworks and weak_sdk_frameworks are disallowed attributes.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:161 ObjcConfiguration.disallowSdkFrameworksAttributes
    #[starlark(attribute)]
    fn disallow_sdk_frameworks_attributes(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().disallow_sdk_frameworks_attributes())
    }

    /// Returns whether objc_library and objc_import should default to alwayslink=True.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:167 ObjcConfiguration.alwayslinkByDefault
    #[starlark(attribute)]
    fn alwayslink_by_default(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().alwayslink_by_default())
    }

    /// Looks at any explicit value for alwayslink on ctx and then falls back to the value of
    /// alwayslink_by_default.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:176 ObjcConfiguration.targetShouldAlwayslink
    fn target_should_alwayslink<'v>(
        this: &ObjcConfigurationValue,
        ctx: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let rule_context = ctx_param("target_should_alwayslink", ctx)?;
        fail_if_called_outside_default_allowlist(eval).map_err(starlark::Error::new_other)?;

        let attributes = rule_context.attributes();
        if attributes.is_attribute_value_explicitly_specified("alwayslink") {
            return Ok(match attributes.get("alwayslink", Type::Boolean) {
                Some(AttrValue::Bool(b)) => b,
                other => panic!(
                    "ClassCastException: attribute alwayslink of {} is not a boolean: {other:?}",
                    rule_context.get_label()
                ),
            });
        }

        Ok(this.fragment().alwayslink_by_default())
    }

    /// Returns whether executable strip action should use flag -x, which does not break dynamic
    /// symbol resolution.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:193 ObjcConfiguration.stripExecutableSafely
    #[starlark(attribute)]
    fn strip_executable_safely(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().strip_executable_safely())
    }

    /// Returns whether to emit a strip action as part of objc linking.
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:199 ObjcConfiguration.builtinObjcStripAction
    #[starlark(attribute)]
    fn builtin_objc_strip_action(this: &ObjcConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().builtin_objc_strip_action())
    }
}

#[starlark_module]
fn j2objc_configuration_methods(builder: &mut MethodsBuilder) {
    /// The list of flags to be used when the j2objc compiler is invoked.
    // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:89 J2ObjcConfiguration.getTranslationFlags
    #[starlark(attribute)]
    fn translation_flags(this: &J2ObjcConfigurationValue) -> starlark::Result<Vec<String>> {
        Ok(this.fragment().get_translation_flags().to_vec())
    }
}
