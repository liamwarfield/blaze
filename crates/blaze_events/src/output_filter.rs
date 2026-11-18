//! Port of `com.google.devtools.build.lib.events.OutputFilter`.

use std::fmt;
use std::sync::Arc;

use regex::Regex;

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
/// PORT: Java uses `java.util.regex.Pattern`; Rust uses the `regex` crate, whose syntax differs
/// for look-around and back-references (rejected at compile time with an error).
// Java: com/google/devtools/build/lib/events/OutputFilter.java:33 OutputFilter.RegexOutputFilter
pub struct RegexOutputFilter {
    pattern: Regex,
}

impl RegexOutputFilter {
    /// Returns an output filter for the given regex (by compiling it).
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:35 RegexOutputFilter.forRegex
    pub fn for_regex(regex: &str) -> Result<Arc<dyn OutputFilter>, regex::Error> {
        Ok(RegexOutputFilter::for_pattern(Regex::new(regex)?))
    }

    /// Returns an output filter for the given pattern.
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:40 RegexOutputFilter.forPattern
    pub fn for_pattern(pattern: Regex) -> Arc<dyn OutputFilter> {
        Arc::new(RegexOutputFilter { pattern })
    }
}

impl OutputFilter for RegexOutputFilter {
    // Java: com/google/devtools/build/lib/events/OutputFilter.java:51 RegexOutputFilter.showOutput
    fn show_output(&self, tag: &str) -> bool {
        // PORT: StringEncoding identity (internalToUnicode).
        self.pattern.is_match(tag)
    }
}

// Java: com/google/devtools/build/lib/events/OutputFilter.java:56 RegexOutputFilter.toString
impl fmt::Display for RegexOutputFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.pattern.as_str())
    }
}
