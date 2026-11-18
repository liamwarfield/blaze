//! Port of `rules/apple/AppleConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::apple_command_line_options` and its `ConfigurationDistinguisher`
//! enum is `blaze_optdefs::fragment::apple_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::apple_command_line_options::{
    AppleCommandLineOptions, DEFAULT_CATALYST_CPU, DEFAULT_MACOS_CPU, DEFAULT_TVOS_CPU,
    DEFAULT_VISIONOS_CPU, DEFAULT_WATCHOS_CPU,
};
use blaze_optdefs::fragment::apple_configuration::AppleConfigurationConfigurationDistinguisher;
use blaze_optdefs::fragment::apple_platform::apple_platform_platform_type as platform_type;
use blaze_optdefs::fragment::dotted_version::DottedVersion;

use crate::build_options::BuildOptions;
use crate::fragment::{
    AddToMnemonicException, Fragment, FragmentClass, FragmentClassInfo,
    InvalidConfigurationException, OutputDirectoriesContext, required,
};
use crate::fragment_options_class::APPLE_COMMAND_LINE_OPTIONS;

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:50 AppleConfiguration.DEVELOPER_DIR_ENV_NAME
/// Environment variable name for the developer dir of the selected Xcode.
pub const DEVELOPER_DIR_ENV_NAME: &str = "DEVELOPER_DIR";

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:57 AppleConfiguration.XCODE_VERSION_ENV_NAME
/// Environment variable name for the Xcode version. The value of this environment variable
/// should be set to the version (for example, "7.2") of Xcode to use when invoking part of the
/// apple toolkit in action execution.
pub const XCODE_VERSION_ENV_NAME: &str = "XCODE_VERSION_OVERRIDE";

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:63 AppleConfiguration.APPLE_SDK_PLATFORM_ENV_NAME
/// Environment variable name for the apple SDK platform. This should be set for all actions that
/// require an apple SDK. The valid values consist of `ApplePlatform` names.
pub const APPLE_SDK_PLATFORM_ENV_NAME: &str = "APPLE_SDK_PLATFORM";

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:66 AppleConfiguration.SIMULATOR_ENVIRONMENT_CPU_PREFIX
/// Prefix for simulator environment cpu values
pub const SIMULATOR_ENVIRONMENT_CPU_PREFIX: &str = "sim_";

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:69 AppleConfiguration.DEVICE_ENVIRONMENT_CPU_PREFIX
/// Prefix for device environment cpu values
pub const DEVICE_ENVIRONMENT_CPU_PREFIX: &str = "device_";

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:73 AppleConfiguration.DEFAULT_IOS_CPU
/// Default cpu for iOS builds.
// PORT: `CPU.getCurrent() == CPU.AARCH64` is the architecture Blaze runs on.
pub const DEFAULT_IOS_CPU: &str = if cfg!(target_arch = "aarch64") {
    "sim_arm64"
} else {
    "x86_64"
};

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:118 AppleConfiguration.AppleCpus
/// A class that contains information pertaining to Apple CPUs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppleConfigurationAppleCpus {
    apple_split_cpu: String,
    ios_multi_cpus: Vec<String>,
    visionos_cpus: Vec<String>,
    watchos_cpus: Vec<String>,
    tvos_cpus: Vec<String>,
    macos_cpus: Vec<String>,
    catalyst_cpus: Vec<String>,
}

impl AppleConfigurationAppleCpus {
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:119 AppleConfiguration.AppleCpus.create
    pub fn create(options: &AppleCommandLineOptions) -> AppleConfigurationAppleCpus {
        let or_default = |cpus: &Vec<String>, default: &str| -> Vec<String> {
            if cpus.is_empty() {
                vec![default.to_string()]
            } else {
                cpus.clone()
            }
        };
        let apple_split_cpu = options.apple_split_cpu.clone();
        let ios_multi_cpus = or_default(&options.ios_multi_cpus, DEFAULT_IOS_CPU);
        let visionos_cpus = or_default(&options.visionos_cpus, DEFAULT_VISIONOS_CPU);
        let watchos_cpus = or_default(&options.watchos_cpus, DEFAULT_WATCHOS_CPU);
        let tvos_cpus = or_default(&options.tvos_cpus, DEFAULT_TVOS_CPU);
        let macos_cpus = or_default(&options.macos_cpus, DEFAULT_MACOS_CPU);
        let catalyst_cpus = or_default(&options.catalyst_cpus, DEFAULT_CATALYST_CPU);

        AppleConfigurationAppleCpus {
            apple_split_cpu,
            ios_multi_cpus,
            visionos_cpus,
            watchos_cpus,
            tvos_cpus,
            macos_cpus,
            catalyst_cpus,
        }
    }

    pub fn apple_split_cpu(&self) -> &str {
        &self.apple_split_cpu
    }

    pub fn ios_multi_cpus(&self) -> &[String] {
        &self.ios_multi_cpus
    }

    pub fn visionos_cpus(&self) -> &[String] {
        &self.visionos_cpus
    }

    pub fn watchos_cpus(&self) -> &[String] {
        &self.watchos_cpus
    }

    pub fn tvos_cpus(&self) -> &[String] {
        &self.tvos_cpus
    }

    pub fn macos_cpus(&self) -> &[String] {
        &self.macos_cpus
    }

    pub fn catalyst_cpus(&self) -> &[String] {
        &self.catalyst_cpus
    }
}

// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:48 AppleConfiguration
/// A configuration containing flags required for Apple platforms and tools.
///
/// PORT: Java's `equals`/`hashCode` compare the options only; fragments are not compared in
/// Blaze, so they are not ported.
#[derive(Debug)]
pub struct AppleConfiguration {
    apple_platform_type: String,
    configuration_distinguisher: AppleConfigurationConfigurationDistinguisher,
    xcode_config_label: Label,
    options: AppleCommandLineOptions,
    apple_cpus: AppleConfigurationAppleCpus,
    xcode_version_flag: Option<String>,
    ios_sdk_version_flag: Option<DottedVersion>,
    mac_os_sdk_version_flag: Option<DottedVersion>,
    tv_os_sdk_version_flag: Option<DottedVersion>,
    watch_os_sdk_version_flag: Option<DottedVersion>,
    ios_minimum_os_flag: Option<DottedVersion>,
    macos_minimum_os_flag: Option<DottedVersion>,
    tvos_minimum_os_flag: Option<DottedVersion>,
    watchos_minimum_os_flag: Option<DottedVersion>,
    prefer_mutual_xcode: bool,
    include_xcode_exec_requirements: bool,
}

/// `AppleConfiguration.class`.
pub static APPLE_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.apple.AppleConfiguration",
    starlark_name: Some("apple"),
    required_options: || vec![&APPLE_COMMAND_LINE_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(AppleConfiguration::new(options));
    Ok(fragment)
}

impl AppleConfiguration {
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:92 AppleConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> AppleConfiguration {
        let options = required::<AppleCommandLineOptions>(build_options);
        AppleConfiguration {
            options: options.clone(),
            apple_cpus: AppleConfigurationAppleCpus::create(options),
            apple_platform_type: options.apple_platform_type.clone(),
            configuration_distinguisher: options.configuration_distinguisher,
            xcode_config_label: options.xcode_version_config.get().clone(),
            // AppleConfiguration should not have this knowledge. This is a temporary workaround
            // for Starlarkification, until apple rules are toolchainized.
            xcode_version_flag: options.xcode_version.clone(),
            ios_sdk_version_flag: DottedVersion::maybe_unwrap(options.ios_sdk_version.as_ref()),
            mac_os_sdk_version_flag: DottedVersion::maybe_unwrap(
                options.mac_os_sdk_version.as_ref(),
            ),
            tv_os_sdk_version_flag: DottedVersion::maybe_unwrap(options.tv_os_sdk_version.as_ref()),
            watch_os_sdk_version_flag: DottedVersion::maybe_unwrap(
                options.watch_os_sdk_version.as_ref(),
            ),
            ios_minimum_os_flag: DottedVersion::maybe_unwrap(options.ios_minimum_os.as_ref()),
            macos_minimum_os_flag: DottedVersion::maybe_unwrap(options.macos_minimum_os.as_ref()),
            tvos_minimum_os_flag: DottedVersion::maybe_unwrap(options.tvos_minimum_os.as_ref()),
            watchos_minimum_os_flag: DottedVersion::maybe_unwrap(
                options.watchos_minimum_os.as_ref(),
            ),
            prefer_mutual_xcode: options.prefer_mutual_xcode,
            include_xcode_exec_requirements: options.include_xcode_execution_requirements,
        }
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:172 AppleConfiguration.getAppleCpusForStarlark
    /// The `AppleCpus` behind `apple.apple_cpus` (Java builds a Starlark struct of it).
    pub fn get_apple_cpus(&self) -> &AppleConfigurationAppleCpus {
        &self.apple_cpus
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:185 AppleConfiguration.getApplePlatformType
    pub fn get_apple_platform_type(&self) -> &str {
        &self.apple_platform_type
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:189 AppleConfiguration.getOptions
    pub fn get_options(&self) -> &AppleCommandLineOptions {
        &self.options
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:212 AppleConfiguration.getSingleArchitecture
    /// Gets the single "effective" architecture for this configuration's `PlatformType` (for
    /// example, "i386" or "arm64").
    ///
    /// Single effective architecture is determined using the following rules:
    ///
    /// 1. If `--apple_split_cpu` is set (done via prior configuration transition), then that is
    ///    the effective architecture.
    /// 2. If the multi cpus flag (e.g. `--ios_multi_cpus`) is set and non-empty, then the first
    ///    such architecture is returned.
    /// 3. In the case of iOS, use `--cpu` if it leads with "ios_" for backwards compatibility.
    /// 4. In the case of macOS, use `--cpu` if it leads with "darwin_" for backwards
    ///    compatibility.
    /// 5. Use the default.
    pub fn get_single_architecture(&self) -> String {
        Self::get_unprefixed_apple_cpu(&self.apple_platform_type, &self.apple_cpus)
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:216 AppleConfiguration.getUnprefixedAppleCpu
    fn get_unprefixed_apple_cpu(
        apple_platform_type: &str,
        apple_cpus: &AppleConfigurationAppleCpus,
    ) -> String {
        // The environment data prefix is removed from the CPU string,
        // - e.g. whether the target CPU is for simulator, device or catalyst.
        //  For older CPUs no environment may be provided.
        let cpu = Self::get_prefixed_apple_cpu(apple_platform_type, apple_cpus);
        if let Some(rest) = cpu.strip_prefix(SIMULATOR_ENVIRONMENT_CPU_PREFIX) {
            return rest.to_string();
        } else if let Some(rest) = cpu.strip_prefix(DEVICE_ENVIRONMENT_CPU_PREFIX) {
            return rest.to_string();
        }
        cpu.to_string()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:220 AppleConfiguration.getPrefixedAppleCpu
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
            _ => panic!(
                "IllegalArgumentException: Unhandled platform type {}",
                apple_platform_type
            ),
        }
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:256 AppleConfiguration.getSingleArchPlatform
    /// Gets the single "effective" platform for this configuration's `PlatformType` and
    /// architecture.
    pub fn get_single_arch_platform(&self) -> ! {
        // ApplePlatform.forTarget(applePlatformType, getPrefixedAppleCpu(...)): the ApplePlatform
        // enum is not ported.
        blaze_util::unported!("ApplePlatform.forTarget")
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:263 AppleConfiguration.getXcodeVersionFlag
    pub fn get_xcode_version_flag(&self) -> Option<&str> {
        self.xcode_version_flag.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:268 AppleConfiguration.iosSdkVersionFlag
    pub fn ios_sdk_version_flag(&self) -> Option<&DottedVersion> {
        self.ios_sdk_version_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:273 AppleConfiguration.macOsSdkVersionFlag
    pub fn mac_os_sdk_version_flag(&self) -> Option<&DottedVersion> {
        self.mac_os_sdk_version_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:278 AppleConfiguration.tvOsSdkVersionFlag
    pub fn tv_os_sdk_version_flag(&self) -> Option<&DottedVersion> {
        self.tv_os_sdk_version_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:283 AppleConfiguration.watchOsSdkVersionFlag
    pub fn watch_os_sdk_version_flag(&self) -> Option<&DottedVersion> {
        self.watch_os_sdk_version_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:288 AppleConfiguration.iosMinimumOsFlag
    pub fn ios_minimum_os_flag(&self) -> Option<&DottedVersion> {
        self.ios_minimum_os_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:293 AppleConfiguration.macOsMinimumOsFlag
    pub fn mac_os_minimum_os_flag(&self) -> Option<&DottedVersion> {
        self.macos_minimum_os_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:298 AppleConfiguration.tvOsMinimumOsFlag
    pub fn tv_os_minimum_os_flag(&self) -> Option<&DottedVersion> {
        self.tvos_minimum_os_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:303 AppleConfiguration.watchOsMinimumOsFlag
    pub fn watch_os_minimum_os_flag(&self) -> Option<&DottedVersion> {
        self.watchos_minimum_os_flag.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:308 AppleConfiguration.shouldPreferMutualXcode
    pub fn should_prefer_mutual_xcode(&self) -> bool {
        self.prefer_mutual_xcode
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:313 AppleConfiguration.includeXcodeExecRequirementsFlag
    pub fn include_xcode_exec_requirements_flag(&self) -> bool {
        self.include_xcode_exec_requirements
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:325 AppleConfiguration.getXcodeConfigLabel
    /// Returns the label of the xcode_config rule to use for resolving the exec system Xcode
    /// version.
    pub fn get_xcode_config_label(&self) -> &Label {
        &self.xcode_config_label
    }
}

impl Fragment for AppleConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &APPLE_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:330 AppleConfiguration.processForOutputPathMnemonic
    fn process_for_output_path_mnemonic(
        &self,
        ctx: &mut dyn OutputDirectoriesContext,
    ) -> Result<(), AddToMnemonicException> {
        let mut components: Vec<String> = Vec::new();
        if !self.apple_cpus.apple_split_cpu().is_empty() {
            components.push(self.apple_platform_type.clone());
            components.push(self.apple_cpus.apple_split_cpu().to_string());

            if let Some(minimum_os_version) = self.options.get_minimum_os_version() {
                components.push(format!("min{}", minimum_os_version));
            }
        }
        if self.configuration_distinguisher != AppleConfigurationConfigurationDistinguisher::UNKNOWN
        {
            components.push(
                self.configuration_distinguisher
                    .get_file_system_name()
                    .to_string(),
            );
        }

        if !components.is_empty() {
            ctx.add_to_mnemonic(Some(components.join("-").as_str()))?;
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment_options_class::all_known_fragment_options_classes;

    fn create(args: &[&str]) -> AppleConfiguration {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        AppleConfiguration::new(&options)
    }

    #[test]
    fn single_architecture_strips_environment_prefix() {
        let config = create(&[
            "--apple_platform_type=ios",
            "--ios_multi_cpus=sim_arm64,x86_64",
        ]);
        assert_eq!(config.get_single_architecture(), "arm64");
        let config = create(&[
            "--apple_platform_type=ios",
            "--apple_split_cpu=device_arm64e",
        ]);
        assert_eq!(config.get_single_architecture(), "arm64e");
    }

    #[test]
    fn default_cpus() {
        let config = create(&[]);
        assert_eq!(config.get_apple_cpus().ios_multi_cpus(), &[DEFAULT_IOS_CPU]);
        assert_eq!(
            config.get_apple_cpus().catalyst_cpus(),
            &[DEFAULT_CATALYST_CPU]
        );
    }

    // XcodeConfigTest exercises xcode_config rules and needs full analysis.
    #[test]
    #[ignore = "needs analysis (XcodeConfigTest)"]
    fn xcode_config_tests() {}
}
