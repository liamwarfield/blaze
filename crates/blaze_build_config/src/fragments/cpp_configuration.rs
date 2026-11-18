//! Port of `rules/cpp/CppConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::cpp_options` and its `DynamicMode`/`StripMode` enums are
//! `blaze_optdefs::fragment::cpp_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::compilation_mode::CompilationMode;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::cpp_configuration::{
    CppConfigurationDynamicMode, CppConfigurationStripMode,
};
use blaze_optdefs::fragment::cpp_options::CppOptions;
use blaze_optdefs::fragment::per_label_options::PerLabelOptions;
use blaze_options::OptionScalar;
use blaze_options::converters::JavaEnum;
use blaze_vfs::PathFragment;

use crate::build_options::BuildOptions;
use crate::fragment::{
    AddToMnemonicException, Fragment, FragmentClass, FragmentClassInfo,
    InvalidConfigurationException, OutputDirectoriesContext, required,
};
use crate::fragment_options_class::CPP_OPTIONS;

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:53 CppConfiguration.BAZEL_TOOLS_REPO
const BAZEL_TOOLS_REPO: &str = "@bazel_tools";

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:56 CppConfiguration.CC_FLAGS_MAKE_VARIABLE_NAME
/// String constant for CC_FLAGS make variable name
pub const CC_FLAGS_MAKE_VARIABLE_NAME: &str = "CC_FLAGS";

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:140 CppConfiguration.FDO_STAMP_MACRO
/// This macro will be passed as a command-line parameter (eg. -DBUILD_FDO_TYPE="AUTOFDO"). For
/// possible values see `CppModel.getFdoBuildStamp()`.
pub const FDO_STAMP_MACRO: &str = "BUILD_FDO_TYPE";

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:59 CppConfiguration.Tool
/// An enumeration of all the tools that comprise a toolchain.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CppConfigurationTool {
    AR,
    CPP,
    GCC,
    GCOV,
    GCOVTOOL,
    LD,
    LLVM_COV,
    NM,
    OBJCOPY,
    OBJDUMP,
    STRIP,
    DWP,
    LLVM_PROFDATA,
}

impl CppConfigurationTool {
    /// `Tool.values()`.
    pub const VALUES: [CppConfigurationTool; 13] = [
        CppConfigurationTool::AR,
        CppConfigurationTool::CPP,
        CppConfigurationTool::GCC,
        CppConfigurationTool::GCOV,
        CppConfigurationTool::GCOVTOOL,
        CppConfigurationTool::LD,
        CppConfigurationTool::LLVM_COV,
        CppConfigurationTool::NM,
        CppConfigurationTool::OBJCOPY,
        CppConfigurationTool::OBJDUMP,
        CppConfigurationTool::STRIP,
        CppConfigurationTool::DWP,
        CppConfigurationTool::LLVM_PROFDATA,
    ];

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:80 CppConfiguration.Tool.getNamePart
    pub fn get_name_part(&self) -> &'static str {
        match self {
            CppConfigurationTool::AR => "ar",
            CppConfigurationTool::CPP => "cpp",
            CppConfigurationTool::GCC => "gcc",
            CppConfigurationTool::GCOV => "gcov",
            CppConfigurationTool::GCOVTOOL => "gcov-tool",
            CppConfigurationTool::LD => "ld",
            CppConfigurationTool::LLVM_COV => "llvm-cov",
            CppConfigurationTool::NM => "nm",
            CppConfigurationTool::OBJCOPY => "objcopy",
            CppConfigurationTool::OBJDUMP => "objdump",
            CppConfigurationTool::STRIP => "strip",
            CppConfigurationTool::DWP => "dwp",
            CppConfigurationTool::LLVM_PROFDATA => "llvm-profdata",
        }
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:88 CppConfiguration.HeadersCheckingMode
/// Values for the --hdrs_check option. Note that Bazel only supports and will default to
/// "strict".
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CppConfigurationHeadersCheckingMode {
    /// Legacy behavior: Silently allow any source header file in any of the directories of the
    /// containing package to be included by sources in this rule and dependent rules.
    LOOSE,
    /// Disallow undeclared headers.
    STRICT,
}

impl CppConfigurationHeadersCheckingMode {
    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:97 CppConfiguration.HeadersCheckingMode.getValue
    pub fn get_value(value: &str) -> CppConfigurationHeadersCheckingMode {
        if value.eq_ignore_ascii_case("loose") || value.eq_ignore_ascii_case("warn") {
            return CppConfigurationHeadersCheckingMode::LOOSE;
        }
        if value.eq_ignore_ascii_case("strict") {
            return CppConfigurationHeadersCheckingMode::STRICT;
        }
        panic!("IllegalArgumentException")
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:51 CppConfiguration
/// This class represents the C/C++ parts of the `BuildConfiguration`, including the host
/// architecture, target architecture, compiler version, and a standard library version.
#[derive(Debug)]
pub struct CppConfiguration {
    fdo_path: Option<String>,
    fdo_optimize_label: Option<Label>,

    cs_fdo_absolute_path: Option<String>,
    propeller_optimize_absolute_cc_profile: Option<String>,
    propeller_optimize_absolute_ld_profile: Option<String>,

    conlyopts: Vec<String>,

    copts: Vec<String>,
    cxxopts: Vec<String>,
    objcopts: Vec<String>,

    linkopts: Vec<String>,
    ltoindex_options: Vec<String>,
    ltobackend_options: Vec<String>,

    cpp_options: CppOptions,

    // The dynamic mode for linking.
    strip_binaries: bool,
    compilation_mode: CompilationMode,
    collect_code_coverage: bool,

    apple_generate_dsym: bool,
}

/// `CppConfiguration.class`.
pub static CPP_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.cpp.CppConfiguration",
    starlark_name: Some("cpp"),
    required_options: || vec![&CPP_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(CppConfiguration::new(options)?);
    Ok(fragment)
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:60 FileSystemUtils.checkBaseName
/// Throws `IllegalArgumentException` if `base_name` is not a valid base name.
// PORT: ported locally (blaze_vfs keeps its copy private); the `Err` is the
// IllegalArgumentException's message.
fn check_base_name(base_name: &str) -> Result<(), String> {
    if base_name.is_empty() {
        return Err("Child must not be empty string ('')".to_string());
    }
    if base_name == "." || base_name == ".." {
        return Err(format!("baseName must not be '{}'", base_name));
    }
    if base_name.contains('/') {
        return Err(format!(
            "baseName must not contain a slash: '{}'",
            base_name
        ));
    }
    Ok(())
}

impl CppConfiguration {
    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:168 CppConfiguration.<init>
    pub fn new(options: &BuildOptions) -> Result<CppConfiguration, InvalidConfigurationException> {
        let cpp_options = required::<CppOptions>(options);

        let common_options = required::<CoreOptions>(options);
        let compilation_mode = common_options.compilation_mode;

        let mut linkopts_builder: Vec<String> = Vec::new();
        linkopts_builder.extend(cpp_options.linkopt_list.iter().cloned());
        if cpp_options.experimental_omitfp {
            linkopts_builder.push("-Wl,--eh-frame-hdr".to_string());
        }

        let mut fdo_path: Option<PathFragment> = None;
        let mut fdo_profile_label: Option<Label> = None;
        if let Some(fdo_optimize) = cpp_options.get_fdo_optimize() {
            if fdo_optimize.starts_with("//") {
                fdo_profile_label = Some(
                    Label::parse_canonical(fdo_optimize)
                        .map_err(|e| InvalidConfigurationException::new(e.get_message()))?,
                );
            } else {
                if !cpp_options.enable_fdo_profile_absolute_path {
                    return Err(InvalidConfigurationException::new(
                        "Please use --fdo_profile instead of an absolute path set with \
                         --fdo_optimize. Using absolute paths may be temporary reenabled with \
                         --enable_fdo_profile_absolute_path",
                    ));
                }
                let path = PathFragment::create(fdo_optimize);
                if !path.is_absolute() {
                    return Err(InvalidConfigurationException::new(format!(
                        "Path of '{}' in --fdo_optimize has to be either an absolute path or a \
                         label.",
                        path.get_path_string()
                    )));
                }
                // We don't check for file existence, but at least the filename should be
                // well-formed.
                check_base_name(path.get_base_name())
                    .map_err(InvalidConfigurationException::new)?;
                fdo_path = Some(path);
            }
        }

        let mut cs_fdo_absolute_path: Option<PathFragment> = None;
        if let Some(cs_fdo) = &cpp_options.cs_fdo_absolute_path_for_build {
            if !cpp_options.enable_fdo_profile_absolute_path {
                return Err(InvalidConfigurationException::new(
                    "Please use --cs_fdo_optimize instead of an absolute path set with \
                     --cs_fdo_absolute_path.Using absolute paths may be temporary reenabled with \
                     --enable_fdo_profile_absolute_path",
                ));
            }
            let path = PathFragment::create(cs_fdo);
            if !path.is_absolute() {
                return Err(InvalidConfigurationException::new(format!(
                    "Path of '{}' in --cs_fdo_absolute_path is not an absolute path.",
                    path.get_path_string()
                )));
            }
            check_base_name(path.get_base_name()).map_err(InvalidConfigurationException::new)?;
            cs_fdo_absolute_path = Some(path);
        }

        let mut propeller_optimize_absolute_cc_profile: Option<PathFragment> = None;
        if let Some(profile) = &cpp_options.propeller_optimize_absolute_cc_profile {
            if !cpp_options.enable_propeller_optimize_absolute_path {
                return Err(InvalidConfigurationException::new(
                    "Please use --propeller_optimize instead of an absolute path set with \
                     --propeller_optimize_absolute_cc_profile. Using absolute paths may be \
                     temporary reenabled with --enable_propeller_optimize_absolute_paths",
                ));
            }
            let path = PathFragment::create(profile);
            if !path.is_absolute() {
                return Err(InvalidConfigurationException::new(format!(
                    "Path of '{}' in --propeller_optimize_absolute_cc_profile is not an absolute \
                     path.",
                    path.get_path_string()
                )));
            }
            check_base_name(path.get_base_name()).map_err(InvalidConfigurationException::new)?;
            propeller_optimize_absolute_cc_profile = Some(path);
        }

        let mut propeller_optimize_absolute_ld_profile: Option<PathFragment> = None;
        if let Some(profile) = &cpp_options.propeller_optimize_absolute_ld_profile {
            if !cpp_options.enable_propeller_optimize_absolute_path {
                return Err(InvalidConfigurationException::new(
                    "Please use --propeller_optimize instead of an absolute path set with \
                     --propeller_optimize_absolute_ld_profile. Using absolute paths may be \
                     temporary reenabled with --enable_fdo_profile_absolute_path",
                ));
            }
            let path = PathFragment::create(profile);
            if !path.is_absolute() {
                return Err(InvalidConfigurationException::new(format!(
                    "Path of '{}' in --propeller_optimize_absolute_ld_profile is not an absolute \
                     path.",
                    path.get_path_string()
                )));
            }
            check_base_name(path.get_base_name()).map_err(InvalidConfigurationException::new)?;
            propeller_optimize_absolute_ld_profile = Some(path);
        }

        Ok(CppConfiguration {
            fdo_path: fdo_path.map(|p| p.get_path_string().to_string()),
            fdo_optimize_label: fdo_profile_label,
            cs_fdo_absolute_path: cs_fdo_absolute_path.map(|p| p.get_path_string().to_string()),
            propeller_optimize_absolute_cc_profile: propeller_optimize_absolute_cc_profile
                .map(|p| p.get_path_string().to_string()),
            propeller_optimize_absolute_ld_profile: propeller_optimize_absolute_ld_profile
                .map(|p| p.get_path_string().to_string()),
            conlyopts: cpp_options.conlyopt_list.clone(),
            copts: cpp_options.copt_list.clone(),
            cxxopts: cpp_options.cxxopt_list.clone(),
            objcopts: cpp_options.objcopt_list.clone(),
            linkopts: linkopts_builder,
            ltoindex_options: cpp_options.ltoindexopt_list.clone(),
            ltobackend_options: cpp_options.ltobackendopt_list.clone(),
            cpp_options: cpp_options.clone(),
            strip_binaries: cpp_options.strip_binaries == CppConfigurationStripMode::ALWAYS
                || (cpp_options.strip_binaries == CppConfigurationStripMode::SOMETIMES
                    && compilation_mode == CompilationMode::FASTBUILD),
            compilation_mode,
            collect_code_coverage: common_options.collect_code_coverage,
            apple_generate_dsym: cpp_options.apple_generate_dsym,
        })
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:310 CppConfiguration.getFdoZipper
    /// The zipper label for FDO.
    pub fn get_fdo_zipper(&self) -> Option<Label> {
        if self.get_fdo_optimize_label().is_some()
            || self.get_fdo_profile_label().is_some()
            || self.fdo_path.is_some()
            || self.get_mem_prof_profile_label().is_some()
            || self.get_x_fdo_profile_label().is_some()
        {
            return Some(Label::parse_canonical_unchecked(&format!(
                "{}//tools/zip:unzip_fdo",
                BAZEL_TOOLS_REPO
            )));
        }
        None
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:322 CppConfiguration.getCompilationMode
    /// Returns the configured current compilation mode.
    pub fn get_compilation_mode(&self) -> CompilationMode {
        self.compilation_mode
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:327 CppConfiguration.getCompilationModeForStarlark
    /// `compilationMode.toString()`.
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_compilation_mode_for_starlark(&self) -> String {
        self.compilation_mode.java_to_string()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:333 CppConfiguration.getLtoIndexOptionsForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_lto_index_options(&self) -> &[String] {
        &self.ltoindex_options
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:341 CppConfiguration.getLtoBackendOptions
    /// Returns the set of command-line LTO backend options.
    pub fn get_lto_backend_options(&self) -> &[String] {
        &self.ltobackend_options
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:350 CppConfiguration.getMinimumOsVersion
    /// The minimum OS version for C/C++ compilation.
    pub fn get_minimum_os_version(&self) -> Option<&str> {
        self.cpp_options.minimum_os_version.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:355 CppConfiguration.getDynamicModeFlag
    /// Returns the value of the --dynamic_mode flag.
    pub fn get_dynamic_mode_flag(&self) -> CppConfigurationDynamicMode {
        self.cpp_options.dynamic_mode
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:362 CppConfiguration.getDynamicModeFlagString
    /// Whether C/C++ binaries/tests were requested to be linked dynamically.
    pub fn get_dynamic_mode_flag_string(&self) -> &'static str {
        self.cpp_options.dynamic_mode.name()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:366 CppConfiguration.useArgsParamsFile
    pub fn use_args_params_file(&self) -> bool {
        self.cpp_options.use_args_params_file
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:371 CppConfiguration.shouldStripBinaries
    /// Returns whether or not to strip the binaries.
    pub fn should_strip_binaries(&self) -> bool {
        self.strip_binaries
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:385 CppConfiguration.getStripOpts
    /// Returns the additional options to pass to strip when generating a `<name>.stripped`
    /// binary by this build.
    pub fn get_strip_opts(&self) -> Vec<String> {
        self.cpp_options.stripopt_list.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:396 CppConfiguration.getSaveTemps
    /// Returns whether temporary outputs from gcc will be saved.
    pub fn get_save_temps(&self) -> bool {
        self.cpp_options.save_temps
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:410 CppConfiguration.getPerFileCopts
    /// Returns the `PerLabelOptions` to apply to the gcc command line, if the label of the
    /// compiled file matches the regular expression.
    pub fn get_per_file_copts(&self) -> Vec<PerLabelOptions> {
        self.cpp_options.per_file_copts.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:418 CppConfiguration.getPerFileLtoBackendOpts
    /// Returns the `PerLabelOptions` to apply to the LTO Backend command line, if the compiled
    /// object matches the regular expression.
    pub fn get_per_file_lto_backend_opts(&self) -> Vec<PerLabelOptions> {
        self.cpp_options.per_file_lto_backend_opts.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:428 CppConfiguration.customMalloc
    /// Returns the custom malloc library label.
    pub fn custom_malloc(&self) -> Option<&Label> {
        self.cpp_options.custom_malloc.as_ref().map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:433 CppConfiguration.processHeadersInDependencies
    /// Returns whether we are processing headers in dependencies of built C++ targets.
    pub fn process_headers_in_dependencies(&self) -> bool {
        self.cpp_options.process_headers_in_dependencies
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:438 CppConfiguration.fissionIsActiveForCurrentCompilationMode
    /// Returns true if --fission contains the current compilation mode.
    pub fn fission_is_active_for_current_compilation_mode(&self) -> bool {
        self.cpp_options
            .fission_modes
            .contains(&self.compilation_mode)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:443 CppConfiguration.buildTestDwpIsActivated
    /// Returns true if --build_test_dwp is set on this build.
    pub fn build_test_dwp_is_activated(&self) -> bool {
        self.cpp_options.build_test_dwp
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:459 CppConfiguration.forcePic
    /// Returns true if all C++ compilations should produce position-independent code, links
    /// should produce position-independent executables, and dependencies with equivalent
    /// pre-built pic and nopic versions should apply the pic versions. Returns false if default
    /// settings should be applied (i.e. make no special provisions for pic code).
    pub fn force_pic(&self) -> bool {
        self.cpp_options.force_pic
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:464 CppConfiguration.startEndLibIsRequested
    /// Returns true if --start_end_lib is set on this build.
    pub fn start_end_lib_is_requested(&self) -> bool {
        self.cpp_options.use_start_end_lib
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:474 CppConfiguration.experimentalLinkStaticLibrariesOnce
    pub fn experimental_link_static_libraries_once(&self) -> bool {
        self.cpp_options.experimental_link_static_libraries_once
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:479 CppConfiguration.legacyWholeArchiveForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn legacy_whole_archive(&self) -> bool {
        self.cpp_options.legacy_whole_archive
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:488 CppConfiguration.removeLegacyWholeArchiveForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn remove_legacy_whole_archive(&self) -> bool {
        self.cpp_options.remove_legacy_whole_archive
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:493 CppConfiguration.getInmemoryDotdFiles
    pub fn get_inmemory_dotd_files(&self) -> bool {
        self.cpp_options.inmemory_dotd_files
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:497 CppConfiguration.getUseInterfaceSharedLibraries
    pub fn get_use_interface_shared_libraries(&self) -> bool {
        self.cpp_options.use_interface_shared_objects
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:509 CppConfiguration.isOmitfp
    /// Returns whether this configuration will use libunwind for stack unwinding.
    pub fn is_omitfp(&self) -> bool {
        self.cpp_options.experimental_omitfp
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:515 CppConfiguration.getCopts
    /// Returns flags passed to Bazel by --copt option.
    pub fn get_copts(&self) -> Vec<String> {
        if self.is_omitfp() {
            let mut result: Vec<String> = vec![
                "-fomit-frame-pointer".to_string(),
                "-fasynchronous-unwind-tables".to_string(),
                "-DNO_FRAME_POINTER".to_string(),
            ];
            result.extend(self.copts.iter().cloned());
            return result;
        }
        self.copts.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:529 CppConfiguration.getCxxopts
    /// Returns flags passed to Bazel by --cxxopt option.
    pub fn get_cxxopts(&self) -> &[String] {
        &self.cxxopts
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:535 CppConfiguration.getConlyopts
    /// Returns flags passed to Bazel by --conlyopt option.
    pub fn get_conlyopts(&self) -> &[String] {
        &self.conlyopts
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:541 CppConfiguration.getObjcopts
    /// Returns flags passed to Bazel by --objccopt option.
    pub fn get_objcopts(&self) -> &[String] {
        &self.objcopts
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:547 CppConfiguration.getLinkopts
    /// Returns flags passed to Bazel by --linkopt option.
    pub fn get_linkopts(&self) -> &[String] {
        &self.linkopts
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:608 CppConfiguration.shareNativeDeps
    /// Returns true if we should share identical native libraries between different targets.
    pub fn share_native_deps(&self) -> bool {
        self.cpp_options.share_native_deps
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:612 CppConfiguration.isStrictSystemIncludes
    pub fn is_strict_system_includes(&self) -> bool {
        self.cpp_options.strict_system_includes
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:617 CppConfiguration.getFdoInstrument
    pub fn get_fdo_instrument(&self) -> Option<&str> {
        self.cpp_options.fdo_instrument_for_build.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:627 CppConfiguration.getFdoPathForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_fdo_path(&self) -> Option<&str> {
        self.fdo_path.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:633 CppConfiguration.getFdoOptimizeLabel
    /// The label specified in --fdo_optimize
    pub fn get_fdo_optimize_label(&self) -> Option<&Label> {
        self.fdo_optimize_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:637 CppConfiguration.getCSFdoInstrument
    pub fn get_cs_fdo_instrument(&self) -> Option<&str> {
        self.cpp_options.cs_fdo_instrument_for_build.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:654 CppConfiguration.getCsFdoPathForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_cs_fdo_path(&self) -> Option<&str> {
        self.cs_fdo_absolute_path.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:665 CppConfiguration.getPropellerOptimizeAbsoluteCcProfileForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_propeller_optimize_absolute_cc_profile(&self) -> Option<&str> {
        self.propeller_optimize_absolute_cc_profile.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:677 CppConfiguration.getPropellerOptimizeAbsoluteLdProfileForStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_propeller_optimize_absolute_ld_profile(&self) -> Option<&str> {
        self.propeller_optimize_absolute_ld_profile.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:692 CppConfiguration.getFdoPrefetchHintsLabel
    pub fn get_fdo_prefetch_hints_label(&self) -> Option<&Label> {
        self.cpp_options.get_fdo_prefetch_hints_label()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:312 CppConfiguration.getFdoProfileLabel
    /// The label specified in --fdo_profile
    pub fn get_fdo_profile_label(&self) -> Option<&Label> {
        self.cpp_options.fdo_profile_label.as_ref().map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:704 CppConfiguration.getCSFdoProfileLabel
    pub fn get_cs_fdo_profile_label(&self) -> Option<&Label> {
        self.cpp_options
            .cs_fdo_profile_label
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:712 CppConfiguration.getPropellerOptimizeLabel
    pub fn get_propeller_optimize_label(&self) -> Option<&Label> {
        if self.cpp_options.fdo_instrument_for_build.is_some()
            || self.cpp_options.cs_fdo_instrument_for_build.is_some()
        {
            return None;
        }
        self.cpp_options.get_propeller_optimize_label()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:315 CppConfiguration.getXFdoProfileLabel
    /// The label specified in --xbinary_fdo
    pub fn get_x_fdo_profile_label(&self) -> Option<&Label> {
        if self.cpp_options.fdo_optimize_for_build.is_some()
            || self.cpp_options.fdo_instrument_for_build.is_some()
            || self.cpp_options.fdo_profile_label.is_some()
            || self.collect_code_coverage
        {
            return None;
        }

        self.cpp_options
            .xfdo_profile_label
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:314 CppConfiguration.getMemProfProfileLabel
    pub fn get_mem_prof_profile_label(&self) -> Option<&Label> {
        self.cpp_options.get_mem_prof_profile_label()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:740 CppConfiguration.useLLVMCoverageMapFormat
    pub fn use_llvm_coverage_map_format(&self) -> bool {
        self.cpp_options.use_llvm_coverage_map_format
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:757 CppConfiguration.getLibcTopLabel
    /// Returns the value of the libc top-level directory (--grte_top) as specified on the
    /// command line
    pub fn get_libc_top_label(&self) -> Option<&Label> {
        self.cpp_options.libc_top_label.as_ref().map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:774 CppConfiguration.dontEnableHostNonhost
    pub fn dont_enable_host_nonhost(&self) -> bool {
        self.cpp_options.dont_enable_host_nonhost
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:778 CppConfiguration.collectCodeCoverage
    pub fn collect_code_coverage(&self) -> bool {
        self.collect_code_coverage
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:782 CppConfiguration.saveFeatureState
    pub fn save_feature_state(&self) -> bool {
        self.cpp_options.save_feature_state
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:786 CppConfiguration.useSpecificToolFiles
    pub fn use_specific_tool_files(&self) -> bool {
        self.cpp_options.use_specific_tool_files
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:799 CppConfiguration.disableNoCopts
    pub fn disable_no_copts(&self) -> bool {
        self.cpp_options.disable_no_copts
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:810 CppConfiguration.appleGenerateDsym
    pub fn apple_generate_dsym(&self) -> bool {
        self.apple_generate_dsym
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:814 CppConfiguration.useCppCompileHeaderMnemonic
    pub fn use_cpp_compile_header_mnemonic(&self) -> bool {
        self.cpp_options.use_cpp_compile_header_mnemonic
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:818 CppConfiguration.generateLlvmLCov
    pub fn generate_llvm_l_cov(&self) -> bool {
        self.cpp_options.generate_llvm_lcov
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:823 CppConfiguration.shouldScanIncludes
    /// Returns true iff we should do "include scanning" during this build.
    pub fn should_scan_includes(&self) -> bool {
        self.cpp_options.include_scanning || self.cpp_options.include_scanning_internal
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:834 CppConfiguration.shouldGenerateDotdFilesStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn should_generate_dotd_files(&self) -> bool {
        self.cpp_options.generate_dotd_files
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:843 CppConfiguration.objcShouldGenerateDotdFilesStarlark
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn objc_should_generate_dotd_files(&self) -> bool {
        self.cpp_options.objc_generate_dotd_files
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:849 CppConfiguration.objcGenerateLinkmap
    pub fn objc_generate_linkmap(&self) -> bool {
        self.cpp_options.objc_generate_linkmap
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:853 CppConfiguration.objcEnableBinaryStripping
    pub fn objc_enable_binary_stripping(&self) -> bool {
        self.cpp_options.objc_enable_binary_stripping
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:882 CppConfiguration.experimentalCcImplementationDeps
    pub fn experimental_cc_implementation_deps(&self) -> bool {
        self.cpp_options.experimental_cc_implementation_deps
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:886 CppConfiguration.experimentalCppModules
    pub fn experimental_cpp_modules(&self) -> bool {
        self.cpp_options.experimental_cpp_modules
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:890 CppConfiguration.getExperimentalCppCompileResourcesEstimation
    pub fn get_experimental_cpp_compile_resources_estimation(&self) -> bool {
        self.cpp_options
            .experimental_cpp_compile_resources_estimation
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:895 CppConfiguration.macosSetInstallName
    pub fn macos_set_install_name(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:954 CppConfiguration.objcShouldStripBinary
    pub fn objc_should_strip_binary(&self) -> bool {
        self.objc_enable_binary_stripping() && self.get_compilation_mode() == CompilationMode::OPT
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:959 CppConfiguration.getProtoProfilePath
    pub fn get_proto_profile_path(&self) -> Option<&Label> {
        self.cpp_options
            .proto_profile_path
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:964 CppConfiguration.getProtoProfile
    // PORT: the Starlark method's allowlist check belongs to the Starlark wrapper.
    pub fn get_proto_profile(&self) -> bool {
        self.cpp_options.proto_profile
    }
}

impl Fragment for CppConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &CPP_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:552 CppConfiguration.reportInvalidOptions
    fn report_invalid_options(&self, reporter: &dyn EventHandler, build_options: &BuildOptions) {
        let cpp_options = required::<CppOptions>(build_options);
        if self.strip_binaries {
            let mut warn = cpp_options.copt_list.iter().any(|o| o == "-g");
            for opt in &cpp_options.per_file_copts {
                warn |= opt.get_options().iter().any(|o| o == "-g");
            }
            if warn {
                reporter.handle(&Event::warn(
                    "Stripping enabled, but '--copt=-g' (or --per_file_copt=...@-g) specified. \
                     Debug information will be generated and then stripped away. This is \
                     probably not what you want! Use '-c dbg' for debug mode, or use \
                     '--strip=never' to disable stripping",
                ));
            }
        }

        // FDO
        if cpp_options.get_fdo_optimize().is_some() && cpp_options.fdo_profile_label.is_some() {
            reporter.handle(&Event::error(
                "Both --fdo_optimize and --fdo_profile specified",
            ));
        }

        if cpp_options.fdo_instrument_for_build.is_some() {
            if cpp_options.get_fdo_optimize().is_some() || cpp_options.fdo_profile_label.is_some() {
                reporter.handle(&Event::error(
                    "Cannot instrument and optimize for FDO at the same time. Remove one of the \
                     '--fdo_instrument' and '--fdo_optimize/--fdo_profile' options",
                ));
            }
            if !cpp_options.copt_list.iter().any(|o| o == "-Wno-error") {
                // This is effectively impossible. --fdo_instrument adds this value, and only
                // invocation policy could remove it.
                reporter.handle(&Event::error(
                    "Cannot instrument FDO without --copt including -Wno-error.",
                ));
            }
        }

        // This is an assertion check vs. user error because users can't trigger this state.
        // TODO(b/253313672): uncomment the below and check tests don't fail. This was originally
        // set check the exec configuration doesn't apply FDO settings. With the host
        // configuration gone we should migrate this check to the exec config. Since there's a
        // chance of breakage it's best to test this as its own dedicated change.
        // Verify.verify(
        //   !(buildOptions.get(CoreOptions.class).isExec && cppOptions.isFdo()),
        // "FDO state should not propagate to the exec configuration");
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:599 CppConfiguration.processForOutputPathMnemonic
    fn process_for_output_path_mnemonic(
        &self,
        ctx: &mut dyn OutputDirectoriesContext,
    ) -> Result<(), AddToMnemonicException> {
        ctx.mark_as_explicit_in_output_path_for("cc_output_directory_tag");
        if !self.cpp_options.output_directory_tag.is_empty() {
            ctx.add_to_mnemonic(Some(self.cpp_options.output_directory_tag.as_str()))?;
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
    use blaze_events::{EventCollector, EventKind};

    fn create(args: &[&str]) -> Result<CppConfiguration, InvalidConfigurationException> {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        CppConfiguration::new(&options)
    }

    #[test]
    fn linkopts_with_omitfp() {
        let config = create(&["--linkopt=-lm", "--experimental_omitfp"]).unwrap();
        assert_eq!(config.get_linkopts(), &["-lm", "-Wl,--eh-frame-hdr"]);
        assert_eq!(
            config.get_copts()[..3],
            [
                "-fomit-frame-pointer",
                "-fasynchronous-unwind-tables",
                "-DNO_FRAME_POINTER"
            ]
        );
    }

    #[test]
    fn fdo_optimize_label_parsed() {
        let config = create(&["--fdo_optimize=//foo:profile"]).unwrap();
        assert_eq!(
            config.get_fdo_optimize_label(),
            Some(&Label::parse_canonical_unchecked("//foo:profile"))
        );
        assert_eq!(
            config.get_fdo_zipper(),
            Some(Label::parse_canonical_unchecked(
                "@bazel_tools//tools/zip:unzip_fdo"
            ))
        );
    }

    #[test]
    fn fdo_optimize_absolute_path_disallowed() {
        let e = create(&[
            "--fdo_optimize=/tmp/profile.zip",
            "--noenable_remaining_fdo_absolute_paths",
        ])
        .unwrap_err();
        assert_eq!(
            e.get_message(),
            "Please use --fdo_profile instead of an absolute path set with --fdo_optimize. Using \
             absolute paths may be temporary reenabled with --enable_fdo_profile_absolute_path"
        );
    }

    #[test]
    fn fdo_optimize_relative_path_rejected() {
        let e = create(&["--fdo_optimize=tmp/profile.zip"]).unwrap_err();
        assert_eq!(
            e.get_message(),
            "Path of 'tmp/profile.zip' in --fdo_optimize has to be either an absolute path or a \
             label."
        );
    }

    #[test]
    fn strip_warning_for_copt_g() {
        let options = BuildOptions::of_args(
            &all_known_fragment_options_classes(),
            &["--strip=always", "--copt=-g"],
        )
        .unwrap();
        let config = CppConfiguration::new(&options).unwrap();
        let collector = EventCollector::new();
        config.report_invalid_options(&collector, &options);
        let events: Vec<_> = collector.iter().collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get_kind(), EventKind::Warning);
    }

    #[test]
    fn output_directory_tag_check() {
        assert!(create(&["--cc_output_directory_tag=foo"]).is_ok());
    }

    // CppConfigurationStarlarkTest needs Starlark rules and full analysis.
    #[test]
    #[ignore = "needs analysis"]
    fn test_minimum_os_version() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_null_minimum_os_version() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_copts() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_cxxopts() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_conlyopts() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_objcopts() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_linkopts() {}

    #[test]
    #[ignore = "needs analysis"]
    fn test_expanded_api_blocked() {}
}
