//! Port of `analysis/config/CompilationMode.java`.

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CompilationMode.java:24 CompilationMode
    /// This class represents the debug/optimization mode the binaries will be built for.
    pub enum CompilationMode {
        /// Fast build mode (-g0).
        FASTBUILD = "fastbuild",
        /// Debug mode (-g).
        DBG = "dbg",
        /// Release mode (-g0 -O2 -DNDEBUG).
        OPT = "opt",
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CompilationMode.java:47 CompilationMode.Converter
blaze_options::enum_converter!(pub CompilationModeConverter, CompilationMode, "compilation mode");
