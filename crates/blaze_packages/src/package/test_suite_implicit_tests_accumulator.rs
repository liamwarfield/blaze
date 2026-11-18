//! Port of `packages/TestSuiteImplicitTestsAccumulator.java`: gathers tests for `test_suite`'s
//! `$implicit_tests` attribute.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use crate::attribute::AttributeMap;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::target::ImplicitTestsRef;
use crate::target::Rule;
use crate::target::Target;
use crate::target::target_utils;
use crate::types::AttrValue;
use crate::types::Type;

/// Java `TestSuiteImplicitTestsAccumulator.ImplicitTestsAccumulator`.
// Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:103 TestSuiteImplicitTestsAccumulator.ImplicitTestsAccumulator
#[derive(Debug)]
struct ImplicitTestsAccumulator {
    required_tags: HashSet<String>,
    excluded_tags: HashSet<String>,
    tests: ImplicitTestsRef,
}

impl ImplicitTestsAccumulator {
    // Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:108 TestSuiteImplicitTestsAccumulator.ImplicitTestsAccumulator.<init>
    fn new(test_tags: &BTreeSet<String>) -> Self {
        let tags: Vec<&String> = test_tags.iter().collect();
        let (required_tags, excluded_tags) = target_utils::sort_tags_by_sense(&tags);
        ImplicitTestsAccumulator {
            required_tags,
            excluded_tags,
            tests: Arc::new(parking_lot::RwLock::new(Vec::new())),
        }
    }
}

/// Java `TestSuiteImplicitTestsAccumulator`.
// Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:46 TestSuiteImplicitTestsAccumulator
#[derive(Debug, Default)]
pub struct TestSuiteImplicitTestsAccumulator {
    // Java keys by ImmutableSet<String> (set equality).
    test_suite_implicit_tests: HashMap<BTreeSet<String>, ImplicitTestsAccumulator>,
}

impl TestSuiteImplicitTestsAccumulator {
    /// The shared list of tests matching `tags` (or all tests if empty), populated by
    /// [`TestSuiteImplicitTestsAccumulator::process_rule`].
    // Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:54 TestSuiteImplicitTestsAccumulator.getTestSuiteImplicitTestsRefForTags
    pub fn get_test_suite_implicit_tests_ref_for_tags(
        &mut self,
        tags: &[String],
    ) -> ImplicitTestsRef {
        let key: BTreeSet<String> = tags.iter().cloned().collect();
        let accumulator_for_tags = self
            .test_suite_implicit_tests
            .entry(key.clone())
            .or_insert_with(|| ImplicitTestsAccumulator::new(&key));
        accumulator_for_tags.tests.clone()
    }

    /// Clears all accumulated tests.
    // Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:62 TestSuiteImplicitTestsAccumulator.clearAccumulatedTests
    pub fn clear_accumulated_tests(&mut self) {
        for acc in self.test_suite_implicit_tests.values() {
            acc.tests.write().clear();
        }
    }

    /// Adds `rule` to the matching `$implicit_tests` lists.
    // Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:70 TestSuiteImplicitTestsAccumulator.processRule
    pub fn process_rule(&mut self, rule: &Arc<Rule>) {
        if self.test_suite_implicit_tests.is_empty() {
            // No test suites requiring implicit test accumulation encountered.
            return;
        }

        let target = Target::Rule(rule.clone());
        if target_utils::is_test_rule(&target) && !target_utils::has_manual_tag(&target) {
            let mapper = NonconfigurableAttributeMapper::of(&**rule);
            let mut test_suite_tags: HashSet<String> = HashSet::new();
            if let Some(AttrValue::StringList(tags)) = mapper.get("tags", Type::StringList) {
                test_suite_tags.extend(tags);
            }
            if let Some(AttrValue::String(size)) = mapper.get("size", Type::String) {
                test_suite_tags.insert(size);
            }
            for acc in self.test_suite_implicit_tests.values() {
                if target_utils::test_matches_filters(
                    &test_suite_tags,
                    &acc.required_tags,
                    &acc.excluded_tags,
                    true,
                ) {
                    acc.tests.write().push(rule.get_label().clone());
                }
            }
        }
    }

    /// Sorts all accumulated test lists.
    // Java: com/google/devtools/build/lib/packages/TestSuiteImplicitTestsAccumulator.java:94 TestSuiteImplicitTestsAccumulator.sortTests
    pub fn sort_tests(&mut self) {
        for acc in self.test_suite_implicit_tests.values() {
            acc.tests.write().sort();
        }
    }
}
