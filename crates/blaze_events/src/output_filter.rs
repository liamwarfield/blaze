//! Port of `com.google.devtools.build.lib.events.OutputFilter`.

use std::fmt;
use std::sync::Arc;

use blaze_util::java_regex::Pattern;

/// An output filter for warnings. Port of `OutputFilter`.
// Java: com/google/devtools/build/lib/events/OutputFilter.java:21 OutputFilter
pub trait OutputFilter: Send + Sync {
    /// Returns true iff the given tag matches the output filter.
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:30 OutputFilter.showOutput
    fn show_output(&self, tag: &str) -> bool;
}

struct ConstantOutputFilter(bool);

impl OutputFilter for ConstantOutputFilter {
    fn show_output(&self, _tag: &str) -> bool {
        self.0
    }
}

/// An output filter that matches everything.
// Java: com/google/devtools/build/lib/events/OutputFilter.java:24 OutputFilter.OUTPUT_EVERYTHING
pub fn output_everything() -> Arc<dyn OutputFilter> {
    Arc::new(ConstantOutputFilter(true))
}

/// An output filter that matches nothing.
// Java: com/google/devtools/build/lib/events/OutputFilter.java:27 OutputFilter.OUTPUT_NOTHING
pub fn output_nothing() -> Arc<dyn OutputFilter> {
    Arc::new(ConstantOutputFilter(false))
}

/// An output filter using regular expression matching. Port of `OutputFilter.RegexOutputFilter`.
///
/// `java.util.regex.Pattern` is `blaze_util::java_regex::Pattern` (Java's dialect on the `regex`
/// crate; its module docs list what is `unported!`).
// Java: com/google/devtools/build/lib/events/OutputFilter.java:33 OutputFilter.RegexOutputFilter
pub struct RegexOutputFilter {
    pattern: Pattern,
}

impl RegexOutputFilter {
    /// Returns an output filter for the given regex (by compiling it).
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:35 RegexOutputFilter.forRegex
    /// The error is `PatternSyntaxException.getMessage()`.
    pub fn for_regex(regex: &str) -> Result<Arc<dyn OutputFilter>, String> {
        Ok(RegexOutputFilter::for_pattern(Pattern::compile(regex, 0)?))
    }

    /// Returns an output filter for the given pattern.
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:40 RegexOutputFilter.forPattern
    pub fn for_pattern(pattern: Pattern) -> Arc<dyn OutputFilter> {
        Arc::new(RegexOutputFilter { pattern })
    }
}

impl OutputFilter for RegexOutputFilter {
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:51 RegexOutputFilter.showOutput
    fn show_output(&self, tag: &str) -> bool {
        self.pattern
            .is_match(&blaze_util::string_encoding::internal_to_unicode(tag))
    }
}

// Java: com/google/devtools/build/lib/events/OutputFilter.java:56 RegexOutputFilter.toString
impl fmt::Display for RegexOutputFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.pattern.as_str())
    }
}
