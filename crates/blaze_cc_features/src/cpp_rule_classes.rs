//! Port of `com.google.devtools.build.lib.rules.cpp.CppRuleClasses`: feature and exec-group name
//! constants (the Java class holds nothing else at the pin).

// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:22 CppRuleClasses.LANG_OBJC
pub const LANG_OBJC: &str = "lang_objc";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:25 CppRuleClasses.UNFILTERED_COMPILE_FLAGS_FEATURE_NAME
pub const UNFILTERED_COMPILE_FLAGS_FEATURE_NAME: &str = "unfiltered_compile_flags";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:28 CppRuleClasses.PARSE_HEADERS
pub const PARSE_HEADERS: &str = "parse_headers";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:34 CppRuleClasses.MODULE_MAPS
pub const MODULE_MAPS: &str = "module_maps";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:37 CppRuleClasses.CPP_MODULES
pub const CPP_MODULES: &str = "cpp_modules";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:43 CppRuleClasses.SERIALIZED_DIAGNOSTICS_FILE
pub const SERIALIZED_DIAGNOSTICS_FILE: &str = "serialized_diagnostics_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:46 CppRuleClasses.MODULE_MAP_HOME_CWD
pub const MODULE_MAP_HOME_CWD: &str = "module_map_home_cwd";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:58 CppRuleClasses.MODULE_MAP_WITHOUT_EXTERN_MODULE
pub const MODULE_MAP_WITHOUT_EXTERN_MODULE: &str = "module_map_without_extern_module";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:61 CppRuleClasses.LAYERING_CHECK
pub const LAYERING_CHECK: &str = "layering_check";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:69 CppRuleClasses.USE_HEADER_MODULES
pub const USE_HEADER_MODULES: &str = "use_header_modules";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:79 CppRuleClasses.GENERATE_SUBMODULES
pub const GENERATE_SUBMODULES: &str = "generate_submodules";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:88 CppRuleClasses.NO_LEGACY_FEATURES
pub const NO_LEGACY_FEATURES: &str = "no_legacy_features";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:91 CppRuleClasses.PER_OBJECT_DEBUG_INFO
pub const PER_OBJECT_DEBUG_INFO: &str = "per_object_debug_info";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:99 CppRuleClasses.PIC
pub const PIC: &str = "pic";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:102 CppRuleClasses.SUPPORTS_PIC
pub const SUPPORTS_PIC: &str = "supports_pic";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:109 CppRuleClasses.PREFER_PIC_FOR_OPT_BINARIES
pub const PREFER_PIC_FOR_OPT_BINARIES: &str = "prefer_pic_for_opt_binaries";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:112 CppRuleClasses.PREPROCESSOR_DEFINES
pub const PREPROCESSOR_DEFINES: &str = "preprocessor_defines";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:115 CppRuleClasses.INCLUDES
pub const INCLUDES: &str = "includes";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:118 CppRuleClasses.INCLUDE_PATHS
pub const INCLUDE_PATHS: &str = "include_paths";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:121 CppRuleClasses.EXTERNAL_INCLUDE_PATHS
pub const EXTERNAL_INCLUDE_PATHS: &str = "external_include_paths";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:124 CppRuleClasses.STATIC_LINKING_MODE
pub const STATIC_LINKING_MODE: &str = "static_linking_mode";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:127 CppRuleClasses.DYNAMIC_LINKING_MODE
pub const DYNAMIC_LINKING_MODE: &str = "dynamic_linking_mode";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:130 CppRuleClasses.THIN_LTO
pub const THIN_LTO: &str = "thin_lto";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:133 CppRuleClasses.NO_USE_LTO_INDEXING_BITCODE_FILE
pub const NO_USE_LTO_INDEXING_BITCODE_FILE: &str = "no_use_lto_indexing_bitcode_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:136 CppRuleClasses.USE_LTO_NATIVE_OBJECT_DIRECTORY
pub const USE_LTO_NATIVE_OBJECT_DIRECTORY: &str = "use_lto_native_object_directory";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:141 CppRuleClasses.AUTOFDO_IMPLICIT_THINLTO
pub const AUTOFDO_IMPLICIT_THINLTO: &str = "autofdo_implicit_thinlto";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:146 CppRuleClasses.ENABLE_AFDO_THINLTO
pub const ENABLE_AFDO_THINLTO: &str = "enable_afdo_thinlto";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:151 CppRuleClasses.ENABLE_FDO_THINLTO
pub const ENABLE_FDO_THINLTO: &str = "enable_fdo_thinlto";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:156 CppRuleClasses.ENABLE_XFDO_THINLTO
pub const ENABLE_XFDO_THINLTO: &str = "enable_xbinaryfdo_thinlto";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:159 CppRuleClasses.SPLIT_FUNCTIONS
pub const SPLIT_FUNCTIONS: &str = "split_functions";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:162 CppRuleClasses.ENABLE_FDO_SPLIT_FUNCTIONS
pub const ENABLE_FDO_SPLIT_FUNCTIONS: &str = "enable_fdo_split_functions";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:165 CppRuleClasses.FSAFDO
pub const FSAFDO: &str = "fsafdo";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:168 CppRuleClasses.ENABLE_FSAFDO
pub const ENABLE_FSAFDO: &str = "enable_fsafdo";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:171 CppRuleClasses.ENABLE_AUTOFDO_MEMPROF_OPTIMIZE
pub const ENABLE_AUTOFDO_MEMPROF_OPTIMIZE: &str = "enable_autofdo_memprof_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:174 CppRuleClasses.AUTOFDO_IMPLICIT_MEMPROF_OPTIMIZE
pub const AUTOFDO_IMPLICIT_MEMPROF_OPTIMIZE: &str = "autofdo_implicit_memprof_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:181 CppRuleClasses.THIN_LTO_LINKSTATIC_TESTS_USE_SHARED_NONLTO_BACKENDS
pub const THIN_LTO_LINKSTATIC_TESTS_USE_SHARED_NONLTO_BACKENDS: &str =
    "thin_lto_linkstatic_tests_use_shared_nonlto_backends";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:188 CppRuleClasses.THIN_LTO_ALL_LINKSTATIC_USE_SHARED_NONLTO_BACKENDS
pub const THIN_LTO_ALL_LINKSTATIC_USE_SHARED_NONLTO_BACKENDS: &str =
    "thin_lto_all_linkstatic_use_shared_nonlto_backends";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:192 CppRuleClasses.NATIVE_DEPS_LINK
pub const NATIVE_DEPS_LINK: &str = "native_deps_link";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:195 CppRuleClasses.JAVA_LAUNCHER_LINK
pub const JAVA_LAUNCHER_LINK: &str = "java_launcher_link";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:198 CppRuleClasses.PY_LAUNCHER_LINK
pub const PY_LAUNCHER_LINK: &str = "py_launcher_link";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:204 CppRuleClasses.GENERATE_PDB_FILE
pub const GENERATE_PDB_FILE: &str = "generate_pdb_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:207 CppRuleClasses.COPY_DYNAMIC_LIBRARIES_TO_BINARY
pub const COPY_DYNAMIC_LIBRARIES_TO_BINARY: &str = "copy_dynamic_libraries_to_binary";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:210 CppRuleClasses.STATIC_LINK_CPP_RUNTIMES
pub const STATIC_LINK_CPP_RUNTIMES: &str = "static_link_cpp_runtimes";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:215 CppRuleClasses.TARGETS_WINDOWS
pub const TARGETS_WINDOWS: &str = "targets_windows";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:220 CppRuleClasses.SUPPORTS_INTERFACE_SHARED_LIBRARIES
pub const SUPPORTS_INTERFACE_SHARED_LIBRARIES: &str = "supports_interface_shared_libraries";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:224 CppRuleClasses.PARSE_SHOWINCLUDES
pub const PARSE_SHOWINCLUDES: &str = "parse_showincludes";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:227 CppRuleClasses.NO_DOTD_FILE
pub const NO_DOTD_FILE: &str = "no_dotd_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:233 CppRuleClasses.SHORTEN_VIRTUAL_INCLUDES
pub const SHORTEN_VIRTUAL_INCLUDES: &str = "shorten_virtual_includes";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:238 CppRuleClasses.FDO_INSTRUMENT
pub const FDO_INSTRUMENT: &str = "fdo_instrument";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:241 CppRuleClasses.CS_FDO_INSTRUMENT
pub const CS_FDO_INSTRUMENT: &str = "cs_fdo_instrument";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:244 CppRuleClasses.FDO_OPTIMIZE
pub const FDO_OPTIMIZE: &str = "fdo_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:247 CppRuleClasses.CS_FDO_OPTIMIZE
pub const CS_FDO_OPTIMIZE: &str = "cs_fdo_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:250 CppRuleClasses.FDO_PREFETCH_HINTS
pub const FDO_PREFETCH_HINTS: &str = "fdo_prefetch_hints";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:253 CppRuleClasses.PROPELLER_OPTIMIZE
pub const PROPELLER_OPTIMIZE: &str = "propeller_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:256 CppRuleClasses.MEMPROF_OPTIMIZE
pub const MEMPROF_OPTIMIZE: &str = "memprof_optimize";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:259 CppRuleClasses.AUTOFDO
pub const AUTOFDO: &str = "autofdo";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:263 CppRuleClasses.XBINARYFDO
pub const XBINARYFDO: &str = "xbinaryfdo";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:266 CppRuleClasses.COVERAGE
pub const COVERAGE: &str = "coverage";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:269 CppRuleClasses.LLVM_COVERAGE_MAP_FORMAT
pub const LLVM_COVERAGE_MAP_FORMAT: &str = "llvm_coverage_map_format";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:272 CppRuleClasses.GCC_COVERAGE_MAP_FORMAT
pub const GCC_COVERAGE_MAP_FORMAT: &str = "gcc_coverage_map_format";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:275 CppRuleClasses.SUPPORTS_START_END_LIB
pub const SUPPORTS_START_END_LIB: &str = "supports_start_end_lib";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:281 CppRuleClasses.SUPPORTS_DYNAMIC_LINKER
pub const SUPPORTS_DYNAMIC_LINKER: &str = "supports_dynamic_linker";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:284 CppRuleClasses.COMPILER_PARAM_FILE
pub const COMPILER_PARAM_FILE: &str = "compiler_param_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:290 CppRuleClasses.ARCHIVE_PARAM_FILE
pub const ARCHIVE_PARAM_FILE: &str = "archive_param_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:293 CppRuleClasses.GCC_QUOTING_FOR_PARAM_FILES
pub const GCC_QUOTING_FOR_PARAM_FILES: &str = "gcc_quoting_for_param_files";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:299 CppRuleClasses.GENERATE_DSYM_FILE_FEATURE_NAME
pub const GENERATE_DSYM_FILE_FEATURE_NAME: &str = "generate_dsym_file";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:309 CppRuleClasses.NO_GENERATE_DEBUG_SYMBOLS_FEATURE_NAME
pub const NO_GENERATE_DEBUG_SYMBOLS_FEATURE_NAME: &str = "no_generate_debug_symbols";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:312 CppRuleClasses.GENERATE_LINKMAP_FEATURE_NAME
pub const GENERATE_LINKMAP_FEATURE_NAME: &str = "generate_linkmap";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:315 CppRuleClasses.DEAD_STRIP_FEATURE_NAME
pub const DEAD_STRIP_FEATURE_NAME: &str = "dead_strip";
// Java: com/google/devtools/build/lib/rules/cpp/CppRuleClasses.java:318 CppRuleClasses.CPP_LINK_EXEC_GROUP
pub const CPP_LINK_EXEC_GROUP: &str = "cpp_link";
