//! Port of `analysis/util/BuildViewTestCase.java` and `analysis/util/AnalysisTestCase.java`
//! (src/test/java), with the parts of `testutil/FoundationTestCase.java`,
//! `analysis/util/BuildViewForTesting.java` and `testutil/MoreAsserts.java` they use: a scratch
//! workspace with the mock client of `analysis_mock.rs`, analyzed by the real loading,
//! configuration and analysis SkyFunctions as the `blaze` binary wires them
//! ([`SkyframeExecutor`]).
//!
//! Differences from the Java test environment (each is also marked `PORT:` where it applies):
//!
//! - **One evaluator per state.** Blaze's evaluator has no invalidation (one command per
//!   process), so where Java invalidates (`invalidatePackages`, `useConfiguration`,
//!   `setBuildLanguageOptions`, `rewriteModuleDotBazel`, `scratchRule`, `AnalysisTestCase.update`
//!   which invalidates every file) the harness drops its executor and builds a new one on the
//!   next request. It also does so after any scratch file was written or deleted, so a test sees
//!   its latest files even where Java would have needed an explicit invalidation; events of
//!   packages loaded again are then reported again.
//! - **Real file system.** The workspace, the output base and the install base live under a
//!   fresh temporary directory (`/workspace` and an `InMemoryFileSystem` in Java). As in Java the
//!   install base is the workspace directory (`ServerDirectories(rootDirectory, outputBase,
//!   outputBase)`), so `@bazel_tools` is the mock's `embedded_tools/`.
//! - **Repositories.** Every module of the mock is a built-in module with a local path override
//!   (Java `BazelAnalysisMock.getBuiltinModules`), installed through
//!   [`SkyframeExecutor::create_with_builtin_modules`]; repositories are fetched by the real
//!   repository functions with the mock's `local_repository` rule. The registry is an empty
//!   directory under the test directory (Java's `FakeRegistry`): no network.
//! - **Rule class provider.** Bazel's (`BazelRuleClassProvider`), as the binary uses. Java's
//!   `TestRuleClassProvider` adds test-only native rules (`testing_dummy_rule`,
//!   `rule_with_*`, ...) and native test aspects; tests that use them stay ignored ("needs
//!   TestRuleClassProvider's test rules").
//! - **Mock toolchains.** The C++, Java, Python, Objective-C and proto mocks are not ported (see
//!   `analysis_mock.rs`).
//! - **Fail-fast handler.** Java's `failFastHandler` fails the test from inside the event
//!   handler. Here it records the first error event it receives; every harness call that
//!   evaluates checks it afterwards and panics with the event (so a test that expects errors
//!   removes it first, `reporter.remove_handler(&fail_fast_handler)`, as in Java).
//! - **Starlark flags** in `use_configuration` (`--//pkg:flag=value`) need
//!   `StarlarkOptionsParser`, which lives in `blaze_runtime` (downstream of this crate): they
//!   fail with `unported!` (see the report's requests).
//! - **Top-level aspects** (`update` with aspects): `BuildView.createTopLevelAspectKeys` is not
//!   ported; `update` passes them through and the product fails loudly.
//! - `getConfiguredTarget` evaluates the `ConfiguredTargetKey` directly and wraps the value with
//!   its target and configuration (Java runs `ConfiguredTargetAndDataProducer` in a test state
//!   machine); errors go through `SkyframeErrorProcessor.processAnalysisErrors` as in Java.

use std::sync::Arc;

use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::providers::file_provider::{
    FileProvider, FilesToRunProvider, RunfilesProvider,
};
use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_events::{
    Event, EventBus, EventCollector, EventHandler, EventKind, EventKindSet, ExtendedEventHandler,
    Postable, Reporter,
};
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::definition::OptionsClassRef;
use blaze_options::{Options, OptionsClass, OptionsParser};
use blaze_packages::target::Target;
use blaze_sky_loading::package_manager::{NoSuchThingOrInterrupted, TargetProvider};
use blaze_skyframe::{EvaluationResult, Key, SkyKey};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::{
    DEFAULT_THREAD_COUNT, SkyframeExecutor, SkyframeExecutorInputs, get_configuration_from_graph,
};
use crate::testing::analysis_mock::{
    BazelAnalysisMock, PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS, PRODUCT_SPECIFIC_FLAGS,
};
use crate::testing::mock_tools_config::MockToolsConfig;
use crate::testing::scratch::{Scratch, TestDirectory};

/// `BuildViewTestCase.LOADING_PHASE_THREADS`.
pub const LOADING_PHASE_THREADS: usize = 20;

/// Java `FoundationTestCase.failFastHandler`: fails the test on an error event.
///
/// PORT: records the first error; the harness panics with it after the evaluation that reported
/// it (module documentation).
// Java: com/google/devtools/build/lib/testutil/FoundationTestCase.java:62 FoundationTestCase.failFastHandler
#[derive(Default)]
pub struct FailFastHandler {
    first_error: Mutex<Option<String>>,
}

impl EventHandler for FailFastHandler {
    fn handle(&self, event: &Event) {
        if EventKind::ERRORS.contains(event.get_kind()) {
            let mut first = self.first_error.lock();
            if first.is_none() {
                *first = Some(event.to_string());
            }
        }
    }
}

impl FailFastHandler {
    fn check(&self) {
        if let Some(event) = self.first_error.lock().take() {
            panic!("{event}");
        }
    }
}

/// Java `FoundationTestCase.eventBus` with the recorders tests register on it
/// (`BuildViewTestBase.AnalysisFailureRecorder`, `LoadingFailureRecorder`, ...): every posted
/// event, in order.
#[derive(Default)]
pub struct RecordingEventBus {
    events: Mutex<Vec<Arc<dyn Postable>>>,
}

impl EventBus for RecordingEventBus {
    fn post(&self, event: Arc<dyn Postable>) {
        self.events.lock().push(event);
    }
}

impl RecordingEventBus {
    /// The posted events of type `T`, in order (a `@Subscribe` method for `T`).
    pub fn get<T: Postable + Clone>(&self) -> Vec<T> {
        self.events
            .lock()
            .iter()
            .filter_map(|e| {
                let any: &dyn std::any::Any = &**e;
                any.downcast_ref::<T>().cloned()
            })
            .collect()
    }

    /// Forgets the posted events (Java: a new `EventBus` per `update`).
    pub fn clear(&self) {
        self.events.lock().clear();
    }
}

/// `AnalysisTestCase.Flag`.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:110 AnalysisTestCase.Flag
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flag {
    KeepGoing,
    /// Configurations that only include the fragments a target needs to properly analyze.
    TrimmedConfigurations,
    /// Use --default_visibility=public.
    PublicVisibility,
    /// Use --cpu=k8 --host_cpu=k8.
    CpuK8,
    /// Use the product-specific flags (TestConstants.PRODUCT_SPECIFIC_FLAGS).
    ProductSpecificFlags,
}

/// `AnalysisTestCase.FlagBuilder`.
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:129 AnalysisTestCase.FlagBuilder
#[derive(Clone, Debug, Default)]
pub struct FlagBuilder {
    flags: IndexSet<Flag>,
}

impl FlagBuilder {
    pub fn with(mut self, flag: Flag) -> Self {
        self.flags.insert(flag);
        self
    }

    pub fn contains(&self, flag: Flag) -> bool {
        self.flags.contains(&flag)
    }
}

/// Which Java base class the harness stands for: they differ in their configuration defaults
/// and in how `getConfiguredTarget` relates to `update`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestCaseKind {
    /// `BuildViewTestCase`.
    BuildView,
    /// `AnalysisTestCase` (`defaultFlags()`: public visibility, `--cpu=k8`, product flags).
    Analysis,
}

/// The evaluator and the configurations of the current state.
struct ExecutorState {
    executor: SkyframeExecutor,
    target_config: Arc<BuildConfigurationValue>,
    exec_config: Arc<BuildConfigurationValue>,
    /// The options the configurations were created from (`AnalysisTestCase.buildOptions`).
    build_options: Arc<BuildOptions>,
    /// The parsed options (`AnalysisTestCase.optionsParser`).
    options: Arc<OptionsParser>,
}

/// Java `BuildViewTestCase` / `AnalysisTestCase` (see the module documentation).
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:212 BuildViewTestCase
pub struct BuildViewTestCase {
    kind: TestCaseKind,
    directory: TestDirectory,
    /// `FoundationTestCase.scratch`, working directory `rootDirectory`.
    pub scratch: Scratch,
    /// `FoundationTestCase.rootDirectory` (the workspace, also the install base).
    pub root_directory: Path,
    /// `FoundationTestCase.outputBase`.
    pub output_base: Path,
    /// `FoundationTestCase.eventCollector` (errors, warnings and info).
    pub event_collector: Arc<EventCollector>,
    /// `FoundationTestCase.reporter`.
    pub reporter: Arc<Reporter>,
    /// `FoundationTestCase.eventBus` (the reporter's).
    pub event_bus: Arc<RecordingEventBus>,
    /// `FoundationTestCase.failFastHandler`, registered on [`Self::reporter`].
    pub fail_fast_handler: Arc<dyn EventHandler>,
    fail_fast: Arc<FailFastHandler>,
    pub analysis_mock: BazelAnalysisMock,
    pub mock_tools_config: MockToolsConfig,
    /// `moduleRoot` (the fake registry's directory).
    pub module_root: Path,
    /// `configurationArgs` (without the defaults).
    configuration_args: Vec<String>,
    /// `setBuildLanguageOptions` arguments.
    build_language_options: Vec<String>,
    /// `setPackageOptions` arguments.
    package_options: Vec<String>,
    /// `useLoadingOptions` arguments.
    loading_options: Vec<String>,
    /// `BuildViewTestCase.getDefaultVisibility()` (overridable with
    /// [`Self::set_default_visibility`]).
    default_visibility: String,
    /// `AnalysisTestCase.defaultFlags()` (overridable, e.g. `WithKeepGoing`).
    default_flags: FlagBuilder,
    state: Option<ExecutorState>,
    /// `AnalysisTestCase.analysisResult`.
    analysis_result: Option<AnalysisResult>,
}

impl BuildViewTestCase {
    /// `@Before` of `FoundationTestCase` and `BuildViewTestCase.initializeSkyframeExecutor`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:280 BuildViewTestCase.initializeSkyframeExecutor
    pub fn new() -> BuildViewTestCase {
        Self::create(TestCaseKind::BuildView)
    }

    /// `@Before` of `FoundationTestCase` and `AnalysisTestCase.createMocks`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:167 AnalysisTestCase.createMocks
    pub fn analysis_test_case() -> BuildViewTestCase {
        Self::create(TestCaseKind::Analysis)
    }

    fn create(kind: TestCaseKind) -> BuildViewTestCase {
        // Java: FoundationTestCase.initializeFileSystemAndDirectories
        let directory = TestDirectory::new();
        let root_directory = directory.root.get_child("workspace");
        let output_base = directory.root.get_relative(&PathFragment::create(
            "usr/local/google/_blaze_jrluser/FAKEMD5",
        ));
        std::fs::create_dir_all(root_directory.get_path_string()).expect("create workspace");
        std::fs::create_dir_all(output_base.get_path_string()).expect("create output base");
        let scratch = Scratch::new(directory.fs.clone(), root_directory.clone());
        scratch.file(
            root_directory.get_child("MODULE.bazel").get_path_string(),
            &[],
        );

        // Java: FoundationTestCase.initializeLogging
        let event_collector = Arc::new(EventCollector::with_mask(
            EventKind::ERRORS_WARNINGS_AND_INFO,
        ));
        let event_bus = Arc::new(RecordingEventBus::default());
        let reporter = Reporter::with_handlers(
            Some(event_bus.clone() as Arc<dyn EventBus>),
            [event_collector.clone() as Arc<dyn EventHandler>],
        );
        let fail_fast = Arc::new(FailFastHandler::default());
        let fail_fast_handler: Arc<dyn EventHandler> = fail_fast.clone();
        reporter.add_handler(fail_fast_handler.clone());

        let analysis_mock = BazelAnalysisMock;
        let module_root = scratch.dir("modules");
        let mock_tools_config = MockToolsConfig::new(root_directory.clone());
        analysis_mock.setup_mock_tools_repository(&mock_tools_config);
        // Java: initializeMockClient
        analysis_mock.setup_mock_client(&mock_tools_config);

        let mut test_case = BuildViewTestCase {
            kind,
            directory,
            scratch,
            root_directory,
            output_base,
            event_collector,
            reporter,
            event_bus,
            fail_fast_handler,
            fail_fast,
            analysis_mock,
            mock_tools_config,
            module_root,
            configuration_args: Vec::new(),
            build_language_options: Vec::new(),
            package_options: Vec::new(),
            loading_options: Vec::new(),
            default_visibility: "public".to_string(),
            default_flags: FlagBuilder::default()
                .with(Flag::PublicVisibility)
                .with(Flag::CpuK8)
                .with(Flag::ProductSpecificFlags),
            state: None,
            analysis_result: None,
        };
        // useConfiguration()
        test_case.use_configuration(&[]);
        test_case
    }

    /// The file system of the test.
    pub fn file_system(&self) -> &blaze_vfs::FileSystemRef {
        &self.directory.fs
    }

    /// A string of a Java test with Java's workspace directory (`/workspace`) replaced by this
    /// test's.
    ///
    /// PORT: Java's tests run in `/workspace` of an in-memory file system; event locations here
    /// carry the temporary directory (module documentation).
    pub fn ws(&self, java_string: &str) -> String {
        java_string.replace("/workspace", self.root_directory.get_path_string())
    }

    // -----------------------------------------------------------------------------------------
    // Events (FoundationTestCase, MoreAsserts).

    /// The reporter as the executor's event handler.
    pub fn event_handler(&self) -> Arc<dyn ExtendedEventHandler> {
        self.reporter.clone()
    }

    /// The fail-fast check (Java `failFastHandler`, installed on the reporter unless a test
    /// called `reporter.removeHandler(failFastHandler)`): fails the test on an error event.
    pub fn check_fail_fast(&self) {
        self.fail_fast.check();
    }

    /// `eventsToString`: the collected events, one per line.
    fn events_to_string(&self) -> String {
        let mut s = String::new();
        for event in self.event_collector.iter() {
            s.push_str(&event.to_string());
            s.push('\n');
        }
        s
    }

    // Java: com/google/devtools/build/lib/testutil/FoundationTestCase.java:111 FoundationTestCase.assertNoEvents
    pub fn assert_no_events(&self) {
        let events_string = self.events_to_string();
        assert!(events_string.is_empty(), "{events_string}");
    }

    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:286 MoreAsserts.assertContainsEvent
    pub fn assert_contains_event(&self, expected_event: &str) -> Event {
        self.assert_contains_event_of_kinds(expected_event, EventKind::ALL_EVENTS)
    }

    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:286 MoreAsserts.assertContainsEvent
    pub fn assert_contains_event_of_kinds(
        &self,
        expected_event: &str,
        kinds: EventKindSet,
    ) -> Event {
        for event in self.event_collector.iter() {
            // We want to be able to check for the location and the message type (error /
            // warning). Consequently, we use toString() instead of getMessage().
            if event.to_string().contains(expected_event) && kinds.contains(event.get_kind()) {
                return event;
            }
        }
        let events_string = self.events_to_string();
        panic!(
            "Event '{}' not found{}",
            expected_event,
            if events_string.is_empty() {
                String::new()
            } else {
                format!("; found these though:{events_string}")
            }
        );
    }

    /// `assertContainsEvent(Pattern)`: the first event whose `toString()` matches `predicate`.
    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:319 MoreAsserts.assertContainsEvent
    pub fn assert_contains_event_matching(
        &self,
        description: &str,
        predicate: impl Fn(&str) -> bool,
    ) -> Event {
        for event in self.event_collector.iter() {
            if predicate(&event.to_string()) {
                return event;
            }
        }
        let events_string = self.events_to_string();
        let mut failure_message = format!("Event matching '{description}' not found");
        if !events_string.is_empty() {
            failure_message.push_str("; found these though: ");
            failure_message.push_str(&events_string);
        }
        panic!("{failure_message}");
    }

    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:353 MoreAsserts.assertDoesNotContainEvent
    pub fn assert_does_not_contain_event(&self, unexpected_event: &str) {
        for event in self.event_collector.iter() {
            let message = event.get_message();
            assert!(
                !message.contains(unexpected_event),
                "Unexpected string '{unexpected_event}' matched following event:\n{message}"
            );
        }
    }

    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:380 MoreAsserts.assertContainsEventWithFrequency
    pub fn assert_contains_event_with_frequency(
        &self,
        expected_message: &str,
        expected_frequency: usize,
    ) {
        let count = self
            .event_collector
            .iter()
            .filter(|e| e.get_message().contains(expected_message))
            .count();
        assert_eq!(
            count,
            expected_frequency,
            "Expected '{expected_message}' to appear {expected_frequency} times; events: {}",
            self.events_to_string()
        );
    }

    /// `assertContainsEventsInOrder`.
    // Java: com/google/devtools/build/lib/testutil/MoreAsserts.java:400 MoreAsserts.assertContainsEventsInOrder
    pub fn assert_contains_events_in_order(&self, expected_messages: &[&str]) {
        let events: Vec<String> = self
            .event_collector
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect();
        let mut i = 0;
        for message in &events {
            if i < expected_messages.len() && message.contains(expected_messages[i]) {
                i += 1;
            }
        }
        assert_eq!(
            i,
            expected_messages.len(),
            "Expected {expected_messages:?} in order; events: {events:?}"
        );
    }

    /// `reporter.removeHandler(failFastHandler)`.
    pub fn remove_fail_fast_handler(&self) {
        self.reporter.remove_handler(&self.fail_fast_handler);
    }

    // -----------------------------------------------------------------------------------------
    // Configuration.

    /// `TestConstants.PRODUCT_SPECIFIC_FLAGS` (`BuildViewTestCase.getDefaultsForConfiguration`)
    /// or `AnalysisTestCase.defaultFlags()`'s options.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:632 BuildViewTestCase.getDefaultsForConfiguration
    fn get_defaults_for_configuration(&self) -> Vec<String> {
        match self.kind {
            TestCaseKind::BuildView => PRODUCT_SPECIFIC_FLAGS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            TestCaseKind::Analysis => {
                // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:291 AnalysisTestCase.useConfiguration
                let mut args = Vec::new();
                if self.default_flags.contains(Flag::PublicVisibility) {
                    args.push("--default_visibility=public".to_string());
                }
                if self.default_flags.contains(Flag::CpuK8) {
                    args.push("--cpu=k8".to_string());
                    args.push("--host_cpu=k8".to_string());
                }
                if self.default_flags.contains(Flag::ProductSpecificFlags) {
                    args.extend(PRODUCT_SPECIFIC_FLAGS.iter().map(|s| s.to_string()));
                }
                args
            }
        }
    }

    /// `AnalysisTestCase.defaultFlags()`; a test class that overrides it (Java
    /// `StarlarkDefinedAspectsTest.WithKeepGoing`) calls [`Self::set_default_flags`].
    pub fn default_flags(&self) -> FlagBuilder {
        self.default_flags.clone()
    }

    /// Overrides `getDefaultVisibility()` (Java: a test class overriding the method).
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:554 BuildViewTestCase.getDefaultVisibility
    pub fn set_default_visibility(&mut self, visibility: &str) {
        self.default_visibility = visibility.to_string();
        self.invalidate_executor();
    }

    pub fn set_default_flags(&mut self, flags: FlagBuilder) {
        self.default_flags = flags;
        self.invalidate_executor();
    }

    /// Sets exec and target configuration using the specified options, falling back to the
    /// default options for unspecified ones, and recreates the build view.
    ///
    /// PORT: the configurations are created with the next evaluator (module documentation).
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:646 BuildViewTestCase.useConfiguration
    pub fn use_configuration(&mut self, args: &[&str]) {
        self.configuration_args = args.iter().map(|s| s.to_string()).collect();
        if !self.scratch.resolve("platform/BUILD").exists() {
            self.scratch
                .overwrite_file("platform/BUILD", &["platform(name = 'exec')"]);
        }
        self.invalidate_executor();
        if self.kind == TestCaseKind::BuildView {
            // Java creates the configurations here.
            self.ensure_executor();
        }
    }

    /// `setBuildLanguageOptions`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:534 BuildViewTestCase.setBuildLanguageOptions
    pub fn set_build_language_options(&mut self, options: &[&str]) {
        self.build_language_options = options.iter().map(|s| s.to_string()).collect();
        self.invalidate_executor();
    }

    /// `setPackageOptions`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:527 BuildViewTestCase.setPackageOptions
    pub fn set_package_options(&mut self, options: &[&str]) {
        self.package_options = options.iter().map(|s| s.to_string()).collect();
        self.invalidate_executor();
    }

    /// `useLoadingOptions`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1944 BuildViewTestCase.useLoadingOptions
    pub fn use_loading_options(&mut self, options: &[&str]) {
        self.loading_options = options.iter().map(|s| s.to_string()).collect();
    }

    /// Invalidates all existing packages and configurations.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:593 BuildViewTestCase.invalidatePackages
    pub fn invalidate_packages(&mut self) {
        self.invalidate_executor();
    }

    /// Rewrites the MODULE.bazel file and reinitializes everything.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1220 BuildViewTestCase.rewriteModuleDotBazel
    pub fn rewrite_module_dot_bazel(&mut self, lines: &[&str]) {
        self.scratch.overwrite_file("MODULE.bazel", lines);
        self.invalidate_packages();
    }

    fn invalidate_executor(&mut self) {
        if let Some(state) = self.state.take() {
            state.executor.after_command();
        }
    }

    /// The option classes of the test's options parser: Java `AnalysisTestCase.useConfiguration`'s
    /// list, the fragment options, and `RepositoryOptions` (Java injects the repository
    /// precomputed values directly).
    fn options_classes() -> Vec<OptionsClassRef> {
        let mut classes: Vec<OptionsClassRef> = vec![
            ExecutionOptions::class(),
            PackageOptions::class(),
            BuildLanguageOptions::class(),
            BuildRequestOptions::class(),
            AnalysisOptions::class(),
            KeepGoingOption::class(),
            LoadingPhaseThreadsOption::class(),
            LoadingOptions::class(),
            RepositoryOptions::class(),
        ];
        for class in BazelRuleClassProvider::create().get_options_classes() {
            let class = class.options_class();
            if !classes.iter().any(|c| std::ptr::eq(*c, class)) {
                classes.push(class);
            }
        }
        classes
    }

    /// Parses the options of the current state.
    fn parse_options(&self) -> OptionsParser {
        let mut parser = OptionsParser::builder()
            .options_classes(&Self::options_classes())
            .skip_starlark_option_prefixes()
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        let parse = |parser: &mut OptionsParser, args: &[String]| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            parser
                .parse(&args)
                .unwrap_or_else(|e| panic!("OptionsParsingException: {}", e.get_message()));
        };
        // PORT: Java's AnalysisMock.getPrecomputedValues and the FakeRegistry (module
        // documentation): the registry is the empty `modules` directory, the repository cache
        // and the repo contents cache are off the workspace.
        parse(
            &mut parser,
            &[
                format!("--registry=file://{}", self.module_root.get_path_string()),
                format!(
                    "--repository_cache={}",
                    self.directory
                        .root
                        .get_child("repository_cache")
                        .get_path_string()
                ),
                "--repo_contents_cache=".to_string(),
                "--lockfile_mode=update".to_string(),
                "--check_direct_dependencies=warning".to_string(),
            ],
        );
        // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:558 BuildViewTestCase.parsePackageOptions
        if self.kind == TestCaseKind::BuildView {
            parse(
                &mut parser,
                &[format!("--default_visibility={}", self.default_visibility)],
            );
        }
        parse(&mut parser, &self.package_options);
        // getDefaultBuildLanguageOptions
        parse(
            &mut parser,
            &PRODUCT_SPECIFIC_BUILD_LANG_OPTIONS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        );
        parse(&mut parser, &self.build_language_options);
        parse(&mut parser, &self.get_defaults_for_configuration());
        parse(&mut parser, &self.configuration_args);
        if !parser.get_skipped_args().is_empty() {
            blaze_util::unported!(
                "StarlarkOptionsParser.parse",
                "Starlark flags in useConfiguration: {:?}",
                parser.get_skipped_args()
            );
        }
        parser
    }

    /// Builds the executor and the configurations of the current state if there is none or a
    /// scratch file changed since.
    fn ensure_executor(&mut self) {
        if self.scratch.take_modified() {
            self.invalidate_executor();
        }
        if self.state.is_some() {
            return;
        }
        let options = Arc::new(self.parse_options());
        // Java: createBuildOptions (createBuildOptionsForTesting).
        let build_options = Arc::new(blaze_build_config::top_level::create_build_options(
            &BazelRuleClassProvider::create(),
            &options,
        ));
        let inputs = SkyframeExecutorInputs {
            workspace: self.root_directory.clone(),
            working_directory: self.root_directory.clone(),
            output_base: self.output_base.clone(),
            // ServerDirectories(rootDirectory, outputBase, outputBase): the install base is the
            // workspace.
            install_base: self.root_directory.clone(),
            exec_root_base: self.output_base.get_child("execroot"),
            product_name: self.analysis_mock.get_product_name().to_string(),
            workspace_name: "_main".to_string(),
            options: options.clone(),
            reporter: self.event_handler(),
            client_env: IndexMap::new(),
            repo_env: IndexMap::new(),
            nonstrict_repo_env: IndexMap::new(),
            repo_precomputed: Default::default(),
            keep_going: false,
            top_level_build_options: Some((*build_options).clone()),
            execution: None,
        };
        let executor = SkyframeExecutor::create_with_builtin_modules(
            inputs,
            None,
            Some(self.analysis_mock.get_builtin_modules(&self.root_directory)),
        )
        .unwrap_or_else(|e| panic!("cannot create the SkyframeExecutor: {e}"));
        // Java: createBuildOptionsForTesting evaluates the main repository mapping.
        if let Err(e) =
            executor.get_main_repo_mapping(false, LOADING_PHASE_THREADS, self.event_handler())
        {
            panic!("InvalidConfigurationException: Cannot find main repository mapping: {e:?}");
        }
        // Java: createConfiguration.
        let target_config = executor
            .create_configuration(self.event_handler(), build_options.clone(), false)
            .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()));
        // Java: AnalysisTestUtil.execOptions.
        let exec_options = exec_options(target_config.get_options(), self.event_handler());
        let exec_config = executor
            .get_configuration(self.event_handler(), Arc::new(exec_options), false)
            .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()));
        // Java: createBuildView's precondition.
        blaze_util::check_state!(
            Arc::ptr_eq(&exec_config, &target_config) || exec_config.is_exec_configuration(),
            "Exec configuration is not an exec configuration"
        );
        self.state = Some(ExecutorState {
            executor,
            target_config,
            exec_config,
            build_options,
            options,
        });
        self.check_fail_fast();
    }

    fn state(&mut self) -> &ExecutorState {
        self.ensure_executor();
        self.state.as_ref().expect("executor state")
    }

    /// `getSkyframeExecutor()`.
    pub fn skyframe_executor(&mut self) -> &SkyframeExecutor {
        &self.state().executor
    }

    /// `getTargetConfiguration()`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1924 BuildViewTestCase.getTargetConfiguration
    pub fn get_target_configuration(&mut self) -> Arc<BuildConfigurationValue> {
        self.state().target_config.clone()
    }

    /// `getExecConfiguration()`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1928 BuildViewTestCase.getExecConfiguration
    pub fn get_exec_configuration(&mut self) -> Arc<BuildConfigurationValue> {
        self.state().exec_config.clone()
    }

    /// `getConfiguration(ConfiguredTarget)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1940 BuildViewTestCase.getConfiguration
    pub fn get_configuration(
        &mut self,
        ct: &ConfiguredTarget,
    ) -> Option<Arc<BuildConfigurationValue>> {
        let handler = self.event_handler();
        let key = ct.get_configuration_key()?;
        self.state()
            .executor
            .get_configuration_for_key(handler, &key)
    }

    /// `createConfiguration(args...)`: a configuration for the given options (with the current
    /// defaults), in the current evaluator.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:453 BuildViewTestCase.createConfiguration
    pub fn create_configuration(&mut self, args: &[&str]) -> Arc<BuildConfigurationValue> {
        let saved = std::mem::replace(
            &mut self.configuration_args,
            args.iter().map(|s| s.to_string()).collect(),
        );
        let options = self.parse_options();
        self.configuration_args = saved;
        let build_options = Arc::new(blaze_build_config::top_level::create_build_options(
            &BazelRuleClassProvider::create(),
            &options,
        ));
        let handler = self.event_handler();
        let result = self
            .state()
            .executor
            .get_configuration(handler, build_options, false)
            .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()));
        self.check_fail_fast();
        result
    }

    /// The parsed options of the current state (`AnalysisTestCase.optionsParser`).
    pub fn get_options(&mut self) -> Arc<OptionsParser> {
        self.state().options.clone()
    }

    /// `AnalysisTestCase.buildOptions`.
    pub fn get_build_options(&mut self) -> Arc<BuildOptions> {
        self.state().build_options.clone()
    }

    // -----------------------------------------------------------------------------------------
    // Loading.

    /// `getTarget(label)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:468 BuildViewTestCase.getTarget
    pub fn get_target(&mut self, label: &str) -> Result<Target, NoSuchThingOrInterrupted> {
        let label = parse_label(label);
        let handler = self.event_handler();
        let result = self
            .state()
            .executor
            .get_package_manager()
            .get_target(handler, &label);
        self.check_fail_fast();
        result
    }

    /// Checks that loading the given target fails with the expected error message.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:489 BuildViewTestCase.assertTargetError
    pub fn assert_target_error(&mut self, label: &str, expected_error: &str) {
        if self.get_target(label).is_ok() {
            panic!("Expected loading phase failure for target {label}");
        }
        self.assert_contains_event(expected_error);
    }

    /// Create and return a scratch rule.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1292 BuildViewTestCase.scratchRule
    pub fn scratch_rule(&mut self, package_name: &str, rule_name: &str, lines: &[&str]) -> Target {
        // Allow to create the BUILD file also in the top package.
        let build_file_path_string = if package_name.is_empty() {
            "BUILD".to_string()
        } else {
            format!("{package_name}/BUILD")
        };
        self.scratch.file(&build_file_path_string, lines);
        // PORT: invalidateFilesUnderPathForTesting: the scratch write rebuilds the evaluator.
        self.get_target(&format!("//{package_name}:{rule_name}"))
            .unwrap_or_else(|e| panic!("{e:?}"))
    }

    // -----------------------------------------------------------------------------------------
    // Configured targets.

    /// `getConfiguredTarget(label)`, in the target configuration.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1081 BuildViewTestCase.getConfiguredTarget
    pub fn get_configured_target(&mut self, label: &str) -> Option<ConfiguredTarget> {
        let config = self.get_target_configuration();
        self.get_configured_target_in(label, Some(&config))
    }

    /// `getConfiguredTarget(label, config)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1113 BuildViewTestCase.getConfiguredTarget
    pub fn get_configured_target_in(
        &mut self,
        label: &str,
        config: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<ConfiguredTarget> {
        self.get_configured_target_and_data_in(label, config)
            .map(|ctad| ctad.get_configured_target().clone())
    }

    /// `getConfiguredTargetAndData(label)`, in the target configuration.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1137 BuildViewTestCase.getConfiguredTargetAndData
    pub fn get_configured_target_and_data(
        &mut self,
        label: &str,
    ) -> Option<ConfiguredTargetAndData> {
        let config = self.get_target_configuration();
        self.get_configured_target_and_data_in(label, Some(&config))
    }

    /// `getConfiguredTargetAndData(label, config)` (Java
    /// `SkyframeExecutor.getConfiguredTargetAndDataForTesting`).
    ///
    /// PORT: evaluates the `ConfiguredTargetKey` and wraps the value as
    /// `ConfiguredTargetAndDataProducer` does (module documentation).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4532 SkyframeExecutor.getConfiguredTargetAndDataForTesting
    pub fn get_configured_target_and_data_in(
        &mut self,
        label: &str,
        config: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<ConfiguredTargetAndData> {
        let label = parse_label(label);
        let key = ConfiguredTargetKey::builder()
            .set_label(label.clone())
            .set_configuration(config.map(|c| c.as_ref()))
            .build();
        let handler = self.event_handler();
        let state = self.state();
        let executor = &state.executor;
        let result = executor
            .evaluate(
                &[SkyKey::new(key.clone())],
                false,
                DEFAULT_THREAD_COUNT,
                handler.clone(),
            )
            .unwrap_or_else(|_| panic!("InterruptedException"));
        let ctad = result.get(&key).map(|value: Arc<ConfiguredTargetValue>| {
            let configured_target = value.get_configured_target().clone();
            let target = match value.get_target() {
                Some(t) => t.clone(),
                None => executor
                    .get_package_manager()
                    .get_target(handler.clone(), &configured_target.get_original_label())
                    .unwrap_or_else(|e| panic!("{e:?}")),
            };
            let configuration = get_configuration_from_graph(
                &*executor.get_walkable_graph(),
                configured_target.get_configuration_key().as_ref(),
            );
            ConfiguredTargetAndData::new(configured_target, target, configuration, Vec::new())
        });
        if result.has_error() {
            let _ = crate::skyframe_error_processor::process_analysis_errors(
                &result,
                executor.get_cycles_reporter(),
                &handler,
                /* keep_going= */ true,
            );
        }
        self.check_fail_fast();
        ctad
    }

    /// `getExecConfiguredTarget(label)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1169 BuildViewTestCase.getExecConfiguredTarget
    pub fn get_exec_configured_target(&mut self, label: &str) -> Option<ConfiguredTarget> {
        let config = self.get_exec_configuration();
        self.get_configured_target_in(label, Some(&config))
    }

    /// `getFileConfiguredTarget(label)`: the artifact of a file target.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1149 BuildViewTestCase.getFileConfiguredTarget
    pub fn get_file_configured_target(&mut self, label: &str) -> ConfiguredTarget {
        let ct = self
            .get_configured_target(label)
            .unwrap_or_else(|| panic!("no configured target {label}"));
        assert!(
            matches!(
                ct,
                ConfiguredTarget::InputFile(_) | ConfiguredTarget::OutputFile(_)
            ),
            "ClassCastException: {label} is not a FileConfiguredTarget"
        );
        ct
    }

    /// `getArtifact(label)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1157 BuildViewTestCase.getArtifact
    pub fn get_artifact(&mut self, label: &str) -> Artifact {
        let target = self
            .get_configured_target(label)
            .unwrap_or_else(|| panic!("no configured target {label}"));
        let files = get_files_to_build(&target);
        files.get_singleton().clone()
    }

    /// `scratchConfiguredTarget(packageName, ruleName, lines...)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1233 BuildViewTestCase.scratchConfiguredTarget
    pub fn scratch_configured_target(
        &mut self,
        package_name: &str,
        rule_name: &str,
        lines: &[&str],
    ) -> Option<ConfiguredTarget> {
        self.scratch_configured_target_and_data(package_name, rule_name, lines)
            .map(|ctad| ctad.get_configured_target().clone())
    }

    /// `scratchConfiguredTargetAndData(packageName, ruleName, lines...)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1263 BuildViewTestCase.scratchConfiguredTargetAndData
    pub fn scratch_configured_target_and_data(
        &mut self,
        package_name: &str,
        rule_name: &str,
        lines: &[&str],
    ) -> Option<ConfiguredTargetAndData> {
        let rule = self.scratch_rule(package_name, rule_name, lines);
        let config = self.get_target_configuration();
        self.get_configured_target_and_data_in(
            &rule.get_label().get_canonical_form(),
            Some(&config),
        )
    }

    /// `view.hasErrors(target)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewForTesting.java:210 BuildViewForTesting.hasErrors
    pub fn has_errors(configured_target: Option<&ConfiguredTarget>) -> bool {
        configured_target.is_none()
    }

    /// Check that configuration of the target named 'ruleName' in the specified BUILD file fails
    /// with an error message containing 'expectedErrorMessage'.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1314 BuildViewTestCase.checkError
    pub fn check_error(
        &mut self,
        package_name: &str,
        rule_name: &str,
        expected_error_message: &str,
        lines: &[&str],
    ) -> Event {
        self.event_collector.clear();
        self.remove_fail_fast_handler(); // expect errors
        let target = self.scratch_configured_target(package_name, rule_name, lines);
        if target.is_some() {
            assert!(
                Self::has_errors(target.as_ref()),
                "Rule '//{package_name}:{rule_name}' did not contain an error"
            );
        }
        self.assert_contains_event(expected_error_message)
    }

    /// Check that configuration of the target named 'label' fails with an error message
    /// containing 'expectedErrorMessage'.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1362 BuildViewTestCase.checkError
    pub fn check_error_for_label(&mut self, label: &str, expected_error_message: &str) -> Event {
        self.event_collector.clear();
        self.remove_fail_fast_handler(); // expect errors
        let target = self.get_configured_target(label);
        if target.is_some() {
            assert!(
                Self::has_errors(target.as_ref()),
                "Rule '{label}' did not contain an error"
            );
        }
        self.assert_contains_event(expected_error_message)
    }

    /// Checks whether loading the given target results in the specified error message.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1380 BuildViewTestCase.checkLoadingPhaseError
    pub fn check_loading_phase_error(&mut self, target: &str, expected_error_message: &str) {
        self.remove_fail_fast_handler();
        // The error happens during the loading of the Starlark file so checkError doesn't work
        // here
        assert!(
            self.get_target(target).is_err(),
            "expected an exception loading {target}"
        );
        self.assert_contains_event(expected_error_message);
    }

    /// Check that configuration of the target named 'ruleName' in the specified BUILD file
    /// reports a warning message ending in 'expectedWarningMessage', and that no errors were
    /// reported.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1397 BuildViewTestCase.checkWarning
    pub fn check_warning(
        &mut self,
        package_name: &str,
        rule_name: &str,
        expected_warning_message: &str,
        lines: &[&str],
    ) -> Event {
        self.event_collector.clear();
        let target = self.scratch_configured_target(package_name, rule_name, lines);
        assert!(
            !Self::has_errors(target.as_ref()),
            "Rule '//{package_name}:{rule_name}' did contain an error"
        );
        self.assert_contains_event(expected_warning_message)
    }

    /// `getExecutable(label)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1892 BuildViewTestCase.getExecutable
    pub fn get_executable_of(&mut self, label: &str) -> Option<Artifact> {
        let target = self
            .get_configured_target(label)
            .unwrap_or_else(|| panic!("no configured target {label}"));
        get_executable(&target)
    }

    /// `getFilesToRun(label)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1904 BuildViewTestCase.getFilesToRun
    pub fn get_files_to_run_of(&mut self, label: &str) -> NestedSet<Artifact> {
        let target = self
            .get_configured_target(label)
            .unwrap_or_else(|| panic!("no configured target {label}"));
        get_files_to_run(&target)
    }

    /// `getKnownConfigurations(label)`: the configurations in which the label was configured.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1183 BuildViewTestCase.getKnownConfigurations
    pub fn get_known_configurations(
        &mut self,
        label: &str,
    ) -> Vec<Option<blaze_build_config::build_configuration_key::BuildConfigurationKey>> {
        let parsed = parse_label(label);
        let evaluator = self.state().executor.get_evaluator().clone();
        let mut cts = Vec::new();
        for key in evaluator.get_all_keys_for_testing() {
            if key.function_name() != ConfiguredTargetKey::FUNCTION
                || evaluator.get_existing_value(&key).is_none()
            {
                continue;
            }
            let Some(ct_key) = key.downcast::<ConfiguredTargetKey>() else {
                continue;
            };
            if ct_key.get_label() == &parsed {
                let config = ct_key.get_configuration_key().cloned();
                if !cts.contains(&config) {
                    cts.push(config);
                }
            }
        }
        cts
    }

    // -----------------------------------------------------------------------------------------
    // Actions.

    /// The actions registered by `label`'s configured target (in the target configuration).
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1874 BuildViewTestCase.getActions
    pub fn get_actions(&mut self, label: &str) -> Vec<blaze_actions::ActionRef> {
        let config = self.get_target_configuration();
        let key = ConfiguredTargetKey::builder()
            .set_label(parse_label(label))
            .set_configuration(Some(config.as_ref()))
            .build();
        self.get_configured_target(label)
            .unwrap_or_else(|| panic!("no configured target {label}"));
        let value = crate::skyframe_executor::get_configured_target_value(
            &*self.state().executor.get_walkable_graph(),
            &SkyKey::new(key),
        )
        .expect("configured target value");
        value.get_actions().to_vec()
    }

    /// `getActions(label, mnemonic)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1866 BuildViewTestCase.getActions
    pub fn get_actions_with_mnemonic(
        &mut self,
        label: &str,
        mnemonic: &str,
    ) -> Vec<blaze_actions::ActionRef> {
        self.get_actions(label)
            .into_iter()
            .filter(|a| a.get_mnemonic() == mnemonic)
            .collect()
    }

    /// `getGeneratingAction(artifact)`: the action of the artifact's owner that has it as an
    /// output.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1001 BuildViewTestCase.getGeneratingAction
    pub fn get_generating_action(
        &mut self,
        artifact: &Artifact,
    ) -> Option<blaze_actions::ActionRef> {
        if artifact.is_source_artifact() {
            return None;
        }
        let blaze_actions::actions::action_lookup::ArtifactOwner::ActionLookupKey(owner) =
            artifact.get_artifact_owner()
        else {
            return None;
        };
        // PORT: only configured targets own actions until aspects are ported.
        let key = SkyKey::new(
            owner
                .0
                .as_any()
                .downcast_ref::<ConfiguredTargetKey>()
                .unwrap_or_else(|| unported_owner(&owner))
                .clone(),
        );
        let graph = self.state().executor.get_walkable_graph();
        let value = graph.get_value(&key)?;
        let value = value.downcast::<ConfiguredTargetValue>().ok()?;
        value
            .get_actions()
            .iter()
            .find(|a| a.get_outputs().iter().any(|o| o == artifact))
            .cloned()
    }

    /// `getGeneratingAction(target, outputName)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:991 BuildViewTestCase.getGeneratingAction
    pub fn get_generating_action_for_output(
        &mut self,
        target: &ConfiguredTarget,
        output_name: &str,
    ) -> Option<blaze_actions::ActionRef> {
        let artifact = get_files_to_build(target)
            .to_list()
            .iter()
            .find(|a| {
                a.get_root_relative_path()
                    .get_path_string()
                    .ends_with(output_name)
            })
            .cloned()?;
        self.get_generating_action(&artifact)
    }

    // -----------------------------------------------------------------------------------------
    // update (AnalysisTestCase / BuildViewTestCase).

    /// `AnalysisTestCase.update(labels...)` with `defaultFlags()`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:470 AnalysisTestCase.update
    pub fn update(&mut self, labels: &[&str]) -> Result<&AnalysisResult, BuildViewError> {
        let flags = self.default_flags();
        self.update_with(&flags, &[], labels)
    }

    /// `AnalysisTestCase.update(aspects, labels...)`.
    ///
    /// PORT: top-level aspects are not ported; a non-empty `aspects` fails loudly in
    /// `BuildView.update` (`BuildView.createTopLevelAspectKeys`) until the aspects wave lands.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:474 AnalysisTestCase.update
    pub fn update_with_aspects(
        &mut self,
        aspects: &[&str],
        labels: &[&str],
    ) -> Result<&AnalysisResult, BuildViewError> {
        let flags = self.default_flags();
        self.update_with(&flags, aspects, labels)
    }

    /// `AnalysisTestCase.update(aspects, aspectsParameters, labels...)`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:478 AnalysisTestCase.update
    pub fn update_with_aspects_parameters(
        &mut self,
        aspects: &[&str],
        aspects_parameters: &[(&str, &str)],
        labels: &[&str],
    ) -> Result<&AnalysisResult, BuildViewError> {
        let flags = self.default_flags();
        let aspects_parameters: IndexMap<String, String> = aspects_parameters
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.update_with_parameters(&flags, aspects, &aspects_parameters, labels)
    }

    /// `AnalysisTestCase.update(eventBus, config, explicitTargetPatterns, aspects,
    /// aspectsParameters, labels...)`.
    ///
    /// PORT: `aspectsParameters` and `explicitTargetPatterns` are always empty in the callers
    /// ported so far; the event bus is the reporter's (none).
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:364 AnalysisTestCase.update
    pub fn update_with(
        &mut self,
        config: &FlagBuilder,
        aspects: &[&str],
        labels: &[&str],
    ) -> Result<&AnalysisResult, BuildViewError> {
        self.update_with_parameters(config, aspects, &IndexMap::new(), labels)
    }

    /// `AnalysisTestCase.update(eventBus, config, explicitTargetPatterns, aspects,
    /// aspectsParameters, labels...)` with `aspectsParameters`.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:364 AnalysisTestCase.update
    fn update_with_parameters(
        &mut self,
        config: &FlagBuilder,
        aspects: &[&str],
        aspects_parameters: &IndexMap<String, String>,
        labels: &[&str],
    ) -> Result<&AnalysisResult, BuildViewError> {
        let keep_going = config.contains(Flag::KeepGoing);
        // Java: update(String...) posts to a new EventBus.
        self.event_bus.clear();
        // Java: invalidateFilesUnderPathForTesting(EVERYTHING_MODIFIED).
        self.invalidate_executor();
        self.analysis_result = None;
        let result = self.update_internal(
            &labels.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            aspects,
            aspects_parameters,
            keep_going,
            LOADING_PHASE_THREADS,
        );
        self.check_fail_fast();
        let result = result?;
        self.analysis_result = Some(result);
        Ok(self.analysis_result.as_ref().expect("analysis result"))
    }

    /// `BuildViewTestCase.update(targets, aspects, keepGoing, loadingPhaseThreads, doAnalysis,
    /// eventBus)`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1970 BuildViewTestCase.update
    pub fn update_targets(
        &mut self,
        targets: &[&str],
        aspects: &[&str],
        keep_going: bool,
        loading_phase_threads: usize,
    ) -> Result<&AnalysisResult, BuildViewError> {
        let result = self.update_internal(
            &targets.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            aspects,
            &IndexMap::new(),
            keep_going,
            loading_phase_threads,
        );
        self.check_fail_fast();
        let result = result?;
        self.analysis_result = Some(result);
        Ok(self.analysis_result.as_ref().expect("analysis result"))
    }

    fn update_internal(
        &mut self,
        targets: &[String],
        aspects: &[&str],
        aspects_parameters: &IndexMap<String, String>,
        keep_going: bool,
        loading_phase_threads: usize,
    ) -> Result<AnalysisResult, BuildViewError> {
        let loading_options: LoadingOptions = if self.loading_options.is_empty() {
            Options::<LoadingOptions>::get_defaults()
        } else {
            let mut parser = OptionsParser::builder()
                .options_classes(&[LoadingOptions::class()])
                .build()
                .unwrap_or_else(|e| panic!("{}", e.get_message()));
            let args: Vec<&str> = self.loading_options.iter().map(String::as_str).collect();
            parser
                .parse(&args)
                .unwrap_or_else(|e| panic!("{}", e.get_message()));
            parser
                .get_options::<LoadingOptions>()
                .expect("LoadingOptions")
        };
        let analysis_options: AnalysisOptions = Options::<AnalysisOptions>::get_defaults();
        let handler = self.event_handler();
        let state = self.state();
        let loading_result = match state.executor.load_target_patterns_with_filters(
            handler.clone(),
            targets,
            &PathFragment::create(""),
            &loading_options,
            loading_phase_threads,
            keep_going,
            /* determine_tests= */ false,
        ) {
            Ok(v) => v,
            Err(crate::skyframe_executor::TargetPatternPhaseError::TargetParsing(e)) => {
                return Err(BuildViewError::ViewCreationFailed {
                    message: e.get_message().to_string(),
                    failure_detail: Default::default(),
                });
            }
            Err(crate::skyframe_executor::TargetPatternPhaseError::Interrupted) => {
                return Err(BuildViewError::Interrupted);
            }
        };
        let options = BuildViewUpdateOptions {
            keep_going,
            check_for_action_conflicts: true,
            aspects: aspects.iter().map(|s| s.to_string()).collect(),
            explicit_target_patterns: IndexSet::new(),
            skip_incompatible_explicit_targets: analysis_options.skip_incompatible_explicit_targets,
            report_incompatible_targets: true,
            include_execution_phase: false,
        };
        let build_options = state.target_config.get_options().clone();
        // Java passes AnalysisTestUtil.TOP_LEVEL_ARTIFACT_CONTEXT (output groups
        // OutputGroupInfo.DEFAULT_GROUPS) to BuildView.update as `topLevelOptions`.
        // PORT: Blaze's BuildView.update reads it from the executor (see
        // `SkyframeExecutor::set_top_level_artifact_context_for_conflict_pruning`).
        // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestUtil.java:82 AnalysisTestUtil.TOP_LEVEL_ARTIFACT_CONTEXT
        state
            .executor
            .set_top_level_artifact_context_for_conflict_pruning(
                blaze_analysis::providers::output_group_info::default_groups()
                    .into_iter()
                    .collect(),
            );
        build_view::update_with_aspects_parameters(
            &state.executor,
            &loading_result,
            &build_options,
            &options,
            aspects_parameters,
            handler,
        )
    }

    /// `AnalysisTestCase.getAnalysisResult()`.
    pub fn get_analysis_result(&self) -> &AnalysisResult {
        self.ensure_update_was_called();
        self.analysis_result.as_ref().expect("analysis result")
    }

    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:356 AnalysisTestCase.ensureUpdateWasCalled
    pub fn ensure_update_was_called(&self) {
        blaze_util::check_state!(
            self.analysis_result.is_some(),
            "You must run update() first!"
        );
    }

    /// The evaluator of the current state (Java `skyframeExecutor.getEvaluator()`).
    pub fn get_evaluator(&mut self) -> Arc<blaze_skyframe::Evaluator> {
        self.state().executor.get_evaluator().clone()
    }

    /// Evaluates `keys` in the current evaluator (Java `SkyframeExecutorTestUtils.evaluate`).
    pub fn evaluate(&mut self, keys: &[SkyKey], keep_going: bool) -> EvaluationResult {
        let handler = self.event_handler();
        let result = self
            .state()
            .executor
            .evaluate(keys, keep_going, DEFAULT_THREAD_COUNT, handler)
            .unwrap_or_else(|_| panic!("InterruptedException"));
        self.check_fail_fast();
        result
    }

    /// `AnalysisTestCase.getStarlarkProvider(ConfiguredTarget target, String providerSymbol)`:
    /// the provider defined by the .bzl file of the target's rule class.
    ///
    /// PORT: Java overloads `getStarlarkProvider`; this overload is named after its argument.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:702 AnalysisTestCase.getStarlarkProvider
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:882 BuildViewTestCase.getStarlarkProvider
    pub fn get_starlark_provider_by_symbol(
        &mut self,
        target: &ConfiguredTarget,
        provider_symbol: &str,
    ) -> Option<starlark::values::OwnedFrozenValue> {
        let rule_target = self
            .get_target(&target.get_label().to_string())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let rule = rule_target
            .get_associated_rule()
            .expect("NullPointerException: getAssociatedRule")
            .clone();
        let bzl = rule
            .get_rule_class_object()
            .get_rule_definition_environment_label()
            .expect("NullPointerException: getRuleDefinitionEnvironmentLabel")
            .to_string();
        Self::get_starlark_provider(target, &bzl, provider_symbol)
    }

    /// `AnalysisTestCase.getStarlarkProvider(ProviderCollection target, String label, String
    /// providerSymbol)` for an aspect.
    // Java: com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:715 AnalysisTestCase.getStarlarkProvider
    pub fn get_starlark_provider_of_aspect(
        aspect: &AspectValue,
        bzl_label: &str,
        provider_symbol: &str,
    ) -> Option<starlark::values::OwnedFrozenValue> {
        let key = blaze_packages::rule_class::ProviderKey::Starlark {
            extension_label: parse_label(bzl_label),
            exported_name: provider_symbol.to_string(),
        };
        aspect.get_providers().get(&key).cloned()
    }

    /// `BuildViewTestCase.getAspect(label)`: the only done aspect node whose aspect name is
    /// `label`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1205 BuildViewTestCase.getAspect
    pub fn get_aspect(&mut self, label: &str) -> Arc<AspectValue> {
        let ev = self.get_evaluator();
        let values: Vec<Arc<AspectValue>> = ev
            .get_all_keys_for_testing()
            .into_iter()
            .filter(|k| {
                k.downcast::<AspectKey>()
                    .is_some_and(|a| a.get_aspect_name() == label)
            })
            .filter_map(|k| ev.get_existing_value(&k))
            .map(|v| {
                v.downcast::<AspectValue>()
                    .unwrap_or_else(|_| panic!("ClassCastException: AspectValue"))
            })
            .collect();
        assert_eq!(
            values.len(),
            1,
            "onlyElement: {} aspect values named {label}",
            values.len()
        );
        values.into_iter().next().unwrap()
    }

    /// `BuildViewTestCase.getDirectPrerequisites(target)` (Java
    /// `BuildViewForTesting.getDirectPrerequisitesForTesting`).
    ///
    /// PORT: Java re-resolves the target's dependencies with `DependencyResolver`; here they are
    /// the configured targets of the `ConfiguredTargetKey` edges of the target's node in the
    /// graph, in edge order.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:744 BuildViewTestCase.getDirectPrerequisites
    pub fn get_direct_prerequisites(&mut self, target: &ConfiguredTarget) -> Vec<ConfiguredTarget> {
        let key = SkyKey::new(
            ConfiguredTargetKey::builder()
                .set_label(target.get_label())
                .set_configuration_key(target.get_configuration_key())
                .build(),
        );
        let graph = self.skyframe_executor().get_walkable_graph();
        let mut result = Vec::new();
        for dep in graph.get_direct_deps(&key) {
            if dep.downcast::<ConfiguredTargetKey>().is_none() {
                continue;
            }
            if let Some(value) =
                crate::skyframe_executor::get_configured_target_value(&*graph, &dep)
            {
                result.push(value.get_configured_target().clone());
            }
        }
        result
    }

    /// `BuildViewTestCase.getDirectPrerequisite(target, label)`: the first direct prerequisite
    /// whose original label is `label`.
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:753 BuildViewTestCase.getDirectPrerequisite
    pub fn get_direct_prerequisite(
        &mut self,
        target: &ConfiguredTarget,
        label: &str,
    ) -> Option<ConfiguredTarget> {
        let candidate_label = parse_label(label);
        self.get_direct_prerequisites(target)
            .into_iter()
            .find(|candidate| candidate.get_original_label() == candidate_label)
    }

    /// `getStarlarkProvider(target, providerSymbol)` with the provider's key (`load(bzl,
    /// symbol)` in Java: the caller builds the `StarlarkProvider.Key`).
    // Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:882 BuildViewTestCase.getStarlarkProvider
    pub fn get_starlark_provider(
        target: &ConfiguredTarget,
        bzl_label: &str,
        provider_symbol: &str,
    ) -> Option<starlark::values::OwnedFrozenValue> {
        let key = blaze_packages::rule_class::ProviderKey::Starlark {
            extension_label: parse_label(bzl_label),
            exported_name: provider_symbol.to_string(),
        };
        target.get(&key)
    }
}

/// `AnalysisTestUtil.execOptions(targetOptions, skyframeExecutor, handler)`.
///
/// PORT: the Starlark flags' scope step (`BuildOptionsScopeValue`) is only needed with Starlark
/// options, which the harness does not parse yet (module documentation).
// Java: com/google/devtools/build/lib/analysis/util/AnalysisTestUtil.java:393 AnalysisTestUtil.execOptions
pub fn exec_options(
    target_options: &Arc<BuildOptions>,
    handler: Arc<dyn ExtendedEventHandler>,
) -> BuildOptions {
    if !target_options.get_starlark_options().is_empty() {
        blaze_util::unported!("BuildOptionsScopeValue.Key.create");
    }
    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let exec_transition = blaze_build_config::starlark_transition::load_starlark_exec_transition(
        Some(target_options),
        &semantics,
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("the Starlark exec transition");
    let host_platform = target_options
        .get::<blaze_optdefs::fragment::platform_options::PlatformOptions>()
        .map(|p| p.host_platform.0.clone());
    blaze_build_config::function_transition_util::adjust_for_exec(
        target_options,
        &exec_transition,
        host_platform.as_ref(),
        &semantics,
        handler.as_ref(),
    )
}

fn unported_owner(owner: &blaze_actions::ActionLookupKeyRef) -> &'static ConfiguredTargetKey {
    blaze_util::unported!(
        "BuildViewTestCase.getGeneratingAction",
        "artifact owned by {owner:?}"
    )
}

/// `Label.parseCanonical` as tests use it (a syntax error fails the test).
pub fn parse_label(label: &str) -> Label {
    Label::parse_canonical(label).unwrap_or_else(|e| panic!("LabelSyntaxException: {e:?}"))
}

/// `AnalysisResult.hasError()`.
// Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:139 AnalysisResult.hasError
pub fn has_error(result: &AnalysisResult) -> bool {
    result.get_failure_detail().is_some()
}

/// `getFilesToBuild(target)`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1845 BuildViewTestCase.getFilesToBuild
pub fn get_files_to_build(target: &ConfiguredTarget) -> NestedSet<Artifact> {
    target
        .get_provider::<FileProvider>()
        .expect("FileProvider")
        .get_files_to_build()
        .clone()
}

/// `getExecutable(target)`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1896 BuildViewTestCase.getExecutable
pub fn get_executable(target: &ConfiguredTarget) -> Option<Artifact> {
    target
        .get_provider::<FilesToRunProvider>()
        .expect("FilesToRunProvider")
        .get_executable()
        .cloned()
}

/// `getFilesToRun(target)`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1900 BuildViewTestCase.getFilesToRun
pub fn get_files_to_run(target: &ConfiguredTarget) -> NestedSet<Artifact> {
    target
        .get_provider::<FilesToRunProvider>()
        .expect("FilesToRunProvider")
        .get_files_to_run()
        .clone()
}

/// `getDefaultRunfiles(target)`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1916 BuildViewTestCase.getDefaultRunfiles
pub fn get_default_runfiles(target: &ConfiguredTarget) -> Runfiles {
    target
        .get_provider::<RunfilesProvider>()
        .expect("RunfilesProvider")
        .get_default_runfiles()
        .clone()
}

/// `getDataRunfiles(target)`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1920 BuildViewTestCase.getDataRunfiles
pub fn get_data_runfiles(target: &ConfiguredTarget) -> Runfiles {
    target
        .get_provider::<RunfilesProvider>()
        .expect("RunfilesProvider")
        .get_data_runfiles()
        .clone()
}

/// `getOutputGroup(target, outputGroup)`: the group's files, empty without `OutputGroupInfo`.
// Java: com/google/devtools/build/lib/analysis/util/BuildViewTestCase.java:1879 BuildViewTestCase.getOutputGroup
pub fn get_output_group(target: &ConfiguredTarget, output_group: &str) -> NestedSet<Artifact> {
    use blaze_analysis::providers::output_group_info::OutputGroupInfo;
    match target.get(&blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO.key()) {
        Some(info) => blaze_analysis::info::downcast_info::<OutputGroupInfo>(&info)
            .expect("ClassCastException: OutputGroupInfo")
            .get_output_group(output_group),
        None => blaze_collect::NestedSetBuilder::empty_set(blaze_collect::Order::STABLE_ORDER),
    }
}

/// `ActionsTestUtil.prettyArtifactNames`: each artifact's `prettyPrint()`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:626 ActionsTestUtil.prettyArtifactNames
pub fn pretty_artifact_names<'a>(artifacts: impl IntoIterator<Item = &'a Artifact>) -> Vec<String> {
    artifacts.into_iter().map(|a| a.pretty_print()).collect()
}

/// `ActionsTestUtil.baseArtifactNames`: each artifact's exec path base name.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:600 ActionsTestUtil.baseArtifactNames
pub fn base_artifact_names<'a>(artifacts: impl IntoIterator<Item = &'a Artifact>) -> Vec<String> {
    artifacts
        .into_iter()
        .map(|a| a.get_exec_path().get_base_name().to_string())
        .collect()
}

/// `ActionsTestUtil.execPaths`.
// Java: com/google/devtools/build/lib/actions/util/ActionsTestUtil.java:610 ActionsTestUtil.execPaths
pub fn exec_paths<'a>(artifacts: impl IntoIterator<Item = &'a Artifact>) -> Vec<String> {
    artifacts
        .into_iter()
        .map(|a| a.get_exec_path_string().to_string())
        .collect()
}

/// `assertThat(actual).containsExactly(expected...)` (any order, same multiplicity).
pub fn assert_contains_exactly(actual: &[String], expected: &[&str]) {
    let mut a: Vec<&str> = actual.iter().map(String::as_str).collect();
    let mut e: Vec<&str> = expected.to_vec();
    a.sort();
    e.sort();
    assert_eq!(
        a, e,
        "containsExactly: actual {actual:?}, expected {expected:?}"
    );
}

/// `assertThat(actual).containsExactly(expected...).inOrder()`.
pub fn assert_contains_exactly_in_order(actual: &[String], expected: &[&str]) {
    let a: Vec<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(a, expected.to_vec());
}

/// Java `AnalysisTestCase`: the same harness, created with
/// [`BuildViewTestCase::analysis_test_case`].
pub type AnalysisTestCase = BuildViewTestCase;
