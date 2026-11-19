//! Port of `query2/aquery/AqueryActionFilter.java`.

use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{
    AqueryActionFilter as AqueryActionFilterTrait, AqueryFilterFunction,
};
use indexmap::IndexMap;
use blaze_util::java_regex::Pattern;

/// Represents the filters (inputs, outputs, mnemonic) of an aquery expression.
// Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:28 AqueryActionFilter
#[derive(Clone, Debug, Default)]
pub struct AqueryActionFilter {
    /// Java `ImmutableMultimap<String, Pattern>` (from a `HashMultimap`).
    // PORT: java-hash-order: only membership and all-match are observable.
    filter_map: IndexMap<String, Vec<Pattern>>,
}

impl AqueryActionFilter {
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:35 AqueryActionFilter.emptyInstance
    pub fn empty_instance() -> Self {
        Self::builder().build()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:39 AqueryActionFilter.builder
    pub fn builder() -> AqueryActionFilterBuilder {
        AqueryActionFilterBuilder {
            filter_map: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:43 AqueryActionFilter.hasFilterForFunction
    pub fn has_filter_for_function(&self, function: &str) -> bool {
        self.filter_map.contains_key(function)
    }

    /// Returns whether the input string matches ALL the filter patterns of a specific type parsed
    /// from aquery command.
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:54 AqueryActionFilter.matchesAllPatternsForFunction
    pub fn matches_all_patterns_for_function(&self, function: &str, input: &str) -> bool {
        if !self.has_filter_for_function(function) {
            return false;
        }
        // Pattern.matcher(input).matches(): the whole input must match.
        self.filter_map[function]
            .iter()
            .all(|pattern| pattern.matches(input))
    }
}

fn function_name(function: AqueryFilterFunction) -> &'static str {
    match function {
        AqueryFilterFunction::MNEMONIC => "mnemonic",
        AqueryFilterFunction::INPUTS => "inputs",
        AqueryFilterFunction::OUTPUTS => "outputs",
    }
}

impl AqueryActionFilterTrait for AqueryActionFilter {
    fn has_filter_for_function(&self, function: AqueryFilterFunction) -> bool {
        AqueryActionFilter::has_filter_for_function(self, function_name(function))
    }

    fn matches_all_patterns_for_function(
        &self,
        function: AqueryFilterFunction,
        value: &str,
    ) -> bool {
        AqueryActionFilter::matches_all_patterns_for_function(self, function_name(function), value)
    }
}

/// Builder class for `AqueryActionFilter`.
// Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:63 AqueryActionFilter.Builder
pub struct AqueryActionFilterBuilder {
    filter_map: IndexMap<String, Vec<Pattern>>,
}

impl AqueryActionFilterBuilder {
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:70 AqueryActionFilter.Builder.put
    pub fn put(&mut self, key: &str, value: Pattern) -> &mut Self {
        // HashMultimap: a (key, value) pair is stored once.
        let values = self.filter_map.entry(key.to_owned()).or_default();
        if !values.iter().any(|v| v.pattern() == value.pattern()) {
            values.push(value);
        }
        self
    }

    // Java: com/google/devtools/build/lib/query2/aquery/AqueryActionFilter.java:75 AqueryActionFilter.Builder.build
    pub fn build(&self) -> AqueryActionFilter {
        AqueryActionFilter {
            filter_map: self.filter_map.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_require_the_whole_input() {
        let mut builder = AqueryActionFilter::builder();
        builder.put("mnemonic", Pattern::compile("Cpp.*", 0).unwrap());
        let filter = builder.build();
        assert!(filter.matches_all_patterns_for_function("mnemonic", "CppCompile"));
        assert!(!filter.matches_all_patterns_for_function("mnemonic", "ObjcCppCompile"));
        assert!(!filter.matches_all_patterns_for_function("inputs", "CppCompile"));
    }

    #[test]
    fn empty_instance_has_no_filters() {
        let filter = AqueryActionFilter::empty_instance();
        assert!(!filter.has_filter_for_function("mnemonic"));
    }
}
