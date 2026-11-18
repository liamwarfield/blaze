//! The Starlark value of the `cpp` configuration fragment (`CppConfiguration`'s
//! `@StarlarkMethod`s and `CppConfigurationApi`), registered with
//! `blaze_analysis::starlark::fragment_collection`.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_analysis::starlark::fragment_collection::{
    FragmentStarlarkFactory, register_fragment_starlark_factory,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::Fragment;
use blaze_build_config::fragments::cpp_configuration::{CPP_CONFIGURATION, CppConfiguration};
use blaze_cmdline::Label;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::list::alloc_immutable_list;
use starlark::values::none::NoneOr;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use super::util::{check_private_starlarkification_allowlist, errorf};

/// The `cpp` fragment as a Starlark value (Java: the `CppConfiguration` object itself).
// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:51 CppConfiguration
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct CppConfigurationValue(#[allocative(skip)] pub Arc<dyn Fragment>);

starlark_simple_value!(CppConfigurationValue);

impl CppConfigurationValue {
    pub fn cpp(&self) -> &CppConfiguration {
        self.0
            .downcast_ref::<CppConfiguration>()
            .expect("the cpp fragment is a CppConfiguration")
    }
}

impl fmt::Debug for CppConfigurationValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CppConfiguration")
    }
}

impl fmt::Display for CppConfigurationValue {
    // Java: StarlarkValue.repr default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<unknown object com.google.devtools.build.lib.rules.cpp.CppConfiguration>")
    }
}

#[starlark_value(type = "cpp")]
impl<'v> StarlarkValue<'v> for CppConfigurationValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("cpp", cpp_configuration_methods);
        Some(RES.methods())
    }

    // Java: Object.equals (CppConfiguration does not override it): the fragment's identity.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<CppConfigurationValue>()
            .is_some_and(|o| Arc::ptr_eq(&o.0, &self.0)))
    }

    /// Hashable (`Fragment.isImmutable()` is true) by identity (Java `Object.hashCode`): the
    /// address of the shared fragment.
    // Java: com/google/devtools/build/lib/analysis/config/Fragment.java:50 Fragment.isImmutable
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (Arc::as_ptr(&self.0) as *const () as usize).hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/config/Fragment.java:50 Fragment.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// The `CppConfiguration` behind a `cpp` value, or Java's parameter type error.
pub fn cpp_configuration_param<'v>(
    x: Value<'v>,
    function: &str,
    param: &str,
) -> starlark::Result<&'v CppConfiguration> {
    x.downcast_ref::<CppConfigurationValue>()
        .map(|c| c.cpp())
        .ok_or_else(|| {
            errorf(format!(
                "in call to {function}(), parameter '{param}' got value of type '{}', want 'cpp'",
                x.get_type()
            ))
        })
}

/// An `ImmutableList<String>` / `StarlarkList.immutableCopyOf(...)` result: an immutable list
/// (`Starlark.fromJava` wraps a Java `List` with `StarlarkList.immutableCopyOf`).
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
fn string_list<'v>(heap: Heap<'v>, items: &[String]) -> Value<'v> {
    let values: Vec<Value<'v>> = items.iter().map(|s| heap.alloc_str(s).to_value()).collect();
    alloc_immutable_list(heap, &values)
}

fn label_or_none<'v>(heap: Heap<'v>, label: Option<&Label>) -> Value<'v> {
    match label {
        Some(l) => heap.alloc(LabelValue(l.clone())),
        None => Value::new_none(),
    }
}

/// `checkInExpandedApiAllowlist(thread, feature)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:899 CppConfiguration.checkInExpandedApiAllowlist
fn check_in_expanded_api_allowlist(eval: &Evaluator, feature: &str) -> starlark::Result<()> {
    check_private_starlarkification_allowlist(eval)
        .map_err(|e| errorf(format!("{e} (feature '{feature}' in CppConfiguration)")))
}

#[starlark_module]
fn cpp_configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:326 CppConfiguration.getCompilationModeForStarlark
    fn compilation_mode<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().get_compilation_mode_for_starlark())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:332 CppConfiguration.getLtoIndexOptionsForStarlark
    fn lto_index_options<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(string_list(eval.heap(), this.cpp().get_lto_index_options()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:340 CppConfiguration.getLtoBackendOptions
    #[starlark(attribute)]
    fn lto_backend_options<'v>(
        this: &CppConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, this.cpp().get_lto_backend_options()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:350 CppConfiguration.getMinimumOsVersion
    fn minimum_os_version(this: &CppConfigurationValue) -> starlark::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.cpp().get_minimum_os_version().map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:362 CppConfiguration.getDynamicModeFlagString
    fn dynamic_mode(this: &CppConfigurationValue) -> starlark::Result<String> {
        Ok(this.cpp().get_dynamic_mode_flag_string().to_owned())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:375 CppConfiguration.shouldStripBinariesForStarlark
    fn should_strip_binaries<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().should_strip_binaries())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:389 CppConfiguration.getStripOptsStarlark
    fn strip_opts<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(string_list(eval.heap(), &this.cpp().get_strip_opts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:400 CppConfiguration.getSaveTempsForStarlark
    fn save_temps<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().get_save_temps())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:425 CppConfiguration.customMalloc
    #[starlark(attribute)]
    fn custom_malloc<'v>(
        this: &CppConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(label_or_none(heap, this.cpp().custom_malloc()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:451 CppConfiguration.buildTestDwpIsActivatedStarlark
    fn build_test_dwp<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().build_test_dwp_is_activated())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:468 CppConfiguration.startEndLibIsRequestedForStarlark
    fn start_end_lib<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().start_end_lib_is_requested())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:478 CppConfiguration.legacyWholeArchiveForStarlark
    fn legacy_whole_archive<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().legacy_whole_archive())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:488 CppConfiguration.removeLegacyWholeArchiveForStarlark
    fn incompatible_remove_legacy_whole_archive<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().remove_legacy_whole_archive())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:501 CppConfiguration.getUseInterfaceSharedLibrariesforStarlark
    fn interface_shared_objects<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().get_use_interface_shared_libraries())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:514 CppConfiguration.getCopts
    #[starlark(attribute)]
    fn copts<'v>(this: &CppConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, &this.cpp().get_copts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:529 CppConfiguration.getCxxopts
    #[starlark(attribute)]
    fn cxxopts<'v>(this: &CppConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, this.cpp().get_cxxopts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:535 CppConfiguration.getConlyopts
    #[starlark(attribute)]
    fn conlyopts<'v>(this: &CppConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, this.cpp().get_conlyopts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:541 CppConfiguration.getObjcopts
    #[starlark(attribute)]
    fn objccopts<'v>(this: &CppConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, this.cpp().get_objcopts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:547 CppConfiguration.getLinkopts
    #[starlark(attribute)]
    fn linkopts<'v>(this: &CppConfigurationValue, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(string_list(heap, this.cpp().get_linkopts()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:627 CppConfiguration.getFdoPathForStarlark
    fn fdo_path<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(NoneOr::from_option(
            this.cpp().get_fdo_path().map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:642 CppConfiguration.csFdoInstrumentStarlark
    fn cs_fdo_instrument<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(NoneOr::from_option(
            this.cpp().get_cs_fdo_instrument().map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:654 CppConfiguration.getCsFdoPathForStarlark
    fn cs_fdo_path<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(NoneOr::from_option(
            this.cpp().get_cs_fdo_path().map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:665 CppConfiguration.getPropellerOptimizeAbsoluteCcProfileForStarlark
    fn propeller_optimize_absolute_cc_profile<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(NoneOr::from_option(
            this.cpp()
                .get_propeller_optimize_absolute_cc_profile()
                .map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:677 CppConfiguration.getPropellerOptimizeAbsoluteLdProfileForStarlark
    fn propeller_optimize_absolute_ld_profile<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(NoneOr::from_option(
            this.cpp()
                .get_propeller_optimize_absolute_ld_profile()
                .map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:692 CppConfiguration.getFdoPrefetchHintsLabel
    #[starlark(attribute)]
    fn _fdo_prefetch_hints_label<'v>(
        this: &CppConfigurationValue,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(label_or_none(
            heap,
            this.cpp().get_fdo_prefetch_hints_label(),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:748 CppConfiguration.useLlvmCoverageMapFormatStarlark
    fn use_llvm_coverage_map_format<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().use_llvm_coverage_map_format())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:762 CppConfiguration.getLibcTopLabelStarlark
    fn grte_top<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(label_or_none(eval.heap(), this.cpp().get_libc_top_label()))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:768 CppConfiguration.shareNativeDepsStarlark
    fn share_native_deps<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().share_native_deps())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:773 CppConfiguration.dontEnableHostNonhost
    #[starlark(attribute)]
    fn _dont_enable_host_nonhost(this: &CppConfigurationValue) -> starlark::Result<bool> {
        Ok(this.cpp().dont_enable_host_nonhost())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:794 CppConfiguration.useSpecificToolFilesForStarlark
    fn incompatible_use_specific_tool_files<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().use_specific_tool_files())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:804 CppConfiguration.disableNocoptsStarlark
    fn disable_nocopts<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().disable_no_copts())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:809 CppConfiguration.appleGenerateDsym
    #[starlark(attribute)]
    fn apple_generate_dsym(this: &CppConfigurationValue) -> starlark::Result<bool> {
        Ok(this.cpp().apple_generate_dsym())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:827 CppConfiguration.shouldScanIncludesForStarlark
    fn include_scanning<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().should_scan_includes())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:833 CppConfiguration.shouldGenerateDotdFilesStarlark
    fn should_generate_dotd_files<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().should_generate_dotd_files())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:843 CppConfiguration.objcShouldGenerateDotdFilesStarlark
    fn objc_should_generate_dotd_files<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().objc_should_generate_dotd_files())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:849 CppConfiguration.objcGenerateLinkmap
    #[starlark(attribute)]
    fn objc_generate_linkmap(this: &CppConfigurationValue) -> starlark::Result<bool> {
        Ok(this.cpp().objc_generate_linkmap())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:861 CppConfiguration.objcEnableBinaryStrippingForStarlark
    fn objc_enable_binary_stripping<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().objc_enable_binary_stripping())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:870 CppConfiguration.experimentalCcImplementationDepsForStarlark
    fn experimental_cc_implementation_deps<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().experimental_cc_implementation_deps())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:876 CppConfiguration.experimentalCppModulesForStarlark
    fn experimental_cpp_modules<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().experimental_cpp_modules())
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CppConfigurationApi.java:106 CppConfigurationApi.macosSetInstallName
    #[starlark(attribute)]
    fn do_not_use_macos_set_install_name(this: &CppConfigurationValue) -> starlark::Result<bool> {
        Ok(this.cpp().macos_set_install_name())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:909 CppConfiguration.forcePicStarlark
    fn force_pic<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_in_expanded_api_allowlist(eval, "force_pic")?;
        Ok(this.cpp().force_pic())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:915 CppConfiguration.generateLlvmLcovStarlark
    fn generate_llvm_lcov<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_in_expanded_api_allowlist(eval, "generate_llvm_lcov")?;
        Ok(this.cpp().generate_llvm_l_cov())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:922 CppConfiguration.fdoInstrumentStarlark
    fn fdo_instrument<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneOr<String>> {
        check_in_expanded_api_allowlist(eval, "fdo_instrument")?;
        Ok(NoneOr::from_option(
            this.cpp().get_fdo_instrument().map(str::to_owned),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:928 CppConfiguration.processHeadersInDependenciesStarlark
    fn process_headers_in_dependencies<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().process_headers_in_dependencies())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:934 CppConfiguration.saveFeatureStateStarlark
    fn save_feature_state<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().save_feature_state())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:940 CppConfiguration.fissionActiveForCurrentCompilationModeStarlark
    fn fission_active_for_current_compilation_mode<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().fission_is_active_for_current_compilation_mode())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:947 CppConfiguration.getExperimentalLinkStaticLibrariesOnce
    fn experimental_link_static_libraries_once<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().experimental_link_static_libraries_once())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:954 CppConfiguration.objcShouldStripBinary
    #[starlark(attribute)]
    fn objc_should_strip_binary(this: &CppConfigurationValue) -> starlark::Result<bool> {
        Ok(this.cpp().objc_should_strip_binary())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:963 CppConfiguration.getProtoProfile
    fn proto_profile<'v>(
        this: &CppConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_private_starlarkification_allowlist(eval)?;
        Ok(this.cpp().get_proto_profile())
    }
}

/// Creates the `cpp` value of a configuration's `CppConfiguration` fragment.
struct CppFragmentFactory;

impl FragmentStarlarkFactory for CppFragmentFactory {
    fn create<'v>(
        &self,
        fragment: &Arc<dyn Fragment>,
        _configuration: &Arc<BuildConfigurationValue>,
        heap: Heap<'v>,
    ) -> Value<'v> {
        heap.alloc(CppConfigurationValue(fragment.clone()))
    }
}

/// Registers the `cpp` fragment's Starlark value factory.
pub fn register_cpp_fragment() {
    register_fragment_starlark_factory(&CPP_CONFIGURATION, Arc::new(CppFragmentFactory));
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/rules/cpp/CppConfigurationStarlarkTest.java
    // These run a Starlark rule reading `ctx.fragments.cpp`; they need the analysis test harness
    // (BuildViewTestCase).

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_minimum_os_version() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_null_minimum_os_version() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_copts() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_cxxopts() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_conlyopts() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_objcopts() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_linkopts() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (Starlark rule analysis harness)"]
    fn test_expanded_api_blocked() {}
}
