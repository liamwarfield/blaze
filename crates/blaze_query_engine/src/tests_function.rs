//! Port of `query2/engine/TestsFunction.java`.

use std::collections::HashSet;
use std::sync::Mutex;

use blaze_protos::failure_details::{FailureDetail, Query, failure_detail::Category, query};

use crate::callback::{Callback, callback_fn};
use crate::detailed_exit_code::DetailedExitCode;
use crate::query_environment::{
    Argument, ArgumentType, QueryEnvironment, QueryFunction, QueryFunctionBase, Setting,
    TargetAccessor,
};
use crate::query_exception::{QueryError, QueryException};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture, as_any_futures};
use crate::uniquifier::Uniquifier;

/// A tests(x) filter expression, which returns all the tests in set x, expanding test_suite rules
/// into their constituents.
///
/// Unfortunately this class reproduces a substantial amount of logic from
/// `TestSuiteConfiguredTarget`, albeit in a somewhat simplified form. This is basically
/// inevitable since the expansion of test_suites cannot be done during the loading phase, because
/// it involves inter-package references. We make no attempt to validate the input, or report
/// errors or warnings other than missing target.
///
/// `expr ::= TESTS '(' expr ')'`
// Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:49 TestsFunction
#[derive(Debug, Default)]
pub struct TestsFunction;

impl TestsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:51 TestsFunction.<init>
    pub fn new() -> Self {
        TestsFunction
    }

    // TODO(ulfjack): This must match the code in TestTargetUtils. However, we don't currently want
    // to depend on the packages library. Extract to a neutral place?
    /// Decides whether to include a test in a test_suite or not.
    ///
    /// - `test_tags`: Collection of all tags exhibited by a given test.
    /// - `positive_tags`: Tags declared by the suite. A test must match ALL of these.
    /// - `negative_tags`: Tags declared by the suite. A test must match NONE of these.
    ///
    /// Returns false is the test is to be removed.
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:298 TestsFunction.includeTest
    fn include_test_tags(
        test_tags: &[String],
        positive_tags: &HashSet<String>,
        negative_tags: &HashSet<String>,
    ) -> bool {
        // Add this test if it matches ALL of the positive tags and NONE of the negative tags in
        // the tags attribute.
        for tag in negative_tags {
            if test_tags.contains(tag) {
                return false;
            }
        }
        for tag in positive_tags {
            if !test_tags.contains(tag) {
                return false;
            }
        }
        true
    }

    /// Separates a list of text "tags" into a Pair of Collections, where the first element are
    /// the required or positive tags and the second element are the excluded or negative tags.
    /// This should work on tag list provided from the command line --test_tags_filters flag or on
    /// tag filters explicitly declared in the suite.
    ///
    /// Keep this function in sync with the version in
    /// java.com.google.devtools.build.lib.view.packages.TestTargetUtils.sortTagsBySense
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:330 TestsFunction.sortTagsBySense
    fn sort_tags_by_sense(
        tag_list: &[String],
        required_tags: &mut HashSet<String>,
        excluded_tags: &mut HashSet<String>,
    ) {
        for tag in tag_list {
            if let Some(rest) = tag.strip_prefix('-') {
                excluded_tags.insert(rest.to_owned());
            } else if let Some(rest) = tag.strip_prefix('+') {
                required_tags.insert(rest.to_owned());
            } else if tag == "manual" {
                // Ignore manual attribute because it is an exception: it is not a filter but a
                // property of test_suite
                continue;
            } else {
                required_tags.insert(tag.clone());
            }
        }
    }
}

impl QueryFunctionBase for TestsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:54 TestsFunction.getName
    fn get_name(&self) -> String {
        "tests".to_owned()
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:59 TestsFunction.getMandatoryArguments
    fn get_mandatory_arguments(&self) -> i32 {
        1
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:64 TestsFunction.getArgumentTypes
    fn get_argument_types(&self) -> Vec<ArgumentType> {
        vec![ArgumentType::Expression]
    }
}

impl<T: Clone + 'static> QueryFunction<T> for TestsFunction {
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:69 TestsFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        let closure = Closure::new(expression, callback, env);

        // A callback that appropriately feeds top-level test and test_suite targets to
        // 'closure'.
        let visit_all_test_suites_callback = callback_fn(|partial_result: &[T]| {
            let partition_result = closure.partition(partial_result);
            for test_suite in
                closure.get_unique_test_suites(&partition_result.test_suite_targets)?
            {
                closure.visit_unique_tests_in_unique_suite(test_suite);
            }
            callback.process(&closure.get_unique_tests(&partition_result.test_targets)?)
        });

        // Get a future that represents full evaluation of the argument expression.
        let test_suite_visitation_started_future = env.eval(
            args[0].get_expression(),
            context,
            &visit_all_test_suites_callback,
        );

        env.transform_async(
            // When this future is done, all top-level test_suite targets have already been fed to
            // the 'closure', meaning that ...
            test_suite_visitation_started_future,
            // ... 'closure.getTopLevelRecursiveVisitationFutures()' represents the full visitation
            // of all these test_suite targets.
            |_dummy_val| {
                let futures = closure.get_top_level_recursive_visitation_futures();
                env.when_all_succeed(&as_any_futures(&futures))
            },
        )
    }
}

// Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:105 TestsFunction.PartitionResult
struct PartitionResult<T> {
    test_targets: Vec<T>,
    test_suite_targets: Vec<T>,
    other_targets: Vec<T>,
}

/// A closure over the state needed to do asynchronous test_suite visitation and expansion.
// Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:117 TestsFunction.Closure
struct Closure<'a, T> {
    expression: &'a QueryExpression,
    callback: &'a dyn Callback<T>,
    /// The environment in which this query is being evaluated.
    env: &'a dyn QueryEnvironment<T>,

    accessor: &'a dyn TargetAccessor<T>,
    strict: bool,
    test_uniquifier: Box<dyn Uniquifier<T>>,
    test_suite_uniquifier: Box<dyn Uniquifier<T>>,
    top_level_recursive_visitation_futures: Mutex<Vec<QueryTaskFuture<()>>>,
}

impl<'a, T: Clone + 'static> Closure<'a, T> {
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:130 Closure.<init>
    fn new(
        expression: &'a QueryExpression,
        callback: &'a dyn Callback<T>,
        env: &'a dyn QueryEnvironment<T>,
    ) -> Self {
        Closure {
            expression,
            callback,
            env,
            accessor: env.get_accessor(),
            strict: env.is_setting_enabled(Setting::TestsExpressionStrict),
            test_uniquifier: env.create_uniquifier(),
            test_suite_uniquifier: env.create_uniquifier(),
            top_level_recursive_visitation_futures: Mutex::new(Vec::new()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:140 Closure.getUniqueTests
    fn get_unique_tests(&self, tests: &[T]) -> Result<Vec<T>, QueryException> {
        self.test_uniquifier.unique_all(tests)
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:144 Closure.getUniqueTestSuites
    fn get_unique_test_suites(&self, test_suites: &[T]) -> Result<Vec<T>, QueryException> {
        self.test_suite_uniquifier.unique_all(test_suites)
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:148 Closure.visitUniqueTestsInUniqueSuite
    fn visit_unique_tests_in_unique_suite(&self, test_suite: T) {
        let future = self
            .env
            .execute_async(|| self.recursively_visit_unique_tests_in_unique_suite(&test_suite));
        self.top_level_recursive_visitation_futures
            .lock()
            .unwrap()
            .push(future);
    }

    /// Returns all the futures representing the work items entailed by all the previous calls to
    /// `visit_unique_tests_in_unique_suite`.
    ///
    /// PORT: Java copies the list; it is read once, so this moves the futures out.
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:157 Closure.getTopLevelRecursiveVisitationFutures
    fn get_top_level_recursive_visitation_futures(&self) -> Vec<QueryTaskFuture<()>> {
        std::mem::take(&mut *self.top_level_recursive_visitation_futures.lock().unwrap())
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:161 Closure.recursivelyVisitUniqueTestsInUniqueSuite
    fn recursively_visit_unique_tests_in_unique_suite(
        &self,
        test_suite: &T,
    ) -> QueryTaskFuture<()> {
        let tags_attribute = self.accessor.get_string_list_attr(test_suite, "tags");
        // Split the tags list into positive and negative tags
        let mut required_tags = HashSet::new();
        let mut excluded_tags = HashSet::new();
        TestsFunction::sort_tags_by_sense(&tags_attribute, &mut required_tags, &mut excluded_tags);

        let mut tests_to_process: Vec<T> = Vec::new();
        let test_suites: Vec<T>;

        let body = (|| -> Result<Vec<T>, QueryError> {
            let partition_result = self.partition(&self.get_prerequisites(test_suite, "tests")?);

            for test_target in &partition_result.test_targets {
                if self.include_test(&required_tags, &excluded_tags, test_target)
                    && self.test_uniquifier.unique(test_target)?
                {
                    tests_to_process.push(test_target.clone());
                }
            }

            let test_suites = self
                .test_suite_uniquifier
                .unique_all(&partition_result.test_suite_targets)?;

            // If strict mode is enabled, then give an error for any non-test, non-test-suite
            // target.
            if self.strict {
                for other_target in &partition_result.other_targets {
                    let message = format!(
                        "The label '{}' in the test_suite '{}' does not refer to a test or \
                         test_suite rule!",
                        self.accessor.get_label(other_target),
                        self.accessor.get_label(test_suite)
                    );
                    self.env.handle_error(
                        self.expression,
                        &message,
                        Some(&DetailedExitCode::of(FailureDetail {
                            message: message.clone(),
                            category: Some(Category::Query(Query {
                                code: query::Code::InvalidLabelInTestSuite as i32,
                            })),
                        })),
                    )?;
                }
            }

            // Add implicit dependencies on tests in same package, if any.
            for target in self.get_prerequisites(test_suite, "$implicit_tests")? {
                // The Package construction of $implicit_tests ensures that this check never
                // fails, but we add it here anyway for compatibility with future code.
                if self.accessor.is_test_rule(&target)
                    && self.include_test(&required_tags, &excluded_tags, &target)
                    && self.test_uniquifier.unique(&target)?
                {
                    tests_to_process.push(target);
                }
            }
            Ok(test_suites)
        })();
        match body {
            Ok(suites) => test_suites = suites,
            Err(QueryError::Interrupted(_)) => return self.env.immediate_cancelled_future(),
            Err(QueryError::Query(e)) => return self.env.immediate_failed_future(e),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }

        // Process all tests, asynchronously.
        let all_tests_processed_future = self.env.execute(|| {
            self.callback.process(&tests_to_process)?;
            Ok(())
        });

        // Visit all suites recursively, asynchronously.
        let suite_futures: Vec<QueryTaskFuture<()>> = test_suites
            .iter()
            .map(|s| self.recursively_visit_unique_tests_in_unique_suite(s))
            .collect();
        let all_test_suits_visited_future =
            self.env.when_all_succeed(&as_any_futures(&suite_futures));

        self.env
            .when_all_succeed(&[&all_tests_processed_future, &all_test_suits_visited_future])
    }

    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:235 Closure.partition
    fn partition(&self, targets: &[T]) -> PartitionResult<T> {
        let mut test_targets_builder = Vec::new();
        let mut test_suite_targets_builder = Vec::new();
        let mut other_targets_builder = Vec::new();

        for target in targets {
            if self.accessor.is_test_rule(target) {
                test_targets_builder.push(target.clone());
            } else if self.accessor.is_test_suite(target) {
                test_suite_targets_builder.push(target.clone());
            } else {
                other_targets_builder.push(target.clone());
            }
        }

        PartitionResult {
            test_targets: test_targets_builder,
            test_suite_targets: test_suite_targets_builder,
            other_targets: other_targets_builder,
        }
    }

    /// Returns the set of rules named by the attribute 'attrName' of test_suite rule 'testSuite'.
    /// The attribute must be a list of labels. If a target cannot be resolved, then an error is
    /// reported to the environment (which may throw an exception if `keep_going` is disabled).
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:261 Closure.getPrerequisites
    fn get_prerequisites(&self, test_suite: &T, attr_name: &str) -> Result<Vec<T>, QueryError> {
        self.accessor.get_prerequisites(
            self.expression,
            test_suite,
            attr_name,
            &format!(
                "couldn't expand '{}' attribute of test_suite {}: ",
                attr_name,
                self.accessor.get_label(test_suite)
            ),
        )
    }

    /// Filters 'tests' (by mutation) according to the 'tags' attribute, specifically those that
    /// match ALL of the tags in tagsAttribute.
    // Java: com/google/devtools/build/lib/query2/engine/TestsFunction.java:281 Closure.includeTest
    fn include_test(
        &self,
        required_tags: &HashSet<String>,
        excluded_tags: &HashSet<String>,
        test: &T,
    ) -> bool {
        let mut test_tags = self.accessor.get_string_list_attr(test, "tags");
        // PORT: Java adds the (possibly null) size; a null element never equals a tag, so it is
        // left out.
        if let Some(size) = self.accessor.get_string_attr(test, "size") {
            test_tags.push(size);
        }
        TestsFunction::include_test_tags(&test_tags, required_tags, excluded_tags)
    }
}
