//! The `Class<? extends FragmentOptions>` handles of Bazel's registered fragment options.
//!
//! Java code passes `Class` objects around (`BuildOptions.get(Class)`, `getClass().getName()` as
//! the sort key and in `cacheKey()`). The Rust options classes ([`OptionsClassRef`]) carry the
//! Rust struct name only, so this module pairs each registered class with its Java binary class
//! name and the type-erased `FragmentOptions` hooks (`getDefault`, `getNormalized`).

use std::any::TypeId;
use std::fmt;

use blaze_optdefs::fragment::FragmentOptions;
use blaze_optdefs::fragment::android_configuration::AndroidConfigurationOptions;
use blaze_optdefs::fragment::apple_command_line_options::AppleCommandLineOptions;
use blaze_optdefs::fragment::bazel_android_configuration::BazelAndroidConfigurationOptions;
use blaze_optdefs::fragment::bazel_python_configuration::BazelPythonConfigurationOptions;
use blaze_optdefs::fragment::bazel_rule_class_provider::BazelRuleClassProviderStrictActionEnvOptions;
use blaze_optdefs::fragment::config_feature_flag_options::ConfigFeatureFlagOptions;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::coverage_configuration::CoverageConfigurationCoverageOptions;
use blaze_optdefs::fragment::cpp_options::CppOptions;
use blaze_optdefs::fragment::j2objc_command_line_options::J2ObjcCommandLineOptions;
use blaze_optdefs::fragment::java_options::JavaOptions;
use blaze_optdefs::fragment::objc_command_line_options::ObjcCommandLineOptions;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_optdefs::fragment::proto_configuration::ProtoConfigurationOptions;
use blaze_optdefs::fragment::python_options::PythonOptions;
use blaze_optdefs::fragment::shell_configuration::ShellConfigurationOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_options::OptionsClass;
use blaze_options::definition::OptionDefinitionRef;
use blaze_options::definition::{OptionsBase, OptionsClassRef};

/// A registered `FragmentOptions` subclass (Java: the `Class<? extends FragmentOptions>` object).
pub struct FragmentOptionsClassInfo {
    /// `Class.getName()`: the Java binary class name (`Outer$Inner` for nested classes).
    java_name: &'static str,
    class: fn() -> OptionsClassRef,
    get_default: fn() -> Box<dyn OptionsBase>,
    get_normalized: fn(&dyn OptionsBase) -> Box<dyn OptionsBase>,
}

/// A reference to a registered fragment options class. Equality is identity (Java `Class`).
pub type FragmentOptionsClass = &'static FragmentOptionsClassInfo;

impl FragmentOptionsClassInfo {
    /// `Class.getName()`.
    pub fn get_name(&self) -> &'static str {
        self.java_name
    }

    /// `Class.getSimpleName()`: the part after the last `.` or `$`.
    pub fn get_simple_name(&self) -> &'static str {
        let n = self.java_name;
        let i = n.rfind(['.', '$']).map(|i| i + 1).unwrap_or(0);
        &n[i..]
    }

    /// The Rust options class.
    pub fn options_class(&self) -> OptionsClassRef {
        (self.class)()
    }

    pub fn type_id(&self) -> TypeId {
        self.options_class().type_id()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:47 FragmentOptions.getDefault
    pub fn get_default(&self) -> Box<dyn OptionsBase> {
        (self.get_default)()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:71 FragmentOptions.getNormalized
    pub fn get_normalized(&self, options: &dyn OptionsBase) -> Box<dyn OptionsBase> {
        (self.get_normalized)(options)
    }

    /// `OptionsParser.getOptionDefinitions(clazz)`: sorted by option name.
    pub fn get_option_definitions(&self) -> impl Iterator<Item = OptionDefinitionRef> {
        self.options_class().definitions()
    }
}

impl PartialEq for FragmentOptionsClassInfo {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for FragmentOptionsClassInfo {}
impl std::hash::Hash for FragmentOptionsClassInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.java_name.hash(state)
    }
}
impl fmt::Debug for FragmentOptionsClassInfo {
    /// Java `Class.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.java_name)
    }
}

fn default_of<T: FragmentOptions>() -> Box<dyn OptionsBase> {
    Box::new(T::get_default())
}

fn normalized_of<T: FragmentOptions>(o: &dyn OptionsBase) -> Box<dyn OptionsBase> {
    let typed = o
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("ClassCastException: {:?}", o.options_class()));
    Box::new(typed.get_normalized())
}

macro_rules! fragment_options_classes {
    ($( $(#[$m:meta])* $konst:ident: $ty:ty = $java:literal; )*) => {
        $(
            $(#[$m])*
            pub static $konst: FragmentOptionsClassInfo = FragmentOptionsClassInfo {
                java_name: $java,
                class: <$ty as OptionsClass>::class,
                get_default: default_of::<$ty>,
                get_normalized: normalized_of::<$ty>,
            };
        )*

        /// Every fragment options class of Bazel 9.3.2's rule class provider, in no particular
        /// order (see [`crate::fragment_registry::FragmentRegistry`] for the sorted set).
        pub fn all_known_fragment_options_classes() -> Vec<FragmentOptionsClass> {
            vec![$( &$konst ),*]
        }
    };
}

fragment_options_classes! {
    // Java: com/google/devtools/build/lib/analysis/PlatformOptions.java:41 PlatformOptions
    PLATFORM_OPTIONS: PlatformOptions = "com.google.devtools.build.lib.analysis.PlatformOptions";
    // Java: com/google/devtools/build/lib/analysis/ShellConfiguration.java:71 ShellConfiguration.Options
    SHELL_CONFIGURATION_OPTIONS: ShellConfigurationOptions =
        "com.google.devtools.build.lib.analysis.ShellConfiguration$Options";
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptions.java:64 CoreOptions
    CORE_OPTIONS: CoreOptions = "com.google.devtools.build.lib.analysis.config.CoreOptions";
    // Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:38 CoverageConfiguration.CoverageOptions
    COVERAGE_OPTIONS: CoverageConfigurationCoverageOptions =
        "com.google.devtools.build.lib.analysis.test.CoverageConfiguration$CoverageOptions";
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:83 TestConfiguration.TestOptions
    TEST_OPTIONS: TestConfigurationTestOptions =
        "com.google.devtools.build.lib.analysis.test.TestConfiguration$TestOptions";
    // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:72 BazelRuleClassProvider.StrictActionEnvOptions
    STRICT_ACTION_ENV_OPTIONS: BazelRuleClassProviderStrictActionEnvOptions =
        "com.google.devtools.build.lib.bazel.rules.BazelRuleClassProvider$StrictActionEnvOptions";
    // Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:44 BazelPythonConfiguration.Options
    BAZEL_PYTHON_OPTIONS: BazelPythonConfigurationOptions =
        "com.google.devtools.build.lib.bazel.rules.python.BazelPythonConfiguration$Options";
    // Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:198 AndroidConfiguration.Options
    ANDROID_OPTIONS: AndroidConfigurationOptions =
        "com.google.devtools.build.lib.rules.android.AndroidConfiguration$Options";
    // Java: com/google/devtools/build/lib/rules/android/BazelAndroidConfiguration.java:43 BazelAndroidConfiguration.Options
    BAZEL_ANDROID_OPTIONS: BazelAndroidConfigurationOptions =
        "com.google.devtools.build.lib.rules.android.BazelAndroidConfiguration$Options";
    // Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:36 AppleCommandLineOptions
    APPLE_COMMAND_LINE_OPTIONS: AppleCommandLineOptions =
        "com.google.devtools.build.lib.rules.apple.AppleCommandLineOptions";
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagOptions.java:24 ConfigFeatureFlagOptions
    CONFIG_FEATURE_FLAG_OPTIONS: ConfigFeatureFlagOptions =
        "com.google.devtools.build.lib.rules.config.ConfigFeatureFlagOptions";
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:37 CppOptions
    CPP_OPTIONS: CppOptions = "com.google.devtools.build.lib.rules.cpp.CppOptions";
    // Java: com/google/devtools/build/lib/rules/java/JavaOptions.java:35 JavaOptions
    JAVA_OPTIONS: JavaOptions = "com.google.devtools.build.lib.rules.java.JavaOptions";
    // Java: com/google/devtools/build/lib/rules/objc/J2ObjcCommandLineOptions.java:30 J2ObjcCommandLineOptions
    J2OBJC_OPTIONS: J2ObjcCommandLineOptions =
        "com.google.devtools.build.lib.rules.objc.J2ObjcCommandLineOptions";
    // Java: com/google/devtools/build/lib/rules/objc/ObjcCommandLineOptions.java:28 ObjcCommandLineOptions
    OBJC_OPTIONS: ObjcCommandLineOptions =
        "com.google.devtools.build.lib.rules.objc.ObjcCommandLineOptions";
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:48 ProtoConfiguration.Options
    PROTO_OPTIONS: ProtoConfigurationOptions =
        "com.google.devtools.build.lib.rules.proto.ProtoConfiguration$Options";
    // Java: com/google/devtools/build/lib/rules/python/PythonOptions.java:33 PythonOptions
    PYTHON_OPTIONS: PythonOptions = "com.google.devtools.build.lib.rules.python.PythonOptions";
}

/// The registered class whose Rust options class is `class`, if any.
pub fn fragment_options_class_of(class: OptionsClassRef) -> Option<FragmentOptionsClass> {
    all_known_fragment_options_classes()
        .into_iter()
        .find(|c| c.options_class() == class)
}

/// The registered class of a statically typed options class. Panics if `T` is not registered.
pub fn fragment_options_class<T: OptionsClass>() -> FragmentOptionsClass {
    fragment_options_class_of(T::class())
        .unwrap_or_else(|| panic!("{:?} is not a registered FragmentOptions class", T::class()))
}
