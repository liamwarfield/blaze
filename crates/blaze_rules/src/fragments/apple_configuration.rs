//! The Starlark value of the `apple` fragment (`rules/apple/AppleConfiguration.java`,
//! `starlarkbuildapi/apple/AppleConfigurationApi.java`). `xcode_config` (apple_support) reads it
//! in both corpus projects; rules_cc's `unix_cc_toolchain_config` reads `single_arch_platform`.

use std::hash::Hash;

use blaze_build_config::fragments::apple_configuration::{
    APPLE_CONFIGURATION, AppleConfiguration, AppleConfigurationAppleCpus,
};
use blaze_optdefs::fragment::apple_platform::apple_platform_platform_type as platform_type;
use blaze_optdefs::fragment::dotted_version::DottedVersion;
use blaze_options::definition::OptionsBase;
use blaze_starlark::structs::{StarlarkInfo, StructProvider};
use starlark::collections::StarlarkHasher;
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneOr;
use starlark::values::tuple::AllocTuple;
use starlark::values::{Heap, Value};

use super::fragment_value;
use crate::apple::{ApplePlatform, StarlarkDottedVersion};

fragment_value!(
    /// `ctx.fragments.apple`: a configuration fragment for Apple platforms.
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:48 AppleConfiguration
    AppleConfigurationValue,
    AppleConfiguration,
    "apple",
    "com.google.devtools.build.lib.rules.apple.AppleConfiguration",
    apple_configuration_methods,
    equals = apple_configuration_equals,
    hash = apple_configuration_hash
);

/// Two `AppleConfiguration`s are equal when their options are (Java `OptionsBase.equals`: same
/// class and equal option values).
// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:351 AppleConfiguration.equals
fn apple_configuration_equals(this: &AppleConfiguration, that: &AppleConfiguration) -> bool {
    // `PartialEq` of an options class is `OptionsBase.equals` (blaze_options' `options!`).
    this.get_options() == that.get_options()
}

/// Java `options.hashCode()` (`OptionsBase.hashCode`: the class's hash plus `asMap().hashCode()`).
///
/// PORT: Java's hash value is never observable; any function of the option values agrees with
/// `apple_configuration_equals`. The options' cache key (`OptionsBase.cacheKey`, a rendering of
/// `asMap()`) is hashed.
// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:362 AppleConfiguration.hashCode
fn apple_configuration_hash(this: &AppleConfiguration, hasher: &mut StarlarkHasher) {
    <dyn OptionsBase>::cache_key(this.get_options()).hash(hasher);
}

pub(super) fn register() {
    super::register(&APPLE_CONFIGURATION, AppleConfigurationValue::alloc);
}

/// Java `AppleConfiguration.getPrefixedAppleCpu` (private there; the Rust fragment keeps it
/// private too).
// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:229 AppleConfiguration.getPrefixedAppleCpu
fn get_prefixed_apple_cpu<'a>(
    apple_platform_type: &str,
    apple_cpus: &'a AppleConfigurationAppleCpus,
) -> &'a str {
    if !apple_cpus.apple_split_cpu().is_empty() {
        return apple_cpus.apple_split_cpu();
    }
    let first = |cpus: &'a [String]| -> &'a str {
        cpus.first().map(String::as_str).unwrap_or_else(|| {
            panic!("IndexOutOfBoundsException: Index 0 out of bounds for length 0")
        })
    };
    match apple_platform_type {
        platform_type::IOS => first(apple_cpus.ios_multi_cpus()),
        platform_type::VISIONOS => first(apple_cpus.visionos_cpus()),
        platform_type::WATCHOS => first(apple_cpus.watchos_cpus()),
        platform_type::TVOS => first(apple_cpus.tvos_cpus()),
        platform_type::MACOS => first(apple_cpus.macos_cpus()),
        platform_type::CATALYST => first(apple_cpus.catalyst_cpus()),
        _ => panic!("IllegalArgumentException: Unhandled platform type {apple_platform_type}"),
    }
}

/// Gets the single "effective" platform for this configuration's `PlatformType` and
/// architecture.
// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:256 AppleConfiguration.getSingleArchPlatform
pub fn get_single_arch_platform(config: &AppleConfiguration) -> ApplePlatform {
    let apple_platform_type = config.get_apple_platform_type();
    ApplePlatform::for_target(
        apple_platform_type,
        get_prefixed_apple_cpu(apple_platform_type, config.get_apple_cpus()),
    )
}

/// An optional `DottedVersion` result (`allowReturnNones = true`).
fn dotted(v: Option<&DottedVersion>) -> NoneOr<StarlarkDottedVersion> {
    match v {
        Some(v) => NoneOr::Other(StarlarkDottedVersion(v.clone())),
        None => NoneOr::None,
    }
}

#[starlark_module]
fn apple_configuration_methods(builder: &mut MethodsBuilder) {
    /// The single "effective" architecture for this configuration (e.g., i386 or arm64) in the
    /// context of rule logic that is only concerned with a single architecture.
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:212 AppleConfiguration.getSingleArchitecture
    #[starlark(attribute)]
    fn single_arch_cpu(this: &AppleConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_single_architecture())
    }

    /// The platform of the current configuration.
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:256 AppleConfiguration.getSingleArchPlatform
    #[starlark(attribute)]
    fn single_arch_platform(this: &AppleConfigurationValue) -> starlark::Result<ApplePlatform> {
        Ok(get_single_arch_platform(this.fragment()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:172 AppleConfiguration.getAppleCpusForStarlark
    #[starlark(attribute)]
    fn apple_cpus<'v>(
        this: &AppleConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let apple_cpus = this.fragment().get_apple_cpus();
        let tuple = |cpus: &[String]| -> Value<'v> {
            heap.alloc(AllocTuple(cpus.iter().map(|c| c.as_str())))
        };
        // Java fills a HashMap; StructProvider.STRUCT.create sorts the fields by name.
        let fields: Vec<(String, Value<'v>)> = vec![
            (
                "apple_split_cpu".to_owned(),
                heap.alloc(apple_cpus.apple_split_cpu()),
            ),
            (
                "ios_multi_cpus".to_owned(),
                tuple(apple_cpus.ios_multi_cpus()),
            ),
            (
                "visionos_cpus".to_owned(),
                tuple(apple_cpus.visionos_cpus()),
            ),
            ("watchos_cpus".to_owned(), tuple(apple_cpus.watchos_cpus())),
            ("tvos_cpus".to_owned(), tuple(apple_cpus.tvos_cpus())),
            ("macos_cpus".to_owned(), tuple(apple_cpus.macos_cpus())),
            (
                "catalyst_cpus".to_owned(),
                tuple(apple_cpus.catalyst_cpus()),
            ),
        ];
        let provider = heap.alloc(StructProvider);
        Ok(heap.alloc(StarlarkInfo::new(provider, fields)))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:185 AppleConfiguration.getApplePlatformType
    #[starlark(attribute)]
    fn apple_platform_type(this: &AppleConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_apple_platform_type().to_owned())
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:263 AppleConfiguration.getXcodeVersionFlag
    #[starlark(attribute)]
    fn xcode_version_flag(this: &AppleConfigurationValue) -> starlark::Result<NoneOr<String>> {
        Ok(match this.fragment().get_xcode_version_flag() {
            Some(v) => NoneOr::Other(v.to_owned()),
            None => NoneOr::None,
        })
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:268 AppleConfiguration.iosSdkVersionFlag
    #[starlark(attribute)]
    fn ios_sdk_version_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().ios_sdk_version_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:273 AppleConfiguration.macOsSdkVersionFlag
    #[starlark(attribute)]
    fn macos_sdk_version_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().mac_os_sdk_version_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:278 AppleConfiguration.tvOsSdkVersionFlag
    #[starlark(attribute)]
    fn tvos_sdk_version_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().tv_os_sdk_version_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:283 AppleConfiguration.watchOsSdkVersionFlag
    #[starlark(attribute)]
    fn watchos_sdk_version_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().watch_os_sdk_version_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:288 AppleConfiguration.iosMinimumOsFlag
    #[starlark(attribute)]
    fn ios_minimum_os_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().ios_minimum_os_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:293 AppleConfiguration.macOsMinimumOsFlag
    #[starlark(attribute)]
    fn macos_minimum_os_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().mac_os_minimum_os_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:298 AppleConfiguration.tvOsMinimumOsFlag
    #[starlark(attribute)]
    fn tvos_minimum_os_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().tv_os_minimum_os_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:303 AppleConfiguration.watchOsMinimumOsFlag
    #[starlark(attribute)]
    fn watchos_minimum_os_flag(
        this: &AppleConfigurationValue,
    ) -> starlark::Result<NoneOr<StarlarkDottedVersion>> {
        Ok(dotted(this.fragment().watch_os_minimum_os_flag()))
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:308 AppleConfiguration.shouldPreferMutualXcode
    #[starlark(attribute)]
    fn prefer_mutual_xcode(this: &AppleConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().should_prefer_mutual_xcode())
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:313 AppleConfiguration.includeXcodeExecRequirementsFlag
    #[starlark(attribute)]
    fn include_xcode_exec_requirements(this: &AppleConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().include_xcode_exec_requirements_flag())
    }
}

#[cfg(test)]
mod tests {
    use blaze_build_config::build_options::BuildOptions;
    use blaze_build_config::fragment_options_class::all_known_fragment_options_classes;

    use super::*;

    fn create(args: &[&str]) -> AppleConfiguration {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        AppleConfiguration::new(&options)
    }

    #[test]
    fn single_arch_platform_default_is_macos() {
        // Default --apple_platform_type is macos; the default macOS cpu maps to darwin_<arch>.
        assert_eq!(get_single_arch_platform(&create(&[])), ApplePlatform::MACOS);
    }

    #[test]
    fn single_arch_platform_ios_simulator() {
        let config = create(&["--apple_platform_type=ios", "--ios_multi_cpus=sim_arm64"]);
        assert_eq!(
            get_single_arch_platform(&config),
            ApplePlatform::IOS_SIMULATOR
        );
    }

    // Java: AppleFragmentTest.appleFragmentSingleArchCpuOnExtraExecPlatform
    #[test]
    #[ignore = "needs analysis test harness (BuildViewTestCase, extra execution platforms)"]
    fn apple_fragment_single_arch_cpu_on_extra_exec_platform() {}
}
