//! Port of `rules/cpp/CppOptions.java`.

use blaze_cmdline::Label;
use blaze_options::{
    ConversionContext, Converter, OptionsParsingException, Value, converter::non_null, options,
};

use super::compilation_mode::{CompilationMode, CompilationModeConverter};
use super::converters::{ConvResult, EmptyToNullLabelConverter, LabelConverter, OptionLabel};
use super::cpp_configuration::{CppConfigurationDynamicMode, CppConfigurationStripMode};
use super::fragment_options::FragmentOptions;
use super::per_label_options::{PerLabelOptions, PerLabelOptionsConverter};

// Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:39 CppOptions.FissionOptionConverter
/// Converts a comma-separated list of compilation mode settings to a properly typed List.
#[derive(Clone, Copy, Debug, Default)]
pub struct FissionOptionConverter;

impl FissionOptionConverter {
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:41 CppOptions.FissionOptionConverter.convert
    pub fn convert_modes(
        &self,
        input: &str,
    ) -> Result<Vec<CompilationMode>, OptionsParsingException> {
        // ImmutableSet.Builder: insertion order, duplicates dropped.
        let mut modes: Vec<CompilationMode> = Vec::new();
        let mut add = |m: CompilationMode| {
            if !modes.contains(&m) {
                modes.push(m);
            }
        };
        if input == "yes" {
            // Special case: enable all modes.
            for m in [
                CompilationMode::FASTBUILD,
                CompilationMode::DBG,
                CompilationMode::OPT,
            ] {
                add(m);
            }
        } else if input != "no" {
            // "no" is another special case that disables all modes.
            for mode in input.split(',') {
                add(CompilationModeConverter
                    .convert(Some(mode), None)?
                    .expect("enum converters return a value")
                    .get::<CompilationMode>());
            }
        }
        Ok(modes)
    }
}

impl Converter for FissionOptionConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list_of(self.convert_modes(non_null(input))?)))
    }
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:55 CppOptions.FissionOptionConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a set of compilation modes".into()
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:61 CppOptions.DynamicModeConverter
blaze_options::enum_converter!(pub DynamicModeConverter, CppConfigurationDynamicMode, "dynamic mode");

// Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:68 CppOptions.StripModeConverter
blaze_options::enum_converter!(pub StripModeConverter, CppConfigurationStripMode, "strip mode");

// Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:77 CppOptions.LibcTopLabelConverter
/// Converts a String, which is a package label into a label that can be used for a LibcTop
/// object.
#[derive(Clone, Copy, Debug, Default)]
pub struct LibcTopLabelConverter;

impl Converter for LibcTopLabelConverter {
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:82 CppOptions.LibcTopLabelConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input == "default" {
            // This is needed for defining config_setting() values, the syntactic form
            // of which must be a String, to match absence of a --grte_top option.
            // "--grte_top=default" works on the command-line too,
            // but that's an inconsequential side-effect, not the intended purpose.
            return Ok(None);
        } else if !input.starts_with("//") {
            return Err(OptionsParsingException::new("Not a label"));
        }
        let label = LabelConverter.convert_label(input, ctx)?;
        Ok(Some(Value::new(OptionLabel(Label::create_unvalidated(
            label.get_package_identifier().clone(),
            "everything",
        )))))
    }
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:97 CppOptions.LibcTopLabelConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a label".into()
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:37 CppOptions
options! {
    /// Command-line options for C++.
    pub struct CppOptions {
        #[option(
            name = "crosstool_top",
            default_value = "@bazel_tools//tools/cpp:toolchain",
            converter = LabelConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            help = "No-op flag. Will be removed in a future release.",
        )]
        pub crosstool_top: OptionLabel,

        #[option(
            name = "compiler",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            help = "The C++ compiler to use for compiling the target.",
        )]
        pub cpp_compiler: Option<String>,

        #[option(
            name = "host_compiler",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            help = "No-op flag. Will be removed in a future release.",
        )]
        pub host_cpp_compiler: Option<String>,

        #[option(
            name = "cc_output_directory_tag",
            default_value = "",
            documentation_category = TOOLCHAIN,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Specifies a suffix to be added to the configuration directory.",
        )]
        pub output_directory_tag: String,

        #[option(
            name = "minimum_os_version",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = "The minimum OS version which your compilation targets.",
        )]
        pub minimum_os_version: Option<String>,

        #[option(
            name = "start_end_lib",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, CHANGES_INPUTS, AFFECTS_OUTPUTS],
            metadata_tags = [HIDDEN],
            help = "Use the --start-lib/--end-lib ld options if supported by the toolchain.",
        )]
        pub use_start_end_lib: bool,

        #[option(
            name = "interface_shared_objects",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "Use interface shared objects if supported by the toolchain. ",
                "All ELF toolchains currently support this setting.",
            ),
        )]
        pub use_interface_shared_objects: bool,

        #[option(
            name = "fission",
            default_value = "no",
            converter = FissionOptionConverter,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOADING_AND_ANALYSIS, ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies which compilation modes use fission for C++ compilations and links.  May be",
                " any combination of {'fastbuild', 'dbg', 'opt'} or the special values 'yes'  to",
                " enable all modes and 'no' to disable all modes.",
            ),
        )]
        pub fission_modes: Vec<CompilationMode>,

        #[option(
            name = "build_test_dwp",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "If enabled, when building C++ tests statically and with fission the .dwp file ",
                " for the test binary will be automatically built as well.",
            ),
        )]
        pub build_test_dwp: bool,

        #[option(
            name = "dynamic_mode",
            default_value = "default",
            converter = DynamicModeConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "Determines whether C++ binaries will be linked dynamically.  'default' means ",
                "Bazel will choose whether to link dynamically.  'fully' means all libraries ",
                "will be linked dynamically. 'off' means that all libraries will be linked ",
                "in mostly static mode.",
            ),
        )]
        pub dynamic_mode: CppConfigurationDynamicMode,

        #[option(
            name = "force_pic",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "If enabled, all C++ compilations produce position-independent code (\"-fPIC\"),",
                " links prefer PIC pre-built libraries over non-PIC libraries, and links produce",
                " position-independent executables (\"-pie\").",
            ),
        )]
        pub force_pic: bool,

        #[option(
            name = "process_headers_in_dependencies",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [EXECUTION],
            help = concat!(
                "When building a target //a:a, process headers in all targets that //a:a depends ",
                "on (if header processing is enabled for the toolchain).",
            ),
        )]
        pub process_headers_in_dependencies: bool,

        #[option(
            name = "copt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional options to pass to gcc.",
        )]
        pub copt_list: Vec<String>,

        #[option(
            name = "cxxopt",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional option to pass to gcc when compiling C++ source files.",
        )]
        pub cxxopt_list: Vec<String>,

        #[option(
            name = "conlyopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional option to pass to gcc when compiling C source files.",
        )]
        pub conlyopt_list: Vec<String>,

        #[option(
            name = "objccopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "Additional options to pass to gcc when compiling Objective-C/C++ source files.",
        )]
        pub objcopt_list: Vec<String>,

        #[option(
            name = "linkopt",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional option to pass to gcc when linking.",
        )]
        pub linkopt_list: Vec<String>,

        #[option(
            name = "ltoindexopt",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional option to pass to the LTO indexing step (under --features=thin_lto).",
        )]
        pub ltoindexopt_list: Vec<String>,

        #[option(
            name = "ltobackendopt",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional option to pass to the LTO backend step (under --features=thin_lto).",
        )]
        pub ltobackendopt_list: Vec<String>,

        #[option(
            name = "stripopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = "Additional options to pass to strip when generating a '<name>.stripped' binary.",
        )]
        pub stripopt_list: Vec<String>,

        #[option(
            name = "custom_malloc",
            default_value = "null",
            documentation_category = TOOLCHAIN,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies a custom malloc implementation. This setting overrides malloc ",
                "attributes in build rules.",
            ),
            converter = LabelConverter,
        )]
        pub custom_malloc: Option<OptionLabel>,

        #[option(
            name = "legacy_whole_archive",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            metadata_tags = [DEPRECATED],
            help = concat!(
                "Deprecated, superseded by --incompatible_remove_legacy_whole_archive ",
                "(see https://github.com/bazelbuild/bazel/issues/7362 for details). ",
                "When on, use --whole-archive for cc_binary rules that have ",
                "linkshared=True and either linkstatic=True or '-static' in linkopts. ",
                "This is for backwards compatibility only. ",
                "A better alternative is to use alwayslink=1 where required.",
            ),
        )]
        pub legacy_whole_archive: bool,

        #[option(
            name = "strip",
            default_value = "sometimes",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Specifies whether to strip binaries and shared libraries ",
                " (using \"-Wl,--strip-debug\").  The default value of 'sometimes'",
                " means strip iff --compilation_mode=fastbuild.",
            ),
            converter = StripModeConverter,
        )]
        pub strip_binaries: CppConfigurationStripMode,

        #[option(
            name = "fdo_instrument",
            default_value = "null",
            implicit_requirements = ["--copt=-Wno-error"],
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Generate binaries with FDO instrumentation. With Clang/LLVM compiler, it also accepts",
                " the directory name under which the raw profile file(s) will be dumped at",
                " runtime.",
            ),
        )]
        pub fdo_instrument_for_build: Option<String>,

        #[option(
            name = "fdo_optimize",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Use FDO profile information to optimize compilation. Specify the name ",
                "of a zip file containing a .gcda file tree, an afdo file containing ",
                "an auto profile, or an LLVM profile file. This flag also accepts files ",
                "specified as labels (e.g. `//foo/bar:file.afdo` - you may need to add ",
                "an `exports_files` directive to the corresponding package) and labels ",
                "pointing to `fdo_profile` targets. This flag will be superseded by the ",
                "`fdo_profile` rule.",
            ),
        )]
        pub fdo_optimize_for_build: Option<String>,

        #[option(
            name = "cs_fdo_instrument",
            default_value = "null",
            implicit_requirements = ["--copt=-Wno-error"],
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Generate binaries with context sensitive FDO instrumentation. With Clang/LLVM compiler, ",
                "it also accepts the directory name under which the raw profile file(s) will be ",
                "dumped at runtime.",
            ),
        )]
        pub cs_fdo_instrument_for_build: Option<String>,

        #[option(
            name = "cs_fdo_absolute_path",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Use CSFDO profile information to optimize compilation. Specify the absolute path name ",
                "of the zip file containing the profile file, a raw or an indexed ",
                "LLVM profile file.",
            ),
        )]
        pub cs_fdo_absolute_path_for_build: Option<String>,

        #[option(
            name = "xbinary_fdo",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            converter = EmptyToNullLabelConverter,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Use XbinaryFDO profile information to optimize compilation. Specify the name ",
                "of default cross binary profile. When the option is used together with ",
                "--fdo_instrument/--fdo_optimize/--fdo_profile, those options will always ",
                "prevail as if xbinary_fdo is never specified. ",
            ),
        )]
        pub xfdo_profile_label: Option<OptionLabel>,

        #[option(
            name = "fdo_prefetch_hints",
            default_value = "null",
            converter = LabelConverter,
            category = "flags",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Use cache prefetch hints.",
        )]
        pub fdo_prefetch_hints_label: Option<OptionLabel>,

        #[option(
            name = "fdo_profile",
            default_value = "null",
            category = "flags",
            converter = EmptyToNullLabelConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "The fdo_profile representing the profile to be used for optimization.",
        )]
        pub fdo_profile_label: Option<OptionLabel>,

        #[option(
            name = "cs_fdo_profile",
            default_value = "null",
            category = "flags",
            converter = LabelConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "The cs_fdo_profile representing the context sensitive profile to be used for",
                " optimization.",
            ),
        )]
        pub cs_fdo_profile_label: Option<OptionLabel>,

        #[option(
            name = "enable_remaining_fdo_absolute_paths",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "If set, any use of absolute paths for FDO will raise an error.",
        )]
        pub enable_fdo_profile_absolute_path: bool,

        #[option(
            name = "enable_propeller_optimize_absolute_paths",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "If set, any use of absolute paths for propeller optimize will raise an error.",
        )]
        pub enable_propeller_optimize_absolute_path: bool,

        #[option(
            name = "propeller_optimize_absolute_cc_profile",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Absolute path name of cc_profile file for Propeller Optimized builds.",
            deprecation_warning = "Deprecated. Use --propeller_optimize instead.",
        )]
        pub propeller_optimize_absolute_cc_profile: Option<String>,

        #[option(
            name = "propeller_optimize_absolute_ld_profile",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Absolute path name of ld_profile file for Propeller Optimized builds.",
            deprecation_warning = "Deprecated. Use --propeller_optimize instead.",
        )]
        pub propeller_optimize_absolute_ld_profile: Option<String>,

        #[option(
            name = "propeller_optimize",
            default_value = "null",
            converter = LabelConverter,
            category = "flags",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Use Propeller profile information to optimize the build target.",
                "A propeller profile must consist of at least one of two files, a cc profile ",
                "and a ld profile.  This flag accepts a build label which must refer to ",
                "the propeller profile input files. For example, the BUILD file that ",
                "defines the label, in a/b/BUILD:",
                "propeller_optimize(",
                "    name = \"propeller_profile\",",
                "    cc_profile = \"propeller_cc_profile.txt\",",
                "    ld_profile = \"propeller_ld_profile.txt\",",
                ")",
                "An exports_files directive may have to be added to the corresponding package ",
                "to make these files visible to Bazel. The option must be used as: ",
                "--propeller_optimize=//a/b:propeller_profile",
            ),
        )]
        pub propeller_optimize_label: Option<OptionLabel>,

        #[option(
            name = "memprof_profile",
            default_value = "null",
            converter = LabelConverter,
            category = "flags",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Use memprof profile.",
        )]
        pub memprof_profile_label: Option<OptionLabel>,

        #[option(
            name = "proto_profile",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            default_value = "true",
            help = "Whether to pass profile_path to the proto compiler.",
        )]
        pub proto_profile: bool,

        #[option(
            name = "proto_profile_path",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            default_value = "null",
            converter = LabelConverter,
            help = concat!(
                "The profile to pass to the proto compiler as profile_path. If unset, but ",
                " --proto_profile is true (the default), infers the path from --fdo_optimize.",
            ),
        )]
        pub proto_profile_path: Option<OptionLabel>,

        #[option(
            name = "save_temps",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If set, temporary outputs from gcc will be saved.  ",
                "These include .s files (assembler code), .i files (preprocessed C) and ",
                ".ii files (preprocessed C++).",
            ),
        )]
        pub save_temps: bool,

        #[option(
            name = "per_file_copt",
            allow_multiple = true,
            converter = PerLabelOptionsConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional options to selectively pass to gcc when compiling certain files. ",
                "This option can be passed multiple times. ",
                "Syntax: regex_filter@option_1,option_2,...,option_n. Where regex_filter stands ",
                "for a list of include and exclude regular expression patterns (Also see ",
                "--instrumentation_filter). option_1 to option_n stand for ",
                "arbitrary command line options. If an option contains a comma it has to be ",
                "quoted with a backslash. Options can contain @. Only the first @ is used to ",
                "split the string. Example: ",
                "--per_file_copt=//foo/.*\\.cc,-//foo/bar\\.cc@-O0 adds the -O0 ",
                "command line option to the gcc command line of all cc files in //foo/ ",
                "except bar.cc.",
            ),
        )]
        pub per_file_copts: Vec<PerLabelOptions>,

        #[option(
            name = "per_file_ltobackendopt",
            allow_multiple = true,
            converter = PerLabelOptionsConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional options to selectively pass to LTO backend (under --features=thin_lto) when ",
                "compiling certain backend objects. This option can be passed multiple times. ",
                "Syntax: regex_filter@option_1,option_2,...,option_n. Where regex_filter stands ",
                "for a list of include and exclude regular expression patterns. ",
                "option_1 to option_n stand for arbitrary command line options. ",
                "If an option contains a comma it has to be quoted with a backslash. ",
                "Options can contain @. Only the first @ is used to split the string. Example: ",
                "--per_file_ltobackendopt=//foo/.*\\.o,-//foo/bar\\.o@-O0 adds the -O0 ",
                "command line option to the LTO backend command line of all o files in //foo/ ",
                "except bar.o.",
            ),
        )]
        pub per_file_lto_backend_opts: Vec<PerLabelOptions>,

        #[option(
            name = "host_copt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional options to pass to the C compiler for tools built in the exec",
                " configurations.",
            ),
        )]
        pub host_copt_list: Vec<String>,

        #[option(
            name = "host_cxxopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional options to pass to C++ compiler for tools built in the exec",
                " configurations.",
            ),
        )]
        pub host_cxxopt_list: Vec<String>,

        #[option(
            name = "host_conlyopt",
            allow_multiple = true,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional option to pass to the C compiler when compiling C (but not C++) source files",
                " in the exec configurations.",
            ),
        )]
        pub host_conlyopt_list: Vec<String>,

        #[option(
            name = "host_per_file_copt",
            allow_multiple = true,
            converter = PerLabelOptionsConverter,
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional options to selectively pass to the C/C++ compiler when ",
                "compiling certain files in the exec configurations. ",
                "This option can be passed multiple times. ",
                "Syntax: regex_filter@option_1,option_2,...,option_n. Where regex_filter stands ",
                "for a list of include and exclude regular expression patterns (Also see ",
                "--instrumentation_filter). option_1 to option_n stand for ",
                "arbitrary command line options. If an option contains a comma it has to be ",
                "quoted with a backslash. Options can contain @. Only the first @ is used to ",
                "split the string. Example: ",
                "--host_per_file_copt=//foo/.*\\.cc,-//foo/bar\\.cc@-O0 adds the -O0 ",
                "command line option to the gcc command line of all cc files in //foo/ ",
                "except bar.cc.",
            ),
        )]
        pub host_per_file_copts_list: Vec<PerLabelOptions>,

        #[option(
            name = "host_linkopt",
            default_value = "null",
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "Additional option to pass to linker when linking tools in the exec",
                " configurations.",
            ),
        )]
        pub host_linkopt_list: Vec<String>,

        #[option(
            name = "grte_top",
            default_value = "null",
            converter = LibcTopLabelConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "A label to a checked-in libc library. The default value is selected by the crosstool ",
                "toolchain, and you almost never need to override it.",
            ),
        )]
        pub libc_top_label: Option<OptionLabel>,

        #[option(
            name = "host_grte_top",
            default_value = "null",
            converter = LibcTopLabelConverter,
            documentation_category = TOOLCHAIN,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            help = concat!(
                "If specified, this setting overrides the libc top-level directory (--grte_top) ",
                "for the exec configuration.",
            ),
        )]
        pub host_libc_top_label: Option<OptionLabel>,

        #[option(
            name = "experimental_inmemory_dotd_files",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled, C++ .d files will be passed through in memory directly from the remote ",
                "build nodes instead of being written to disk.",
            ),
        )]
        pub inmemory_dotd_files: bool,

        #[option(
            name = "experimental_omitfp",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES, AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, use libunwind for stack unwinding, and compile with ",
                "-fomit-frame-pointer and -fasynchronous-unwind-tables.",
            ),
        )]
        pub experimental_omitfp: bool,

        #[option(
            name = "share_native_deps",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = concat!(
                "If true, native libraries that contain identical functionality ",
                "will be shared among different targets",
            ),
        )]
        pub share_native_deps: bool,

        #[option(
            name = "strict_system_includes",
            default_value = "false",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [LOADING_AND_ANALYSIS, EAGERNESS_TO_EXIT],
            help = concat!(
                "If true, headers found through system include paths (-isystem) are also required to be ",
                "declared.",
            ),
        )]
        pub strict_system_includes: bool,

        #[option(
            name = "experimental_use_llvm_covmap",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If specified, Bazel will generate llvm-cov coverage map information rather than ",
                "gcov when collect_code_coverage is enabled.",
            ),
        )]
        pub use_llvm_coverage_map_format: bool,

        #[option(
            name = "incompatible_dont_enable_host_nonhost_crosstool_features",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, Bazel will not enable 'host' and 'nonhost' features in the c++ toolchain ",
                "(see https://github.com/bazelbuild/bazel/issues/7407 for more information).",
            ),
        )]
        pub dont_enable_host_nonhost: bool,

        #[option(
            name = "incompatible_make_thinlto_command_lines_standalone",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This flag is a noop and scheduled for removal.",
        )]
        pub use_standalone_lto_indexing_command_lines: bool,

        #[option(
            name = "incompatible_require_ctx_in_configure_features",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This flag is a noop and scheduled for removal.",
        )]
        pub require_ctx_in_configure_features: bool,

        #[option(
            name = "incompatible_validate_top_level_header_inclusions",
            default_value = "true",
            documentation_category = INPUT_STRICTNESS,
            effect_tags = [NO_OP],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "This flag is a noop and scheduled for removal.",
        )]
        pub validate_top_level_header_inclusions: bool,

        #[option(
            name = "incompatible_remove_legacy_whole_archive",
            default_value = "true",
            documentation_category = TOOLCHAIN,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, Bazel will not link library dependencies as whole archive by default ",
                "(see https://github.com/bazelbuild/bazel/issues/7362 for migration instructions).",
            ),
        )]
        pub remove_legacy_whole_archive: bool,

        #[option(
            name = "incompatible_disable_legacy_cc_provider",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op flag. Will be removed in a future release.",
        )]
        pub disable_legacy_cc_provider: bool,

        #[option(
            name = "incompatible_enable_cc_toolchain_resolution",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "No-op flag. Will be removed in a future release.",
        )]
        pub enable_cc_toolchain_resolution_no_op: bool,

        #[option(
            name = "experimental_save_feature_state",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = "Save the state of enabled and requested feautres as an output of compilation.",
        )]
        pub save_feature_state: bool,

        #[option(
            name = "incompatible_use_specific_tool_files",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "Use cc toolchain's compiler_files, as_files, and ar_files as inputs to appropriate ",
                "actions. See https://github.com/bazelbuild/bazel/issues/8531",
            ),
        )]
        pub use_specific_tool_files: bool,

        #[option(
            name = "incompatible_disable_nocopts",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [ACTION_COMMAND_LINES],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "When enabled, it removes nocopts attribute from C++ rules. See",
                " https://github.com/bazelbuild/bazel/issues/8706 for details.",
            ),
        )]
        pub disable_no_copts: bool,

        #[option(
            name = "apple_generate_dsym",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS, ACTION_COMMAND_LINES],
            help = "Whether to generate debug symbol(.dSYM) file(s).",
        )]
        pub apple_generate_dsym: bool,

        #[option(
            name = "objc_generate_linkmap",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Specifies whether to generate a linkmap file.",
        )]
        pub objc_generate_linkmap: bool,

        #[option(
            name = "objc_enable_binary_stripping",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = concat!(
                "Whether to perform symbol and dead-code strippings on linked binaries. Binary ",
                "strippings will be performed if both this flag and --compilation_mode=opt are ",
                "specified.",
            ),
        )]
        pub objc_enable_binary_stripping: bool,

        #[option(
            name = "experimental_generate_llvm_lcov",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "If true, coverage for clang will generate an LCOV report.",
        )]
        pub generate_llvm_lcov: bool,

        #[option(
            name = "incompatible_use_cpp_compile_header_mnemonic",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If enabled, give distinguishing mnemonic to header processing actions",
        )]
        pub use_cpp_compile_header_mnemonic: bool,

        #[option(
            name = "experimental_use_cpp_compile_action_args_params_file",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = "If enabled, write CppCompileAction exposed action.args to parameters file.",
        )]
        pub use_args_params_file: bool,

        #[option(
            name = "experimental_unsupported_and_brittle_include_scanning",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Whether to narrow inputs to C/C++ compilation by parsing #include lines from input",
                " files. This can improve performance and incrementality by decreasing the size of",
                " compilation input trees. However, it can also break builds because the include",
                " scanner does not fully implement C preprocessor semantics. In particular, it does",
                " not understand dynamic #include directives and ignores preprocessor conditional",
                " logic. Use at your own risk. Any issues relating to this flag that are filed will",
                " be closed.",
                " At Google without this flag your build will most likely fail.",
            ),
        )]
        pub include_scanning: bool,

        #[option(
            name = "cc_include_scanning",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Whether to narrow inputs to C/C++ compilation by parsing #include lines from input",
                " files. This can improve performance and incrementality by decreasing the size of",
                " compilation input trees. However, it can also break builds because the include",
                " scanner does not fully implement C preprocessor semantics. In particular, it does",
                " not understand dynamic #include directives and ignores preprocessor conditional",
                " logic. Use at your own risk. Any issues relating to this flag that are filed will",
                " be closed.",
                " At Google without this flag your build will most likely fail.",
            ),
        )]
        pub include_scanning_internal: bool,

        #[option(
            name = "cc_dotd_files",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, CHANGES_INPUTS],
            default_value = "true",
            help = "Whether to generate and analyze .d files.",
        )]
        pub generate_dotd_files: bool,

        #[option(
            name = "objc_use_dotd_pruning",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [CHANGES_INPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "If set, .d files emitted by clang will be used to prune the set of inputs passed into ",
                "objc compiles.",
            ),
        )]
        pub objc_generate_dotd_files: bool,

        #[option(
            name = "experimental_cc_implementation_deps",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "If enabled, cc_library targets can use attribute `implementation_deps`.",
        )]
        pub experimental_cc_implementation_deps: bool,

        #[option(
            name = "experimental_cpp_modules",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION, CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Enables experimental C++20 modules support. Use it with `module_interfaces` attribute on",
                " `cc_binary` and `cc_library`. While the support is behind the experimental flag,",
                " there are no guarantees about incompatible changes to it or even keeping the",
                " support in the future. Consider those risks when using it.",
            ),
        )]
        pub experimental_cpp_modules: bool,

        #[option(
            name = "experimental_link_static_libraries_once",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE, EXPERIMENTAL],
            help = concat!(
                "If enabled, cc_shared_library will link all libraries statically linked into it, that",
                " should only be linked once.",
            ),
        )]
        pub experimental_link_static_libraries_once: bool,

        #[option(
            name = "experimental_cpp_compile_resource_estimation",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If enabled, will estimate precise resource usage for local execution of",
                " CppCompileAction.",
            ),
        )]
        pub experimental_cpp_compile_resources_estimation: bool,
    }
}

impl CppOptions {
    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:380 CppOptions.getFdoOptimize
    /// Returns the --fdo_optimize value if FDO is specified and active for this configuration,
    /// the default value otherwise.
    pub fn get_fdo_optimize(&self) -> Option<&str> {
        self.fdo_optimize_for_build.as_deref()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:431 CppOptions.getFdoPrefetchHintsLabel
    /// Returns the --fdo_prefetch_hints value.
    pub fn get_fdo_prefetch_hints_label(&self) -> Option<&Label> {
        self.fdo_prefetch_hints_label
            .as_ref()
            .map(OptionLabel::label)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:514 CppOptions.getPropellerOptimizeLabel
    pub fn get_propeller_optimize_label(&self) -> Option<&Label> {
        self.propeller_optimize_label
            .as_ref()
            .map(OptionLabel::label)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppOptions.java:529 CppOptions.getMemProfProfileLabel
    /// Returns the --memprof_profile value.
    pub fn get_mem_prof_profile_label(&self) -> Option<&Label> {
        self.memprof_profile_label.as_ref().map(OptionLabel::label)
    }
}

impl FragmentOptions for CppOptions {}

#[cfg(test)]
mod tests {
    use super::super::test_util::{assert_defaults_round_trip, assert_option_names, parse};
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<CppOptions>(&[
            "crosstool_top",
            "compiler",
            "host_compiler",
            "cc_output_directory_tag",
            "minimum_os_version",
            "start_end_lib",
            "interface_shared_objects",
            "fission",
            "build_test_dwp",
            "dynamic_mode",
            "force_pic",
            "process_headers_in_dependencies",
            "copt",
            "cxxopt",
            "conlyopt",
            "objccopt",
            "linkopt",
            "ltoindexopt",
            "ltobackendopt",
            "stripopt",
            "custom_malloc",
            "legacy_whole_archive",
            "strip",
            "fdo_instrument",
            "fdo_optimize",
            "cs_fdo_instrument",
            "cs_fdo_absolute_path",
            "xbinary_fdo",
            "fdo_prefetch_hints",
            "fdo_profile",
            "cs_fdo_profile",
            "enable_remaining_fdo_absolute_paths",
            "enable_propeller_optimize_absolute_paths",
            "propeller_optimize_absolute_cc_profile",
            "propeller_optimize_absolute_ld_profile",
            "propeller_optimize",
            "memprof_profile",
            "proto_profile",
            "proto_profile_path",
            "save_temps",
            "per_file_copt",
            "per_file_ltobackendopt",
            "host_copt",
            "host_cxxopt",
            "host_conlyopt",
            "host_per_file_copt",
            "host_linkopt",
            "grte_top",
            "host_grte_top",
            "experimental_inmemory_dotd_files",
            "experimental_omitfp",
            "share_native_deps",
            "strict_system_includes",
            "experimental_use_llvm_covmap",
            "incompatible_dont_enable_host_nonhost_crosstool_features",
            "incompatible_make_thinlto_command_lines_standalone",
            "incompatible_require_ctx_in_configure_features",
            "incompatible_validate_top_level_header_inclusions",
            "incompatible_remove_legacy_whole_archive",
            "incompatible_disable_legacy_cc_provider",
            "incompatible_enable_cc_toolchain_resolution",
            "experimental_save_feature_state",
            "incompatible_use_specific_tool_files",
            "incompatible_disable_nocopts",
            "apple_generate_dsym",
            "objc_generate_linkmap",
            "objc_enable_binary_stripping",
            "experimental_generate_llvm_lcov",
            "incompatible_use_cpp_compile_header_mnemonic",
            "experimental_use_cpp_compile_action_args_params_file",
            "experimental_unsupported_and_brittle_include_scanning",
            "cc_include_scanning",
            "cc_dotd_files",
            "objc_use_dotd_pruning",
            "experimental_cc_implementation_deps",
            "experimental_cpp_modules",
            "experimental_link_static_libraries_once",
            "experimental_cpp_compile_resource_estimation",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<CppOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<CppOptions>(&[
            "--copt=-O2",
            "--copt=-g",
            "--fission=dbg,opt,dbg",
            "--grte_top=//libc:foo",
            "--dynamic_mode=fully",
            "--strip=always",
            "--per_file_copt=//foo/.*\\.cc,-//foo/bar\\.cc@-O0",
        ])
        .unwrap();
        assert_eq!(o.copt_list, vec!["-O2", "-g"]);
        assert_eq!(
            o.fission_modes,
            vec![CompilationMode::DBG, CompilationMode::OPT]
        );
        assert_eq!(
            o.libc_top_label.as_ref().unwrap().to_string(),
            "//libc:everything"
        );
        assert_eq!(o.dynamic_mode, CppConfigurationDynamicMode::FULLY);
        assert_eq!(o.strip_binaries, CppConfigurationStripMode::ALWAYS);
        assert_eq!(o.per_file_copts[0].get_options(), ["-O0".to_string()]);
        let o = parse::<CppOptions>(&["--fission=yes", "--grte_top=default"]).unwrap();
        assert_eq!(o.fission_modes.len(), 3);
        assert!(o.libc_top_label.is_none());
        assert_eq!(
            parse::<CppOptions>(&["--grte_top=foo"])
                .unwrap_err()
                .get_message(),
            "While parsing option --grte_top=foo: Not a label"
        );
    }
}
