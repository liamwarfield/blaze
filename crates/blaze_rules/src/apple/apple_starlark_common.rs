//! `_builtins.internal.apple_common`: port of `rules/objc/AppleStarlarkCommon.java` (the
//! Starlark API of `starlarkbuildapi/objc/AppleCommonApi.java`).
//!
//! `@_builtins//:common/objc/apple_common.bzl` builds the public `apple_common` struct in
//! Starlark and only forwards `dotted_version` to this object; `common/xcode/providers.bzl`
//! (`apple_common.XcodeVersionConfig`'s init) calls `dotted_version` too. At load time the object
//! is only read into module globals, like W3's stand-in.

use std::fmt;

use allocative::Allocative;
use blaze_optdefs::fragment::dotted_version::DottedVersion;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value};

use super::apple_platform::ApplePlatform;
use super::dotted_version::StarlarkDottedVersion;

/// A class that exposes apple rule implementation internals to Starlark.
// Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:28 AppleStarlarkCommon
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct AppleStarlarkCommon;

starlark_simple_value!(AppleStarlarkCommon);

// Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:32 AppleStarlarkCommon.BAD_KEY_ERROR
pub const BAD_KEY_ERROR: &str =
    "Argument %s not a recognized key, 'strict_include', or 'providers'.";

// Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:36 AppleStarlarkCommon.NOT_SET_ERROR
pub const NOT_SET_ERROR: &str = "Value for key %s must be a set, instead found %s.";

/// Java's default `StarlarkValue.repr`.
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
const APPLE_COMMON_REPR: &str =
    "<unknown object com.google.devtools.build.lib.rules.objc.AppleStarlarkCommon>";

impl fmt::Display for AppleStarlarkCommon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(APPLE_COMMON_REPR)
    }
}

impl AppleStarlarkCommon {
    // Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:43 AppleStarlarkCommon.getPlatformStruct
    // PORT: Java caches the struct in a field; it is a struct of enum constants, so building it
    // per access gives an equal value.
    pub fn get_platform_struct<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        ApplePlatform::get_starlark_struct(heap)
    }

    // Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:51 AppleStarlarkCommon.dottedVersion
    pub fn dotted_version(&self, version: &str) -> starlark::Result<DottedVersion> {
        DottedVersion::from_string(version)
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{}", e.get_message())))
    }
}

/// `new AppleStarlarkCommon()`, frozen: `_builtins.internal.apple_common` for the
/// analysis-capable builtins environment (`blaze_rules::bazel_environment`).
// Java: com/google/devtools/build/lib/bazel/rules/ObjcRules.java:36 ObjcRules.init
pub fn apple_common_value() -> OwnedFrozenValue {
    blaze_packages::builtins::env::freeze_value(|m| m.heap().alloc(AppleStarlarkCommon))
}

#[starlark_value(type = "apple_common")]
impl<'v> StarlarkValue<'v> for AppleStarlarkCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("apple_common", apple_common_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(APPLE_COMMON_REPR);
    }
}

/// The `AppleCommonApi` defaults that only exist for documentation (Java
/// `throw new UnsupportedOperationException(); // just for docs`): reaching one crashes Bazel.
fn just_for_docs(method: &str) -> ! {
    panic!("UnsupportedOperationException: AppleCommonApi.{method} (just for docs)")
}

#[allow(non_snake_case)]
#[starlark_module]
fn apple_common_methods(builder: &mut MethodsBuilder) {
    /// Utilities for resolving items from the apple toolchain.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:43 AppleCommonApi.getAppleToolchain
    fn apple_toolchain<'v>(this: &AppleStarlarkCommon) -> starlark::Result<Value<'v>> {
        let _ = this;
        just_for_docs("getAppleToolchain")
    }

    /// An enum-like struct that contains the fields corresponding to Apple platform types.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:60 AppleCommonApi.getPlatformTypeStruct
    #[starlark(attribute)]
    fn platform_type<'v>(this: &AppleStarlarkCommon) -> starlark::Result<Value<'v>> {
        let _ = this;
        just_for_docs("getPlatformTypeStruct")
    }

    /// An enum-like struct that contains the fields corresponding to Apple platforms.
    // Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:43 AppleStarlarkCommon.getPlatformStruct
    #[starlark(attribute)]
    fn platform<'v>(this: &AppleStarlarkCommon, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(this.get_platform_struct(heap))
    }

    /// The constructor/key for the XcodeVersionProperties provider.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:96 AppleCommonApi.getXcodeVersionPropertiesConstructor
    #[starlark(attribute)]
    fn XcodeProperties<'v>(this: &AppleStarlarkCommon) -> starlark::Result<Value<'v>> {
        let _ = this;
        just_for_docs("getXcodeVersionPropertiesConstructor")
    }

    /// The constructor/key for the XcodeVersionConfig provider.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:104 AppleCommonApi.getXcodeVersionConfigConstructor
    #[starlark(attribute)]
    fn XcodeVersionConfig<'v>(this: &AppleStarlarkCommon) -> starlark::Result<Value<'v>> {
        let _ = this;
        just_for_docs("getXcodeVersionConfigConstructor")
    }

    /// Returns a dict of environment variables that should be set for actions that need to run
    /// build tools on an Apple host system.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:122 AppleCommonApi.getAppleHostSystemEnv
    fn apple_host_system_env<'v>(
        this: &AppleStarlarkCommon,
        #[starlark(require = pos)] xcode_config: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = (this, xcode_config);
        just_for_docs("getAppleHostSystemEnv")
    }

    /// Returns a dict of environment variables that should be set for actions that build
    /// targets of the given Apple platform type.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:141 AppleCommonApi.getTargetAppleEnvironment
    fn target_apple_env<'v>(
        this: &AppleStarlarkCommon,
        #[starlark(require = pos)] xcode_config: Value<'v>,
        #[starlark(require = pos)] platform: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = (this, xcode_config, platform);
        just_for_docs("getTargetAppleEnvironment")
    }

    /// Creates a new DottedVersion instance.
    // Java: com/google/devtools/build/lib/rules/objc/AppleStarlarkCommon.java:51 AppleStarlarkCommon.dottedVersion
    fn dotted_version(
        this: &AppleStarlarkCommon,
        version: &str,
    ) -> starlark::Result<StarlarkDottedVersion> {
        Ok(StarlarkDottedVersion(this.dotted_version(version)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_version_parses() {
        let v = AppleStarlarkCommon.dotted_version("10.11").unwrap();
        assert_eq!(v.to_string(), "10.11");
    }

    #[test]
    fn dotted_version_rejects_bad_versions() {
        let e = AppleStarlarkCommon.dotted_version("foo").unwrap_err();
        assert!(
            e.to_string()
                .contains("Dotted version components must all start with the form"),
            "{e}"
        );
    }
}
