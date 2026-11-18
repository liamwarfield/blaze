//! The options of `rules/android/AndroidConfiguration.java` (`AndroidConfiguration.Options`, its
//! converters and value enums). The fragment itself is not ported here.

use blaze_options::{CommaSeparatedOptionListConverter, Void, options};

use super::converters::{
    EmptyToNullLabelConverter, LabelConverter, LabelOrderedSetConverter, OptionLabel,
};
use super::cpp_configuration::CppConfigurationDynamicMode;
use super::cpp_options::DynamicModeConverter;
use super::fragment_options::FragmentOptions;

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:52 AndroidConfiguration.ConfigurationDistinguisherConverter
blaze_options::enum_converter!(
    pub ConfigurationDistinguisherConverter,
    AndroidConfigurationConfigurationDistinguisher,
    "Android configuration distinguisher"
);

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:60 AndroidConfiguration.ApkSigningMethodConverter
blaze_options::enum_converter!(
    pub ApkSigningMethodConverter,
    AndroidConfigurationApkSigningMethod,
    "apk signing method"
);

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:67 AndroidConfiguration.AndroidManifestMergerConverter
blaze_options::enum_converter!(
    pub AndroidManifestMergerConverter,
    AndroidConfigurationAndroidManifestMerger,
    "android manifest merger"
);

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:75 AndroidConfiguration.ManifestMergerOrderConverter
blaze_options::enum_converter!(
    pub ManifestMergerOrderConverter,
    AndroidConfigurationManifestMergerOrder,
    "android manifest merger order"
);

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:93 AndroidConfiguration.ConfigurationDistinguisher
    /// Value used to avoid multiple configurations from conflicting.
    ///
    /// This is set to `ANDROID` in Android configurations and to `MAIN` otherwise. This
    /// influences the output directory name: if it didn't, an Android and a non-Android
    /// configuration would conflict if they had the same toolchain identifier.
    pub enum AndroidConfigurationConfigurationDistinguisher { MAIN, ANDROID }
}

impl AndroidConfigurationConfigurationDistinguisher {
    /// The Java enum's private `suffix` field.
    pub fn suffix(&self) -> Option<&'static str> {
        match self {
            AndroidConfigurationConfigurationDistinguisher::MAIN => None,
            AndroidConfigurationConfigurationDistinguisher::ANDROID => Some("android"),
        }
    }
}

fragment_enum! {
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:116 AndroidConfiguration.ApkSigningMethod
    /// Which APK signing method to use with the debug key for rules that build APKs.
    ///
    /// - V1 uses the apksigner attribute from the android_sdk and signs the APK as a JAR.
    /// - V2 uses the apksigner attribute from the android_sdk and signs the APK according to the
    ///   APK Signing Schema V2 that is only supported on Android N and later.
    /// - V4 uses the apksigner attribute from the android_sdk and signs the APK according to the
    ///   APK Signing Schema V4 that is only supported on Android R/build tools 30 and later. It
    ///   generates a V4 signature file alongside the APK file.
    pub enum AndroidConfigurationApkSigningMethod { V1, V2, V1_V2, V4 }
}

impl AndroidConfigurationApkSigningMethod {
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:137 AndroidConfiguration.ApkSigningMethod.signV1
    /// Whether to JAR sign the APK with the apksigner tool.
    pub fn sign_v1(&self) -> bool {
        matches!(self, Self::V1 | Self::V1_V2)
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:142 AndroidConfiguration.ApkSigningMethod.signV2
    /// Whether to sign the APK with the apksigner tool with APK Signature Schema V2.
    pub fn sign_v2(&self) -> bool {
        matches!(self, Self::V2 | Self::V1_V2)
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:153 AndroidConfiguration.ApkSigningMethod.signV4
    /// Whether to sign the APK with the apksigner tool with APK Signature Schema V4.
    ///
    /// If null/unset, the V4 signing flag should not be passed to apksigner. This extra level of
    /// control is needed to support environments where older build tools may be used.
    pub fn sign_v4(&self) -> Option<bool> {
        match self {
            Self::V4 => Some(true),
            _ => None,
        }
    }
}

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:159 AndroidConfiguration.AndroidManifestMerger
    /// Types of android manifest mergers.
    pub enum AndroidConfigurationAndroidManifestMerger { LEGACY, ANDROID, FORCE_ANDROID }
}

impl AndroidConfigurationAndroidManifestMerger {
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:164 AndroidConfiguration.AndroidManifestMerger.getAttributeValues
    pub fn get_attribute_values() -> Vec<String> {
        vec![
            "legacy".to_string(),
            "android".to_string(),
            "force_android".to_string(),
            Self::get_rule_attribute_default().to_string(),
        ]
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:172 AndroidConfiguration.AndroidManifestMerger.getRuleAttributeDefault
    pub fn get_rule_attribute_default() -> &'static str {
        "auto"
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:177 AndroidConfiguration.AndroidManifestMerger.fromString
    pub fn from_string(value: &str) -> Option<AndroidConfigurationAndroidManifestMerger> {
        use blaze_options::converters::JavaEnum;
        Self::values()
            .iter()
            .copied()
            .find(|m| m.name().eq_ignore_ascii_case(value))
    }
}

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:188 AndroidConfiguration.ManifestMergerOrder
    /// Orders for merging android manifests.
    pub enum AndroidConfigurationManifestMergerOrder {
        /// Manifests are sorted alphabetically by exec path.
        ALPHABETICAL,
        /// Manifests are sorted alphabetically by configuration-relative path.
        ALPHABETICAL_BY_CONFIGURATION,
        /// Library manifests come before the manifests of their dependencies.
        DEPENDENCY,
    }
}

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:198 AndroidConfiguration.Options
options! {
    /// Android configuration options.
    pub struct AndroidConfigurationOptions {
        #[option(
            name = "Android configuration distinguisher",
            default_value = "MAIN",
            converter = ConfigurationDistinguisherConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [INTERNAL],
        )]
        pub configuration_distinguisher: AndroidConfigurationConfigurationDistinguisher,

        #[option(
            name = "android_compiler",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = "The Android target compiler.",
        )]
        pub cpp_compiler: Option<String>,

        #[option(
            name = "android_dynamic_mode",
            default_value = "off",
            converter = DynamicModeConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "Determines whether C++ deps of Android rules will be linked dynamically when a ",
                "cc_binary does not explicitly create a shared library. ",
                "'default' means bazel will choose whether to link dynamically.  ",
                "'fully' means all libraries will be linked dynamically. ",
                "'off' means that all libraries will be linked in mostly static mode.",
            ),
        )]
        pub dynamic_mode: CppConfigurationDynamicMode,

        #[option(
            name = "android_platforms",
            converter = LabelOrderedSetConverter,
            documentation_category = TOOLCHAIN,
            default_value = "",
            effect_tags = [CHANGES_INPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Sets the platforms that android_binary targets use. If multiple platforms are",
                " specified, then the binary is a fat APKs, which contains native binaries for",
                " each specified target platform.",
            ),
        )]
        pub android_platforms: Vec<OptionLabel>,

        #[option(
            name = "fat_apk_hwasan",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [NO_OP],
            help = "No-op flag. Will be removed in a future release.",
        )]
        pub fat_apk_hwasan: bool,

        #[option(
            name = "desugar_for_android",
            old_name = "experimental_desugar_for_android",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = "Whether to desugar Java 8 bytecode before dexing.",
        )]
        pub desugar_java8: bool,

        #[option(
            name = "desugar_java8_libs",
            old_name = "experimental_desugar_java8_libs",
            old_name_warning = false,
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "Whether to include supported Java 8 libraries in apps for legacy devices.",
        )]
        pub desugar_java8_libs: bool,

        #[option(
            name = "experimental_check_desugar_deps",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [EAGERNESS_TO_EXIT, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Whether to double-check correct desugaring at Android binary level.",
        )]
        pub check_desugar_deps: bool,

        #[option(
            name = "incremental_dexing",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = "Does most of the work for dexing separately for each Jar file.",
        )]
        pub incremental_dexing: bool,

        #[option(
            name = "experimental_incremental_dexing_after_proguard",
            default_value = "50",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Whether to use incremental dexing tools when building proguarded Android binaries.  ",
                "Values > 0 turn the feature on, values > 1 run that many dexbuilder shards.",
            ),
        )]
        pub incremental_dexing_shards_after_proguard: i32,

        /// Whether to use a separate tool to shard classes before merging them into final dex files.
        #[option(
            name = "experimental_use_dex_splitter_for_incremental_dexing",
            default_value = "true",
            metadata_tags = [EXPERIMENTAL],
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "Do not use.",
        )]
        pub incremental_dexing_use_dex_sharder: bool,

        #[option(
            name = "experimental_incremental_dexing_after_proguard_by_default",
            default_value = "true",
            metadata_tags = [EXPERIMENTAL],
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Whether to use incremental dexing for proguarded Android binaries by default.  ",
                "Use incremental_dexing attribute to override default for a particular ",
                "android_binary.",
            ),
        )]
        pub incremental_dexing_after_proguard_by_default: bool,

        #[option(
            name = "experimental_android_assume_minsdkversion",
            default_value = "false",
            metadata_tags = [EXPERIMENTAL],
            documentation_category = UNDOCUMENTED,
            effect_tags = [ACTION_COMMAND_LINES, LOADING_AND_ANALYSIS],
            help = concat!(
                "When enabled, the minSdkVersion is parsed from the merged AndroidManifest and used to ",
                "instruct Proguard on valid Android build versions.",
            ),
        )]
        pub assume_min_sdk_version: bool,

        #[option(
            name = "experimental_android_use_parallel_dex2oat",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [LOADING_AND_ANALYSIS, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            metadata_tags = [EXPERIMENTAL],
            help = "Use dex2oat in parallel to possibly speed up android_test.",
        )]
        pub use_parallel_dex2_oat: bool,

        #[option(
            name = "break_build_on_parallel_dex2oat_failure",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true dex2oat action failures will cause the build to break ",
                "instead of executing dex2oat during test runtime.",
            ),
        )]
        pub break_build_on_parallel_dex2_oat_failure: bool,

        #[option(
            name = "non_incremental_per_target_dexopts",
            converter = CommaSeparatedOptionListConverter,
            default_value = "--positions",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "dx flags that that prevent incremental dexing for binary targets that list any of ",
                "the flags listed here in their 'dexopts' attribute, which are ignored with ",
                "incremental dexing (superseding --dexopts_supported_in_incremental_dexing).  ",
                "Defaults to --positions for safety but can in general be used ",
                "to make sure the listed dx flags are honored, with additional build latency.  ",
                "Please notify us if you find yourself needing this flag.",
            ),
        )]
        pub non_incremental_per_target_dexopts: Vec<String>,

        #[option(
            name = "dexopts_supported_in_incremental_dexing",
            converter = CommaSeparatedOptionListConverter,
            default_value = "--no-optimize,--no-locals",
            documentation_category = UNDOCUMENTED,
            effect_tags = [ACTION_COMMAND_LINES, LOADING_AND_ANALYSIS],
            help = "dx flags supported when converting Jars to dex archives incrementally.",
        )]
        pub dexopts_supported_in_incremental_dexing: Vec<String>,

        #[option(
            name = "dexopts_supported_in_dexmerger",
            converter = CommaSeparatedOptionListConverter,
            default_value = "--minimal-main-dex,--set-max-idx-number",
            documentation_category = UNDOCUMENTED,
            effect_tags = [ACTION_COMMAND_LINES, LOADING_AND_ANALYSIS],
            help = "dx flags supported in tool that merges dex archives into final classes.dex files.",
        )]
        pub dexopts_supported_in_dex_merger: Vec<String>,

        #[option(
            name = "dexopts_supported_in_dexsharder",
            converter = CommaSeparatedOptionListConverter,
            default_value = "--minimal-main-dex",
            documentation_category = UNDOCUMENTED,
            effect_tags = [ACTION_COMMAND_LINES, LOADING_AND_ANALYSIS],
            help = "dx flags supported in tool that groups classes for inclusion in final .dex files.",
        )]
        pub dexopts_supported_in_dex_sharder: Vec<String>,

        #[option(
            name = "experimental_android_rewrite_dexes_with_rex",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "use rex tool to rewrite dex files",
        )]
        pub use_rex_to_compress_dex_files: bool,

        #[option(
            name = "experimental_allow_android_library_deps_without_srcs",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "No-op. Kept here for backwards compatibility.",
            deprecated = true,
        )]
        pub allow_android_library_deps_without_srcs: bool,

        #[option(
            name = "experimental_android_resource_shrinking",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enables resource shrinking for android_binary APKs that use ProGuard.",
        )]
        pub use_experimental_android_resource_shrinking: bool,

        #[option(
            name = "android_resource_shrinking",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Enables resource shrinking for android_binary APKs that use ProGuard.",
        )]
        pub use_android_resource_shrinking: bool,

        #[option(
            name = "experimental_android_resource_cycle_shrinking",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Enables more shrinking of code and resources by instructing AAPT2 ",
                "to emit conditional Proguard keep rules.",
            ),
        )]
        pub use_android_resource_cycle_shrinking: bool,

        #[option(
            name = "experimental_android_resource_path_shortening",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enables shortening of resource file paths within android_binary APKs.",
        )]
        pub use_android_resource_path_shortening: bool,

        #[option(
            name = "experimental_android_resource_name_obfuscation",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Enables obfuscation of resource names within android_binary APKs.",
        )]
        pub use_android_resource_name_obfuscation: bool,

        #[option(
            name = "android_manifest_merger",
            default_value = "android",
            converter = AndroidManifestMergerConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "Selects the manifest merger to use for android_binary rules. Flag to help the ",
                "transition to the Android manifest merger from the legacy merger.",
            ),
        )]
        pub manifest_merger: AndroidConfigurationAndroidManifestMerger,

        #[option(
            name = "android_manifest_merger_order",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, EXECUTION],
            default_value = "alphabetical",
            converter = ManifestMergerOrderConverter,
            help = concat!(
                "Sets the order of manifests passed to the manifest merger for Android binaries. ",
                "ALPHABETICAL means manifests are sorted by path relative to the execroot. ",
                "ALPHABETICAL_BY_CONFIGURATION means manifests are sorted by paths relative ",
                "to the configuration directory within the output directory. ",
                "DEPENDENCY means manifests are ordered with each library's manifest coming ",
                "before the manifests of its dependencies.",
            ),
        )]
        pub manifest_merger_order: AndroidConfigurationManifestMergerOrder,

        #[option(
            name = "apk_signing_method",
            converter = ApkSigningMethodConverter,
            default_value = "v1_v2",
            documentation_category = SIGNING,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = "Implementation to use to sign APKs",
        )]
        pub apk_signing_method: AndroidConfigurationApkSigningMethod,

        #[option(
            name = "experimental_android_compress_java_resources",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "Compress Java resources in APKs",
        )]
        pub compress_java_resources: bool,

        #[option(
            name = "experimental_android_databinding_v2",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "Use android databinding v2. This flag is a no-op.",
        )]
        pub data_binding_v2: bool,

        #[option(
            name = "android_databinding_use_v3_4_args",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "Use android databinding v2 with 3.4.0 argument. This flag is a no-op.",
        )]
        pub data_binding_updated_args: bool,

        #[option(
            name = "android_databinding_use_androidx",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Generate AndroidX-compatible data-binding files. ",
                "This is only used with databinding v2. This flag is a no-op.",
            ),
        )]
        pub data_binding_android_x: bool,

        #[option(
            name = "experimental_android_library_exports_manifest_default",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = "The default value of the exports_manifest attribute on android_library.",
        )]
        pub exports_manifest_default: bool,

        #[option(
            name = "experimental_omit_resources_info_provider_from_android_binary",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Omit AndroidResourcesInfo provider from android_binary rules.",
                " Propagating resources out to other binaries is usually unintentional.",
            ),
        )]
        pub omit_resources_info_provider_from_android_binary: bool,

        #[option(
            name = "android_fixed_resource_neverlinking",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "If true, resources will properly not get propagated through neverlinked libraries.",
                " Otherwise, the old behavior of propagating those resources if no",
                " resource-related attributes are specified in the neverlink library",
                " will be preserved.",
            ),
        )]
        pub fixed_resource_neverlinking: bool,

        #[option(
            name = "android_migration_tag_check",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "If enabled, strict usage of the Starlark migration tag is enabled for android rules. ",
                "Prefer using --incompatible_disable_native_android_rules.",
            ),
        )]
        pub check_for_migration_tag: bool,

        #[option(
            name = "incompatible_disable_native_android_rules",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [EAGERNESS_TO_EXIT],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If enabled, direct usage of the native Android rules is disabled. Please use the",
                " Starlark Android rules from https://github.com/bazelbuild/rules_android",
            ),
        )]
        pub disable_native_android_rules: bool,

        #[option(
            name = "experimental_filter_r_jars_from_android_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = "If enabled, R Jars will be filtered from the test apk built by android_test.",
        )]
        pub filter_r_jars_from_android_test: bool,

        #[option(
            name = "experimental_one_version_enforcement_use_transitive_jars_for_binary_under_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, ACTION_COMMAND_LINES],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled, one version enforcement for android_test uses the binary_under_test's ",
                "transitive classpath, otherwise it uses the deploy jar",
            ),
        )]
        pub one_version_enforcement_use_transitive_jars_for_binary_under_test: bool,

        #[option(
            name = "experimental_persistent_aar_extractor",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "Enable persistent aar extractor by using workers.",
        )]
        pub persistent_aar_extractor: bool,

        #[option(
            name = "persistent_android_resource_processor",
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = "Enable persistent Android resource processor by using workers.",
            // This implementation uses unique workers for each tool in the busybox.
            expansion = ["--internal_persistent_busybox_tools", "--strategy=AaptPackage=worker", "--strategy=AndroidResourceParser=worker", "--strategy=AndroidResourceValidator=worker", "--strategy=AndroidResourceCompiler=worker", "--strategy=RClassGenerator=worker", "--strategy=AndroidResourceLink=worker", "--strategy=AndroidAapt2=worker", "--strategy=AndroidAssetMerger=worker", "--strategy=AndroidResourceMerger=worker", "--strategy=AndroidCompiledResourceMerger=worker", "--strategy=ManifestMerger=worker", "--strategy=AndroidManifestMerger=worker", "--strategy=Aapt2Optimize=worker", "--strategy=AARGenerator=worker", "--strategy=ProcessDatabinding=worker", "--strategy=GenerateDataBindingBaseClasses=worker"],
        )]
        pub persistent_resource_processor: Void,

        #[option(
            name = "persistent_multiplex_android_resource_processor",
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = "Enable persistent multiplexed Android resource processor by using workers.",
            expansion = ["--persistent_android_resource_processor", "--modify_execution_info=AaptPackage=+supports-multiplex-workers", "--modify_execution_info=AndroidResourceParser=+supports-multiplex-workers", "--modify_execution_info=AndroidResourceValidator=+supports-multiplex-workers", "--modify_execution_info=AndroidResourceCompiler=+supports-multiplex-workers", "--modify_execution_info=RClassGenerator=+supports-multiplex-workers", "--modify_execution_info=AndroidResourceLink=+supports-multiplex-workers", "--modify_execution_info=AndroidAapt2=+supports-multiplex-workers", "--modify_execution_info=AndroidAssetMerger=+supports-multiplex-workers", "--modify_execution_info=AndroidResourceMerger=+supports-multiplex-workers", "--modify_execution_info=AndroidCompiledResourceMerger=+supports-multiplex-workers", "--modify_execution_info=ManifestMerger=+supports-multiplex-workers", "--modify_execution_info=AndroidManifestMerger=+supports-multiplex-workers", "--modify_execution_info=Aapt2Optimize=+supports-multiplex-workers", "--modify_execution_info=AARGenerator=+supports-multiplex-workers"],
        )]
        pub persistent_multiplex_android_resource_processor: Void,

        #[option(
            name = "persistent_android_dex_desugar",
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = "Enable persistent Android dex and desugar actions by using workers.",
            expansion = ["--internal_persistent_android_dex_desugar", "--strategy=Desugar=worker", "--strategy=DexBuilder=worker"],
        )]
        pub persistent_android_dex_desugar: Void,

        #[option(
            name = "persistent_multiplex_android_dex_desugar",
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = "Enable persistent multiplexed Android dex and desugar actions by using workers.",
            expansion = ["--persistent_android_dex_desugar", "--internal_persistent_multiplex_android_dex_desugar"],
        )]
        pub persistent_multiplex_android_dex_desugar: Void,

        #[option(
            name = "persistent_multiplex_android_tools",
            default_value = "null",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = concat!(
                "Enable persistent and multiplexed Android tools (dexing, desugaring, resource ",
                "processing).",
            ),
            expansion = ["--internal_persistent_multiplex_busybox_tools", "--persistent_multiplex_android_resource_processor", "--persistent_multiplex_android_dex_desugar"],
        )]
        pub persistent_multiplex_android_tools: Void,

        /// We use this option to decide when to enable workers for busybox tools. This flag is also a
        /// guard against enabling workers using nothing but --persistent_android_resource_processor.
        ///
        /// <p>Consequently, we use this option to decide between param files or regular command line
        /// parameters. If we're not using workers or on Windows, there's no need to always use param
        /// files for I/O performance reasons.
        #[option(
            name = "internal_persistent_busybox_tools",
            documentation_category = UNDOCUMENTED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            default_value = "false",
            help = "Tracking flag for when busybox workers are enabled.",
        )]
        pub persistent_busybox_tools: bool,

        #[option(
            name = "internal_persistent_multiplex_busybox_tools",
            documentation_category = UNDOCUMENTED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            default_value = "false",
            help = "Tracking flag for when multiplexed busybox workers are enabled.",
        )]
        pub persistent_multiplex_busybox_tools: bool,

        /// We use this option to decide when to enable workers for busybox tools. This flag is also a
        /// guard against enabling workers using nothing but --persistent_android_resource_processor.
        ///
        /// <p>Consequently, we use this option to decide between param files or regular command line
        /// parameters. If we're not using workers or on Windows, there's no need to always use param
        /// files for I/O performance reasons.
        #[option(
            name = "internal_persistent_android_dex_desugar",
            documentation_category = UNDOCUMENTED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            default_value = "false",
            help = "Tracking flag for when dexing and desugaring workers are enabled.",
        )]
        pub persistent_dex_desugar: bool,

        #[option(
            name = "internal_persistent_multiplex_android_dex_desugar",
            documentation_category = UNDOCUMENTED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            default_value = "false",
            help = "Tracking flag for when multiplexed dexing and desugaring workers are enabled.",
        )]
        pub persistent_multiplex_dex_desugar: bool,

        #[option(
            name = "experimental_remove_r_classes_from_instrumentation_test_jar",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled and the test instruments an application, all the R classes from the test's ",
                "deploy jar will be removed.",
            ),
        )]
        pub remove_r_classes_from_instrumentation_test_jar: bool,

        #[option(
            name = "experimental_always_filter_duplicate_classes_from_android_test",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled and the android_test defines a binary_under_test, the class filterering ",
                "applied to the test's deploy jar will always filter duplicate classes based ",
                "solely on matching class and package name, ignoring hash values.",
            ),
        )]
        pub always_filter_duplicate_classes_from_android_test: bool,

        #[option(
            name = "experimental_filter_library_jar_with_program_jar",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [ACTION_COMMAND_LINES],
            metadata_tags = [EXPERIMENTAL],
            help = "Filter the ProGuard ProgramJar to remove any classes also present in the LibraryJar.",
        )]
        pub filter_library_jar_with_program_jar: bool,

        #[option(
            name = "experimental_use_rtxt_from_merged_resources",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = "Use R.txt from the merging action, instead of from the validation action.",
        )]
        pub use_r_txt_from_merged_resources: bool,

        #[option(
            name = "output_library_merged_assets",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "If disabled, does not produce merged asset.zip outputs for library targets",
        )]
        pub output_library_merged_assets: bool,

        #[option(
            name = "legacy_main_dex_list_generator",
            default_value = "null",
            converter = LabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a binary to use to generate the list of classes that must be in the main",
                " dex when compiling legacy multidex.",
            ),
        )]
        pub legacy_main_dex_list_generator: Option<OptionLabel>,

        #[option(
            name = "optimizing_dexer",
            default_value = "null",
            converter = EmptyToNullLabelConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Specifies a binary to use to do dexing without sharding.",
        )]
        pub optimizing_dexer: Option<OptionLabel>,

        #[option(
            name = "experimental_disable_instrumentation_manifest_merge",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Disables manifest merging when an android_binary has instruments set (i.e. is used ",
                "for instrumentation testing).",
            ),
        )]
        pub disable_instrumentation_manifest_merging: bool,

        #[option(
            name = "experimental_get_android_java_resources_from_optimized_jar",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Get Java resources from _proguard.jar instead of _deploy.jar in android_binary when ",
                "bundling the final APK.",
            ),
        )]
        pub get_java_resources_from_optimized_jar: bool,

        #[option(
            name = "incompatible_remove_ctx_android_fragment",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BUILD_FILE_SEMANTICS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When true, Android build flags are defined with Android rules (in BUIILD files) and",
                " ctx.fragments.android is undefined. This is a migration flag to move all",
                " Android flags  from core Bazel to Python rules.",
            ),
        )]
        pub disable_android_fragment: bool,
    }
}

impl FragmentOptions for AndroidConfigurationOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<AndroidConfigurationOptions>(&[
            "Android configuration distinguisher",
            "android_compiler",
            "android_dynamic_mode",
            "android_platforms",
            "fat_apk_hwasan",
            "desugar_for_android",
            "desugar_java8_libs",
            "experimental_check_desugar_deps",
            "incremental_dexing",
            "experimental_incremental_dexing_after_proguard",
            "experimental_use_dex_splitter_for_incremental_dexing",
            "experimental_incremental_dexing_after_proguard_by_default",
            "experimental_android_assume_minsdkversion",
            "experimental_android_use_parallel_dex2oat",
            "break_build_on_parallel_dex2oat_failure",
            "non_incremental_per_target_dexopts",
            "dexopts_supported_in_incremental_dexing",
            "dexopts_supported_in_dexmerger",
            "dexopts_supported_in_dexsharder",
            "experimental_android_rewrite_dexes_with_rex",
            "experimental_allow_android_library_deps_without_srcs",
            "experimental_android_resource_shrinking",
            "android_resource_shrinking",
            "experimental_android_resource_cycle_shrinking",
            "experimental_android_resource_path_shortening",
            "experimental_android_resource_name_obfuscation",
            "android_manifest_merger",
            "android_manifest_merger_order",
            "apk_signing_method",
            "experimental_android_compress_java_resources",
            "experimental_android_databinding_v2",
            "android_databinding_use_v3_4_args",
            "android_databinding_use_androidx",
            "experimental_android_library_exports_manifest_default",
            "experimental_omit_resources_info_provider_from_android_binary",
            "android_fixed_resource_neverlinking",
            "android_migration_tag_check",
            "incompatible_disable_native_android_rules",
            "experimental_filter_r_jars_from_android_test",
            "experimental_one_version_enforcement_use_transitive_jars_for_binary_under_test",
            "experimental_persistent_aar_extractor",
            "persistent_android_resource_processor",
            "persistent_multiplex_android_resource_processor",
            "persistent_android_dex_desugar",
            "persistent_multiplex_android_dex_desugar",
            "persistent_multiplex_android_tools",
            "internal_persistent_busybox_tools",
            "internal_persistent_multiplex_busybox_tools",
            "internal_persistent_android_dex_desugar",
            "internal_persistent_multiplex_android_dex_desugar",
            "experimental_remove_r_classes_from_instrumentation_test_jar",
            "experimental_always_filter_duplicate_classes_from_android_test",
            "experimental_filter_library_jar_with_program_jar",
            "experimental_use_rtxt_from_merged_resources",
            "output_library_merged_assets",
            "legacy_main_dex_list_generator",
            "optimizing_dexer",
            "experimental_disable_instrumentation_manifest_merge",
            "experimental_get_android_java_resources_from_optimized_jar",
            "incompatible_remove_ctx_android_fragment",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<AndroidConfigurationOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<AndroidConfigurationOptions>(&[
            "--android_platforms=//p:b,//p:a,//p:b",
            "--apk_signing_method=v1_v2",
            "--android_manifest_merger=force_android",
            "--internal_persistent_android_dex_desugar",
        ])
        .unwrap();
        assert_eq!(
            o.android_platforms
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>(),
            vec!["//p:b", "//p:a"]
        );
        assert!(o.apk_signing_method.sign_v1() && o.apk_signing_method.sign_v2());
        assert_eq!(
            o.manifest_merger,
            AndroidConfigurationAndroidManifestMerger::FORCE_ANDROID
        );
        assert!(o.persistent_dex_desugar);
        assert_eq!(
            AndroidConfigurationAndroidManifestMerger::from_string("Legacy"),
            Some(AndroidConfigurationAndroidManifestMerger::LEGACY)
        );
    }
}
