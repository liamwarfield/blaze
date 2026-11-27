//! Test support for the query crates: the Rust counterpart of Java's query test helpers
//! (`src/test/java/com/google/devtools/build/lib/query2/testutil`).
//!
//! [`QueryHelper`] is `AbstractQueryHelper` with `SkyframeQueryHelper`, built on the analysis
//! test harness (`blaze_sky_top::testing::BuildViewTestCase`) rather than on the in-process
//! multi-command fixture (`blaze_buildtool_testing`):
//!
//! - Java's `SkyframeQueryHelper` is a `SkyframeExecutor` over a scratch workspace set up with
//!   `AnalysisMock` (mock tools, the mock C++ and Python support, the mock testing rules
//!   `//test_defs:*`), and creates the query environment directly with `QueryEnvironmentFactory`
//!   (no runtime, no command, no dispatcher). `BuildViewTestCase` is exactly that set-up in Rust
//!   (the same `AnalysisMock` port, a scratch workspace, one executor rebuilt when a scratch file
//!   changes), so the query environments are made over its executor with the same factory
//!   the `query` command uses (`blaze_cquery::commands::query_adapters`).
//! - `blaze_buildtool_testing` runs whole commands through the dispatcher with the real
//!   `@bazel_tools` of the oracle's install base, which is Java's `BuildIntegrationTestCase`:
//!   the right base for integration tests, not for the query unit tests, whose assertions are on
//!   result sets, `QueryException` messages and failure details, which the command line only
//!   shows as text.
//!
//! The post-analysis helpers (`PostAnalysisQueryHelper` with `ConfiguredTargetQueryHelper` or
//! `ActionGraphQueryHelper`, [`EnvironmentKind::Cquery`] and [`EnvironmentKind::Aquery`]) are the
//! same `QueryHelper` over `BuildViewTestCase::analysis_test_case()` (Java's composed
//! `AnalysisTestCase`): `evaluate_query` analyzes the universe (`update`) and builds the
//! environment from the executor's graph as the `cquery` and `aquery` commands do.
//!
//! Differences from Java (each also a `PORT:` note where it applies):
//!
//! - **The executor.** Java keeps one executor and calls `maybeHandleDiffs` after changing files;
//!   the harness's executor is rebuilt after any scratch file changes (the next query sees the
//!   new files), so no diff handling is needed.
//! - **File system.** Java's workspace is `/workspace` of an in-memory file system; here it is a
//!   temporary directory, so messages that contain the workspace path are compared with
//!   [`QueryHelper::get_root_directory`].
//! - **Module root.** The fake registry of the harness is `<workspace>/modules` (Java:
//!   `<outputBase>/modules`), so a module's source tree written there is inside the workspace.
//! - **Rule class provider.** Java's tests add native mock rules (`MockRule`, `TestAspects`) with
//!   `useRuleClassProvider`; Blaze's rule class providers have no native test rules, so those
//!   tests are ignored with that reason.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::TargetPatternParser;
use blaze_cquery::aquery::action_graph_query_environment::KeyedConfiguredTargetValue;
use blaze_cquery::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, new_action_graph_query_environment_with_settings,
};
use blaze_cquery::buildtool::post_analysis_query_processor::get_transitive_configurations;
use blaze_cquery::cquery::config_function::ConfigFunctionSlot;
use blaze_cquery::cquery::configured_target_query_environment::CqueryNode;
use blaze_cquery::cquery::configured_target_query_environment::{
    ConfiguredTargetQueryEnvironment, cquery_functions,
    new_configured_target_query_environment_with_settings,
};
use blaze_cquery::post_analysis_query_environment::{
    PostAnalysisNode, PostAnalysisQueryEnvironment, TopLevelConfigurations,
};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::FailureDetail;
use blaze_query::commands::query_command::QueryCommandEnvironment;
use blaze_query::common::abstract_blaze_query_environment::{
    AbstractBlazeQueryEnvironment, EvaluateQueryError, LabelFilter,
};
use blaze_query::common::universe_scope::UniverseScope;
use blaze_query::query::blaze_query_environment::TargetDigraph;
use blaze_query::query::graphless_blaze_query_environment::GraphlessBlazeQueryEnvironment;
use blaze_query_engine::{QueryEnvironment, TargetAccessor};
use blaze_query_engine::{QueryEvalResult, QueryExpression, Setting};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_sky_top::testing::{BuildViewTestCase, UpdateError};
use blaze_skyframe::WalkableGraph;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

/// The query environment a suite runs on (Java: the test class's `QueryEnvironmentFactory`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentKind {
    /// `GraphlessQueryTest`: always a `GraphlessBlazeQueryEnvironment`.
    Graphless,
    /// `QueryTest` (and `QueryKeepGoingTest`): the standard `QueryEnvironmentFactory`, a
    /// `BlazeQueryEnvironment`, or a `SkyQueryEnvironment` for an unordered query with a universe.
    Standard,
    /// `ConfiguredTargetQueryHelper`: a `ConfiguredTargetQueryEnvironment` over the graph of an
    /// analysis of the universe (`PostAnalysisQueryHelper`).
    Cquery,
    /// `ActionGraphQueryHelper`: an `ActionGraphQueryEnvironment` over the graph of an analysis of
    /// the universe (`PostAnalysisQueryHelper`).
    Aquery,
}

impl EnvironmentKind {
    /// Whether the helper is a `PostAnalysisQueryHelper`.
    pub fn is_post_analysis(self) -> bool {
        matches!(self, EnvironmentKind::Cquery | EnvironmentKind::Aquery)
    }
}

/// The result nodes of a post-analysis query (Java `Set<T>` with `T` = `CqueryNode` or
/// `ConfiguredTargetValue`), in the iteration order of the environment's set.
#[derive(Clone)]
pub enum PostAnalysisNodes {
    Cquery(Vec<CqueryNode>),
    Aquery(Vec<KeyedConfiguredTargetValue>),
}

/// `PostAnalysisQueryTest.DEFAULT_UNIVERSE`: an invalid target expression, so that a test sets
/// the universe scope by the target patterns of its query or explicitly.
pub const POST_ANALYSIS_DEFAULT_UNIVERSE: &str = "DEFAULT_UNIVERSE";

/// `AbstractQueryHelper.ResultAndTargets`: the evaluation result and the result set, in the order
/// the callback received it.
pub struct ResultAndTargets {
    pub query_eval_result: QueryEvalResult,
    pub targets: Vec<Target>,
    /// The graph of a `DigraphQueryEvalResult` (only a `BlazeQueryEnvironment` returns one).
    pub graph: Option<Rc<RefCell<TargetDigraph>>>,
    /// `helper.getLabel(t)` of each element of the result set, in its order.
    pub labels: Vec<String>,
    /// The nodes of a post-analysis query (`targets` is empty then).
    pub nodes: Option<PostAnalysisNodes>,
}

impl ResultAndTargets {
    pub fn get_query_eval_result(&self) -> &QueryEvalResult {
        &self.query_eval_result
    }

    pub fn get_result_set(&self) -> &[Target] {
        &self.targets
    }

    /// `((DigraphQueryEvalResult<T>) getQueryEvalResult()).getGraph()`; panics as Java's cast
    /// fails when the environment returned a plain `QueryEvalResult`.
    pub fn get_graph(&self) -> Rc<RefCell<TargetDigraph>> {
        self.graph
            .clone()
            .expect("ClassCastException: not a DigraphQueryEvalResult")
    }
}

/// What `evaluateQuery` throws: a `QueryException` or an interrupt.
#[derive(Debug)]
pub enum EvaluateError {
    /// The `QueryException`: `(message of the cause if any, else of the exception, failure
    /// detail)`, as `AbstractQueryTest.evalThrows` reads them.
    Query {
        message: String,
        failure_detail: FailureDetail,
    },
    Interrupted,
    /// What `PostAnalysisQueryHelper.evaluateQuery` throws from the analysis of the universe
    /// (`AnalysisTestCase.update`): a `TargetParsingException`, a `ViewCreationFailedException`
    /// or an `InvalidConfigurationException`.
    Update(UpdateError),
}

/// Java `AbstractQueryHelper<Target>` with `SkyframeQueryHelper` (see the crate documentation).
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:39 AbstractQueryHelper
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:61 SkyframeQueryHelper
pub struct QueryHelper {
    pub t: BuildViewTestCase,
    kind: EnvironmentKind,
    keep_going: bool,
    settings: HashSet<Setting>,
    ordered_results: bool,
    universe_scope: UniverseScope,
    /// `PostAnalysisQueryHelper.wholeTestUniverse`.
    whole_test_universe: bool,
    /// `SkyframeQueryHelper.lazyMacroExpansionPackages`, in the option's syntax (`None`:
    /// `LazyMacroExpansionPackages.NONE`).
    lazy_macro_expansion_packages: Option<String>,
}

/// `SkyframeQueryHelper`'s `loadingPhaseThreads`.
const LOADING_PHASE_THREADS: i32 = 1;

impl QueryHelper {
    /// `setUp`: the scratch workspace with `AnalysisMock`'s mock client and tools, and the
    /// package options of `initTargetPatternEvaluator` (private default visibility).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:86 SkyframeQueryHelper.setUp
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:60 AbstractQueryHelper.setUp
    ///
    /// For the post-analysis kinds, `PostAnalysisQueryHelper.setUp`: an `AnalysisTestCase` (its
    /// fail-fast handler stays installed; `turn_off_fail_fast` removes it).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:83 PostAnalysisQueryHelper.setUp
    pub fn new(kind: EnvironmentKind) -> QueryHelper {
        if kind.is_post_analysis() {
            // SkyQueryEnvironment.DEFAULT_THREAD_COUNT, a static initializer in Java
            // (Runtime.getRuntime().availableProcessors() of the test JVM).
            blaze_cquery::post_analysis_query_environment::set_default_thread_count(
                blaze_runtime::local_resources::available_processors(),
            );
        }
        let t = if kind.is_post_analysis() {
            // PORT: MockProtoSupport.setup and MockObjcSupport.setup are not ported (no mock proto
            // or objc support in blaze_sky_top::testing); the tests that need them name it.
            BuildViewTestCase::analysis_test_case()
        } else {
            let mut t = BuildViewTestCase::new();
            // Java's helper reports events to a collector only; an error event does not fail
            // the test.
            t.remove_fail_fast_handler();
            // packageOptions.defaultVisibility = RuleVisibility.PRIVATE
            t.set_default_visibility("private");
            // The build language options of initTargetPatternEvaluator
            // (experimentalGoogleLegacyApi = !isThisBazel() is false, the default).
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:333 SkyframeQueryHelper.initTargetPatternEvaluator
            t.set_build_language_options(&[
                "--experimental_enable_scl_dialect",
                "--experimental_dormant_deps",
                "--incompatible_autoload_externally=",
            ]);
            // writeFile("MODULE.bazel", ...): the harness has written an empty MODULE.bazel
            // (FoundationTestCase's), which this replaces.
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:139 SkyframeQueryHelper.setUp
            t.scratch.overwrite_file_bytes(
                "MODULE.bazel",
                b"module( name = \"root\", version = \"1.0\")",
            );
            t
        };
        let mut helper = QueryHelper {
            t,
            kind,
            keep_going: false,
            settings: HashSet::new(),
            ordered_results: true,
            universe_scope: UniverseScope::empty(),
            whole_test_universe: false,
            lazy_macro_expansion_packages: None,
        };
        if !kind.is_post_analysis() {
            helper.apply_package_options();
        }
        helper
    }

    /// The package options of `initTargetPatternEvaluator` (the default visibility is the
    /// harness's own setting, see `new`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:325 SkyframeQueryHelper.initTargetPatternEvaluator
    fn apply_package_options(&mut self) {
        let mut options = vec![
            "--show_loading_progress".to_owned(),
            "--legacy_globbing_threads=7".to_owned(),
        ];
        if let Some(value) = &self.lazy_macro_expansion_packages {
            options.push(format!(
                "--experimental_lazy_macro_expansion_packages={value}"
            ));
        }
        let options: Vec<&str> = options.iter().map(String::as_str).collect();
        self.t.set_package_options(&options);
    }

    pub fn kind(&self) -> EnvironmentKind {
        self.kind
    }

    /// `setUniverseScope(universeScope)`: `universeScope.split(",")` (Java's `split` drops
    /// trailing empty strings). A `PostAnalysisQueryHelper` ignores it after
    /// `setWholeTestUniverseScope`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:73 AbstractQueryHelper.setUniverseScope
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:126 PostAnalysisQueryHelper.setUniverseScope
    pub fn set_universe_scope(&mut self, universe_scope: &str) {
        if self.kind.is_post_analysis() && self.whole_test_universe {
            return;
        }
        self.set_universe_scope_internal(universe_scope);
    }

    fn set_universe_scope_internal(&mut self, universe_scope: &str) {
        let mut parts: Vec<String> = universe_scope.split(',').map(str::to_owned).collect();
        while parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
        self.universe_scope = UniverseScope::from_universe_scope_list(parts);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:132 PostAnalysisQueryHelper.setWholeTestUniverseScope
    pub fn set_whole_test_universe_scope(&mut self, universe_scope: &str) {
        self.set_universe_scope_internal(universe_scope);
        self.whole_test_universe = true;
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:117 PostAnalysisQueryHelper.isWholeTestUniverse
    pub fn is_whole_test_universe(&self) -> bool {
        self.whole_test_universe
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:121 PostAnalysisQueryHelper.getUniverseScopeAsStringList
    pub fn get_universe_scope_as_string_list(&self) -> Vec<String> {
        self.universe_scope
            .get_constant_value_maybe()
            .expect("a constant universe scope")
            .to_vec()
    }

    /// `turnOffFailFast()`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:176 PostAnalysisQueryHelper.turnOffFailFast
    pub fn turn_off_fail_fast(&mut self) {
        self.t.remove_fail_fast_handler();
    }

    /// `useConfiguration(args...)`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:310 PostAnalysisQueryHelper.useConfiguration
    pub fn use_configuration(&mut self, args: &[&str]) {
        self.t.use_configuration(args);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:85 AbstractQueryHelper.setOrderedResults
    pub fn set_ordered_results(&mut self, ordered_results: bool) {
        self.ordered_results = ordered_results;
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:90 AbstractQueryHelper.setKeepGoing
    pub fn set_keep_going(&mut self, keep_going: bool) {
        self.keep_going = keep_going;
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:95 AbstractQueryHelper.isKeepGoing
    pub fn is_keep_going(&self) -> bool {
        self.keep_going
    }

    /// `QueryHelper.reportsUniverseEvaluationErrors()`: the interface's default, `true` (no helper
    /// at the pin overrides it).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryTest.java:2817 AbstractQueryTest.QueryHelper.reportsUniverseEvaluationErrors
    pub fn reports_universe_evaluation_errors(&self) -> bool {
        true
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:100 AbstractQueryHelper.setQuerySettings
    pub fn set_query_settings(&mut self, settings: &[Setting]) {
        self.settings = settings.iter().copied().collect();
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:80 AbstractQueryHelper.clearEvents
    pub fn clear_events(&self) {
        self.t.event_collector.clear();
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:105 AbstractQueryHelper.assertContainsEvent
    pub fn assert_contains_event(&self, expected_message: &str) -> Event {
        self.t.assert_contains_event(expected_message)
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:110 AbstractQueryHelper.assertDoesNotContainEvent
    pub fn assert_does_not_contain_event(&self, expected_message: &str) {
        self.t.assert_does_not_contain_event(expected_message);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:115 AbstractQueryHelper.getFirstEvent
    pub fn get_first_event(&self) -> String {
        self.t
            .event_collector
            .iter()
            .next()
            .expect("an event")
            .get_message()
            .into_owned()
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/AbstractQueryHelper.java:120 AbstractQueryHelper.getEvents
    pub fn get_events(&self) -> Vec<Event> {
        self.t.event_collector.iter().collect()
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:165 SkyframeQueryHelper.getRootDirectory
    pub fn get_root_directory(&self) -> &Path {
        &self.t.root_directory
    }

    /// Writes a scratch file; fails if it exists (`Could not create scratch file (file exists)`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:175 SkyframeQueryHelper.writeFile
    pub fn write_file(&self, file_name: &str, lines: &[&str]) {
        let file = self.t.root_directory.get_relative_str(file_name);
        if file.exists() {
            panic!("IOException: Could not create scratch file (file exists) {file_name}");
        }
        self.overwrite_file(file_name, lines);
    }

    /// Writes a scratch file, the lines joined with `\n` (no final newline).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:185 SkyframeQueryHelper.overwriteFile
    pub fn overwrite_file(&self, file_name: &str, lines: &[&str]) {
        self.t
            .scratch
            .overwrite_file_bytes(file_name, lines.join("\n").as_bytes());
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:192 SkyframeQueryHelper.ensureSymbolicLink
    pub fn ensure_symbolic_link(&mut self, link: &str, target: &str) {
        let link_path = self.t.root_directory.get_relative_str(link);
        let target_path = self.t.root_directory.get_relative_str(target);
        let link_os = std::path::PathBuf::from(link_path.get_path_string());
        if let Some(parent) = link_os.parent() {
            std::fs::create_dir_all(parent).expect("create the link's parent directory");
        }
        let _ = std::fs::remove_file(&link_os);
        std::os::unix::fs::symlink(target_path.get_path_string(), &link_os)
            .expect("create the symbolic link");
        // The link is made outside the scratch writer: invalidate the executor ourselves.
        self.t.invalidate_packages();
    }

    /// `clearAllFiles()`: deletes everything below the workspace root.
    ///
    /// PORT: the harness's module root (the fake registry) is `<root>/modules`, inside the
    /// workspace, where Java's is `<outputBase>/modules`; it is deleted with the rest, and
    /// `FakeRegistry::add_module` writes it again.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:197 SkyframeQueryHelper.clearAllFiles
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:156 PostAnalysisQueryHelper.clearAllFiles
    pub fn clear_all_files(&mut self) {
        if self.kind.is_post_analysis() {
            // analysisHelper.getRootDirectory().deleteTree()
            self.t
                .root_directory
                .delete_tree()
                .unwrap_or_else(|e| panic!("IOException: {e}"));
        } else {
            self.t
                .root_directory
                .delete_trees_below()
                .unwrap_or_else(|e| panic!("IOException: {e}"));
        }
        self.t.invalidate_packages();
    }

    /// `getIgnoredSubdirectoriesFile()`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:182 SkyframeQueryHelper.getIgnoredSubdirectoriesFile
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:143 PostAnalysisQueryHelper.getIgnoredSubdirectoriesFile
    pub fn get_ignored_subdirectories_file(&self) -> PathFragment {
        if self.kind.is_post_analysis() {
            return PathFragment::empty_fragment();
        }
        PathFragment::create(".bazelignore")
    }

    /// `maybeHandleDiffs()`.
    ///
    /// PORT: the harness rebuilds its executor after any scratch file changes, so the next
    /// query reads the new files without diff handling.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:175 SkyframeQueryHelper.maybeHandleDiffs
    pub fn maybe_handle_diffs(&mut self) {}

    /// `setLazyMacroExpansionPackages(value)`, with the value in the option's syntax (`*` is
    /// `LazyMacroExpansionPackages.ALL`): `initTargetPatternEvaluator` sets
    /// `packageOptions.lazyMacroExpansionPackages` from it.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:382 SkyframeQueryHelper.setLazyMacroExpansionPackages
    pub fn set_lazy_macro_expansion_packages(&mut self, value: &str) {
        self.lazy_macro_expansion_packages = Some(value.to_owned());
        self.apply_package_options();
    }

    /// `getModuleRoot()`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:425 SkyframeQueryHelper.getModuleRoot
    pub fn get_module_root(&self) -> &Path {
        &self.t.module_root
    }

    /// `addModule(key, moduleFileLines...)`: adds the module to the fake registry.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:315 SkyframeQueryHelper.addModule
    pub fn add_module(&self, key: &blaze_bzlmod_model::ModuleKey, module_file_lines: &[&str]) {
        self.t.registry().add_module(key, module_file_lines);
    }

    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:252 SkyframeQueryHelper.getLabel
    pub fn get_label(&self, target: &Target) -> String {
        target.get_label().to_string()
    }

    /// `getQueryEnvironment()`: a new environment over the current executor.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:200 SkyframeQueryHelper.getQueryEnvironment
    pub fn get_query_environment(&mut self) -> Box<dyn AbstractBlazeQueryEnvironment> {
        let reporter: Arc<dyn ExtendedEventHandler> = self.t.event_handler();
        let root_directory = self.t.root_directory.clone();
        let executor = self.t.skyframe_executor();
        // PORT: Java's mainRepoTargetParser maps "", bazel_tools and platforms
        // (AbstractQueryHelper.DEFAULT_MAIN_REPO_MAPPING); the executor's main repository mapping
        // has those entries for the test workspace.
        let main_repo_mapping = executor
            .get_main_repo_mapping(false, LOADING_PHASE_THREADS as usize, reporter.clone())
            .unwrap_or_else(|e| panic!("main repository mapping: {e:?}"));
        let output: blaze_query::query::output::output_formatter::OutputStream =
            Rc::new(RefCell::new(Vec::<u8>::new()));
        let env: QueryCommandEnvironment =
            blaze_cquery::commands::query_adapters::query_command_environment(
                executor,
                reporter.clone(),
                output,
                PathFragment::empty_fragment(),
                root_directory,
                main_repo_mapping.clone(),
                LOADING_PHASE_THREADS,
            );
        let main_repo_target_parser = TargetPatternParser::new(
            PathFragment::empty_fragment(),
            blaze_cmdline::repository_name::MAIN.clone(),
            main_repo_mapping,
        );
        match self.kind {
            // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:197 PostAnalysisQueryHelper.getQueryEnvironment
            EnvironmentKind::Cquery | EnvironmentKind::Aquery => {
                panic!("UnsupportedOperationException")
            }
            // GraphlessQueryTest's factory.
            EnvironmentKind::Graphless => Box::new(GraphlessBlazeQueryEnvironment::new(
                env.query_transitive_package_preloader.clone(),
                env.target_provider.clone(),
                env.caching_package_locator.clone(),
                env.target_pattern_preloader.clone(),
                main_repo_target_parser,
                self.keep_going,
                /* strict_scope= */ true,
                LOADING_PHASE_THREADS,
                LabelFilter::AllLabels,
                reporter,
                self.settings.clone(),
                /* extra_functions= */ Vec::new(),
                LabelPrinter::legacy(),
            )),
            // QueryEnvironmentFactory.create.
            EnvironmentKind::Standard => {
                blaze_query::commands::query_command::new_query_environment(
                    &env,
                    self.keep_going,
                    self.ordered_results,
                    self.universe_scope.clone(),
                    LOADING_PHASE_THREADS,
                    self.settings.clone(),
                    /* use_graphless_query= */ false,
                    main_repo_target_parser,
                    LabelPrinter::legacy(),
                )
            }
        }
    }

    /// `evaluateQuery(query)`: parses, transforms and evaluates the query in a new environment
    /// with an ordered aggregating callback.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:224 SkyframeQueryHelper.evaluateQuery
    pub fn evaluate_query(&mut self, query: &str) -> Result<ResultAndTargets, EvaluateError> {
        if self.kind.is_post_analysis() {
            return self.evaluate_post_analysis_query(query);
        }
        let env = self.get_query_environment();
        let result = (|| {
            let expr = match QueryExpression::parse(query, env.as_query_environment()) {
                Ok(expr) => expr,
                // Expect valid query syntax in tests.
                Err(e) => panic!("IllegalArgumentException: {}", e.get_message()),
            };
            let expr = env.transform_parsed_query(expr);
            let callback =
                blaze_query_engine::query_util::new_ordered_aggregate_all_output_formatter_callback(
                    env.as_query_environment(),
                );
            match env.evaluate_query(&expr, &*callback) {
                Ok(query_eval_result) => {
                    let targets = callback.get_result();
                    Ok(ResultAndTargets {
                        query_eval_result,
                        labels: targets.iter().map(|t| t.get_label().to_string()).collect(),
                        targets,
                        graph: env.get_result_graph(),
                        nodes: None,
                    })
                }
                Err(EvaluateQueryError::Query(e)) => Err(EvaluateError::Query {
                    // e.getCause() != null ? e.getCause().getMessage() : e.getMessage()
                    message: match e.get_cause() {
                        Some(cause) => cause.to_string(),
                        None => e.get_message().to_owned(),
                    },
                    failure_detail: e.get_failure_detail().clone(),
                }),
                Err(EvaluateQueryError::Interrupted(_)) => Err(EvaluateError::Interrupted),
                // Should be impossible since AggregateAllOutputFormatterCallback doesn't throw
                // IOException.
                Err(EvaluateQueryError::Io(e)) => panic!("IllegalStateException: {e}"),
            }
        })();
        env.close();
        result
    }

    /// The inputs of `getPostAnalysisQueryEnvironment(universe, aspects)`: analyzes the universe
    /// (`analysisHelper.update(aspects, universe)`) and reads the top-level configurations, the
    /// transitive configurations and the top-level aspects of the result.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:205 PostAnalysisQueryHelper.getPostAnalysisQueryEnvironment
    fn analyze_universe(
        &mut self,
        universe: &[String],
        aspects: &[&str],
    ) -> Result<PostAnalysisInputs, EvaluateError> {
        if universe == [POST_ANALYSIS_DEFAULT_UNIVERSE] {
            let e = blaze_query_engine::QueryException::new(
                None,
                "Tests must set universe scope by either having parsable labels in each query \
                 expression or setting explicitly through query helper.",
                blaze_protos::failure_details::query::Code::QueryUnknown,
            );
            return Err(EvaluateError::Query {
                message: e.get_message().to_owned(),
                failure_detail: e.get_failure_detail().clone(),
            });
        }
        let labels: Vec<&str> = universe.iter().map(String::as_str).collect();
        let (top_level_configurations, top_level_aspects) = match self
            .t
            .update_with_aspects(aspects, &labels)
        {
            Ok(analysis_result) => (
                TopLevelConfigurations::new(analysis_result.get_top_level_targets_with_configs()),
                analysis_result.get_aspects_map().clone(),
            ),
            Err(e) => return Err(EvaluateError::Update(e)),
        };
        let reporter = self.t.event_handler();
        let executor = self.t.skyframe_executor();
        // SkyframeExecutorWrappingWalkableGraph.of(analysisHelper.getSkyframeExecutor())
        let walkable_graph = executor.get_walkable_graph();
        let transitive_configurations = get_transitive_configurations(
            &executor.get_transitive_configuration_keys(),
            &*walkable_graph,
        );
        let pkg_path = executor.get_package_path().clone();
        let main_repo_mapping = executor
            .get_main_repo_mapping(false, LOADING_PHASE_THREADS as usize, reporter)
            .unwrap_or_else(|e| panic!("main repository mapping: {e:?}"));
        // AbstractQueryHelper.mainRepoTargetParser (parserPrefix is the empty fragment).
        let main_repo_target_parser = TargetPatternParser::new(
            PathFragment::empty_fragment(),
            blaze_cmdline::repository_name::MAIN.clone(),
            main_repo_mapping,
        );
        Ok(PostAnalysisInputs {
            walkable_graph,
            top_level_configurations,
            transitive_configurations,
            top_level_aspects,
            main_repo_target_parser,
            pkg_path,
        })
    }

    /// `ConfiguredTargetQueryHelper.getPostAnalysisQueryEnvironment(universe, aspects)`, with the
    /// slot of the `config` function filled.
    // Java: src/test/java/com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryHelper.java:39 ConfiguredTargetQueryHelper.getPostAnalysisQueryEnvironment
    pub fn get_configured_target_query_environment(
        &mut self,
        universe: &[String],
        aspects: &[&str],
    ) -> Result<ConfiguredTargetQueryEnvironment, EvaluateError> {
        let inputs = self.analyze_universe(universe, aspects)?;
        let slot: ConfigFunctionSlot = Arc::new(parking_lot::RwLock::new(None));
        // ImmutableList.copyOf(ConfiguredTargetQueryEnvironment.CQUERY_FUNCTIONS)
        let extra_functions = cquery_functions(&slot);
        let env = new_configured_target_query_environment_with_settings(
            self.keep_going,
            self.t.event_handler(),
            extra_functions,
            inputs.top_level_configurations,
            inputs.transitive_configurations,
            &inputs.top_level_aspects,
            inputs.main_repo_target_parser,
            inputs.pkg_path,
            inputs.walkable_graph,
            self.settings.clone(),
            LabelPrinter::legacy(),
        );
        // PORT: ConfigFunction reaches the environment through this slot (config_function.rs).
        *slot.write() = Some(env.graph_lookup());
        Ok(env)
    }

    /// `ActionGraphQueryHelper.getPostAnalysisQueryEnvironment(universe, aspects)`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/aquery/ActionGraphQueryHelper.java:33 ActionGraphQueryHelper.getPostAnalysisQueryEnvironment
    pub fn get_action_graph_query_environment(
        &mut self,
        universe: &[String],
        aspects: &[&str],
    ) -> Result<ActionGraphQueryEnvironment, EvaluateError> {
        let inputs = self.analyze_universe(universe, aspects)?;
        // ImmutableList.copyOf(ActionGraphQueryEnvironment.AQUERY_FUNCTIONS): empty.
        Ok(new_action_graph_query_environment_with_settings(
            self.keep_going,
            self.t.event_handler(),
            Vec::new(),
            inputs.top_level_configurations,
            inputs.transitive_configurations,
            inputs.main_repo_target_parser,
            inputs.pkg_path,
            inputs.walkable_graph,
            self.settings.clone(),
            LabelPrinter::legacy(),
        ))
    }

    /// `PostAnalysisQueryHelper.evaluateQuery(query)`.
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:261 PostAnalysisQueryHelper.evaluateQuery
    fn evaluate_post_analysis_query(
        &mut self,
        query: &str,
    ) -> Result<ResultAndTargets, EvaluateError> {
        let universe = self.get_universe_scope_as_string_list();
        match self.kind {
            EnvironmentKind::Cquery => {
                let env = self.get_configured_target_query_environment(&universe, &[])?;
                let (query_eval_result, nodes) =
                    evaluate_on_post_analysis_environment(&env, query)?;
                Ok(ResultAndTargets {
                    query_eval_result,
                    targets: Vec::new(),
                    graph: None,
                    labels: nodes
                        .iter()
                        .map(|n| n.get_original_label().to_string())
                        .collect(),
                    nodes: Some(PostAnalysisNodes::Cquery(nodes)),
                })
            }
            EnvironmentKind::Aquery => {
                let env = self.get_action_graph_query_environment(&universe, &[])?;
                let (query_eval_result, nodes) =
                    evaluate_on_post_analysis_environment(&env, query)?;
                Ok(ResultAndTargets {
                    query_eval_result,
                    targets: Vec::new(),
                    graph: None,
                    labels: nodes
                        .iter()
                        .map(|n| n.get_original_label().to_string())
                        .collect(),
                    nodes: Some(PostAnalysisNodes::Aquery(nodes)),
                })
            }
            _ => unreachable!("not a post-analysis helper"),
        }
    }

    /// `getSkyframeExecutor().getConfiguration(getReporter(), key)` (null for a null key).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1930 SkyframeExecutor.getConfiguration
    pub fn get_configuration(
        &mut self,
        key: Option<&BuildConfigurationKey>,
    ) -> Option<Arc<BuildConfigurationValue>> {
        let key = key?;
        let reporter = self.t.event_handler();
        self.t
            .skyframe_executor()
            .get_configuration_for_key(reporter, key)
    }

    /// Package names that must not be loaded (`assertPackageNotLoaded`).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/SkyframeQueryHelper.java:466 SkyframeQueryHelper.assertPackageNotLoaded
    ///
    /// A `PostAnalysisQueryHelper` asserts nothing (it loads all packages of the universe).
    // Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:283 PostAnalysisQueryHelper.assertPackageNotLoaded
    pub fn assert_package_not_loaded(&mut self, package_name: &str) {
        if self.kind.is_post_analysis() {
            return;
        }
        let executor = self.t.skyframe_executor();
        let key =
            blaze_skyframe::SkyKey::new(blaze_sky_loading::package_function::PackageKey::key(
                blaze_cmdline::PackageIdentifier::create_in_main_repo_str(package_name),
            ));
        let evaluator = executor.get_evaluator();
        assert!(
            evaluator.get_existing_value(&key).is_none()
                && evaluator.get_existing_error_for_testing(&key).is_none(),
            "package {package_name} was loaded"
        );
    }
}

/// The target patterns of `query` parsed with the test's `getDefaultFunctions()`
/// (`ConfiguredTargetQueryEnvironment.FUNCTIONS` for cquery, `ActionGraphQueryEnvironment.FUNCTIONS`
/// for aquery), in `collectTargetPatterns` order without duplicates (Java's `LinkedHashSet`).
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryTest.java:128 PostAnalysisQueryTest.maybeParseUniverseScope
pub fn collect_target_patterns_with_default_functions(
    kind: EnvironmentKind,
    query: &str,
) -> Vec<String> {
    let expression = match kind {
        EnvironmentKind::Cquery => {
            let slot: ConfigFunctionSlot = Arc::new(parking_lot::RwLock::new(None));
            blaze_query_engine::QueryParser::<CqueryNode>::parse_with_functions(
                query,
                &blaze_cquery::commands::cquery_command::function_map(&slot),
            )
        }
        EnvironmentKind::Aquery => {
            let functions: std::collections::HashMap<_, _> =
                blaze_cquery::aquery::action_graph_query_environment::functions()
                    .into_iter()
                    .map(|f| (f.get_name(), f))
                    .collect();
            blaze_query_engine::QueryParser::<KeyedConfiguredTargetValue>::parse_with_functions(
                query, &functions,
            )
        }
        _ => unreachable!("not a post-analysis helper"),
    }
    .unwrap_or_else(|e| panic!("QuerySyntaxException: {}", e.get_message()));
    let mut target_pattern_set: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    expression.collect_target_patterns(&mut |p| {
        target_pattern_set.insert(p);
    });
    target_pattern_set.into_iter().collect()
}

/// What `getPostAnalysisQueryEnvironment` reads from the analysis of the universe.
struct PostAnalysisInputs {
    walkable_graph: Arc<dyn WalkableGraph>,
    top_level_configurations: TopLevelConfigurations,
    transitive_configurations: IndexMap<String, Arc<BuildConfigurationValue>>,
    top_level_aspects: IndexMap<AspectKey, Arc<AspectValue>>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
}

/// The evaluation part of `PostAnalysisQueryHelper.evaluateQuery`: parses the query, evaluates it
/// into an ordered aggregating callback and returns the result as the environment's set
/// (`env.createThreadSafeMutableSet()` with the callback's result added).
///
/// PORT: `env.transformParsedQuery` is the identity for the post-analysis environments
/// (`QueryEnvironment`'s default), so it is not called.
// Java: src/test/java/com/google/devtools/build/lib/query2/testutil/PostAnalysisQueryHelper.java:261 PostAnalysisQueryHelper.evaluateQuery
fn evaluate_on_post_analysis_environment<T: PostAnalysisNode>(
    env: &PostAnalysisQueryEnvironment<T>,
    query: &str,
) -> Result<(QueryEvalResult, Vec<T>), EvaluateError>
where
    PostAnalysisQueryEnvironment<T>: TargetAccessor<T>,
{
    let callback =
        blaze_query_engine::query_util::new_ordered_aggregate_all_output_formatter_callback(env);
    let expr = match QueryExpression::parse(query, env) {
        Ok(expr) => expr,
        // Expect the user to provide valid syntax.
        Err(e) => panic!("IllegalArgumentException: {}", e.get_message()),
    };
    let query_eval_result = match env.evaluate_query(&expr, &*callback) {
        Ok(r) => r,
        Err(EvaluateQueryError::Query(e)) => {
            return Err(EvaluateError::Query {
                message: match e.get_cause() {
                    Some(cause) => cause.to_string(),
                    None => e.get_message().to_owned(),
                },
                failure_detail: e.get_failure_detail().clone(),
            });
        }
        Err(EvaluateQueryError::Interrupted(_)) => return Err(EvaluateError::Interrupted),
        // Should be impossible since AggregateAllOutputFormatterCallback doesn't throw
        // IOException.
        Err(EvaluateQueryError::Io(e)) => panic!("IllegalStateException: {e}"),
    };
    let targets = env.create_thread_safe_mutable_set();
    targets.add_all(&callback.get_result());
    Ok((query_eval_result, targets.to_vec()))
}
