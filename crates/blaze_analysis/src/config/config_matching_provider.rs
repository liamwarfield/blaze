//! Port of `analysis/config/ConfigMatchingProvider.java`.

use std::any::Any;
use std::fmt;

use blaze_cmdline::Label;
use indexmap::{IndexMap, IndexSet};

use crate::provider_map::TransitiveInfoProvider;

/// A human-readable description of the difference between the expected and actual
/// configuration (Java `MatchResult.NoMatch.Diff`).
// Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:68 ConfigMatchingProvider.MatchResult.NoMatch.Diff
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diff {
    /// The label of the constraint or setting that failed to match.
    pub what: Label,
    /// The actual value of the setting.
    pub got: String,
    /// The expected value of the setting.
    pub want: String,
}

impl Diff {
    pub fn new(what: Label, got: impl Into<String>, want: impl Into<String>) -> Diff {
        Diff {
            what,
            got: got.into(),
            want: want.into(),
        }
    }
}

/// Potential values for the result field (Java `ConfigMatchingProvider.MatchResult`).
// Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:45 ConfigMatchingProvider.MatchResult
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MatchResult {
    /// The configuration matches.
    Match,
    /// The configuration does not match.
    NoMatch(Vec<Diff>),
    /// Errors make the match question irresolvable.
    InError(Vec<String>),
}

impl MatchResult {
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:52 MatchResult.MATCH
    pub const MATCH: MatchResult = MatchResult::Match;

    /// A result for the case in which an analysis error occurred that prevents the match from
    /// being evaluated.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:95 MatchResult.ALREADY_REPORTED_NO_MATCH
    pub fn already_reported_no_match() -> MatchResult {
        MatchResult::NoMatch(Vec::new())
    }

    /// Java `new NoMatch(diff)`.
    pub fn no_match(diff: Diff) -> MatchResult {
        MatchResult::NoMatch(vec![diff])
    }

    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:101 MatchResult.combine
    pub fn combine(previous: MatchResult, current: MatchResult) -> MatchResult {
        match previous {
            // InError is the most severe state and always takes precedence.
            MatchResult::InError(previous_errors) => match current {
                MatchResult::InError(current_errors) => {
                    let mut errors = previous_errors;
                    errors.extend(current_errors);
                    MatchResult::InError(errors)
                }
                _ => MatchResult::InError(previous_errors),
            },
            MatchResult::NoMatch(previous_diffs) => match current {
                MatchResult::InError(_) => current,
                MatchResult::NoMatch(current_diffs) => {
                    let mut diffs = previous_diffs;
                    diffs.extend(current_diffs);
                    MatchResult::NoMatch(diffs)
                }
                MatchResult::Match => MatchResult::NoMatch(previous_diffs),
            },
            MatchResult::Match => current,
        }
    }
}

/// Result of accumulating match results: contains any errors or non-matching labels.
// Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:129 ConfigMatchingProvider.AccumulateResults
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccumulateResults {
    pub non_matching: Vec<Label>,
    /// Java `ImmutableMultimap<Label, String>` (insertion-ordered keys).
    pub errors: IndexMap<Label, Vec<String>>,
}

impl AccumulateResults {
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:131 AccumulateResults.success
    pub fn success(&self) -> bool {
        self.non_matching.is_empty() && self.errors.is_empty()
    }
}

/// A "configuration target" that asserts whether or not it matches the configuration it's bound
/// to (Java `ConfigMatchingProvider`, an `@AutoValue`).
// Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:39 ConfigMatchingProvider
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigMatchingProvider {
    label: Label,
    /// Java `ImmutableMultimap<String, String>`: keys in insertion order, values per key in
    /// insertion order.
    settings_map: Vec<(String, String)>,
    flag_settings_map: IndexMap<Label, String>,
    constraint_values_setting: IndexSet<Label>,
    result: MatchResult,
}

impl ConfigMatchingProvider {
    /// Combine the results from the given providers, returning any errors and non-matching
    /// providers.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:140 ConfigMatchingProvider.accumulateMatchResults
    pub fn accumulate_match_results(providers: &[ConfigMatchingProvider]) -> AccumulateResults {
        let mut non_matching = Vec::new();
        let mut errors: IndexMap<Label, Vec<String>> = IndexMap::new();
        for config_provider in providers {
            match config_provider.result() {
                MatchResult::InError(messages) => {
                    errors
                        .entry(config_provider.label().clone())
                        .or_default()
                        .extend(messages.iter().cloned());
                }
                MatchResult::NoMatch(_) => non_matching.push(config_provider.label().clone()),
                MatchResult::Match => {}
            }
        }
        AccumulateResults {
            non_matching,
            errors,
        }
    }

    /// Create a ConfigMatchingProvider.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:166 ConfigMatchingProvider.create
    pub fn create(
        label: Label,
        settings_map: Vec<(String, String)>,
        flag_settings_map: IndexMap<Label, String>,
        constraint_value_settings: IndexSet<Label>,
        result: MatchResult,
    ) -> ConfigMatchingProvider {
        ConfigMatchingProvider {
            label,
            settings_map,
            flag_settings_map,
            constraint_values_setting: constraint_value_settings,
            result,
        }
    }

    /// The target's label.
    pub fn label(&self) -> &Label {
        &self.label
    }

    /// Java `settingsMap()` as its `entries()`.
    pub fn settings_map(&self) -> &[(String, String)] {
        &self.settings_map
    }

    pub fn flag_settings_map(&self) -> &IndexMap<Label, String> {
        &self.flag_settings_map
    }

    pub fn constraint_values_setting(&self) -> &IndexSet<Label> {
        &self.constraint_values_setting
    }

    /// Whether or not the configuration criteria defined by this target match its actual
    /// configuration.
    pub fn result(&self) -> &MatchResult {
        &self.result
    }

    /// Returns true if this matcher's conditions are a proper superset of another matcher's
    /// conditions, i.e. if this matcher is a specialization of the other one.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:192 ConfigMatchingProvider.refines
    pub fn refines(&self, other: &ConfigMatchingProvider) -> bool {
        let settings: IndexSet<&(String, String)> = self.settings_map.iter().collect();
        let other_settings: IndexSet<&(String, String)> = other.settings_map.iter().collect();
        let flag_settings: IndexSet<(&Label, &String)> = self.flag_settings_map.iter().collect();
        let other_flag_settings: IndexSet<(&Label, &String)> =
            other.flag_settings_map.iter().collect();

        let constraint_value_settings = &self.constraint_values_setting;
        let other_constraint_value_settings = &other.constraint_values_setting;

        if !other_settings.iter().all(|s| settings.contains(s))
            || !other_flag_settings
                .iter()
                .all(|s| flag_settings.contains(s))
            || !other_constraint_value_settings
                .iter()
                .all(|s| constraint_value_settings.contains(s))
        {
            return false; // Not a superset.
        }

        settings.len() > other_settings.len()
            || flag_settings.len() > other_flag_settings.len()
            || constraint_value_settings.len() > other_constraint_value_settings.len()
    }
}

impl fmt::Display for ConfigMatchingProvider {
    /// Format this provider as its label.
    // Java: com/google/devtools/build/lib/analysis/config/ConfigMatchingProvider.java:220 ConfigMatchingProvider.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

impl TransitiveInfoProvider for ConfigMatchingProvider {
    fn java_class_name(&self) -> &'static str {
        "ConfigMatchingProvider"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(s: &str) -> Label {
        Label::parse_canonical_unchecked(s)
    }

    fn provider(
        settings: &[(&str, &str)],
        flags: &[(&str, &str)],
        constraints: &[&str],
    ) -> ConfigMatchingProvider {
        ConfigMatchingProvider::create(
            label("//a:b"),
            settings
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            flags
                .iter()
                .map(|(k, v)| (label(k), (*v).to_owned()))
                .collect(),
            constraints.iter().map(|c| label(c)).collect(),
            MatchResult::Match,
        )
    }

    #[test]
    fn refines_proper_superset() {
        let a = provider(&[("cpu", "k8"), ("compilation_mode", "opt")], &[], &[]);
        let b = provider(&[("cpu", "k8")], &[], &[]);
        assert!(a.refines(&b));
        assert!(!b.refines(&a));
        assert!(!a.refines(&a));
    }

    #[test]
    fn refines_constraints_and_flags() {
        let a = provider(&[], &[("//f:x", "1")], &["//c:linux", "//c:x86"]);
        let b = provider(&[], &[("//f:x", "1")], &["//c:linux"]);
        assert!(a.refines(&b));
        let c = provider(&[], &[("//f:x", "2")], &["//c:linux"]);
        assert!(!a.refines(&c));
    }

    #[test]
    fn combine_results() {
        let d = Diff::new(label("//a:b"), "x", "y");
        assert_eq!(
            MatchResult::combine(MatchResult::Match, MatchResult::no_match(d.clone())),
            MatchResult::no_match(d.clone())
        );
        assert_eq!(
            MatchResult::combine(
                MatchResult::no_match(d.clone()),
                MatchResult::InError(vec!["e".into()])
            ),
            MatchResult::InError(vec!["e".into()])
        );
        assert_eq!(
            MatchResult::combine(
                MatchResult::InError(vec!["a".into()]),
                MatchResult::InError(vec!["b".into()])
            ),
            MatchResult::InError(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            MatchResult::combine(MatchResult::no_match(d.clone()), MatchResult::Match),
            MatchResult::no_match(d)
        );
    }
}
