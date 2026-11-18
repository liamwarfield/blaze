//! Port of `analysis/config/FragmentOptions.java`.

use std::cmp::Ordering;
use std::collections::HashMap;

use blaze_options::{EnvVar, OptionsClass};

// Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:31 FragmentOptions
/// Command-line build options for a Blaze module.
///
/// Implemented by every options class of this module that `extends FragmentOptions` in Java.
/// `clone()` is Rust's `Clone` (the macro derives it).
pub trait FragmentOptions: OptionsClass {
    // Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:47 FragmentOptions.getDefault
    /// Creates a new instance of this `FragmentOptions` with all flags set to their default
    /// values.
    fn get_default() -> Self {
        blaze_options::Options::<Self>::get_defaults()
    }

    // Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:71 FragmentOptions.getNormalized
    /// Returns an instance of `FragmentOptions` with all flags adjusted to be suitable for forming
    /// configurations.
    ///
    /// Motivation: Sometimes a fragment's physical option values, as set by the options parser,
    /// do not correspond to their logical interpretation. For example, an option may need custom
    /// code to determine its logical default value at runtime, but it's limited to a single
    /// hard-coded physical default value in the `Option#defaultValue` annotation field. If two
    /// instances of the fragment have the same logical value but different physical values, a
    /// redundant configuration can be created, which results in an action conflict (particularly
    /// for unshareable actions; see #7808).
    ///
    /// To solve this, we can distinguish between "normalized" and "non-normalized" instances of a
    /// fragment type, and preserve the invariant that configured targets only ever see normalized
    /// instances. This requires that 1) the top-level configuration is normalized, and 2) all
    /// transitions preserve normalization. Step 1) is ensured by `BuildOptions` calling this
    /// method. Step 2) is the responsibility of each transition implementation.
    // PORT: Java returns `this` when already normalized; Rust returns an equal clone.
    fn get_normalized(&self) -> Self {
        self.clone()
    }
}

/// Java `String.compareTo` (UTF-16 code unit order), used where Java sorts strings by their
/// natural order. Rust's `str` order (code points) differs only for supplementary characters.
pub fn java_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:80 FragmentOptions.dedupAndSort
/// Helper method for subclasses to normalize set valued options. In addition to removing
/// duplicates, it picks a deterministic ordering. The fact that the deterministic ordering is
/// based on sorting is an accident and should NOT be relied upon.
pub fn dedup_and_sort(values: Option<&[String]>) -> Vec<String> {
    let Some(values) = values else {
        return Vec::new();
    };
    if values.is_empty() {
        return Vec::new();
    }
    // Use the natural String ordering.
    let mut result = values.to_vec();
    result.sort_by(|a, b| java_string_compare(a, b));
    result.dedup();
    result
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:100 FragmentOptions.normalizeEntries
/// Helper method for subclasses to normalize list of map entries by keeping only the last entry
/// for each key. The order of the entries is preserved.
pub fn normalize_entries<V: Clone>(entries: &[(String, V)]) -> Vec<(String, V)> {
    // LinkedHashMap: a repeated key keeps its first position and takes the last value.
    let mut normalized: Vec<(String, V)> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (k, v) in entries {
        match index.get(k.as_str()) {
            Some(&i) => normalized[i].1 = v.clone(),
            None => {
                index.insert(k, normalized.len());
                normalized.push((k.clone(), v.clone()));
            }
        }
    }
    // If we made no changes, return the same instance we got to reduce churn.
    if normalized.len() == entries.len() {
        return entries.to_vec();
    }
    normalized
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:119 FragmentOptions.normalizeEnvVars
/// Helper method for subclasses to normalize list of `Converters.EnvVar`s by keeping only the
/// last entry for each key. The order of the entries is preserved.
pub fn normalize_env_vars(entries: &[EnvVar]) -> Vec<EnvVar> {
    let mut normalized: Vec<EnvVar> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        match index.get(entry.name()) {
            Some(&i) => normalized[i] = entry.clone(),
            None => {
                index.insert(entry.name(), normalized.len());
                normalized.push(entry.clone());
            }
        }
    }
    // If we made no changes, return the same instance we got to reduce churn.
    if normalized.len() == entries.len() {
        return entries.to_vec();
    }
    normalized
}

// Java: com/google/devtools/build/lib/analysis/config/FragmentOptions.java:135 FragmentOptions.SelectRestriction
/// Tracks limitations on referring to an option in a `config_setting`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentOptionsSelectRestriction {
    visible_within_tools_package: bool,
    error_message: Option<String>,
}

impl FragmentOptionsSelectRestriction {
    pub fn new(visible_within_tools_package: bool, error_message: Option<String>) -> Self {
        FragmentOptionsSelectRestriction {
            visible_within_tools_package,
            error_message,
        }
    }

    /// Whether the option can still be seen by `config_setting`s that are defined by packages
    /// underneath the tools repository's "tools" package, e.g. `@bazel_tools//tools/...`.
    pub fn is_visible_within_tools_package(&self) -> bool {
        self.visible_within_tools_package
    }

    /// An additional explanation to append to the generic error message when a user attempts to
    /// use this option. Should explain why this option is unavailable.
    ///
    /// If null, no content will be appended to the generic error message.
    pub fn get_error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dedup_and_sort_sorts_and_dedups() {
        assert_eq!(dedup_and_sort(None), Vec::<String>::new());
        assert_eq!(
            dedup_and_sort(Some(&s(&["b", "a", "b", "c"]))),
            s(&["a", "b", "c"])
        );
    }

    #[test]
    fn normalize_entries_keeps_last_value_at_first_position() {
        let e = vec![
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            ("a".to_string(), 3),
        ];
        assert_eq!(
            normalize_entries(&e),
            vec![("a".to_string(), 3), ("b".to_string(), 2)]
        );
    }

    #[test]
    fn normalize_env_vars_keeps_last() {
        let e = vec![
            EnvVar::Inherit { name: "A".into() },
            EnvVar::Set {
                name: "B".into(),
                value: "1".into(),
            },
            EnvVar::Unset { name: "A".into() },
        ];
        assert_eq!(
            normalize_env_vars(&e),
            vec![
                EnvVar::Unset { name: "A".into() },
                EnvVar::Set {
                    name: "B".into(),
                    value: "1".into()
                },
            ]
        );
    }
}
