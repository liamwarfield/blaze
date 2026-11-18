//! Port of `rules/apple/ApplePlatform.java` (the enum, as the Starlark type `apple_platform` of
//! `starlarkbuildapi/apple/ApplePlatformApi.java`). Its `PlatformType` string constants are
//! `blaze_optdefs::fragment::apple_platform::apple_platform_platform_type`.

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_optdefs::fragment::apple_platform::apple_platform_platform_type as platform_type;
use blaze_starlark::structs::{DeeplyImmutable, ProviderInfo, StarlarkInfo};
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value,
};

// TODO(b/331163027): Remove this duplicate of common/objc/apple_platform.PLATFORM_TYPE
/// An enum that can be used to distinguish between various apple platforms.
///
/// The enum Platform is being migrated to a Starlark struct PLATFORM in
/// builtins_bzl/common/objc/apple_platform.bzl.
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:40 ApplePlatform
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub enum ApplePlatform {
    IOS_DEVICE,
    IOS_SIMULATOR,
    MACOS,
    TVOS_DEVICE,
    TVOS_SIMULATOR,
    VISIONOS_DEVICE,
    VISIONOS_SIMULATOR,
    WATCHOS_DEVICE,
    WATCHOS_SIMULATOR,
    CATALYST,
}

starlark_simple_value!(ApplePlatform);

// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:52 ApplePlatform.IOS_SIMULATOR_TARGET_CPUS
const IOS_SIMULATOR_TARGET_CPUS: &[&str] =
    &["ios_x86_64", "ios_i386", "ios_sim_arm64", "ios_sim_arm64e"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:54 ApplePlatform.IOS_DEVICE_TARGET_CPUS
const IOS_DEVICE_TARGET_CPUS: &[&str] = &[
    "ios_armv6",
    "ios_arm64",
    "ios_armv7",
    "ios_armv7s",
    "ios_arm64e",
];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:56 ApplePlatform.VISIONOS_SIMULATOR_TARGET_CPUS
const VISIONOS_SIMULATOR_TARGET_CPUS: &[&str] = &["visionos_sim_arm64", "visionos_sim_arm64e"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:58 ApplePlatform.VISIONOS_DEVICE_TARGET_CPUS
const VISIONOS_DEVICE_TARGET_CPUS: &[&str] = &["visionos_arm64", "visionos_arm64e"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:60 ApplePlatform.WATCHOS_SIMULATOR_TARGET_CPUS
const WATCHOS_SIMULATOR_TARGET_CPUS: &[&str] = &[
    "watchos_i386",
    "watchos_x86_64",
    "watchos_arm64",
    "watchos_sim_arm64e",
];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:62 ApplePlatform.WATCHOS_DEVICE_TARGET_CPUS
const WATCHOS_DEVICE_TARGET_CPUS: &[&str] = &[
    "watchos_armv7k",
    "watchos_arm64_32",
    "watchos_device_arm64",
    "watchos_device_arm64e",
];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:65 ApplePlatform.TVOS_SIMULATOR_TARGET_CPUS
const TVOS_SIMULATOR_TARGET_CPUS: &[&str] = &["tvos_x86_64", "tvos_sim_arm64", "tvos_sim_arm64e"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:67 ApplePlatform.TVOS_DEVICE_TARGET_CPUS
const TVOS_DEVICE_TARGET_CPUS: &[&str] = &["tvos_arm64", "tvos_arm64e"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:69 ApplePlatform.CATALYST_TARGET_CPUS
const CATALYST_TARGET_CPUS: &[&str] = &["catalyst_x86_64"];
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:71 ApplePlatform.MACOS_TARGET_CPUS
const MACOS_TARGET_CPUS: &[&str] = &["darwin_x86_64", "darwin_arm64", "darwin_arm64e"];

impl ApplePlatform {
    /// `ApplePlatform.values()`, in declaration order.
    pub const VALUES: [ApplePlatform; 10] = [
        ApplePlatform::IOS_DEVICE,
        ApplePlatform::IOS_SIMULATOR,
        ApplePlatform::MACOS,
        ApplePlatform::TVOS_DEVICE,
        ApplePlatform::TVOS_SIMULATOR,
        ApplePlatform::VISIONOS_DEVICE,
        ApplePlatform::VISIONOS_SIMULATOR,
        ApplePlatform::WATCHOS_DEVICE,
        ApplePlatform::WATCHOS_SIMULATOR,
        ApplePlatform::CATALYST,
    ];

    /// The constructor arguments `(starlarkKey, nameInPlist, platformType, isDevice)`.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:41 ApplePlatform.<init>
    fn data(self) -> (&'static str, &'static str, &'static str, bool) {
        match self {
            ApplePlatform::IOS_DEVICE => ("ios_device", "iPhoneOS", platform_type::IOS, true),
            ApplePlatform::IOS_SIMULATOR => (
                "ios_simulator",
                "iPhoneSimulator",
                platform_type::IOS,
                false,
            ),
            ApplePlatform::MACOS => ("macos", "MacOSX", platform_type::MACOS, true),
            ApplePlatform::TVOS_DEVICE => ("tvos_device", "AppleTVOS", platform_type::TVOS, true),
            ApplePlatform::TVOS_SIMULATOR => (
                "tvos_simulator",
                "AppleTVSimulator",
                platform_type::TVOS,
                false,
            ),
            ApplePlatform::VISIONOS_DEVICE => {
                ("visionos_device", "XROS", platform_type::VISIONOS, true)
            }
            ApplePlatform::VISIONOS_SIMULATOR => (
                "visionos_simulator",
                "XRSimulator",
                platform_type::VISIONOS,
                false,
            ),
            ApplePlatform::WATCHOS_DEVICE => {
                ("watchos_device", "WatchOS", platform_type::WATCHOS, true)
            }
            ApplePlatform::WATCHOS_SIMULATOR => (
                "watchos_simulator",
                "WatchSimulator",
                platform_type::WATCHOS,
                false,
            ),
            ApplePlatform::CATALYST => ("catalyst", "MacOSX", platform_type::CATALYST, true),
        }
    }

    /// Java `name()` of the enum constant.
    fn enum_name(self) -> &'static str {
        match self {
            ApplePlatform::IOS_DEVICE => "IOS_DEVICE",
            ApplePlatform::IOS_SIMULATOR => "IOS_SIMULATOR",
            ApplePlatform::MACOS => "MACOS",
            ApplePlatform::TVOS_DEVICE => "TVOS_DEVICE",
            ApplePlatform::TVOS_SIMULATOR => "TVOS_SIMULATOR",
            ApplePlatform::VISIONOS_DEVICE => "VISIONOS_DEVICE",
            ApplePlatform::VISIONOS_SIMULATOR => "VISIONOS_SIMULATOR",
            ApplePlatform::WATCHOS_DEVICE => "WATCHOS_DEVICE",
            ApplePlatform::WATCHOS_SIMULATOR => "WATCHOS_SIMULATOR",
            ApplePlatform::CATALYST => "CATALYST",
        }
    }

    /// Returns the name aka starlarkKey of this platform.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:93 ApplePlatform.getName
    pub fn get_name(self) -> &'static str {
        self.data().0
    }

    /// Returns the platform type of this platform.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:98 ApplePlatform.getType
    pub fn get_type(self) -> &'static str {
        self.data().2
    }

    /// Returns true if this platform is a device platform, or false if this is a simulator
    /// platform.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:103 ApplePlatform.isDevice
    pub fn is_device(self) -> bool {
        self.data().3
    }

    /// Returns the name of the "platform" as it appears in the CFBundleSupportedPlatforms plist
    /// setting.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:108 ApplePlatform.getNameInPlist
    pub fn get_name_in_plist(self) -> &'static str {
        self.data().1
    }

    /// Returns the name of the "platform" as it appears in the plist when it appears in
    /// all-lowercase.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:115 ApplePlatform.getLowerCaseNameInPlist
    pub fn get_lower_case_name_in_plist(self) -> String {
        self.get_name_in_plist().to_lowercase()
    }

    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:120 ApplePlatform.forTargetCpuNullable
    fn for_target_cpu_nullable(target_cpu: &str) -> Option<ApplePlatform> {
        if IOS_SIMULATOR_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::IOS_SIMULATOR)
        } else if IOS_DEVICE_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::IOS_DEVICE)
        } else if VISIONOS_SIMULATOR_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::VISIONOS_SIMULATOR)
        } else if VISIONOS_DEVICE_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::VISIONOS_DEVICE)
        } else if WATCHOS_SIMULATOR_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::WATCHOS_SIMULATOR)
        } else if WATCHOS_DEVICE_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::WATCHOS_DEVICE)
        } else if TVOS_SIMULATOR_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::TVOS_SIMULATOR)
        } else if TVOS_DEVICE_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::TVOS_DEVICE)
        } else if CATALYST_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::CATALYST)
        } else if MACOS_TARGET_CPUS.contains(&target_cpu) {
            Some(ApplePlatform::MACOS)
        } else {
            None
        }
    }

    /// Returns the platform cpu string for the given target cpu and platform type.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:152 ApplePlatform.cpuStringForTarget
    fn cpu_string_for_target(platform_type: &str, arch: &str) -> String {
        match platform_type {
            platform_type::MACOS => format!("darwin_{arch}"),
            _ => format!("{platform_type}_{arch}"),
        }
    }

    /// Returns the platform for the given target cpu and platform type.
    ///
    /// Panics (Java `IllegalArgumentException`) if there is no valid apple platform for the given
    /// target cpu.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:168 ApplePlatform.forTarget
    pub fn for_target(platform_type: &str, arch: &str) -> ApplePlatform {
        Self::for_target_cpu(&Self::cpu_string_for_target(platform_type, arch))
    }

    /// Returns the platform for the given target cpu.
    ///
    /// Panics (Java `IllegalArgumentException`) if there is no valid apple platform for the given
    /// target cpu.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:178 ApplePlatform.forTargetCpu
    pub fn for_target_cpu(target_cpu: &str) -> ApplePlatform {
        match Self::for_target_cpu_nullable(target_cpu) {
            Some(platform) => platform,
            None => panic!(
                "IllegalArgumentException: No supported apple platform registered for target cpu \
                 {target_cpu}"
            ),
        }
    }

    /// Returns a Starlark struct that contains the instances of this enum.
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:189 ApplePlatform.getStarlarkStruct
    pub fn get_starlark_struct<'v>(heap: Heap<'v>) -> Value<'v> {
        let constructor = heap.alloc(PlatformsProvider);
        let fields: Vec<(String, Value<'v>)> = Self::VALUES
            .iter()
            .map(|t| (t.get_name().to_owned(), heap.alloc(*t)))
            .collect();
        heap.alloc(StarlarkInfo::new(constructor, fields))
    }
}

impl fmt::Display for ApplePlatform {
    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:199 ApplePlatform.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.enum_name().to_ascii_lowercase())
    }
}

#[starlark_value(type = "apple_platform")]
impl<'v> StarlarkValue<'v> for ApplePlatform {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("apple_platform", apple_platform_methods);
        Some(RES.methods())
    }

    // Java: Object.equals of an enum constant (identity).
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.downcast_ref::<ApplePlatform>() == Some(self))
    }

    // Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:88 ApplePlatform.isImmutable
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.enum_name().hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn apple_platform_methods(builder: &mut MethodsBuilder) {
    /// Returns the name aka starlarkKey of this platform.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/apple/ApplePlatformApi.java:56 ApplePlatformApi.getName
    #[starlark(attribute)]
    fn name(this: &ApplePlatform) -> starlark::Result<&'static str> {
        Ok(this.get_name())
    }

    /// Returns the platform type of this platform.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/apple/ApplePlatformApi.java:63 ApplePlatformApi.getType
    #[starlark(attribute)]
    fn platform_type(this: &ApplePlatform) -> starlark::Result<&'static str> {
        Ok(this.get_type())
    }

    /// Returns True if this platform is a device platform or False if it is a simulator
    /// platform.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/apple/ApplePlatformApi.java:73 ApplePlatformApi.isDevice
    #[starlark(attribute)]
    fn is_device(this: &ApplePlatform) -> starlark::Result<bool> {
        Ok(ApplePlatform::is_device(*this))
    }

    /// The name of the platform as it appears in the CFBundleSupportedPlatforms entry of an
    /// Info.plist file and in Xcode's platforms directory, without the extension.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/apple/ApplePlatformApi.java:88 ApplePlatformApi.getNameInPlist
    #[starlark(attribute)]
    fn name_in_plist(this: &ApplePlatform) -> starlark::Result<&'static str> {
        Ok(this.get_name_in_plist())
    }
}

/// The anonymous `BuiltinProvider<StructImpl>("platforms", StructImpl.class)` that
/// `getStarlarkStruct` creates its struct with.
// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:190 ApplePlatform.getStarlarkStruct
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
struct PlatformsProvider;

starlark_simple_value!(PlatformsProvider);

impl fmt::Display for PlatformsProvider {
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java BuiltinProvider.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<function platforms>")
    }
}

#[starlark_value(type = "Provider")]
impl<'v> StarlarkValue<'v> for PlatformsProvider {
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(ProviderInfo {
            printable_name: "platforms".to_owned(),
            exported: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_target_macos() {
        assert_eq!(
            ApplePlatform::for_target(platform_type::MACOS, "x86_64"),
            ApplePlatform::MACOS
        );
        assert_eq!(
            ApplePlatform::for_target(platform_type::IOS, "sim_arm64"),
            ApplePlatform::IOS_SIMULATOR
        );
        assert_eq!(
            ApplePlatform::for_target(platform_type::IOS, "arm64"),
            ApplePlatform::IOS_DEVICE
        );
    }

    #[test]
    #[should_panic(expected = "No supported apple platform registered for target cpu ios_foo")]
    fn for_target_cpu_unknown() {
        ApplePlatform::for_target_cpu("ios_foo");
    }

    #[test]
    fn repr_is_lower_case_enum_name() {
        assert_eq!(ApplePlatform::IOS_SIMULATOR.to_string(), "ios_simulator");
        assert_eq!(ApplePlatform::MACOS.get_type(), "macos");
        assert_eq!(ApplePlatform::MACOS.get_name_in_plist(), "MacOSX");
    }
}
