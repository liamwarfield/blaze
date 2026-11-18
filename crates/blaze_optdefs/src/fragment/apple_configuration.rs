//! The option value enum of `rules/apple/AppleConfiguration.java` (the fragment itself is not
//! ported here).

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:370 AppleConfiguration.ConfigurationDistinguisher
    /// Value used to avoid multiple configurations from conflicting. No two instances of this
    /// transition may exist with the same value in a single Bazel invocation.
    pub enum AppleConfigurationConfigurationDistinguisher {
        UNKNOWN,
        /// Distinguisher for `apple_binary` rule with "ios" platform_type.
        APPLEBIN_IOS,
        /// Distinguisher for `apple_binary` rule with "visionos" platform_type.
        APPLEBIN_VISIONOS,
        /// Distinguisher for `apple_binary` rule with "watchos" platform_type.
        APPLEBIN_WATCHOS,
        /// Distinguisher for `apple_binary` rule with "tvos" platform_type.
        APPLEBIN_TVOS,
        /// Distinguisher for `apple_binary` rule with "macos" platform_type.
        APPLEBIN_MACOS,
        /// Distinguisher for `apple_binary` rule with "catalyst" platform_type.
        APPLEBIN_CATALYST,
        /// Distinguisher for the apple crosstool configuration. We use "apl" for output directory
        /// names instead of "apple_crosstool" to avoid oversized path names, which can be
        /// problematic on OSX.
        APPLE_CROSSTOOL,
    }
}

impl AppleConfigurationConfigurationDistinguisher {
    // Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:401 AppleConfiguration.ConfigurationDistinguisher.getFileSystemName
    /// Returns the distinct string that should be used in creating output directories for a
    /// configuration with this distinguisher.
    pub fn get_file_system_name(&self) -> &'static str {
        use AppleConfigurationConfigurationDistinguisher::*;
        match self {
            UNKNOWN => "unknown",
            APPLEBIN_IOS => "applebin_ios",
            APPLEBIN_VISIONOS => "applebin_visionos",
            APPLEBIN_WATCHOS => "applebin_watchos",
            APPLEBIN_TVOS => "applebin_tvos",
            APPLEBIN_MACOS => "applebin_macos",
            APPLEBIN_CATALYST => "applebin_catalyst",
            APPLE_CROSSTOOL => "apl",
        }
    }
}
