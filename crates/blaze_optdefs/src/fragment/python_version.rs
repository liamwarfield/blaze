//! The `PythonVersion` enum of `rules/python/PythonVersion.java` (only the constants; the
//! attribute-parsing helpers are not ported here).

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:31 PythonVersion
    /// An enum representing Python major versions.
    pub enum PythonVersion {
        /// Target value Python 2. Represents source code that is naturally compatible with Python 2.
        PY2,
        /// Target value Python 3. Represents source code that is naturally compatible with Python 3.
        PY3,
        /// Represents source code that is naturally compatible with both Python 2 and Python 3.
        PY2AND3,
        /// Alias for `PY2`. Deprecated in Bazel; prefer `PY2`.
        PY2ONLY,
        /// Deprecated alias for `PY3`.
        PY3ONLY,
        /// Internal sentinel value used as the default value of the `python_version` and
        /// `default_python_version` attributes.
        _INTERNAL_SENTINEL,
    }
}
