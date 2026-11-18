//! Port of `rules/objc/ObjcConfiguration.java`.

use std::any::Any;
use std::sync::Arc;

use blaze_optdefs::fragment::compilation_mode::CompilationMode;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::dotted_version::DottedVersion;
use blaze_optdefs::fragment::objc_command_line_options::ObjcCommandLineOptions;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::OBJC_OPTIONS;

// Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:41 ObjcConfiguration.DBG_COPTS
pub const DBG_COPTS: &[&str] = &[
    "-O0",
    "-DDEBUG=1",
    "-fstack-protector",
    "-fstack-protector-all",
    "-g",
];

// Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:45 ObjcConfiguration.GLIBCXX_DBG_COPTS
pub const GLIBCXX_DBG_COPTS: &[&str] = &[
    "-D_GLIBCXX_DEBUG",
    "-D_GLIBCXX_DEBUG_PEDANTIC",
    "-D_GLIBCPP_CONCEPT_CHECKS",
];

// Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:50 ObjcConfiguration.OPT_COPTS
pub const OPT_COPTS: &[&str] = &[
    "-Os",
    "-DNDEBUG=1",
    "-Wno-unused-variable",
    "-Winit-self",
    "-Wno-extra",
];

// Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:39 ObjcConfiguration
/// A compiler configuration containing flags required for Objective-C compilation.
#[derive(Debug)]
pub struct ObjcConfiguration {
    ios_simulator_version: Option<DottedVersion>,
    ios_simulator_device: Option<String>,
    run_memleaks: bool,
    compilation_mode: CompilationMode,
    fastbuild_options: Vec<String>,
    signing_cert_name: Option<String>,
    debug_with_glibcxx: bool,
    device_debug_entitlements: bool,
    avoid_hardcoded_compilation_flags: bool,
    disallow_sdk_frameworks_attributes: bool,
    alwayslink_by_default: bool,
    strip_executable_safely: bool,
    builtin_objc_strip_action: bool,
}

/// `ObjcConfiguration.class`.
pub static OBJC_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.objc.ObjcConfiguration",
    starlark_name: Some("objc"),
    required_options: || vec![&OBJC_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(ObjcConfiguration::new(options));
    Ok(fragment)
}

impl ObjcConfiguration {
    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:68 ObjcConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> ObjcConfiguration {
        let options = required::<CoreOptions>(build_options);
        let objc_options = required::<ObjcCommandLineOptions>(build_options);

        ObjcConfiguration {
            ios_simulator_device: objc_options.ios_simulator_device.clone(),
            ios_simulator_version: DottedVersion::maybe_unwrap(
                objc_options.ios_simulator_version.as_ref(),
            ),
            run_memleaks: objc_options.run_memleaks,
            compilation_mode: options.compilation_mode,
            fastbuild_options: objc_options.fastbuild_options.clone(),
            signing_cert_name: objc_options.ios_signing_cert_name.clone(),
            debug_with_glibcxx: objc_options.debug_with_glibcxx,
            device_debug_entitlements: objc_options.device_debug_entitlements,
            avoid_hardcoded_compilation_flags: objc_options
                .incompatible_avoid_hardcoded_objc_compilation_flags,
            disallow_sdk_frameworks_attributes: objc_options
                .incompatible_disallow_sdk_frameworks_attributes,
            alwayslink_by_default: objc_options.incompatible_objc_alwayslink_by_default,
            strip_executable_safely: objc_options.incompatible_strip_executable_safely,
            builtin_objc_strip_action: objc_options.incompatible_builtin_objc_strip_action,
        }
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:92 ObjcConfiguration.getIosSimulatorDevice
    /// Returns the type of device (e.g. 'iPhone 6') to simulate when running on the simulator.
    pub fn get_ios_simulator_device(&self) -> Option<&str> {
        // TODO(bazel-team): Deprecate in favor of getSimulatorDeviceForPlatformType(IOS).
        self.ios_simulator_device.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:98 ObjcConfiguration.getIosSimulatorVersion
    pub fn get_ios_simulator_version(&self) -> Option<&DottedVersion> {
        // TODO(bazel-team): Deprecate in favor of getSimulatorVersionForPlatformType(IOS).
        self.ios_simulator_version.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:104 ObjcConfiguration.runMemleaks
    pub fn run_memleaks(&self) -> bool {
        self.run_memleaks
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:111 ObjcConfiguration.getCompilationMode
    /// Returns the current compilation mode.
    pub fn get_compilation_mode(&self) -> CompilationMode {
        self.compilation_mode
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:119 ObjcConfiguration.getCoptsForCompilationMode
    /// Returns the default set of clang options for the current compilation mode.
    pub fn get_copts_for_compilation_mode(&self) -> Vec<String> {
        match self.compilation_mode {
            CompilationMode::DBG => {
                let mut opts: Vec<String> = Vec::new();
                if !self.avoid_hardcoded_compilation_flags {
                    opts.extend(DBG_COPTS.iter().map(|s| s.to_string()));
                }
                if self.debug_with_glibcxx {
                    opts.extend(GLIBCXX_DBG_COPTS.iter().map(|s| s.to_string()));
                }
                opts
            }
            CompilationMode::FASTBUILD => self.fastbuild_options.clone(),
            CompilationMode::OPT => {
                if self.avoid_hardcoded_compilation_flags {
                    Vec::new()
                } else {
                    OPT_COPTS.iter().map(|s| s.to_string()).collect()
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:144 ObjcConfiguration.getSigningCertName
    /// Returns the flag-supplied certificate name to be used in signing or `None` if no such
    /// certificate was specified.
    pub fn get_signing_cert_name(&self) -> Option<&str> {
        self.signing_cert_name.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:155 ObjcConfiguration.useDeviceDebugEntitlements
    /// Returns whether device debug entitlements should be included when signing an application.
    ///
    /// Note that debug entitlements will be included only if the --device_debug_entitlements flag
    /// is set **and** the compilation mode is not `opt`.
    pub fn use_device_debug_entitlements(&self) -> bool {
        self.device_debug_entitlements && self.compilation_mode != CompilationMode::OPT
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:161 ObjcConfiguration.disallowSdkFrameworksAttributes
    /// Returns whether sdk_frameworks and weak_sdk_frameworks attributes are disallowed.
    pub fn disallow_sdk_frameworks_attributes(&self) -> bool {
        self.disallow_sdk_frameworks_attributes
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:167 ObjcConfiguration.alwayslinkByDefault
    /// Returns whether objc_library and objc_import should default to alwayslink=True.
    pub fn alwayslink_by_default(&self) -> bool {
        self.alwayslink_by_default
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:176 ObjcConfiguration.targetShouldAlwayslink
    /// Looks at any explicit value for alwayslink on ctx and then falls back to the value of
    /// alwayslink_by_default.
    pub fn target_should_alwayslink(&self) -> bool {
        blaze_util::unported!("ObjcConfiguration.targetShouldAlwayslink")
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:193 ObjcConfiguration.stripExecutableSafely
    /// Returns whether executable strip action should use flag -x, which does not break dynamic
    /// symbol resolution.
    pub fn strip_executable_safely(&self) -> bool {
        self.strip_executable_safely
    }

    // Java: com/google/devtools/build/lib/rules/objc/ObjcConfiguration.java:199 ObjcConfiguration.builtinObjcStripAction
    /// Returns whether to emit a strip action as part of objc linking.
    pub fn builtin_objc_strip_action(&self) -> bool {
        self.builtin_objc_strip_action
    }
}

impl Fragment for ObjcConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &OBJC_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
