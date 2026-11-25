//! Port of `buildtool/InstrumentationFilterSupport.java`.

use std::collections::BTreeSet;

use blaze_events::{Event, EventHandler};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::nonconfigurable_attribute_mapper::NonconfigurableAttributeMapper;
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_packages::types::{AttrValue, Type};
use blaze_vfs::PathFragment;

/// Helper class to heuristically compute an instrumentation filter from a list of tests to run.
// Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:37 InstrumentationFilterSupport
pub struct InstrumentationFilterSupport;

// Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:38 InstrumentationFilterSupport.INSTRUMENTATION_FILTER_FLAG
pub const INSTRUMENTATION_FILTER_FLAG: &str = "instrumentation_filter";

impl InstrumentationFilterSupport {
    /// Method implements a heuristic used to set default value of the --instrumentation_filter
    /// option. The following algorithm is used:
    ///
    /// - Identify all test targets on the command line.
    /// - Expand all test suites into the individual test targets.
    /// - Calculate list of package names containing all test targets above.
    /// - Replace all "javatests" directories in packages with "java". Similarly, replace
    ///   "test/java/" with "main/java". Also, strip trailing "/internal", "/public", and "/tests"
    ///   from packages. (See [`Self::get_instrumented_prefix`].)
    /// - Set --instrumentation_filter default value to instrument everything in those packages.
    // Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:55 InstrumentationFilterSupport.computeInstrumentationFilter
    pub fn compute_instrumentation_filter(
        event_handler: &dyn EventHandler,
        test_targets: &[Target],
    ) -> String {
        let mut package_filters: BTreeSet<String> = BTreeSet::new();
        Self::collect_instrumented_packages(test_targets, &mut package_filters);
        Self::optimize_filter_set(&mut package_filters);

        // Joiner.on("[/:],^//").appendTo(new StringBuilder("^//"), packageFilters).append("[/:]")
        let mut instrumentation_filter = String::from("^//");
        let mut first = true;
        for filter in &package_filters {
            if !first {
                instrumentation_filter.push_str("[/:],^//");
            }
            first = false;
            instrumentation_filter.push_str(filter);
        }
        instrumentation_filter.push_str("[/:]");
        // Fix up if one of the test targets is a top-level target. "//foo[/:]" matches everything
        // under //foo and subpackages, but "//[/:]" only matches targets directly under the
        // top-level package.
        if instrumentation_filter == "^//[/:]" {
            instrumentation_filter = "^//".to_owned();
        }
        if !package_filters.is_empty() {
            event_handler.handle(&Event::info(format!(
                "Using default value for --instrumentation_filter: \"{instrumentation_filter}\"."
            )));
            event_handler.handle(&Event::info(format!(
                "Override the above default with --{INSTRUMENTATION_FILTER_FLAG}"
            )));
        }
        instrumentation_filter
    }

    // Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:82 InstrumentationFilterSupport.collectInstrumentedPackages
    fn collect_instrumented_packages(targets: &[Target], package_filters: &mut BTreeSet<String>) {
        for target in targets {
            // Add package-based filters for every test target.
            package_filters.insert(Self::get_instrumented_prefix(
                target.get_label().get_package_name(),
            ));
            if target_utils::is_test_suite_rule(target) {
                let Target::Rule(rule) = target else {
                    unreachable!("a test_suite is a rule")
                };
                let attributes = NonconfigurableAttributeMapper::of(&**rule);
                // We don't need to handle $implicit_tests attribute since we already added
                // test_suite package to the set.
                let labels = match attributes.get("tests", Type::LabelList) {
                    Some(AttrValue::LabelList(labels)) => labels,
                    _ => panic!("NullPointerException: tests of {}", target.get_label()),
                };
                for label in labels {
                    // Add package-based filters for all tests in the test suite.
                    package_filters.insert(Self::get_instrumented_prefix(label.get_package_name()));
                }
            }
        }
    }

    /// Returns prefix string that should be instrumented for a given package. Input string should
    /// be formatted like the output of `Label.getPackageName()`. Generally, package name will be
    /// used as such string with two modifications.
    /// - "javatests/ directories will be substituted with "java/", since we do not want to
    ///   instrument java test code. "java/" directories in "test/" will be replaced by the same in
    ///   "main/".
    /// - "/internal", "/public", and "tests/" package suffix will be dropped, since usually we
    ///   would want to instrument code in the parent package as well
    // Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:110 InstrumentationFilterSupport.getInstrumentedPrefix
    pub fn get_instrumented_prefix(package_name: &str) -> String {
        let mut package_name = package_name;
        if let Some(stripped) = package_name.strip_suffix("/internal") {
            package_name = stripped;
        } else if let Some(stripped) = package_name.strip_suffix("/public") {
            package_name = stripped;
        } else if let Some(stripped) = package_name.strip_suffix("/tests") {
            package_name = stripped;
        }
        let replaced = replace_first_at_segment_start(package_name, "javatests/", "java/");
        replace_first_at_segment_start(&replaced, "test/java/", "main/java/")
    }

    // Java: com/google/devtools/build/lib/buildtool/InstrumentationFilterSupport.java:123 InstrumentationFilterSupport.optimizeFilterSet
    fn optimize_filter_set(package_filters: &mut BTreeSet<String>) {
        // Optimize away nested filters.
        let mut kept: BTreeSet<String> = BTreeSet::new();
        let mut prev: Option<PathFragment> = None;
        for filter in package_filters.iter() {
            let current = PathFragment::create(filter);
            match &prev {
                Some(p) if current.starts_with(p) => {}
                _ => {
                    kept.insert(filter.clone());
                    prev = Some(current);
                }
            }
        }
        *package_filters = kept;
    }
}

/// `s.replaceFirst("(?<=^|/)" + quote(from), to)`: replaces the first occurrence of `from` that
/// starts the string or follows a `/`.
fn replace_first_at_segment_start(s: &str, from: &str, to: &str) -> String {
    let mut start = 0;
    while let Some(pos) = s[start..].find(from) {
        let index = start + pos;
        if index == 0 || s.as_bytes()[index - 1] == b'/' {
            return format!("{}{}{}", &s[..index], to, &s[index + from.len()..]);
        }
        // `from` starts with an ASCII letter, so `index + 1` is a char boundary.
        start = index + 1;
    }
    s.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_instrumented_prefix_rewrites() {
        let f = InstrumentationFilterSupport::get_instrumented_prefix;
        assert_eq!(f("foo/internal"), "foo");
        assert_eq!(f("foo/public"), "foo");
        assert_eq!(f("foo/tests"), "foo");
        assert_eq!(f("javatests/com/foo"), "java/com/foo");
        assert_eq!(f("a/javatests/b"), "a/java/b");
        assert_eq!(f("xjavatests/b"), "xjavatests/b");
        assert_eq!(f("src/test/java/x"), "src/main/java/x");
        assert_eq!(f("tests"), "tests");
        assert_eq!(f(""), "");
    }

    #[test]
    fn optimize_filter_set_drops_nested() {
        let mut set: BTreeSet<String> = ["foo", "foo/bar", "foobar", "a/b", "a"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        InstrumentationFilterSupport::optimize_filter_set(&mut set);
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            vec!["a".to_string(), "foo".to_string(), "foobar".to_string()]
        );
    }
}
