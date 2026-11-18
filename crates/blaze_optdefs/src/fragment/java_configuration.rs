//! The option value enums of `rules/java/JavaConfiguration.java` (the fragment itself is not
//! ported here).

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:49 JavaConfiguration.JavaClasspathMode
    /// Values for the --experimental_java_classpath option
    pub enum JavaConfigurationJavaClasspathMode {
        /// Use full transitive classpaths, the default behavior.
        OFF,
        /// JavaBuilder computes the reduced classpath before invoking javac.
        JAVABUILDER,
        /// Bazel computes the reduced classpath and tries it in a separate action invocation.
        BAZEL,
        /// Bazel uses the reduced classpath, but doesn't fallback to the full transitive classpath
        BAZEL_NO_FALLBACK,
    }
}

blaze_options::option_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:61 JavaConfiguration.OneVersionEnforcementLevel
    /// Values for the --experimental_one_version_enforcement option
    pub enum JavaConfigurationOneVersionEnforcementLevel {
        /// Don't attempt to check for one version violations (the default)
        OFF,
        /// Check for one version violations, emit warnings to stderr if any are found, but don't
        /// break the binary.
        WARNING,
        /// Check for one version violations, emit warnings to stderr if any are found, and break
        /// the rule if it's found.
        ERROR,
    }
}
