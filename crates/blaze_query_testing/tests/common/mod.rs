//! The shared part of the query test suites: Java's `AbstractQueryTest<Target>` (its helper
//! methods and fields), with the overrides of the concrete suites (`GraphlessQueryTest`,
//! `QueryTest`, `AbstractQueryKeepGoingTest`) selected by [`Suite`].
//!
//! Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java

#![allow(dead_code)]

pub mod abstract_query_tests;
pub mod action_graph_query_tests;
pub mod configured_target_query_semantics_tests;
pub mod configured_target_query_tests;
pub mod post_analysis_query_tests;
pub mod query_keep_going_tests;
pub mod query_tests;

use std::collections::BTreeSet;

use blaze_protos::failure_details::FailureDetail;
use blaze_query_engine::Setting;
use blaze_query_testing::{
    EnvironmentKind, EvaluateError, POST_ANALYSIS_DEFAULT_UNIVERSE, QueryHelper, ResultAndTargets,
    collect_target_patterns_with_default_functions,
};

/// The concrete Java test class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// `GraphlessQueryTest`: a `GraphlessBlazeQueryEnvironment`, `--nokeep_going`.
    Graphless,
    /// `QueryKeepGoingTest` (`AbstractQueryKeepGoingTest` over `QueryTest`): the standard
    /// factory, `--keep_going`.
    KeepGoing,
    /// `ConfiguredTargetQueryTest` (`PostAnalysisQueryTest` with `ConfiguredTargetQueryHelper`),
    /// `--nokeep_going`.
    Cquery,
    /// `ActionGraphQueryTest` (`PostAnalysisQueryTest` with `ActionGraphQueryHelper`),
    /// `--nokeep_going`.
    Aquery,
}

impl Suite {
    /// A `PostAnalysisQueryTest`.
    pub fn is_post_analysis(self) -> bool {
        matches!(self, Suite::Cquery | Suite::Aquery)
    }
}

// Java: src/test/java/com/google/devtools/build/lib/testutil/TestConstants.java:178 TestConstants.PLATFORM_LABEL
pub const PLATFORM_LABEL: &str = "@platforms//host";

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:93 AbstractQueryTest.DEFAULT_UNIVERSE
pub const DEFAULT_UNIVERSE: &str = "//...:*";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:293 AbstractQueryTest.AB_FILES
pub const AB_FILES: &str = "//a/b:BUILD //a/b:p //a/b:q";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:294 AbstractQueryTest.A_FILES
pub const A_FILES: &str = "//a:BUILD //a:x //a:y //a:z";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:295 AbstractQueryTest.A_AB_FILES
pub const A_AB_FILES: &str = "//a/b:BUILD //a/b:p //a/b:q //a:BUILD //a:x //a:y //a:z";
// Java: src/test/java/com/google/devtools/build/lib/testutil/TestConstants.java:142 TestConstants.GENRULE_SETUP
pub const GENRULE_SETUP: &str = "@bazel_tools//tools/genrule:genrule-setup.sh";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2662 AbstractQueryTest.REPO_A_RULES
pub const REPO_A_RULES: &str = "@@repo+//a:a_shar";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2663 AbstractQueryTest.REPO_AB_RULES
pub const REPO_AB_RULES: &str = "@@repo+//a/b:a_b_shar";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2664 AbstractQueryTest.REPO_AB_ALL
pub const REPO_AB_ALL: &str =
    "@@repo+//a/b:BUILD @@repo+//a/b:a_b_shar @@repo+//a/b:p @@repo+//a/b:q";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2666 AbstractQueryTest.REPO_A_ALL
pub const REPO_A_ALL: &str =
    "@@repo+//a:BUILD @@repo+//a:a_shar @@repo+//a:x @@repo+//a:y @@repo+//a:z";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2668 AbstractQueryTest.REPO_A_AB_RULES
pub const REPO_A_AB_RULES: &str = "@@repo+//a/b:a_b_shar @@repo+//a:a_shar";
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2669 AbstractQueryTest.REPO_A_AB_ALL
pub const REPO_A_AB_ALL: &str = "@@repo+//a/b:BUILD @@repo+//a/b:a_b_shar @@repo+//a/b:p @@repo+//a/b:q @@repo+//a:BUILD @@repo+//a:a_shar @@repo+//a:x @@repo+//a:y @@repo+//a:z";

/// The settings argument of the helpers (Java's `Setting...` varargs): one setting or a slice.
pub trait SettingList {
    fn into_settings(self) -> Vec<Setting>;
}
impl SettingList for Setting {
    fn into_settings(self) -> Vec<Setting> {
        vec![self]
    }
}
impl SettingList for &[Setting] {
    fn into_settings(self) -> Vec<Setting> {
        self.to_vec()
    }
}
impl<const N: usize> SettingList for &[Setting; N] {
    fn into_settings(self) -> Vec<Setting> {
        self.to_vec()
    }
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:250 AbstractQueryTest.assertContains
pub fn assert_contains(x: TargetSet, y: TargetSet) {
    assert!(
        x.is_superset(&y),
        "x is not a superset of y:\nx = {x:?}\ny = {y:?}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:256 AbstractQueryTest.assertNotContains
pub fn assert_not_contains(x: TargetSet, y: TargetSet) {
    assert!(
        !x.is_superset(&y),
        "x is a superset of y:\nx = {x:?}\ny = {y:?}"
    );
}
// Java: src/test/java/com/google/devtools/build/lib/testutil/TestConstants.java:166 TestConstants.CC_DEPENDENCY_CORRECTION
pub const CC_DEPENDENCY_CORRECTION: &str = " - deps(@bazel_tools//tools/cpp:current_cc_toolchain) - deps(@bazel_tools//tools/cpp:grep-includes)";

/// A result set as Java compares `Set<Target>`s: by element (a target is identified by its label
/// within one query helper).
pub type TargetSet = BTreeSet<String>;

pub fn empty_set() -> TargetSet {
    TargetSet::new()
}

/// A set of the given labels (Java compares result sets with `targetLabels` / label strings).
pub fn labels(labels: &[&str]) -> TargetSet {
    labels.iter().map(|l| (*l).to_owned()).collect()
}

/// `result.getResultSet()` as a [`TargetSet`].
pub fn labels_of(result: &ResultAndTargets) -> TargetSet {
    result.labels.iter().cloned().collect()
}

pub fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// `AbstractQueryTest.EvalThrowsResult`.
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:215 AbstractQueryTest.EvalThrowsResult
#[derive(Debug)]
pub struct EvalThrowsResult {
    pub message: String,
    pub failure_detail: FailureDetail,
}

/// Java `AbstractQueryTest<Target>`: the test's helper and the suite's overrides.
pub struct AbstractQueryTest {
    pub helper: QueryHelper,
    pub suite: Suite,
}

impl AbstractQueryTest {
    /// `initializeQueryHelper` (and the suite's `@Before`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:109 AbstractQueryTest.initializeQueryHelper
    pub fn new(suite: Suite) -> AbstractQueryTest {
        let kind = match suite {
            Suite::Graphless => EnvironmentKind::Graphless,
            Suite::KeepGoing => EnvironmentKind::Standard,
            Suite::Cquery => EnvironmentKind::Cquery,
            Suite::Aquery => EnvironmentKind::Aquery,
        };
        let mut helper = QueryHelper::new(kind);
        // helper.setUniverseScope(getDefaultUniverseScope())
        helper.set_universe_scope(if suite.is_post_analysis() {
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:93 PostAnalysisQueryTest.getDefaultUniverseScope
            POST_ANALYSIS_DEFAULT_UNIVERSE
        } else {
            DEFAULT_UNIVERSE
        });
        if suite.is_post_analysis() {
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:77 PostAnalysisQueryTest.disableOrderedResults
            helper.set_ordered_results(false);
        }
        // analysisMock.setupMockTestingRules(mockToolsConfig): written by the harness's mock
        // client (AnalysisMock.setupMockClient).
        if suite == Suite::KeepGoing {
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:36 AbstractQueryKeepGoingTest.setKeepGoing
            helper.set_keep_going(true);
        }
        AbstractQueryTest { helper, suite }
    }

    /// `testConfigurableAttributes()` (true for both suites).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:141 AbstractQueryTest.testConfigurableAttributes
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:150 PostAnalysisQueryTest.testConfigurableAttributes
    pub fn test_configurable_attributes(&self) -> bool {
        // ConfiguredTargetQuery knows the actual configuration, so it doesn't falsely
        // overapproximate.
        !self.suite.is_post_analysis()
    }

    /// `includeCppToolchainDependencies()` (false for both suites).
    // Java: src/test/java/com/google/devtools/build/lib/query2/engine/GraphlessQueryTest.java:35 GraphlessQueryTest.includeCppToolchainDependencies
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:61 QueryTest.includeCppToolchainDependencies
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:753 AbstractQueryTest.includeCppToolchainDependencies
    pub fn include_cpp_toolchain_dependencies(&self) -> bool {
        // PostAnalysisQueryTest keeps AbstractQueryTest's default.
        self.suite.is_post_analysis()
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:146 AbstractQueryTest.getDependencyCorrection
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:69 PostAnalysisQueryTest.getDependencyCorrection
    pub fn get_dependency_correction(&self) -> String {
        if self.suite.is_post_analysis() {
            // Also filter out platform dependencies.
            return format!(" - deps({PLATFORM_LABEL})");
        }
        String::new()
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:151 AbstractQueryTest.getDependencyCorrectionWithGen
    pub fn get_dependency_correction_with_gen(&self) -> String {
        format!(
            "{} - deps({})",
            self.get_dependency_correction(),
            GENRULE_SETUP
        )
    }

    /// `useExtendedSetOfRules()`: Java adds the native mock rules of `TestAspects` (`BASE_RULE`,
    /// `ASPECT_REQUIRING_RULE`, `EXTRA_ATTRIBUTE_ASPECT_REQUIRING_PROVIDER_RULE`, `HONEST_RULE`,
    /// `SIMPLE_RULE`) to the rule class provider.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1799 AbstractQueryTest.useExtendedSetOfRules
    pub fn use_extended_set_of_rules(&mut self) {
        panic!(
            "unported: TestAspects native mock rules (BASE_RULE, ASPECT_REQUIRING_RULE, \
             EXTRA_ATTRIBUTE_ASPECT_REQUIRING_PROVIDER_RULE, HONEST_RULE, SIMPLE_RULE) and \
             SkyframeQueryHelper.useRuleClassProvider"
        );
    }

    /// `useReducedSetOfRules()`: an empty workspace with only the empty repositories the mock
    /// rule class provider needs.
    ///
    /// PORT: `helper.useRuleClassProvider(analysisMock.createRuleClassProvider())` is the
    /// harness's rule class provider already (Blaze has no test-only rules in it), so only the
    /// files change.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1810 AbstractQueryTest.useReducedSetOfRules
    pub fn use_reduced_set_of_rules(&mut self) {
        let helper = &mut self.helper;
        helper.clear_all_files();
        helper.write_file("embedded_tools/BUILD", &[]);
        helper.write_file(
            "embedded_tools/MODULE.bazel",
            &["module(name='bazel_tools')"],
        );
        helper.write_file("embedded_tools/tools/build_defs/repo/BUILD", &[]);
        helper.write_file(
            "embedded_tools/tools/build_defs/repo/local.bzl",
            &["def _local_repository_impl(rctx):
  path = rctx.workspace_root.get_child(rctx.attr.path)
  rctx.symlink(path, \".\")
local_repository = repository_rule(
  implementation = _local_repository_impl,
  attrs = {\"path\": attr.string()},
)
"],
        );
        for (dir, name) in [
            ("platforms_workspace", "platforms"),
            ("local_config_xcode_workspace", "local_config_xcode"),
            ("rules_java_workspace", "rules_java"),
            ("rules_python_workspace", "rules_python"),
            ("rules_python_internal_workspace", "rules_python_internal"),
            ("bazel_skylib_workspace", "bazel_skylib"),
            ("third_party/protobuf", "com_google_protobuf"),
            ("proto_bazel_features_workspace", "proto_bazel_features"),
            ("bazel_features_workspace", "bazel_features"),
            ("build_bazel_apple_support", "build_bazel_apple_support"),
            ("third_party/bazel_rules/rules_cc", "rules_cc"),
            ("third_party/bazel_rules/rules_shell", "rules_shell"),
        ] {
            helper.write_file(&format!("{dir}/BUILD"), &[]);
            helper.write_file(
                &format!("{dir}/MODULE.bazel"),
                &[&format!("module(name='{name}')")],
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1904 AbstractQueryTest.simpleVisibilityTest
    pub fn simple_visibility_test(&mut self, visibility: &str, expect_visible: bool) {
        self.use_reduced_set_of_rules();
        self.write_file("MODULE.bazel", &[]);
        self.write_file("a/BUILD", &["filegroup(name = 'a', srcs = ['//b:b'])"]);
        self.write_file(
            "b/BUILD",
            &[&format!(
                "filegroup(name = 'b', srcs = ['b.txt'], visibility = ['{visibility}'])"
            )],
        );
        let actual = self.eval_to_string("visible(//a, somepath(//a, //b))");
        if expect_visible {
            assert_eq!(actual, "//a:a //b:b");
        } else {
            assert_eq!(actual, "//a:a");
        }
    }

    /// `writeBzlmodBuildFiles()`.
    ///
    /// PORT: `helper.setMainRepoTargetParser(mapping)` with `my_repo -> repo+`: the helper's
    /// main repository target parser takes the executor's main repository mapping, which has that
    /// entry from the `bazel_dep` of the MODULE.bazel written here.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2634 AbstractQueryTest.writeBzlmodBuildFiles
    pub fn write_bzlmod_build_files(&mut self) {
        self.helper.overwrite_file(
            "MODULE.bazel",
            &["bazel_dep(name= 'repo', version='1.0', repo_name='my_repo')"],
        );
        self.helper.overwrite_file(
            "BUILD",
            &[
                "load('//test_defs:foo_binary.bzl', 'foo_binary')",
                "foo_binary(",
                "name='rinne',",
                "srcs=['rinne.sh'],",
                "deps=['@my_repo//a:x','@my_repo//a/b:p']",
                ")",
            ],
        );
        self.helper.add_module(
            &blaze_sky_top::testing::create_module_key("repo", "1.0"),
            &["module(name = 'repo', version = '1.0')"],
        );
        let module_root = self.helper.get_module_root().clone();
        self.write_file(
            module_root
                .get_relative_str("repo+1.0/REPO.bazel")
                .get_path_string(),
            &[""],
        );
        self.write_file(
            module_root
                .get_relative_str("repo+1.0/a/BUILD")
                .get_path_string(),
            &[
                "exports_files(['x', 'y', 'z'])",
                "filegroup(name = 'a_shar')",
            ],
        );
        self.write_file(
            module_root
                .get_relative_str("repo+1.0/a/b/BUILD")
                .get_path_string(),
            &["exports_files(['p', 'q'])", "filegroup(name = 'a_b_shar')"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:155 AbstractQueryTest.writeFile
    pub fn write_file(&self, path_name: &str, lines: &[&str]) {
        self.helper.write_file(path_name, lines);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:159 AbstractQueryTest.overwriteFile
    pub fn overwrite_file(&self, path_name: &str, lines: &[&str]) {
        self.helper.overwrite_file(path_name, lines);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:168 AbstractQueryTest.assertContainsEvent
    pub fn assert_contains_event(&self, expected_message: impl AsRef<str>) {
        self.helper.assert_contains_event(expected_message.as_ref());
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:172 AbstractQueryTest.assertDoesNotContainEvent
    pub fn assert_does_not_contain_event(&self, not_expected_message: impl AsRef<str>) {
        self.helper
            .assert_does_not_contain_event(not_expected_message.as_ref());
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:180 AbstractQueryTest.assertStartsWith
    pub fn assert_starts_with(&self, expected: impl AsRef<str>, actual: impl AsRef<str>) {
        let (expected, actual) = (expected.as_ref(), actual.as_ref());
        if !actual.starts_with(expected) {
            assert_eq!(actual, expected);
        }
    }

    fn events_string(&self) -> String {
        let events: Vec<String> = self
            .helper
            .get_events()
            .iter()
            .map(|e| e.to_string())
            .collect();
        format!("[{}]", events.join(", "))
    }

    /// `helper.evaluateQuery(query)` with a `QueryException` turned into a panic (Java's test
    /// method throws it).
    pub fn evaluate_query(&mut self, query: &str) -> ResultAndTargets {
        match self.helper.evaluate_query(query) {
            Ok(r) => r,
            Err(e) => panic!("evaluateQuery threw for {query}: {e:?}"),
        }
    }

    /// Evaluate the query, assert that it is successful, and return its results.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:188 AbstractQueryTest.eval
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:106 PostAnalysisQueryTest.eval
    ///
    /// PORT: the result is the set of the labels (`helper.getLabel`) of the result; two cquery
    /// nodes with one label in two configurations are one element here (Java's `Set<CqueryNode>`
    /// holds both). [`Self::eval_result`] keeps them.
    pub fn eval(&mut self, query: &str) -> TargetSet {
        labels_of(&self.eval_result(query))
    }

    /// `eval(query)` with the whole result (the labels in the result set's order, duplicates
    /// kept, and the nodes of a post-analysis query).
    pub fn eval_result(&mut self, query: &str) -> ResultAndTargets {
        self.maybe_parse_universe_scope(query);
        let result = self.evaluate_query(query);
        assert!(
            result.query_eval_result.get_success(),
            "evaluateQuery failed: {query}\n{}",
            self.events_string()
        );
        self.maybe_reset_universe_scope();
        result
    }

    /// At the end of each eval, reset the universe scope to the default if the test doesn't use
    /// a single universe scope (`PostAnalysisQueryTest.eval` and `evalThrows`).
    fn maybe_reset_universe_scope(&mut self) {
        if self.suite.is_post_analysis() && !self.helper.is_whole_test_universe() {
            self.helper
                .set_universe_scope(POST_ANALYSIS_DEFAULT_UNIVERSE);
        }
    }

    /// Parse the universe if the universe has not been set manually through the helper.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:128 PostAnalysisQueryTest.maybeParseUniverseScope
    pub fn maybe_parse_universe_scope(&mut self, query: &str) {
        if !self.suite.is_post_analysis()
            || self.helper.get_universe_scope_as_string_list() != [POST_ANALYSIS_DEFAULT_UNIVERSE]
        {
            return;
        }
        let target_pattern_set =
            collect_target_patterns_with_default_functions(self.helper.kind(), query);
        if !target_pattern_set.is_empty() {
            let mut universe_scope = String::new();
            for target in target_pattern_set {
                universe_scope.push_str(&target);
                universe_scope.push(',');
            }
            self.helper.set_universe_scope(&universe_scope);
        }
    }

    /// `evalThrows`: the suite's version (see [`Suite`]).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:198 AbstractQueryTest.evalThrows
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:70 AbstractQueryKeepGoingTest.evalThrows
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:116 PostAnalysisQueryTest.evalThrows
    pub fn eval_throws(&mut self, query: &str, unconditionally_throws: bool) -> EvalThrowsResult {
        self.maybe_parse_universe_scope(query);
        let result = self.eval_throws_internal(query, unconditionally_throws);
        self.maybe_reset_universe_scope();
        result
    }

    fn eval_throws_internal(
        &mut self,
        query: &str,
        unconditionally_throws: bool,
    ) -> EvalThrowsResult {
        match self.suite {
            Suite::Graphless | Suite::Cquery | Suite::Aquery => {
                match self.helper.evaluate_query(query) {
                    Ok(_) => panic!("evaluateQuery completed normally: {query}"),
                    Err(EvaluateError::Query {
                        message,
                        failure_detail,
                    }) => EvalThrowsResult {
                        message,
                        failure_detail,
                    },
                    Err(EvaluateError::Interrupted) => panic!("InterruptedException"),
                    // Java: the exception of AnalysisTestCase.update propagates out of the test.
                    Err(EvaluateError::Update(e)) => panic!("{e:?}"),
                }
            }
            Suite::KeepGoing => {
                // This method can be called in both keep_going and nokeep_going modes: expect
                // either an exception or an error message.
                match self.helper.evaluate_query(query) {
                    Ok(result) => {
                        // evalFail
                        assert!(
                            !result.query_eval_result.get_success(),
                            "evaluateQuery succeeded: {query}"
                        );
                        assert!(self.helper.is_keep_going());
                        let msg = self.helper.get_first_event();
                        let msg = strip_prefix_re(&msg, "Skipping '", "': ");
                        let msg = strip_evaluation_failed(&msg);
                        EvalThrowsResult {
                            message: msg,
                            failure_detail: result
                                .query_eval_result
                                .get_detailed_exit_code()
                                .get_failure_detail()
                                .cloned()
                                .unwrap_or_default(),
                        }
                    }
                    Err(EvaluateError::Query {
                        message,
                        failure_detail,
                    }) => {
                        if !unconditionally_throws {
                            assert!(!self.helper.is_keep_going());
                        }
                        EvalThrowsResult {
                            message,
                            failure_detail,
                        }
                    }
                    Err(EvaluateError::Interrupted) => panic!("InterruptedException"),
                    Err(EvaluateError::Update(e)) => panic!("{e:?}"),
                }
            }
        }
    }

    /// `assertThrows(TargetParsingException.class, () -> <eval of query>)` in a post-analysis
    /// suite: the analysis of the universe parsed from the query throws a
    /// `TargetParsingException`; returns its message and failure detail.
    pub fn eval_throws_target_parsing_exception(&mut self, query: &str) -> EvalThrowsResult {
        // The exception leaves eval/evalThrows before the universe scope is reset.
        self.maybe_parse_universe_scope(query);
        match self.helper.evaluate_query(query) {
            Err(EvaluateError::Update(blaze_sky_top::testing::UpdateError::TargetParsing(e))) => {
                EvalThrowsResult {
                    message: e.get_message().to_owned(),
                    failure_detail: e
                        .get_detailed_exit_code()
                        .get_failure_detail()
                        .cloned()
                        .unwrap_or_default(),
                }
            }
            Ok(_) => panic!("expected TargetParsingException, evaluation completed: {query}"),
            Err(e) => panic!("expected TargetParsingException, got {e:?}"),
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:377 PostAnalysisQueryTest.writeSimpleToolchain
    pub fn write_simple_toolchain(&self) {
        self.write_file(
            "test/toolchain_def.bzl",
            &["def _impl(ctx):
    return [platform_common.ToolchainInfo()]

test_toolchain = rule(
    implementation = _impl,
)
"],
        );
        self.write_file(
            "test/BUILD",
            &["load(\"//test:toolchain_def.bzl\", \"test_toolchain\")

toolchain_type(name = \"toolchain_type\")

toolchain(
    name = \"toolchain\",
    toolchain = \":toolchain_impl\",
    toolchain_type = \"//test:toolchain_type\",
)

test_toolchain(name = \"toolchain_impl\")
"],
        );
    }

    /// `assertThrows(QueryException.class, () -> eval(query))`: the message and failure detail of
    /// the `QueryException` (the universe scope is not reset, as `eval` throws before it).
    pub fn eval_query_exception(&mut self, query: &str) -> EvalThrowsResult {
        self.maybe_parse_universe_scope(query);
        match self.helper.evaluate_query(query) {
            Err(EvaluateError::Query {
                message,
                failure_detail,
            }) => EvalThrowsResult {
                message,
                failure_detail,
            },
            Ok(_) => panic!("expected QueryException, evaluation completed: {query}"),
            Err(e) => panic!("expected QueryException, got {e:?}"),
        }
    }

    /// `getConfiguration(node)` of the `index`-th node of a post-analysis result
    /// (`getHelper().getSkyframeExecutor().getConfiguration(getReporter(),
    /// node.getConfigurationKey())`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryTest.java:52 ConfiguredTargetQueryTest.getConfiguration
    pub fn get_configuration(
        &mut self,
        result: &ResultAndTargets,
        index: usize,
    ) -> Option<
        std::sync::Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
    > {
        use blaze_cquery::post_analysis_query_environment::PostAnalysisNode;
        let key = match result.nodes.as_ref().expect("a post-analysis result") {
            blaze_query_testing::PostAnalysisNodes::Cquery(nodes) => {
                nodes[index].get_configuration_key()
            }
            blaze_query_testing::PostAnalysisNodes::Aquery(nodes) => {
                nodes[index].get_configuration_key()
            }
        };
        self.helper.get_configuration(key.as_ref())
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQuerySemanticsTest.java:345 ConfiguredTargetQuerySemanticsTest.createConfigTransitioningRuleClass
    pub fn create_config_transitioning_rule_class(&self) {
        self.overwrite_file(
            "tools/allowlists/function_transition_allowlist/BUILD",
            &["package_group(
    name = \"function_transition_allowlist\",
    packages = [
        \"//test/...\",
    ],
)
"],
        );
        self.write_file(
            "test/rules.bzl",
            &["def _rule_impl(ctx):
    return []

string_flag = rule(
    implementation = _rule_impl,
    build_setting = config.string(),
)

def _transition_impl(settings, attr):
    return {\"//test:my_flag\": \"custom string\"}

my_transition = transition(
    implementation = _transition_impl,
    inputs = [],
    outputs = [\"//test:my_flag\"],
)
rule_with_deps_transition = rule(
    implementation = _rule_impl,
    attrs = {
        \"deps\": attr.label_list(cfg = my_transition),
    },
)
simple_rule = rule(
    implementation = _rule_impl,
    attrs = {},
)
"],
        );
    }

    /// `eval(query).stream().map(cf -> cf.getDescription(LabelPrinter.legacy()))` of a cquery
    /// result, in the result set's order.
    pub fn eval_descriptions(&mut self, query: &str) -> Vec<String> {
        let result = self.eval_result(query);
        match result.nodes.as_ref().expect("a post-analysis result") {
            blaze_query_testing::PostAnalysisNodes::Cquery(nodes) => nodes
                .iter()
                .map(|n| n.get_description(&blaze_packages::target::LabelPrinter::legacy()))
                .collect(),
            blaze_query_testing::PostAnalysisNodes::Aquery(_) => {
                panic!("getDescription is a CqueryNode method")
            }
        }
    }

    /// `helper.getToolsRepository()`: `RepositoryName.BAZEL_TOOLS` (`toString()`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:247 SkyframeQueryHelper.getToolsRepository
    pub fn get_tools_repository(&self) -> String {
        "@bazel_tools".to_owned()
    }

    /// Like eval(), but asserts that evaluation completes normally, with an error.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:42 AbstractQueryKeepGoingTest.evalFail
    pub fn eval_fail(&mut self, query: &str) -> ResultAndTargets {
        let result = self.evaluate_query(query);
        assert!(
            !result.query_eval_result.get_success(),
            "evaluateQuery succeeded: {query}"
        );
        result
    }

    /// Like eval(), but makes no assertions about whether evaluation completes with an error.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:66 AbstractQueryKeepGoingTest.evalMaybe
    pub fn eval_maybe(&mut self, query: &str) -> TargetSet {
        labels_of(&self.evaluate_query(query))
    }

    /// Returns the set as a space-separated list of labels in lex order.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:235 AbstractQueryTest.evalToString
    pub fn eval_to_string(&mut self, query: &str) -> String {
        self.eval_to_list_of_strings(query).join(" ")
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:239 AbstractQueryTest.evalToListOfStrings
    pub fn eval_to_list_of_strings(&mut self, query: &str) -> Vec<String> {
        // resultSetToListOfStrings: the labels of the set, sorted (Ordering.natural()).
        sorted(self.eval_result(query).labels)
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:250 AbstractQueryTest.assertContains
    pub fn assert_contains(&self, x: TargetSet, y: TargetSet) {
        assert!(
            x.is_superset(&y),
            "x is not a superset of y:\nx = {x:?}\ny = {y:?}"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:256 AbstractQueryTest.assertNotContains
    pub fn assert_not_contains(&self, x: TargetSet, y: TargetSet) {
        assert!(!x.is_superset(&y));
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:286 AbstractQueryTest.writeBuildFiles1
    pub fn write_build_files1(&self) {
        // Note, these BUILD files contain no rules, only files, so we use the
        // "a/...:*" wildcard to match them.
        self.write_file("a/BUILD", &["exports_files(['x', 'y', 'z'])"]);
        self.write_file("a/b/BUILD", &["exports_files(['p', 'q'])"]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:312 AbstractQueryTest.checkResultofBadTargetLiterals
    pub fn check_resultof_bad_target_literals(
        &self,
        message: String,
        failure_detail: FailureDetail,
    ) {
        assert_eq!(
            target_patterns_code(&failure_detail),
            Some(blaze_protos::failure_details::target_patterns::Code::LabelSyntaxError as i32)
        );
        assert_eq!(
            message,
            "invalid target name '*:*': target names may not contain ':'"
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:352 AbstractQueryTest.writeBuildFiles2
    pub fn write_build_files2(&self) {
        self.write_file(
            "c/BUILD",
            &["load(\"@rules_cc//cc:cc_binary.bzl\", \"cc_binary\")
load(\"@rules_cc//cc:cc_test.bzl\", \"cc_test\")

genrule(name='c', srcs=['p', 'q'], outs=['r', 's'], cmd=':')
cc_binary(name='d', srcs=['e.cc'], data=['r'])
cc_test(name='f', srcs=['g.cc'])
"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:465 AbstractQueryTest.runGenqueryScopeTest
    pub fn run_genquery_scope_test(&mut self, is_post_analysis_query: bool) {
        self.write_file(
            "a/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name='a')",
            ],
        );
        self.write_file(
            "b/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name='b', deps=['//a:a'])",
            ],
        );
        self.write_file(
            "q/BUILD",
            &["genquery(name='q', scope=['//b'], expression='deps(//b)')"],
        );
        // Assure that deps of a genquery rule includes the transitive closure of its scope.
        // This is required for correctness of incremental "blaze build genqueryrule"
        let eval_result = self.eval_to_list_of_strings("deps(//q:q)");
        if is_post_analysis_query {
            assert!(eval_result.contains(&"//q:q".to_owned()));
            assert!(!eval_result.contains(&"//a:a".to_owned()));
            assert!(!eval_result.contains(&"//b:b".to_owned()));
        } else {
            assert_eq!(
                sorted(eval_result),
                sorted(vec!["//q:q".into(), "//a:a".into(), "//b:b".into()])
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:541 AbstractQueryTest.writeBuildFiles3
    pub fn write_build_files3(&self) {
        self.write_file(
            "a/BUILD",
            &[
                "genrule(name='a', srcs=['//b', '//c'], outs=['out'], cmd=':')
exports_files(['a2'])
",
            ],
        );
        self.write_file(
            "b/BUILD",
            &["genrule(name='b', srcs=['//d'], outs=['out'], cmd=':')"],
        );
        self.write_file(
            "c/BUILD",
            &["genrule(name='c', srcs=['//d'], outs=['out'], cmd=':')"],
        );
        self.write_file("d/BUILD", &["exports_files(['d'])"]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:556 AbstractQueryTest.writeBzlAndSclFiles
    pub fn write_bzl_and_scl_files(&self) {
        self.write_file(
            "foo/BUILD",
            &["load('//bar:direct.scl', 'x')
load('//bar:intermediate.bzl', 'y')
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(
    name = 'foo',
    tags = [x, y],
)
"],
        );
        self.write_file("bar/BUILD", &[]);
        self.write_file("bar/direct.scl", &["x = 'X'"]);
        self.write_file(
            "bar/intermediate.bzl",
            &["load(':indirect.scl', _y='y')
y = _y
"],
        );
        self.write_file("bar/indirect.scl", &["y = 'Y'"]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:583 AbstractQueryTest.writeBuildFilesWithConfigurableAttributesUnconditionally
    pub fn write_build_files_with_configurable_attributes_unconditionally(&self) {
        self.write_file(
            "conditions/BUILD",
            &["config_setting(
    name = 'a',
    values = {'foo': 'a'})
config_setting(
    name = 'b',
    values = {'foo': 'b'})
"],
        );
        self.write_file(
            "configurable/BUILD",
            &[
                "load('@rules_cc//cc:cc_binary.bzl', 'cc_binary')",
                "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
                "",
                "cc_binary(",
                "    name = 'main',",
                "    srcs = ['main.cc'],",
                "    deps = select({",
                "        '//conditions:a': [':adep'],",
                "        '//conditions:b': [':bdep'],",
                // BuildType.Selector.DEFAULT_CONDITION_KEY
                "        '//conditions:default': [':defaultdep'],",
                "    }))",
                "cc_library(",
                "    name = 'adep',",
                "    srcs = ['adep.cc'])",
                "cc_library(",
                "    name = 'bdep',",
                "    srcs = ['bdep.cc'])",
                "cc_library(",
                "    name = 'defaultdep',",
                "    srcs = ['defaultdep.cc'])",
            ],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:618 AbstractQueryTest.writeBuildFilesWithConfigurableAttributes
    pub fn write_build_files_with_configurable_attributes(&self) {
        if self.test_configurable_attributes() {
            self.write_build_files_with_configurable_attributes_unconditionally();
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:784 AbstractQueryTest.writeAspectDefinition
    pub fn write_aspect_definition(&mut self, aspect_attrs: &str) {
        self.helper.set_query_settings(&[Setting::IncludeAspects]);
        self.write_file(
            "test/aspect.bzl",
            &[
                "def _aspect_impl(target, ctx):",
                "   return []",
                "def _rule_impl(ctx):",
                "   return []",
                "",
                "MyAspect = aspect(",
                "   implementation=_aspect_impl,",
                "   attr_aspects=['deps'],",
                "   attrs = ",
                aspect_attrs,
                ")",
                "aspect_rule = rule(",
                "   implementation=_rule_impl,",
                "   attrs = { 'attr' : ",
                "             attr.label_list(mandatory=True, allow_files=True, aspects = [MyAspect]),",
                "             'param' : attr.string(),",
                "           },",
                ")",
                "plain_rule = rule(",
                "   implementation=_rule_impl,",
                "   attrs = { 'attr' : ",
                "             attr.label_list(mandatory=False, allow_files=True) ",
                "           },",
                ")",
            ],
        );
        self.write_file(
            "prod/BUILD",
            &["load('//test:aspect.bzl', 'plain_rule')
plain_rule(
     name = 'zzz'
)
"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1044 AbstractQueryTest.setupCycleInStarlarkParentDir
    pub fn setup_cycle_in_starlark_parent_dir(&self) {
        self.write_file(
            "a/BUILD",
            &["load('//a:cycle1.bzl', 'C1')
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 'a')
"],
        );
        self.write_file(
            "a/cycle1.bzl",
            &["load('//a:cycle2.bzl', 'C2')
C1 = struct()
"],
        );
        self.write_file(
            "a/cycle2.bzl",
            &["load('//a:cycle1.bzl', 'C1')
C2 = struct()
"],
        );
        self.write_file(
            "a/subdir/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name = 'subdir')",
            ],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1167 AbstractQueryTest.assertEqualsFiltered
    pub fn assert_equals_filtered(
        &mut self,
        expected: impl AsRef<str>,
        actual: impl AsRef<str>,
        settings: impl SettingList,
    ) {
        let (expected, actual) = (expected.as_ref(), actual.as_ref());
        let settings: &[Setting] = &settings.into_settings();
        self.helper.set_query_settings(settings);
        let actual = self.eval(actual);
        let expected = self.eval(expected);
        assert_eq!(actual, expected);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1173 AbstractQueryTest.runNodepDepsTest
    pub fn run_nodep_deps_test(&mut self, expect_visibility_dep: bool, settings: &[Setting]) {
        self.write_file(
            "foo/BUILD",
            &["load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 't1', deps = [':t2'], visibility = [':pg'])
foo_library(name = 't2')
package_group(name = 'pg')
"],
        );
        self.helper.set_query_settings(settings);
        let pg = only_element(self.eval("//foo:pg"));
        let deps = self.eval("deps(//foo:t1)");
        if expect_visibility_dep {
            assert!(deps.contains(&pg), "{deps:?}");
        } else {
            assert!(!deps.contains(&pg), "{deps:?}");
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1207 AbstractQueryTest.runCycleInStarlarkTest
    pub fn run_cycle_in_starlark_test(&mut self, check_failure_detail: bool) {
        self.write_file(
            "a/BUILD",
            &["load('//a:cycle1.bzl', 'C1')
load('//test_defs:foo_library.bzl', 'foo_library')
foo_library(name = 'a')
"],
        );
        self.write_file(
            "a/cycle1.bzl",
            &["load('//a:cycle2.bzl', 'C2')
C1 = struct()
"],
        );
        self.write_file(
            "a/cycle2.bzl",
            &["load('//a:cycle1.bzl', 'C1')
C2 = struct()
"],
        );
        let result = self.eval_throws("//a:all", false);
        if check_failure_detail {
            assert_eq!(
                target_patterns_code(&result.failure_detail),
                Some(blaze_protos::failure_details::target_patterns::Code::Cycle as i32)
            );
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:1274 AbstractQueryTest.writeBuildFilesWithImplicitAttribute
    pub fn write_build_files_with_implicit_attribute(&self) {
        self.write_file(
            "k/defs.bzl",
            &["def impl(ctx):
  return [DefaultInfo()]
has_implicit_attr = rule(
    implementation=impl,
    attrs = {
        'srcs': attr.label_list(),
        '_implicit': attr.label(default='//k:implicit')
    },
)
"],
        );
        self.write_file(
            "k/BUILD",
            &["load(':defs.bzl', 'has_implicit_attr')
has_implicit_attr(name='k', srcs=['k.txt'])
filegroup(name='implicit')
"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2161 AbstractQueryTest.runBadRuleInDeps
    pub fn run_bad_rule_in_deps(&mut self, code: ExpectedCode) {
        self.write_file(
            "foo/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name = 'foo', deps = ['//bar:bar'])",
            ],
        );
        self.write_file(
            "bar/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name = 'bar', srcs = 'bad_single_file')",
            ],
        );
        let result = self.eval_throws("deps(//foo:foo)", false);
        // comparingExpectedFieldsOnly: only the code set on the expected detail is compared.
        match code {
            ExpectedCode::PackageLoading(code) => {
                assert_eq!(
                    package_loading_code(&result.failure_detail),
                    Some(code as i32)
                )
            }
            ExpectedCode::Query(code) => {
                assert_eq!(query_code(&result.failure_detail), Some(code as i32))
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2623 AbstractQueryTest.assertDependsNotFiltered
    pub fn assert_depends_not_filtered(&mut self, from: &str, to: &str, settings: &[Setting]) {
        let from_deps = format!("deps({from})");
        self.assert_equals_filtered(&from_deps, &format!("{from_deps}-{to}"), settings);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2629 AbstractQueryTest.assertDependsFiltered
    pub fn assert_depends_filtered(&mut self, from: &str, to: &str, settings: &[Setting]) {
        let from_deps = format!("deps({from})");
        self.assert_equals_filtered(to, &format!("{from_deps}^{to}"), settings);
    }
}

/// The code `runBadRuleInDeps` expects (Java passes a `PackageLoading.Code` or a `Query.Code`).
#[derive(Clone, Copy, Debug)]
pub enum ExpectedCode {
    PackageLoading(blaze_protos::failure_details::package_loading::Code),
    Query(blaze_protos::failure_details::query::Code),
}

/// Runs a test body on a thread with a 64 MiB stack.
///
/// PORT: `testDeepNestedLet` nests 500 `let` expressions, which the query engine evaluates
/// recursively. The release binary evaluates it on its command thread (oracle and Blaze agree,
/// both modes), but the frames of a debug build overflow the 2 MiB stack of a libtest thread, so
/// that test runs on a thread sized like a command thread.
pub fn run_with_large_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(f)
        .expect("spawn the test thread")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

/// The edges of a graph as `from -> to` lines in node order, and its nodes without edges
/// (the failure message of `testGraphOrderOfWildcards`; Java prints `visitNodesBeforeEdges`).
pub fn graph_edges(
    graph: &blaze_query::query::blaze_query_environment::TargetDigraph,
) -> Vec<String> {
    let mut out = Vec::new();
    for node in graph.get_nodes() {
        let from = graph.get_label(node).0.get_label().to_string();
        let succ = graph.get_successors(node);
        if succ.is_empty() && graph.get_predecessors(node).is_empty() {
            out.push(from.clone());
        }
        for &to in succ {
            out.push(format!("{from} -> {}", graph.get_label(to).0.get_label()));
        }
    }
    out
}

/// `Iterables.getOnlyElement`.
pub fn only_element(set: TargetSet) -> String {
    assert_eq!(set.len(), 1, "{set:?}");
    set.into_iter().next().expect("one element")
}

/// `failureDetail.getTargetPatterns().getCode()` (`None` when the detail has another category).
pub fn target_patterns_code(failure_detail: &FailureDetail) -> Option<i32> {
    use blaze_protos::failure_details::failure_detail::Category;
    match &failure_detail.category {
        Some(Category::TargetPatterns(t)) => Some(t.code),
        other => {
            // Shown with the failure of the assertion that compares the code.
            println!("failure detail of another category: {other:?} ({failure_detail:?})");
            None
        }
    }
}

/// `failureDetail.getPackageLoading().getCode()`.
pub fn package_loading_code(failure_detail: &FailureDetail) -> Option<i32> {
    use blaze_protos::failure_details::failure_detail::Category;
    match &failure_detail.category {
        Some(Category::PackageLoading(p)) => Some(p.code),
        other => {
            // Shown with the failure of the assertion that compares the code.
            println!("failure detail of another category: {other:?} ({failure_detail:?})");
            None
        }
    }
}

/// `failureDetail.getQuery().getCode()`.
pub fn query_code(failure_detail: &FailureDetail) -> Option<i32> {
    use blaze_protos::failure_details::failure_detail::Category;
    match &failure_detail.category {
        Some(Category::Query(q)) => Some(q.code),
        other => {
            // Shown with the failure of the assertion that compares the code.
            println!("failure detail of another category: {other:?} ({failure_detail:?})");
            None
        }
    }
}

/// `s.replaceAll("^<prefix>[^']+<suffix>", "")` for `^Skipping '[^']+': `.
fn strip_prefix_re(s: &str, prefix: &str, suffix: &str) -> String {
    // `^<prefix>[^']+<suffix>` with the suffix starting at the closing quote.
    if let Some(rest) = s.strip_prefix(prefix)
        && let Some(end) = rest.find('\'')
        && end > 0
        && rest[end..].starts_with(suffix)
    {
        return rest[end + suffix.len()..].to_owned();
    }
    s.to_owned()
}

/// `s.replaceAll("Evaluation of query \"[^\"]+\" failed: ", "")`.
fn strip_evaluation_failed(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    const START: &str = "Evaluation of query \"";
    while let Some(i) = rest.find(START) {
        let after = &rest[i + START.len()..];
        match after.find('"') {
            Some(q) if q > 0 && after[q..].starts_with("\" failed: ") => {
                out.push_str(&rest[..i]);
                rest = &after[q + "\" failed: ".len()..];
            }
            _ => {
                out.push_str(&rest[..i + START.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

impl AbstractQueryTest {
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:470 QueryTest.writeStarlarkDefinedRuleClassBzlFile
    pub fn write_starlark_defined_rule_class_bzl_file(&self) {
        self.write_file(
            "test//starlark/extension.bzl",
            &["def custom_rule_impl(ctx):
    ftb = depset(ctx.attr._secret_labels)
    return DefaultInfo(runfiles = ctx.runfiles(), files = ftb)

def secret_labels_func(prefix, suffix):
    return [
        Label(\"//test/starlark:\" + prefix + \"01\" + suffix),
        Label(\"//test/starlark:\" + prefix + \"02\" + suffix),
    ]

custom_rule = rule(
    implementation = custom_rule_impl,
    attrs = {
        \"prefix\": attr.string(default = \"default_prefix\"),
        \"suffix\": attr.string(default = \"default_suffix\"),
        \"_secret_labels\": attr.label_list(default = secret_labels_func),
    },
)
"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/QueryTest.java:1004 QueryTest.runTestRdepsWithNonDefaultDependencyFilter
    pub fn run_test_rdeps_with_non_default_dependency_filter(
        &mut self,
        query: &str,
        expected: &str,
    ) {
        self.write_file(
            "foo/BUILD",
            &["load(\"//test_defs:foo_binary.bzl\", \"foo_binary\")
genrule(
    name = \"gen\",
    srcs = [\"doesntmatter.txt\"],
    outs = [\"out.txt\"],
    cmd = \"blah\",
    tools = [\":a\"],
)

foo_binary(
    name = \"a\",
)

foo_binary(
    name = \"b\",
    srcs = [\":a\"],
)

foo_binary(
    name = \"c\",
    srcs = [\":out.txt\"],
)
"],
        );
        self.helper.set_query_settings(&[Setting::OnlyTargetDeps]);
        assert_eq!(self.eval_to_string(query), expected);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:268 AbstractQueryKeepGoingTest.assertNoFailFast
    pub fn assert_no_fail_fast(
        &mut self,
        error_msg: &str,
        check_failure_detail: bool,
        keep_going_error_msg: &str,
    ) {
        self.write_file(
            "missingdep/BUILD",
            &["cc_library(
    name = \"missingdep\",
    deps = [\"//i/do/not/exist\"],
)
"],
        );
        let build_file_missing =
            Some(blaze_protos::failure_details::package_loading::Code::BuildFileMissing as i32);
        self.helper.set_keep_going(false);
        let throws_result1 = self.eval_throws("deps(//missingdep)", false);
        assert!(
            throws_result1.message.contains(error_msg),
            "{}",
            throws_result1.message
        );
        if check_failure_detail {
            assert_eq!(
                package_loading_code(&throws_result1.failure_detail),
                build_file_missing
            );
        }

        // (1) --keep_going.
        self.helper.clear_events();
        self.helper.set_keep_going(true);
        // partial results
        let fail_result = self.eval_fail(&format!("deps(//missingdep){CC_DEPENDENCY_CORRECTION}"));
        let missingdep = self.eval("//missingdep");
        assert_eq!(labels_of(&fail_result), missingdep);
        self.assert_contains_event(&format!(
            "Evaluation of query \"deps(//missingdep)\" failed: {keep_going_error_msg}"
        ));
        if check_failure_detail {
            assert_eq!(
                package_loading_code(
                    &fail_result
                        .query_eval_result
                        .get_detailed_exit_code()
                        .get_failure_detail()
                        .cloned()
                        .expect("a failure detail")
                ),
                build_file_missing
            );
        }

        // (2) --nokeep_going.
        self.helper.set_keep_going(false);
        let throws_result2 = self.eval_throws("deps(//missingdep)", false);
        assert!(
            throws_result2.message.contains(error_msg),
            "{}",
            throws_result2.message
        ); // no results
        if check_failure_detail {
            assert_eq!(
                package_loading_code(&throws_result2.failure_detail),
                build_file_missing
            );
        }
    }

    fn write_banana_packages(&self) {
        // The package "//foo" can be loaded and has no errors.
        self.write_file(
            "foo/BUILD",
            &[
                "load('//test_defs:foo_library.bzl', 'foo_library')",
                "foo_library(name='apple', srcs=['apple.sh'])",
            ],
        );
        // The package "//foo/foo" has a load statement that fails. Its ":banana" target does not
        // depend on the load, but because the package failed to load, it does not exist.
        self.write_file(
            "foo/foo/BUILD",
            &["load(\"//bar:lib.bzl\", \"myfunc\")
load('//test_defs:foo_library.bzl', 'foo_library')

foo_library(
    name = \"banana\",
    srcs = [\"banana.sh\"],
)
"],
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:350 AbstractQueryKeepGoingTest.runTestErrorReportedWhenStarlarkLoadRefersToMissingPkgExistingFile
    pub fn run_test_error_reported_when_starlark_load_refers_to_missing_pkg_existing_file(
        &mut self,
        query_expression: &str,
        num_expected_targets: usize,
    ) {
        if self.helper.reports_universe_evaluation_errors() {
            return;
        }
        self.write_banana_packages();
        // This Starlark file is fine, but it has no containing package, so it can't be loaded.
        self.write_file("bar/lib.bzl", &["custom_rule(name = 'myfunc')"]);
        let result = self.eval_fail(query_expression);
        assert_eq!(
            labels_of(&result).len(),
            num_expected_targets,
            "{:?}",
            labels_of(&result)
        );
        self.assert_contains_event(
            "error loading package 'foo/foo': Every .bzl file must have a corresponding package",
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:420 AbstractQueryKeepGoingTest.runTestErrorReportedWhenStarlarkLoadRefersToExistingPkgMissingFile
    pub fn run_test_error_reported_when_starlark_load_refers_to_existing_pkg_missing_file(
        &mut self,
        query_expression: &str,
        num_expected_targets: usize,
    ) {
        if self.helper.reports_universe_evaluation_errors() {
            return;
        }
        self.write_banana_packages();
        // The load statement in "//foo/foo" refers to an existing package, but the Starlark file
        // is missing.
        self.write_file("bar/BUILD", &[]);
        let result = self.eval_fail(query_expression);
        assert_eq!(
            labels_of(&result).len(),
            num_expected_targets,
            "{:?}",
            labels_of(&result)
        );
        self.assert_contains_event(
            "error loading package 'foo/foo': cannot load '//bar:lib.bzl': no such file",
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryKeepGoingTest.java:488 AbstractQueryKeepGoingTest.runTestErrorReportedWhenStarlarkLoadRefersToFileInSymlinkCycle
    pub fn run_test_error_reported_when_starlark_load_refers_to_file_in_symlink_cycle(
        &mut self,
        query_expression: &str,
        num_expected_targets: usize,
    ) {
        if self.helper.reports_universe_evaluation_errors() {
            return;
        }
        self.write_banana_packages();
        // The load statement in "//foo/foo" refers to an existing package, but the Starlark file
        // the load statement refers to points into a symlink cycle.
        self.write_file("bar/BUILD", &[]);
        self.helper
            .ensure_symbolic_link("bar/lib.bzl", "bar/recursion");
        self.helper
            .ensure_symbolic_link("bar/recursion", "bar/recursion");
        let result = self.eval_fail(query_expression);
        assert_eq!(
            labels_of(&result).len(),
            num_expected_targets,
            "{:?}",
            labels_of(&result)
        );
        self.assert_contains_event(
            "error loading package 'foo/foo': Encountered error while reading extension file \
             'bar/lib.bzl': Symlink cycle",
        );
    }
}
