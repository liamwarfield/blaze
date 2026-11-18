//! Port of `rules/android/AndroidConfiguration.java` (the fragment; its options, converters and
//! value enums are `blaze_optdefs::fragment::android_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::android_configuration::{
    AndroidConfigurationAndroidManifestMerger, AndroidConfigurationApkSigningMethod,
    AndroidConfigurationConfigurationDistinguisher, AndroidConfigurationManifestMergerOrder,
    AndroidConfigurationOptions,
};
use blaze_options::converters::JavaEnum;

use crate::build_options::BuildOptions;
use crate::fragment::{
    AddToMnemonicException, Fragment, FragmentClass, FragmentClassInfo,
    InvalidConfigurationException, OutputDirectoriesContext, required,
};
use crate::fragment_options_class::ANDROID_OPTIONS;

// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:46 AndroidConfiguration
/// Configuration fragment for Android rules.
#[derive(Debug)]
pub struct AndroidConfiguration {
    configuration_distinguisher: AndroidConfigurationConfigurationDistinguisher,
    incremental_dexing: bool,
    incremental_dexing_shards_after_proguard: i32,
    incremental_dexing_use_dex_sharder: bool,
    incremental_dexing_after_proguard_by_default: bool,
    assume_min_sdk_version: bool,
    dexopts_supported_in_incremental_dexing: Vec<String>,
    target_dexopts_that_prevent_incremental_dexing: Vec<String>,
    dexopts_supported_in_dex_merger: Vec<String>,
    dexopts_supported_in_dex_sharder: Vec<String>,
    desugar_java8: bool,
    desugar_java8_libs: bool,
    check_desugar_deps: bool,
    use_rex_to_compress_dex_files: bool,
    use_android_resource_shrinking: bool,
    use_android_resource_cycle_shrinking: bool,
    use_android_resource_path_shortening: bool,
    use_android_resource_name_obfuscation: bool,
    manifest_merger: AndroidConfigurationAndroidManifestMerger,
    manifest_merger_order: AndroidConfigurationManifestMergerOrder,
    apk_signing_method: AndroidConfigurationApkSigningMethod,
    compress_java_resources: bool,
    exports_manifest_default: bool,
    use_parallel_dex2_oat: bool,
    break_build_on_parallel_dex2_oat_failure: bool,
    omit_resources_info_provider_from_android_binary: bool,
    fixed_resource_neverlinking: bool,
    check_for_migration_tag: bool,
    one_version_enforcement_use_transitive_jars_for_binary_under_test: bool,
    persistent_aar_extractor: bool,
    persistent_busybox_tools: bool,
    persistent_multiplex_busybox_tools: bool,
    persistent_dex_desugar: bool,
    persistent_multiplex_dex_desugar: bool,
    filter_r_jars_from_android_test: bool,
    remove_r_classes_from_instrumentation_test_jar: bool,
    always_filter_duplicate_classes_from_android_test: bool,
    filter_library_jar_with_program_jar: bool,
    use_r_txt_from_merged_resources: bool,
    output_library_merged_assets: bool,
    legacy_main_dex_list_generator: Option<Label>,
    optimizing_dexer: Option<Label>,
    disable_instrumentation_manifest_merging: bool,
    get_java_resources_from_optimized_jar: bool,
    disable_android_fragment: bool,
}

/// `AndroidConfiguration.class`.
pub static ANDROID_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.android.AndroidConfiguration",
    starlark_name: Some("android"),
    required_options: || vec![&ANDROID_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(AndroidConfiguration::new(options)?);
    Ok(fragment)
}

impl AndroidConfiguration {
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1060 AndroidConfiguration.<init>
    pub fn new(
        build_options: &BuildOptions,
    ) -> Result<AndroidConfiguration, InvalidConfigurationException> {
        let options = required::<AndroidConfigurationOptions>(build_options);
        let configuration = AndroidConfiguration {
            configuration_distinguisher: options.configuration_distinguisher,
            incremental_dexing: options.incremental_dexing,
            incremental_dexing_shards_after_proguard: options
                .incremental_dexing_shards_after_proguard,
            incremental_dexing_use_dex_sharder: options.incremental_dexing_use_dex_sharder,
            incremental_dexing_after_proguard_by_default: options
                .incremental_dexing_after_proguard_by_default,
            assume_min_sdk_version: options.assume_min_sdk_version,
            dexopts_supported_in_incremental_dexing: options
                .dexopts_supported_in_incremental_dexing
                .clone(),
            target_dexopts_that_prevent_incremental_dexing: options
                .non_incremental_per_target_dexopts
                .clone(),
            dexopts_supported_in_dex_merger: options.dexopts_supported_in_dex_merger.clone(),
            dexopts_supported_in_dex_sharder: options.dexopts_supported_in_dex_sharder.clone(),
            desugar_java8: options.desugar_java8,
            desugar_java8_libs: options.desugar_java8_libs,
            check_desugar_deps: options.check_desugar_deps,
            use_android_resource_shrinking: options.use_android_resource_shrinking
                || options.use_experimental_android_resource_shrinking,
            use_android_resource_cycle_shrinking: options.use_android_resource_cycle_shrinking,
            use_android_resource_path_shortening: options.use_android_resource_path_shortening,
            use_android_resource_name_obfuscation: options.use_android_resource_name_obfuscation,
            manifest_merger: options.manifest_merger,
            manifest_merger_order: options.manifest_merger_order,
            apk_signing_method: options.apk_signing_method,
            use_rex_to_compress_dex_files: options.use_rex_to_compress_dex_files,
            compress_java_resources: options.compress_java_resources,
            exports_manifest_default: options.exports_manifest_default,
            use_parallel_dex2_oat: options.use_parallel_dex2_oat,
            break_build_on_parallel_dex2_oat_failure: options
                .break_build_on_parallel_dex2_oat_failure,
            omit_resources_info_provider_from_android_binary: options
                .omit_resources_info_provider_from_android_binary,
            fixed_resource_neverlinking: options.fixed_resource_neverlinking,
            // use --incompatible_disable_native_android_rules, and also the old flag for
            // backwards compatibility
            check_for_migration_tag: options.check_for_migration_tag
                || options.disable_native_android_rules,
            one_version_enforcement_use_transitive_jars_for_binary_under_test: options
                .one_version_enforcement_use_transitive_jars_for_binary_under_test,
            persistent_aar_extractor: options.persistent_aar_extractor,
            persistent_busybox_tools: options.persistent_busybox_tools,
            persistent_multiplex_busybox_tools: options.persistent_multiplex_busybox_tools,
            persistent_dex_desugar: options.persistent_dex_desugar,
            persistent_multiplex_dex_desugar: options.persistent_multiplex_dex_desugar,
            filter_r_jars_from_android_test: options.filter_r_jars_from_android_test,
            remove_r_classes_from_instrumentation_test_jar: options
                .remove_r_classes_from_instrumentation_test_jar,
            always_filter_duplicate_classes_from_android_test: options
                .always_filter_duplicate_classes_from_android_test,
            filter_library_jar_with_program_jar: options.filter_library_jar_with_program_jar,
            use_r_txt_from_merged_resources: options.use_r_txt_from_merged_resources,
            output_library_merged_assets: options.output_library_merged_assets,
            legacy_main_dex_list_generator: options
                .legacy_main_dex_list_generator
                .as_ref()
                .map(|l| l.get().clone()),
            optimizing_dexer: options.optimizing_dexer.as_ref().map(|l| l.get().clone()),
            disable_instrumentation_manifest_merging: options
                .disable_instrumentation_manifest_merging,
            get_java_resources_from_optimized_jar: options.get_java_resources_from_optimized_jar,
            disable_android_fragment: options.disable_android_fragment,
        };

        if configuration.incremental_dexing_shards_after_proguard < 0 {
            return Err(InvalidConfigurationException::new(
                "--experimental_incremental_dexing_after_proguard must be a positive number",
            ));
        }
        if configuration.incremental_dexing_after_proguard_by_default
            && configuration.incremental_dexing_shards_after_proguard == 0
        {
            return Err(InvalidConfigurationException::new(
                "--experimental_incremental_dexing_after_proguard_by_default requires \
                 --experimental_incremental_dexing_after_proguard to be at least 1",
            ));
        }
        if configuration.desugar_java8_libs && !configuration.desugar_java8 {
            return Err(InvalidConfigurationException::new(
                "Java 8 library support requires --desugar_java8 to be enabled.",
            ));
        }
        Ok(configuration)
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1139 AndroidConfiguration.useIncrementalDexing
    /// Returns whether to use incremental dexing.
    pub fn use_incremental_dexing(&self) -> bool {
        self.incremental_dexing
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1145 AndroidConfiguration.incrementalDexingShardsAfterProguard
    /// Returns whether to process proguarded Android binaries with incremental dexing tools.
    pub fn incremental_dexing_shards_after_proguard(&self) -> i32 {
        self.incremental_dexing_shards_after_proguard
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1151 AndroidConfiguration.incrementalDexingUseDexSharder
    /// Whether to use a separate tool to shard classes before merging them into final dex files.
    pub fn incremental_dexing_use_dex_sharder(&self) -> bool {
        self.incremental_dexing_use_dex_sharder
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1157 AndroidConfiguration.incrementalDexingAfterProguardByDefault
    /// Whether to use incremental dexing to build proguarded binaries by default.
    pub fn incremental_dexing_after_proguard_by_default(&self) -> bool {
        self.incremental_dexing_after_proguard_by_default
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1166 AndroidConfiguration.assumeMinSdkVersion
    /// Returns true if an -assumevalues should be generated for Proguard based on the
    /// minSdkVersion of the merged AndroidManifest.
    pub fn assume_min_sdk_version(&self) -> bool {
        self.assume_min_sdk_version
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1172 AndroidConfiguration.getDexoptsSupportedInIncrementalDexing
    /// dx flags supported in incremental dexing actions.
    pub fn get_dexopts_supported_in_incremental_dexing(&self) -> &[String] {
        &self.dexopts_supported_in_incremental_dexing
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1178 AndroidConfiguration.getDexoptsSupportedInDexMerger
    /// dx flags supported in dexmerger actions.
    pub fn get_dexopts_supported_in_dex_merger(&self) -> &[String] {
        &self.dexopts_supported_in_dex_merger
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1184 AndroidConfiguration.getDexoptsSupportedInDexSharder
    /// dx flags supported in dexsharder actions.
    pub fn get_dexopts_supported_in_dex_sharder(&self) -> &[String] {
        &self.dexopts_supported_in_dex_sharder
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1193 AndroidConfiguration.getTargetDexoptsThatPreventIncrementalDexing
    /// Incremental dexing must not be used for binaries that list any of these flags in their
    /// `dexopts` attribute.
    pub fn get_target_dexopts_that_prevent_incremental_dexing(&self) -> &[String] {
        &self.target_dexopts_that_prevent_incremental_dexing
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1198 AndroidConfiguration.desugarJava8
    pub fn desugar_java8(&self) -> bool {
        self.desugar_java8
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1203 AndroidConfiguration.desugarJava8Libs
    pub fn desugar_java8_libs(&self) -> bool {
        self.desugar_java8_libs
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1208 AndroidConfiguration.checkDesugarDeps
    pub fn check_desugar_deps(&self) -> bool {
        self.check_desugar_deps
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1213 AndroidConfiguration.useRexToCompressDexFiles
    pub fn use_rex_to_compress_dex_files(&self) -> bool {
        self.use_rex_to_compress_dex_files
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1218 AndroidConfiguration.useAndroidResourceShrinking
    pub fn use_android_resource_shrinking(&self) -> bool {
        self.use_android_resource_shrinking
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1223 AndroidConfiguration.useAndroidResourceCycleShrinking
    pub fn use_android_resource_cycle_shrinking(&self) -> bool {
        self.use_android_resource_cycle_shrinking
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1228 AndroidConfiguration.useAndroidResourcePathShortening
    pub fn use_android_resource_path_shortening(&self) -> bool {
        self.use_android_resource_path_shortening
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1233 AndroidConfiguration.useAndroidResourceNameObfuscation
    pub fn use_android_resource_name_obfuscation(&self) -> bool {
        self.use_android_resource_name_obfuscation
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1237 AndroidConfiguration.getManifestMerger
    pub fn get_manifest_merger(&self) -> AndroidConfigurationAndroidManifestMerger {
        self.manifest_merger
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1242 AndroidConfiguration.getManifestMergerValue
    pub fn get_manifest_merger_value(&self) -> String {
        self.manifest_merger.name().to_ascii_lowercase()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1246 AndroidConfiguration.getManifestMergerOrder
    pub fn get_manifest_merger_order(&self) -> AndroidConfigurationManifestMergerOrder {
        self.manifest_merger_order
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1250 AndroidConfiguration.getApkSigningMethod
    pub fn get_apk_signing_method(&self) -> AndroidConfigurationApkSigningMethod {
        self.apk_signing_method
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1255 AndroidConfiguration.apkSigningMethodV1
    pub fn apk_signing_method_v1(&self) -> bool {
        self.apk_signing_method.sign_v1()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1260 AndroidConfiguration.apkSigningMethodV2
    pub fn apk_signing_method_v2(&self) -> bool {
        self.apk_signing_method.sign_v2()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1266 AndroidConfiguration.apkSigningMethodV4
    pub fn apk_signing_method_v4(&self) -> Option<bool> {
        self.apk_signing_method.sign_v4()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1271 AndroidConfiguration.useParallelDex2Oat
    pub fn use_parallel_dex2_oat(&self) -> bool {
        self.use_parallel_dex2_oat
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1276 AndroidConfiguration.breakBuildOnParallelDex2OatFailure
    pub fn break_build_on_parallel_dex2_oat_failure(&self) -> bool {
        self.break_build_on_parallel_dex2_oat_failure
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1281 AndroidConfiguration.compressJavaResources
    pub fn compress_java_resources(&self) -> bool {
        self.compress_java_resources
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1286 AndroidConfiguration.getExportsManifestDefault
    pub fn get_exports_manifest_default(&self) -> bool {
        self.exports_manifest_default
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1291 AndroidConfiguration.omitResourcesInfoProviderFromAndroidBinary
    pub fn omit_resources_info_provider_from_android_binary(&self) -> bool {
        self.omit_resources_info_provider_from_android_binary
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1296 AndroidConfiguration.fixedResourceNeverlinking
    pub fn fixed_resource_neverlinking(&self) -> bool {
        self.fixed_resource_neverlinking
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1301 AndroidConfiguration.checkForMigrationTag
    pub fn check_for_migration_tag(&self) -> bool {
        self.check_for_migration_tag
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1306 AndroidConfiguration.getOneVersionEnforcementUseTransitiveJarsForBinaryUnderTest
    pub fn get_one_version_enforcement_use_transitive_jars_for_binary_under_test(&self) -> bool {
        self.one_version_enforcement_use_transitive_jars_for_binary_under_test
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1311 AndroidConfiguration.persistentAarExtractor
    pub fn persistent_aar_extractor(&self) -> bool {
        self.persistent_aar_extractor
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1316 AndroidConfiguration.persistentBusyboxTools
    pub fn persistent_busybox_tools(&self) -> bool {
        self.persistent_busybox_tools
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1321 AndroidConfiguration.persistentMultiplexBusyboxTools
    pub fn persistent_multiplex_busybox_tools(&self) -> bool {
        self.persistent_multiplex_busybox_tools
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1326 AndroidConfiguration.persistentDexDesugar
    pub fn persistent_dex_desugar(&self) -> bool {
        self.persistent_dex_desugar
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1331 AndroidConfiguration.persistentMultiplexDexDesugar
    pub fn persistent_multiplex_dex_desugar(&self) -> bool {
        self.persistent_multiplex_dex_desugar
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1336 AndroidConfiguration.getOutputDirectoryName
    pub fn get_output_directory_name(&self) -> Option<&'static str> {
        self.configuration_distinguisher.suffix()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1350 AndroidConfiguration.filterRJarsFromAndroidTest
    pub fn filter_r_jars_from_android_test(&self) -> bool {
        self.filter_r_jars_from_android_test
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1354 AndroidConfiguration.removeRClassesFromInstrumentationTestJar
    pub fn remove_r_classes_from_instrumentation_test_jar(&self) -> bool {
        self.remove_r_classes_from_instrumentation_test_jar
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1358 AndroidConfiguration.alwaysFilterDuplicateClassesFromAndroidTest
    pub fn always_filter_duplicate_classes_from_android_test(&self) -> bool {
        self.always_filter_duplicate_classes_from_android_test
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1363 AndroidConfiguration.filterLibraryJarWithProgramJar
    pub fn filter_library_jar_with_program_jar(&self) -> bool {
        self.filter_library_jar_with_program_jar
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1367 AndroidConfiguration.useRTxtFromMergedResources
    pub fn use_r_txt_from_merged_resources(&self) -> bool {
        self.use_r_txt_from_merged_resources
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1371 AndroidConfiguration.disableInstrumentationManifestMerging
    pub fn disable_instrumentation_manifest_merging(&self) -> bool {
        self.disable_instrumentation_manifest_merging
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1376 AndroidConfiguration.getJavaResourcesFromOptimizedJar
    pub fn get_java_resources_from_optimized_jar(&self) -> bool {
        self.get_java_resources_from_optimized_jar
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1380 AndroidConfiguration.outputLibraryMergedAssets
    pub fn output_library_merged_assets(&self) -> bool {
        self.output_library_merged_assets
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1390 AndroidConfiguration.getLegacyMainDexListGenerator
    /// Returns the label provided with --legacy_main_dex_list_generator, if any.
    // TODO(b/147692286): Move R8's main dex list tool into tool repository.
    pub fn get_legacy_main_dex_list_generator(&self) -> Option<&Label> {
        self.legacy_main_dex_list_generator.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1399 AndroidConfiguration.getOptimizingDexer
    /// Returns the label provided with --optimizing_dexer, if any.
    pub fn get_optimizing_dexer(&self) -> Option<&Label> {
        self.optimizing_dexer.as_ref()
    }
}

impl Fragment for AndroidConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &ANDROID_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1133 AndroidConfiguration.shouldInclude
    fn should_include(&self) -> bool {
        !self.disable_android_fragment
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1342 AndroidConfiguration.processForOutputPathMnemonic
    // TODO(blaze-configurability-team): Deprecate this.
    fn process_for_output_path_mnemonic(
        &self,
        ctx: &mut dyn OutputDirectoriesContext,
    ) -> Result<(), AddToMnemonicException> {
        ctx.mark_as_explicit_in_output_path_for("Android configuration distinguisher");
        if let Some(suffix) = self.configuration_distinguisher.suffix() {
            ctx.add_to_mnemonic(Some(suffix))?;
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

    fn create(args: &[&str]) -> Result<AndroidConfiguration, InvalidConfigurationException> {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        AndroidConfiguration::new(&options)
    }

    #[test]
    fn desugar_java8_libs_requires_desugar_java8() {
        let e = create(&["--desugar_java8_libs", "--nodesugar_for_android"]).unwrap_err();
        assert_eq!(
            e.get_message(),
            "Java 8 library support requires --desugar_java8 to be enabled."
        );
    }

    #[test]
    fn negative_shards_rejected() {
        let e = create(&["--experimental_incremental_dexing_after_proguard=-1"]).unwrap_err();
        assert_eq!(
            e.get_message(),
            "--experimental_incremental_dexing_after_proguard must be a positive number"
        );
    }

    // AndroidConfigurationTest (testPlatforms_ordering, testPlatforms_duplicates) tests
    // AndroidConfiguration.Options parsing; it belongs with the options in blaze_optdefs.
}
