//! The option value enums of `rules/cpp/CppConfiguration.java` (the fragment itself is not ported
//! here).

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:112 CppConfiguration.DynamicMode
    /// This enumeration is used for the --dynamic_mode option.
    pub enum CppConfigurationDynamicMode { OFF, DEFAULT, FULLY }
}

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:119 CppConfiguration.StripMode
    /// This enumeration is used for the --strip option.
    pub enum CppConfigurationStripMode {
        /// Always strip.
        ALWAYS = "always",
        /// Strip iff compilationMode == FASTBUILD.
        SOMETIMES = "sometimes",
        /// Never strip.
        NEVER = "never",
    }
}
