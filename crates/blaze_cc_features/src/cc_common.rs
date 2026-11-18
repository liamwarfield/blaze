//! Port of the parts of `com.google.devtools.build.lib.rules.cpp.CcCommon` that do not need
//! analysis: `CoptsFilter`.

use regex::Regex;

/// A filter that removes copts from a c++ compile action according to a nocopts regex.
// Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:101 CcCommon.CoptsFilter
// PORT: Java's `java.util.regex` syntax is evaluated with the `regex` crate; the two agree on the
// common subset (no backreferences or lookaround).
#[derive(Clone, Debug)]
pub struct CoptsFilter {
    no_copts_pattern: Option<String>,
    /// The pattern anchored at both ends, for Java's `Matcher.matches()`.
    anchored: Option<Regex>,
    all_passes: bool,
}

impl CoptsFilter {
    /// Creates a filter that filters all matches to a regex; `Err` with the regex error where
    /// `Pattern.compile` throws `PatternSyntaxException`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:111 CcCommon.CoptsFilter.fromRegex
    pub fn from_regex(no_copts_pattern: &str) -> Result<CoptsFilter, regex::Error> {
        let anchored = Regex::new(&format!("^(?:{no_copts_pattern})$"))?;
        Ok(CoptsFilter {
            no_copts_pattern: Some(no_copts_pattern.to_string()),
            anchored: Some(anchored),
            all_passes: false,
        })
    }

    /// Creates a filter that passes on all inputs.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:116 CcCommon.CoptsFilter.alwaysPasses
    pub fn always_passes() -> CoptsFilter {
        CoptsFilter {
            no_copts_pattern: None,
            anchored: None,
            all_passes: true,
        }
    }

    /// Returns true if the provided string passes through the filter, or false if it should be
    /// removed.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:124 CcCommon.CoptsFilter.passesFilter
    pub fn passes_filter(&self, flag: &str) -> bool {
        if self.all_passes {
            true
        } else {
            !self.anchored.as_ref().unwrap().is_match(flag)
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCommon.java:138 CcCommon.CoptsFilter.repr
    pub fn repr(&self) -> String {
        format!(
            "CoptsFilter(noCoptsPattern={}, allPasses={})",
            self.no_copts_pattern.as_deref().unwrap_or("null"),
            self.all_passes
        )
    }
}
