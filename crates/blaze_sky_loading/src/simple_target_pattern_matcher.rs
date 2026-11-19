//! Port of `collect/SimpleTargetPatternMatcher.java`: matches labels against a list of simple
//! target patterns (exact labels, `//pkg/...` wildcards, and `-` negations), as the buildable
//! units of a `PROJECT.scl` use them.
//!
//! PORT: Java's class is in `lib.collect`; `blaze_collect` does not depend on `blaze_cmdline`
//! (Label), so it lives here next to its users (project files).

use std::fmt;

use blaze_cmdline::{Label, LabelSyntaxException};
use blaze_vfs::PathFragment;

/// The result of one sub-matcher.
// Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:135 SimpleTargetPatternMatcher.MatchResult
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatchResult {
    Include,
    Exclude,
    NotRelevant,
}

/// One pattern (Java `SinglePatternMatcher`).
// Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:141 SimpleTargetPatternMatcher.SinglePatternMatcher
#[derive(Clone, Debug)]
enum SinglePatternMatcher {
    /// `ExactMatcher`: equality is by the raw pattern.
    Exact { raw_pattern: String, label: Label },
    /// `NegativeMatcher`.
    Negative(Box<SinglePatternMatcher>),
    /// `WildcardMatcher`.
    Wildcard { package_path: PathFragment },
}

impl PartialEq for SinglePatternMatcher {
    // Java: ExactMatcher.equals, NegativeMatcher.equals, WildcardMatcher.equals
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                SinglePatternMatcher::Exact { raw_pattern: a, .. },
                SinglePatternMatcher::Exact { raw_pattern: b, .. },
            ) => a == b,
            (SinglePatternMatcher::Negative(a), SinglePatternMatcher::Negative(b)) => a == b,
            (
                SinglePatternMatcher::Wildcard { package_path: a },
                SinglePatternMatcher::Wildcard { package_path: b },
            ) => a == b,
            _ => false,
        }
    }
}

impl Eq for SinglePatternMatcher {}

impl SinglePatternMatcher {
    fn matches(&self, label: &Label) -> MatchResult {
        match self {
            // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:157 ExactMatcher.matches
            SinglePatternMatcher::Exact { label: l, .. } => {
                if l == label {
                    MatchResult::Include
                } else {
                    MatchResult::NotRelevant
                }
            }
            // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:190 NegativeMatcher.matches
            SinglePatternMatcher::Negative(inner) => match inner.matches(label) {
                MatchResult::Include => MatchResult::Exclude,
                MatchResult::Exclude | MatchResult::NotRelevant => MatchResult::NotRelevant,
            },
            // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:226 WildcardMatcher.matches
            SinglePatternMatcher::Wildcard { package_path } => {
                if label.get_package_fragment().starts_with(package_path) {
                    MatchResult::Include
                } else {
                    MatchResult::NotRelevant
                }
            }
        }
    }
}

impl fmt::Display for SinglePatternMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: ExactMatcher.toString
            SinglePatternMatcher::Exact { raw_pattern, .. } => f.write_str(raw_pattern),
            // Java: NegativeMatcher.toString
            SinglePatternMatcher::Negative(inner) => write!(f, "-{}", inner),
            // Java: WildcardMatcher.toString
            SinglePatternMatcher::Wildcard { package_path } => write!(f, "//{}/...", package_path),
        }
    }
}

// Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:33 SimpleTargetPatternMatcher
/// Matches labels against simple target patterns; later patterns override earlier ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleTargetPatternMatcher {
    single_pattern_matchers: Vec<SinglePatternMatcher>,
}

impl SimpleTargetPatternMatcher {
    // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:37 SimpleTargetPatternMatcher.create
    pub fn create(patterns: &[String]) -> Result<SimpleTargetPatternMatcher, LabelSyntaxException> {
        let mut single_pattern_matchers = Vec::with_capacity(patterns.len());
        for pattern in patterns {
            single_pattern_matchers.push(Self::create_single_pattern_matcher(pattern)?);
        }
        Ok(SimpleTargetPatternMatcher {
            single_pattern_matchers,
        })
    }

    // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:53 SimpleTargetPatternMatcher.isEmpty
    pub fn is_empty(&self) -> bool {
        self.single_pattern_matchers.is_empty()
    }

    // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:61 SimpleTargetPatternMatcher.contains
    /// Whether `label` matches: the last pattern that includes or excludes it decides.
    pub fn contains(&self, label: &Label) -> bool {
        if self.single_pattern_matchers.is_empty() {
            return false;
        }
        // Check each sub-matcher.
        let mut result = MatchResult::Exclude;
        for matcher in &self.single_pattern_matchers {
            let match_result = matcher.matches(label);
            if match_result == MatchResult::Include || match_result == MatchResult::Exclude {
                result = match_result;
            }
        }
        result == MatchResult::Include
    }

    // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:117 SimpleTargetPatternMatcher.createSinglePatternMatcher
    fn create_single_pattern_matcher(
        pattern: &str,
    ) -> Result<SinglePatternMatcher, LabelSyntaxException> {
        if let Some(rest) = pattern.strip_prefix('-') {
            // Strip off the leading '-' and create a matcher for what remains. This will
            // technically handle a series of nested negative patterns (like
            // `---//exact:target`), but isn't worth detecting and throwing an error.
            let inner = Self::create_single_pattern_matcher(rest)?;
            return Ok(SinglePatternMatcher::Negative(Box::new(inner)));
        } else if pattern.ends_with("/...") {
            // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:220 WildcardMatcher.<init>
            // Strip off the leading "//" and the trailing "/..." and create the wildcard matcher.
            // pattern.substring(2, pattern.lastIndexOf("...")): chars, not bytes.
            let chars: Vec<char> = pattern.chars().collect();
            let end = pattern
                .rfind("...")
                .map(|b| pattern[..b].chars().count())
                .unwrap();
            let inner: String = chars[2.min(end)..end].iter().collect();
            return Ok(SinglePatternMatcher::Wildcard {
                package_path: PathFragment::create(&inner),
            });
        }
        // Just match the pattern as an exact label.
        // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:149 ExactMatcher.<init>
        Ok(SinglePatternMatcher::Exact {
            raw_pattern: pattern.to_owned(),
            label: Label::parse_canonical(pattern)?,
        })
    }
}

impl fmt::Display for SimpleTargetPatternMatcher {
    // Java: com/google/devtools/build/lib/collect/SimpleTargetPatternMatcher.java:75 SimpleTargetPatternMatcher.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}]",
            self.single_pattern_matchers
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
