//! The `PythonVersion` enum of `rules/python/PythonVersion.java`, its value lists and its
//! attribute-parsing helpers.

blaze_options::option_enum! {
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

// The attribute-parsing helpers. No caller in the bazel binary at the pin (the Python rules moved
// to Starlark); ported with their tests (PythonVersionTest).

/// `valueOf(str)` for a name in `values`, else `None`.
fn value_of_in(values: &[PythonVersion], s: &str) -> Option<PythonVersion> {
    use blaze_options::converters::JavaEnum as _;
    values.iter().copied().find(|v| v.name() == s)
}

impl PythonVersion {
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:73 PythonVersion.TARGET_VALUES
    pub const TARGET_VALUES: [PythonVersion; 2] = [PythonVersion::PY2, PythonVersion::PY3];

    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:79 PythonVersion.TARGET_AND_SENTINEL_VALUES
    pub const TARGET_AND_SENTINEL_VALUES: [PythonVersion; 3] = [
        PythonVersion::PY2,
        PythonVersion::PY3,
        PythonVersion::_INTERNAL_SENTINEL,
    ];

    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:86 PythonVersion.SRCS_VALUES
    pub const SRCS_VALUES: [PythonVersion; 5] = [
        PythonVersion::PY2,
        PythonVersion::PY3,
        PythonVersion::PY2AND3,
        PythonVersion::PY2ONLY,
        PythonVersion::PY3ONLY,
    ];

    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:93 PythonVersion.NON_CONVERSION_VALUES
    pub const NON_CONVERSION_VALUES: [PythonVersion; 3] = [
        PythonVersion::PY2AND3,
        PythonVersion::PY2ONLY,
        PythonVersion::PY3ONLY,
    ];

    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:101 PythonVersion.DEFAULT_SRCS_VALUE
    pub const DEFAULT_SRCS_VALUE: PythonVersion = PythonVersion::PY2AND3;

    /// The `toString()` of each value.
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:66 PythonVersion.convertToStrings
    pub fn convert_to_strings(values: &[PythonVersion]) -> Vec<String> {
        use blaze_options::OptionScalar as _;
        values.iter().map(|v| v.java_to_string()).collect()
    }

    /// Returns whether this is one of the target values (`PY2` or `PY3`).
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:126 PythonVersion.isTargetValue
    pub fn is_target_value(&self) -> bool {
        Self::TARGET_VALUES.contains(self)
    }

    /// Converts the string to a target `PythonVersion` value (case-sensitive). `Err` is the
    /// message of Java's `IllegalArgumentException`.
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:135 PythonVersion.parseTargetValue
    pub fn parse_target_value(s: &str) -> Result<PythonVersion, String> {
        value_of_in(&Self::TARGET_VALUES, s).ok_or_else(|| {
            format!("'{s}' is not a valid Python major version. Expected 'PY2' or 'PY3'.")
        })
    }

    /// Converts the string to a target or sentinel `PythonVersion` value (case-sensitive).
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:148 PythonVersion.parseTargetOrSentinelValue
    pub fn parse_target_or_sentinel_value(s: &str) -> Result<PythonVersion, String> {
        // Use the same error message as for parseTargetValue, because the user shouldn't be
        // aware of the sentinel value.
        value_of_in(&Self::TARGET_AND_SENTINEL_VALUES, s).ok_or_else(|| {
            format!("'{s}' is not a valid Python major version. Expected 'PY2' or 'PY3'.")
        })
    }

    /// Converts the string to a sources `PythonVersion` value (case-sensitive).
    // Java: com/google/devtools/build/lib/rules/python/PythonVersion.java:163 PythonVersion.parseSrcsValue
    pub fn parse_srcs_value(s: &str) -> Result<PythonVersion, String> {
        value_of_in(&Self::SRCS_VALUES, s)
            .ok_or_else(|| format!("'{s}' is not a valid Python srcs_version value."))
    }
}

#[cfg(test)]
mod python_version_test {
    // Java: src/test/java/com/google/devtools/build/lib/rules/python/PythonVersionTest.java
    use super::PythonVersion;

    fn assert_is_invalid_for_parse_target_value(value: &str) {
        assert!(
            PythonVersion::parse_target_value(value)
                .unwrap_err()
                .contains("not a valid Python major version")
        );
    }

    fn assert_is_invalid_for_parse_target_or_sentinel_value(value: &str) {
        assert!(
            PythonVersion::parse_target_or_sentinel_value(value)
                .unwrap_err()
                .contains("not a valid Python major version")
        );
    }

    fn assert_is_invalid_for_parse_srcs_value(value: &str) {
        assert!(
            PythonVersion::parse_srcs_value(value)
                .unwrap_err()
                .contains("not a valid Python srcs_version value")
        );
    }

    #[test]
    fn is_target_value() {
        assert!(PythonVersion::PY2.is_target_value());
        assert!(PythonVersion::PY3.is_target_value());
        assert!(!PythonVersion::PY2AND3.is_target_value());
        assert!(!PythonVersion::PY2ONLY.is_target_value());
        assert!(!PythonVersion::PY3ONLY.is_target_value());
        assert!(!PythonVersion::_INTERNAL_SENTINEL.is_target_value());
    }

    #[test]
    fn parse_target_value() {
        assert_eq!(
            PythonVersion::parse_target_value("PY2"),
            Ok(PythonVersion::PY2)
        );
        assert_eq!(
            PythonVersion::parse_target_value("PY3"),
            Ok(PythonVersion::PY3)
        );
        assert_is_invalid_for_parse_target_value("PY2AND3");
        assert_is_invalid_for_parse_target_value("PY2ONLY");
        assert_is_invalid_for_parse_target_value("PY3ONLY");
        assert_is_invalid_for_parse_target_value("_INTERNAL_SENTINEL");
        assert_is_invalid_for_parse_target_value("not an enum value");
    }

    #[test]
    fn parse_target_or_sentinel_value() {
        assert_eq!(
            PythonVersion::parse_target_or_sentinel_value("PY2"),
            Ok(PythonVersion::PY2)
        );
        assert_eq!(
            PythonVersion::parse_target_or_sentinel_value("PY3"),
            Ok(PythonVersion::PY3)
        );
        assert_is_invalid_for_parse_target_or_sentinel_value("PY2AND3");
        assert_is_invalid_for_parse_target_or_sentinel_value("PY2ONLY");
        assert_is_invalid_for_parse_target_or_sentinel_value("PY3ONLY");
        assert_eq!(
            PythonVersion::parse_target_or_sentinel_value("_INTERNAL_SENTINEL"),
            Ok(PythonVersion::_INTERNAL_SENTINEL)
        );
        assert_is_invalid_for_parse_target_or_sentinel_value("not an enum value");
    }

    #[test]
    fn parse_srcs_value() {
        assert_eq!(
            PythonVersion::parse_srcs_value("PY2"),
            Ok(PythonVersion::PY2)
        );
        assert_eq!(
            PythonVersion::parse_srcs_value("PY3"),
            Ok(PythonVersion::PY3)
        );
        assert_eq!(
            PythonVersion::parse_srcs_value("PY2AND3"),
            Ok(PythonVersion::PY2AND3)
        );
        assert_eq!(
            PythonVersion::parse_srcs_value("PY2ONLY"),
            Ok(PythonVersion::PY2ONLY)
        );
        assert_eq!(
            PythonVersion::parse_srcs_value("PY3ONLY"),
            Ok(PythonVersion::PY3ONLY)
        );
        assert_is_invalid_for_parse_srcs_value("_INTERNAL_SENTINEL");
        assert_is_invalid_for_parse_srcs_value("not an enum value");
    }
}
