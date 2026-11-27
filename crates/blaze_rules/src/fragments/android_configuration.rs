//! The Starlark values of the `android` fragment (`rules/android/AndroidConfiguration.java`,
//! `starlarkbuildapi/android/AndroidConfigurationApi.java`) and the `bazel_android` fragment
//! (`rules/android/BazelAndroidConfiguration.java`).

use blaze_build_config::fragments::android_configuration::{
    ANDROID_CONFIGURATION, AndroidConfiguration,
};
use blaze_build_config::fragments::bazel_android_configuration::{
    BAZEL_ANDROID_CONFIGURATION, BazelAndroidConfiguration,
};
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneOr;
use starlark::values::{Heap, Value};

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.android`: configuration fragment for Android rules.
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:46 AndroidConfiguration
    AndroidConfigurationValue,
    AndroidConfiguration,
    "android",
    "com.google.devtools.build.lib.rules.android.AndroidConfiguration",
    android_configuration_methods
);

fragment_value!(
    /// `ctx.fragments.bazel_android`.
    // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:35 BazelAndroidConfiguration
    BazelAndroidConfigurationValue,
    BazelAndroidConfiguration,
    "bazel_android",
    "com.google.devtools.build.lib.rules.android.BazelAndroidConfiguration",
    bazel_android_configuration_methods
);

pub(super) fn register() {
    super::register(&ANDROID_CONFIGURATION, AndroidConfigurationValue::alloc);
    super::register(
        &BAZEL_ANDROID_CONFIGURATION,
        BazelAndroidConfigurationValue::alloc,
    );
}

/// An `ImmutableList<String>` as a Starlark (immutable) list.
fn string_list<'v>(heap: Heap<'v>, items: &[String]) -> Value<'v> {
    let elems: Vec<Value<'v>> = items.iter().map(|s| heap.alloc(s.as_str())).collect();
    starlark::values::list::alloc_immutable_list(heap, &elems)
}

#[starlark_module]
fn android_configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1139 AndroidConfiguration.useIncrementalDexing
    #[starlark(attribute)]
    fn use_incremental_dexing(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_incremental_dexing())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1145 AndroidConfiguration.incrementalDexingShardsAfterProguard
    #[starlark(attribute)]
    fn incremental_dexing_shards_after_proguard(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<i32> {
        Ok(this.fragment().incremental_dexing_shards_after_proguard())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1151 AndroidConfiguration.incrementalDexingUseDexSharder
    #[starlark(attribute)]
    fn incremental_dexing_use_dex_sharder(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().incremental_dexing_use_dex_sharder())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1157 AndroidConfiguration.incrementalDexingAfterProguardByDefault
    #[starlark(attribute)]
    fn incremental_dexing_after_proguard_by_default(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this
            .fragment()
            .incremental_dexing_after_proguard_by_default())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1255 AndroidConfiguration.apkSigningMethodV1
    #[starlark(attribute)]
    fn apk_signing_method_v1(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().apk_signing_method_v1())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1260 AndroidConfiguration.apkSigningMethodV2
    #[starlark(attribute)]
    fn apk_signing_method_v2(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().apk_signing_method_v2())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1266 AndroidConfiguration.apkSigningMethodV4
    #[starlark(attribute)]
    fn apk_signing_method_v4(this: &AndroidConfigurationValue) -> starlark::Result<NoneOr<bool>> {
        Ok(NoneOr::from_option(this.fragment().apk_signing_method_v4()))
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1166 AndroidConfiguration.assumeMinSdkVersion
    #[starlark(attribute)]
    fn assume_min_sdk_version(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().assume_min_sdk_version())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1172 AndroidConfiguration.getDexoptsSupportedInIncrementalDexing
    #[starlark(attribute)]
    fn get_dexopts_supported_in_incremental_dexing<'v>(
        this: &AndroidConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(string_list(
            heap,
            this.fragment()
                .get_dexopts_supported_in_incremental_dexing(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1178 AndroidConfiguration.getDexoptsSupportedInDexMerger
    #[starlark(attribute)]
    fn get_dexopts_supported_in_dex_merger<'v>(
        this: &AndroidConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(string_list(
            heap,
            this.fragment().get_dexopts_supported_in_dex_merger(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1184 AndroidConfiguration.getDexoptsSupportedInDexSharder
    #[starlark(attribute)]
    fn get_dexopts_supported_in_dex_sharder<'v>(
        this: &AndroidConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(string_list(
            heap,
            this.fragment().get_dexopts_supported_in_dex_sharder(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1193 AndroidConfiguration.getTargetDexoptsThatPreventIncrementalDexing
    #[starlark(attribute)]
    fn get_target_dexopts_that_prevent_incremental_dexing<'v>(
        this: &AndroidConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(string_list(
            heap,
            this.fragment()
                .get_target_dexopts_that_prevent_incremental_dexing(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1198 AndroidConfiguration.desugarJava8
    #[starlark(attribute)]
    fn desugar_java8(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().desugar_java8())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1203 AndroidConfiguration.desugarJava8Libs
    #[starlark(attribute)]
    fn desugar_java8_libs(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().desugar_java8_libs())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1208 AndroidConfiguration.checkDesugarDeps
    #[starlark(attribute)]
    fn check_desugar_deps(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().check_desugar_deps())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1213 AndroidConfiguration.useRexToCompressDexFiles
    #[starlark(attribute)]
    fn use_rex_to_compress_dex_files(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_rex_to_compress_dex_files())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1218 AndroidConfiguration.useAndroidResourceShrinking
    #[starlark(attribute)]
    fn use_android_resource_shrinking(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_android_resource_shrinking())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1223 AndroidConfiguration.useAndroidResourceCycleShrinking
    #[starlark(attribute)]
    fn use_android_resource_cycle_shrinking(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().use_android_resource_cycle_shrinking())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1228 AndroidConfiguration.useAndroidResourcePathShortening
    #[starlark(attribute)]
    fn use_android_resource_path_shortening(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().use_android_resource_path_shortening())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1233 AndroidConfiguration.useAndroidResourceNameObfuscation
    #[starlark(attribute)]
    fn use_android_resource_name_obfuscation(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().use_android_resource_name_obfuscation())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1271 AndroidConfiguration.useParallelDex2Oat
    #[starlark(attribute)]
    fn use_parallel_dex2oat(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().use_parallel_dex2_oat())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1276 AndroidConfiguration.breakBuildOnParallelDex2OatFailure
    #[starlark(attribute)]
    fn break_build_on_parallel_dex2oat_failure(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().break_build_on_parallel_dex2_oat_failure())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1281 AndroidConfiguration.compressJavaResources
    #[starlark(attribute)]
    fn compress_java_resources(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().compress_java_resources())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1286 AndroidConfiguration.getExportsManifestDefault
    #[starlark(attribute)]
    fn get_exports_manifest_default(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().get_exports_manifest_default())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1242 AndroidConfiguration.getManifestMergerValue
    #[starlark(attribute)]
    fn manifest_merger(this: &AndroidConfigurationValue) -> starlark::Result<String> {
        Ok(this.fragment().get_manifest_merger_value())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1291 AndroidConfiguration.omitResourcesInfoProviderFromAndroidBinary
    #[starlark(attribute)]
    fn omit_resources_info_provider_from_android_binary(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this
            .fragment()
            .omit_resources_info_provider_from_android_binary())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1296 AndroidConfiguration.fixedResourceNeverlinking
    #[starlark(attribute)]
    fn fixed_resource_neverlinking(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().fixed_resource_neverlinking())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1301 AndroidConfiguration.checkForMigrationTag
    #[starlark(attribute)]
    fn check_for_migration_tag(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().check_for_migration_tag())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1306 AndroidConfiguration.getOneVersionEnforcementUseTransitiveJarsForBinaryUnderTest
    #[starlark(attribute)]
    fn get_one_version_enforcement_use_transitive_jars_for_binary_under_test(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this
            .fragment()
            .get_one_version_enforcement_use_transitive_jars_for_binary_under_test())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1311 AndroidConfiguration.persistentAarExtractor
    #[starlark(attribute)]
    fn persistent_aar_extractor(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().persistent_aar_extractor())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1316 AndroidConfiguration.persistentBusyboxTools
    #[starlark(attribute)]
    fn persistent_busybox_tools(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().persistent_busybox_tools())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1321 AndroidConfiguration.persistentMultiplexBusyboxTools
    #[starlark(attribute)]
    fn persistent_multiplex_busybox_tools(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().persistent_multiplex_busybox_tools())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1326 AndroidConfiguration.persistentDexDesugar
    #[starlark(attribute)]
    fn persistent_android_dex_desugar(this: &AndroidConfigurationValue) -> starlark::Result<bool> {
        Ok(this.fragment().persistent_dex_desugar())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1331 AndroidConfiguration.persistentMultiplexDexDesugar
    #[starlark(attribute)]
    fn persistent_multiplex_android_dex_desugar(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().persistent_multiplex_dex_desugar())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1336 AndroidConfiguration.getOutputDirectoryName
    #[starlark(attribute)]
    fn get_output_directory_name(this: &AndroidConfigurationValue) -> starlark::Result<String> {
        // The method does not declare `allowReturnNones`: a null return is the unchecked
        // NullPointerException of MethodDescriptor.methodInvocationReturnedNull, which ends the
        // server (Bazel: "FATAL: bazel crashed due to an internal error").
        // Java: net/starlark/java/eval/MethodDescriptor.java:321 MethodDescriptor.methodInvocationReturnedNull
        match this.fragment().get_output_directory_name() {
            Some(name) => Ok(name.to_owned()),
            None => panic!(
                "NullPointerException: method invocation returned null: get_output_directory_name()"
            ),
        }
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1363 AndroidConfiguration.filterLibraryJarWithProgramJar
    #[starlark(attribute)]
    fn filter_library_jar_with_program_jar(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().filter_library_jar_with_program_jar())
    }

    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1376 AndroidConfiguration.getJavaResourcesFromOptimizedJar
    #[starlark(attribute)]
    fn get_java_resources_from_optimized_jar(
        this: &AndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().get_java_resources_from_optimized_jar())
    }
}

#[starlark_module]
fn bazel_android_configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:67 BazelAndroidConfiguration.getMergeAndroidManifestPermissions
    #[starlark(attribute)]
    fn merge_android_manifest_permissions(
        this: &BazelAndroidConfigurationValue,
    ) -> starlark::Result<bool> {
        Ok(this.fragment().get_merge_android_manifest_permissions())
    }
}
