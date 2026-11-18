//! Port of `com.google.devtools.build.lib.rules.cpp.CppActionNames`.

/// Java: `CppActionNames`, a holder of C++ action name constants.
// Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:17 CppActionNames
pub struct CppActionNames;

impl CppActionNames {
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:20 CppActionNames.CC_FLAGS_MAKE_VARIABLE
    pub const CC_FLAGS_MAKE_VARIABLE: &str = "cc-flags-make-variable";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:22 CppActionNames.STRIP
    pub const STRIP: &str = "strip";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:24 CppActionNames.OBJ_COPY
    pub const OBJ_COPY: &str = "objcopy_embed_data";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:26 CppActionNames.LINKSTAMP_COMPILE
    pub const LINKSTAMP_COMPILE: &str = "linkstamp-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:28 CppActionNames.C_COMPILE
    pub const C_COMPILE: &str = "c-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:30 CppActionNames.CPP_COMPILE
    pub const CPP_COMPILE: &str = "c++-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:32 CppActionNames.CPP_MODULE_CODEGEN
    pub const CPP_MODULE_CODEGEN: &str = "c++-module-codegen";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:34 CppActionNames.OBJC_COMPILE
    pub const OBJC_COMPILE: &str = "objc-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:36 CppActionNames.OBJCPP_COMPILE
    pub const OBJCPP_COMPILE: &str = "objc++-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:38 CppActionNames.CPP_HEADER_PARSING
    pub const CPP_HEADER_PARSING: &str = "c++-header-parsing";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:41 CppActionNames.CPP_MODULE_DEPS_SCANNING
    pub const CPP_MODULE_DEPS_SCANNING: &str = "c++-module-deps-scanning";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:44 CppActionNames.CPP20_MODULE_COMPILE
    pub const CPP20_MODULE_COMPILE: &str = "c++20-module-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:46 CppActionNames.CPP20_MODULE_CODEGEN
    pub const CPP20_MODULE_CODEGEN: &str = "c++20-module-codegen";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:52 CppActionNames.CPP_MODULE_COMPILE
    pub const CPP_MODULE_COMPILE: &str = "c++-module-compile";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:54 CppActionNames.ASSEMBLE
    pub const ASSEMBLE: &str = "assemble";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:56 CppActionNames.PREPROCESS_ASSEMBLE
    pub const PREPROCESS_ASSEMBLE: &str = "preprocess-assemble";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:63 CppActionNames.CLIF_MATCH
    pub const CLIF_MATCH: &str = "clif-match";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:66 CppActionNames.CPP_LINK_STATIC_LIBRARY
    pub const CPP_LINK_STATIC_LIBRARY: &str = "c++-link-static-library";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:68 CppActionNames.CPP_LINK_NODEPS_DYNAMIC_LIBRARY
    pub const CPP_LINK_NODEPS_DYNAMIC_LIBRARY: &str = "c++-link-nodeps-dynamic-library";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:70 CppActionNames.CPP_LINK_DYNAMIC_LIBRARY
    pub const CPP_LINK_DYNAMIC_LIBRARY: &str = "c++-link-dynamic-library";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:72 CppActionNames.CPP_LINK_EXECUTABLE
    pub const CPP_LINK_EXECUTABLE: &str = "c++-link-executable";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:74 CppActionNames.OBJC_FULLY_LINK
    pub const OBJC_FULLY_LINK: &str = "objc-fully-link";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:76 CppActionNames.OBJC_EXECUTABLE
    pub const OBJC_EXECUTABLE: &str = "objc-executable";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:78 CppActionNames.LTO_INDEXING
    pub const LTO_INDEXING: &str = "lto-indexing";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:80 CppActionNames.LTO_INDEX_DYNAMIC_LIBRARY
    pub const LTO_INDEX_DYNAMIC_LIBRARY: &str = "lto-index-for-dynamic-library";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:82 CppActionNames.LTO_INDEX_NODEPS_DYNAMIC_LIBRARY
    pub const LTO_INDEX_NODEPS_DYNAMIC_LIBRARY: &str = "lto-index-for-nodeps-dynamic-library";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:85 CppActionNames.LTO_INDEX_EXECUTABLE
    pub const LTO_INDEX_EXECUTABLE: &str = "lto-index-for-executable";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:87 CppActionNames.LTO_BACKEND
    pub const LTO_BACKEND: &str = "lto-backend";
    // Java: com/google/devtools/build/lib/rules/cpp/CppActionNames.java:89 CppActionNames.CPP_HEADER_ANALYSIS
    pub const CPP_HEADER_ANALYSIS: &str = "c++-header-analysis";
}
