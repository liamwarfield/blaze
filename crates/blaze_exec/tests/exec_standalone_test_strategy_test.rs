//! Port of `src/test/java/com/google/devtools/build/lib/exec/StandaloneTestStrategyTest.java`.
//!
//! PORT: the Java test analyses a `foo_test` target with `BuildViewTestCase` and takes its
//! `TestRunnerAction`; here the action is built directly with the values that analysis produces
//! (an `sh`-style test executable with a runfiles tree, size small, no tags unless the Java test
//! sets them). The mocked `SpawnStrategy` is a scripted fake registered as the only strategy.
//! `TestedStandaloneTestStrategy.postedResult` is read from the posted `TestResult` events
//! (the real `postTestResult` also writes test.cache_status, which is harmless here).

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use blaze_actions::actions::action::Action;
use blaze_actions::actions::command_line;
use blaze_actions::{
    ActionEnvironment, ActionLookupKeyRef, ActionOwner, Artifact as BlazeArtifact, ArtifactOwner,
    ArtifactRoot, RootType, SpecialArtifactType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_events::{Clock, Event, EventKind, ExtendedEventHandler, JavaClock, StoredEventHandler};
use blaze_exec::actions::spawn_strategy::SpawnStrategy;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ExecException, FileArtifactValue, FilesetOutputTree,
    InputMetadataProvider, ResourceSet, RunfilesArtifactValue, Spawn, SpawnExecException,
    SpawnResult, SpawnResultBuilder, Status, TreeArtifactValue,
};
use blaze_exec::analysis::test::test_action_context::{TestActionContext, TestAttemptResult};
use blaze_exec::analysis::test::test_attempt::TestAttempt;
use blaze_exec::analysis::test::test_result::TestResult;
use blaze_exec::analysis::test::test_runner_action::{
    TestRunnerAction, TestRunnerActionConfiguration,
};
use blaze_exec::analysis::test::test_strategy::TestStrategy;
use blaze_exec::analysis::test::test_target_execution_settings::TestTargetExecutionSettings;
use blaze_exec::analysis::test::test_target_properties::TestTargetProperties;
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::exec::standalone_test_strategy::{
    StandaloneProcessedAttemptResult, StandaloneTestResult, StandaloneTestStrategy,
};
use blaze_exec::exec::test_log_helper;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::{ExecutionOptions, TestOutputFormat};
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::{TestTimeout, TestTimeoutConverter};
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptionsCancelConcurrentTests as CancelConcurrentTests;
use blaze_options::{Options, TriState};
use blaze_protos::blaze::{BlazeTestStatus, TestResultData};
use blaze_protos::build_event_stream::TestStatus;
use blaze_protos::build_event_stream::test_result::ExecutionInfo;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, Root, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};

// ------------------------------------------------------------------------------------------
// Fixture (Java: BuildViewTestCase + the test's helpers)
// ------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key(Label);

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
    }
}

impl blaze_actions::actions::action_lookup::ActionLookupKey for Key {
    fn get_label(&self) -> Option<Label> {
        Some(self.0.clone())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some("abc".to_string())
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(
        &self,
        other: &dyn blaze_actions::actions::action_lookup::ActionLookupKey,
    ) -> bool {
        other
            .as_any()
            .downcast_ref::<Key>()
            .is_some_and(|o| o == self)
    }
    fn key_hash(&self) -> u64 {
        0
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The host platform (`@@platforms//host:host`) every test action executes on.
fn host_platform() -> blaze_actions::PlatformInfoRef {
    let l = |s: &str| Label::parse_canonical(s).unwrap();
    Arc::new(blaze_actions::actions::platform_info::PlatformInfoData {
        label: l("@@platforms//host:host"),
        constraints_parent: None,
        constraints: Vec::new(),
        remote_execution_properties: String::new(),
        exec_properties: IndexMap::new(),
        flags: Vec::new(),
        required_settings: Vec::new(),
        allowed_toolchain_types: Vec::new(),
        check_toolchain_types: false,
        missing_toolchain_error_message: None,
    })
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root_directory: Path,
    output_base: Path,
    exec_root: Path,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let base = fs.get_path(tmp.path().to_str().unwrap());
        let root_directory = base.get_child("workspace");
        let output_base = base.get_child("output_base");
        let exec_root = output_base.get_relative_str("execroot/_main");
        root_directory.create_directory_and_parents().unwrap();
        exec_root.create_directory_and_parents().unwrap();
        Fixture {
            _tmp: tmp,
            root_directory,
            output_base,
            exec_root,
        }
    }

    fn derived(&self, root: &str, exec_path: &str, key: &ActionLookupKeyRef) -> BlazeArtifact {
        BlazeArtifact::create_derived_artifact(
            ArtifactRoot::as_derived_root(
                &self.exec_root,
                RootType::OUTPUT,
                &["bazel-out", "k8-fastbuild", root],
            ),
            PathFragment::create(exec_path),
            key.clone(),
        )
    }

    /// `getTestActions(target)`: the TestRunnerActions of `//standalone:<name>` (one per run).
    fn get_test_actions(
        &self,
        name: &str,
        tags: &[&str],
        flaky: bool,
        runs: i32,
        cancel_concurrent_tests: CancelConcurrentTests,
    ) -> Vec<Arc<TestRunnerAction>> {
        let label = Label::parse_canonical(&format!("//standalone:{name}")).unwrap();
        let key = ActionLookupKeyRef::new(Key(label.clone()));
        let owner = ActionOwner::create_dummy(
            Some(label.clone()),
            Location::from_file("standalone/BUILD"),
            "foo_test rule",
            "k8-fastbuild",
            "abc",
            None,
            false,
            Some(host_platform()),
            Vec::new(),
            IndexMap::new(),
        );
        let bin = |p: &str| self.derived("bin", &format!("bazel-out/k8-fastbuild/bin/{p}"), &key);
        let tools_root = ArtifactRoot::as_external_source_root(Root::from_path(
            self.output_base.get_relative_str("external/bazel_tools"),
        ));
        let tool = |p: &str| {
            BlazeArtifact::new_source_artifact(
                tools_root.clone(),
                PathFragment::create(&format!("external/bazel_tools/tools/test/{p}")),
                ArtifactOwner::NullOwner,
            )
        };
        let test_setup = tool("test-setup.sh");
        let xml_generator = tool("generate-xml.sh");
        let runfiles_tree = BlazeArtifact::create_special_artifact(
            ArtifactRoot::as_derived_root(
                &self.exec_root,
                RootType::OUTPUT,
                &["bazel-out", "k8-fastbuild", "bin"],
            ),
            PathFragment::create(&format!(
                "bazel-out/k8-fastbuild/bin/standalone/{name}.runfiles"
            )),
            key.clone(),
            SpecialArtifactType::RUNFILES,
        );
        let execution_info = TestTargetProperties::compute_execution_info(
            &tags
                .iter()
                .map(|t| (t.to_string(), String::new()))
                .collect(),
            false,
            tags.contains(&"local"),
            tags.contains(&"exclusive"),
            false,
            None,
        );
        let configuration = Arc::new(TestRunnerActionConfiguration {
            checksum: "abc".to_string(),
            action_environment: Arc::new(ActionEnvironment::create(
                [(
                    "PATH".to_string(),
                    "/bin:/usr/bin:/usr/local/bin".to_string(),
                )]
                .into_iter()
                .collect(),
                IndexSet::new(),
            )),
            test_action_environment: Arc::new(ActionEnvironment::Empty),
            code_coverage_enabled: false,
            runfiles_enabled: true,
            test_timeout: TestTimeoutConverter.convert_map("-1").unwrap().0,
            cache_test_results: TriState::AUTO,
            zip_undeclared_test_outputs: false,
            test_runner_fail_fast: false,
            check_sharding_support: true,
            allow_local_tests: true,
        });
        let mut actions = Vec::new();
        for run in 0..runs {
            let dir = if runs > 1 {
                format!("standalone/{name}/run_{}_of_{runs}", run + 1)
            } else {
                format!("standalone/{name}")
            };
            let tl = |p: &str| {
                self.derived(
                    "testlogs",
                    &format!("bazel-out/k8-fastbuild/testlogs/{dir}/{p}"),
                    &key,
                )
            };
            let undeclared = BlazeArtifact::create_special_artifact(
                ArtifactRoot::as_derived_root(
                    &self.exec_root,
                    RootType::OUTPUT,
                    &["bazel-out", "k8-fastbuild", "testlogs"],
                ),
                PathFragment::create(&format!(
                    "bazel-out/k8-fastbuild/testlogs/{dir}/test.outputs"
                )),
                key.clone(),
                SpecialArtifactType::TREE,
            );
            let settings = TestTargetExecutionSettings::new(
                command_line::empty(),
                None,
                0,
                runs,
                None,
                None,
                bin(&format!("standalone/{name}")),
                true,
                Some(self.exec_root.get_relative(runfiles_tree.get_exec_path())),
                Some(bin(&format!("standalone/{name}.runfiles_manifest"))),
                None,
                false,
            );
            let properties = TestTargetProperties::new(
                TestSize::SMALL,
                TestTimeout::SHORT,
                tags.iter().map(|t| t.to_string()).collect(),
                flaky,
                false,
                "foo".to_string(),
                execution_info.clone(),
                Some(IndexMap::new()),
            );
            let action = TestRunnerAction::new(
                owner.clone(),
                NestedSetBuilder::create(
                    Order::STABLE_ORDER,
                    [
                        runfiles_tree.clone(),
                        test_setup.clone(),
                        xml_generator.clone(),
                    ],
                ),
                runfiles_tree.clone(),
                test_setup.clone(),
                xml_generator.clone(),
                None,
                tl("test.log"),
                ActionInputRef::new(tl("test.xml")),
                tl("test.cache_status"),
                None,
                None,
                undeclared,
                properties,
                IndexMap::new(),
                Arc::new(ActionEnvironment::Empty),
                settings,
                0,
                run,
                configuration.clone(),
                "_main".to_string(),
                Some(PathFragment::create("/bin/bash")),
                cancel_concurrent_tests,
                false,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                None,
            );
            action
                .get_test_log()
                .get_path()
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
            actions.push(action);
        }
        actions
    }

    fn get_test_action(&self, name: &str, tags: &[&str], flaky: bool) -> Arc<TestRunnerAction> {
        self.get_test_actions(name, tags, flaky, 1, CancelConcurrentTests::NEVER)
            .remove(0)
    }

    fn tmp_dir_root(&self, options: &ExecutionOptions) -> Path {
        TestStrategy::get_tmp_root(&self.root_directory, &self.output_base, options)
    }

    fn create_temp_out_err(&self, tmp_dir_root: &Path) -> FileOutErr {
        tmp_dir_root.create_directory_and_parents().unwrap();
        FileOutErr::new(
            tmp_dir_root.get_relative_str("test-out.txt"),
            tmp_dir_root.get_relative_str("test-err.txt"),
        )
    }
}

/// What a scripted `exec` call sees of a spawn.
#[derive(Clone, Debug)]
struct SeenSpawn {
    output_count: usize,
    environment: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    local_resources: ResourceSet,
    out_err: (String, String),
}

type Answer = Box<
    dyn Fn(&dyn Spawn, &dyn ActionExecutionContext) -> Result<Vec<SpawnResult>, ExecException>
        + Send
        + Sync,
>;

/// The mocked `SpawnStrategy`: `canExec` is always true; `exec` runs the next scripted answer
/// (the last one repeats, as Mockito's `thenReturn` / `thenAnswer` do).
struct MockSpawnStrategy {
    answers: Mutex<VecDeque<Answer>>,
    seen: Mutex<Vec<SeenSpawn>>,
}

impl MockSpawnStrategy {
    fn new(answers: Vec<Answer>) -> Arc<MockSpawnStrategy> {
        Arc::new(MockSpawnStrategy {
            answers: Mutex::new(answers.into()),
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl fmt::Display for MockSpawnStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("mock")
    }
}

impl SpawnStrategy for MockSpawnStrategy {
    fn exec(
        &self,
        spawn: &dyn Spawn,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.seen.lock().unwrap().push(SeenSpawn {
            output_count: spawn.get_output_files().len(),
            environment: spawn.get_environment().clone(),
            execution_info: spawn.get_execution_info().clone(),
            local_resources: spawn.get_local_resources().unwrap(),
            out_err: (
                ctx.get_file_out_err()
                    .get_output_path()
                    .get_path_string()
                    .to_string(),
                ctx.get_file_out_err()
                    .get_error_path()
                    .get_path_string()
                    .to_string(),
            ),
        });
        let mut answers = self.answers.lock().unwrap();
        let answer = if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            let a = answers.pop_front().expect("no scripted answer");
            let r = a(spawn, ctx);
            answers.push_back(a);
            return r;
        };
        drop(answers);
        answer(spawn, ctx)
    }
    fn can_exec(&self, _spawn: &dyn Spawn, _registry: &dyn ActionContextRegistry) -> bool {
        true
    }
    fn get_simple_class_name(&self) -> String {
        "SpawnStrategy$MockitoMock".to_string()
    }
}

struct EmptyMetadata;

impl InputMetadataProvider for EmptyMetadata {
    fn get_input_metadata(
        &self,
        _input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// `FakeActionExecutionContext`.
struct FakeActionExecutionContext {
    exec_root: Path,
    out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    events: Arc<StoredEventHandler>,
    registry: Arc<SpawnStrategyRegistry>,
    metadata: EmptyMetadata,
}

impl FakeActionExecutionContext {
    fn new(
        f: &Fixture,
        out_err: FileOutErr,
        strategy: Arc<MockSpawnStrategy>,
        events: Arc<StoredEventHandler>,
    ) -> FakeActionExecutionContext {
        let strategy: Arc<dyn SpawnStrategy> = strategy;
        let registry = SpawnStrategyRegistry::builder()
            .register_strategy(strategy, &["mock"])
            .set_default_strategies(&["mock".to_string()])
            .build()
            .unwrap();
        FakeActionExecutionContext {
            exec_root: f.exec_root.clone(),
            out_err,
            client_env: [("PATH".to_string(), "/usr/bin:/bin".to_string())]
                .into_iter()
                .collect(),
            events,
            registry: Arc::new(registry),
            metadata: EmptyMetadata,
        }
    }
}

impl ActionContextRegistry for FakeActionExecutionContext {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        Some(self.registry.clone())
    }
}

impl ActionExecutionContext for FakeActionExecutionContext {
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &self.metadata
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        &*self.events
    }
    fn get_clock(&self) -> &dyn Clock {
        &JavaClock
    }
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {}
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {}
}

fn non_zero_exit_details() -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
            code: failure_details::spawn::Code::NonZeroExit as i32,
            ..Default::default()
        })),
    }
}

fn failed_test_spawn() -> SpawnResult {
    SpawnResultBuilder::new()
        .set_status(Status::NonZeroExit)
        .set_exit_code(1)
        .set_failure_detail(non_zero_exit_details())
        .set_runner_name("test")
        .build()
}

fn passed_test_spawn() -> SpawnResult {
    SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_runner_name("test")
        .build()
}

fn returns(r: SpawnResult) -> Answer {
    Box::new(move |_, _| Ok(vec![r.clone()]))
}

fn throws(r: SpawnResult) -> Answer {
    Box::new(move |_, _| {
        Err(ExecException::Spawn(Box::new(SpawnExecException::new(
            "test failed".to_string(),
            r.clone(),
            false,
        ))))
    })
}

/// A comparable view of a SpawnResult (Java compares by identity/equals).
fn sr(r: &SpawnResult) -> (String, i32, String, i32) {
    (
        r.status().name().to_string(),
        r.exit_code(),
        r.get_runner_name().to_string(),
        r.get_wall_time_in_ms(),
    )
}

fn execute(
    action: &TestRunnerAction,
    ctx: &dyn ActionExecutionContext,
    strategy: &StandaloneTestStrategy,
) -> Vec<SpawnResult> {
    action
        .execute_with_context(ctx, strategy)
        .unwrap()
        .spawn_results()
        .to_vec()
}

fn posted_results(events: &StoredEventHandler) -> Vec<TestResult> {
    events
        .get_posts()
        .iter()
        .filter_map(|p| {
            let any: &dyn std::any::Any = &**p;
            any.downcast_ref::<TestResult>().cloned()
        })
        .collect()
}

fn posted_attempts(events: &StoredEventHandler) -> Vec<TestAttempt> {
    events
        .get_posts()
        .iter()
        .filter_map(|p| {
            let any: &dyn std::any::Any = &**p;
            any.downcast_ref::<TestAttempt>().cloned()
        })
        .collect()
}

fn defaults() -> (Arc<ExecutionOptions>, Arc<TestSummaryOptions>) {
    (
        Arc::new(Options::<ExecutionOptions>::get_defaults()),
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
    )
}

fn with_test_output(output: TestOutputFormat) -> Arc<ExecutionOptions> {
    let mut o = Options::<ExecutionOptions>::get_defaults();
    o.test_output = output;
    Arc::new(o)
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p.get_path_string()).unwrap()
}

// ------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------

#[test]
fn test_create_tmp_dir_for_test() {
    let f = Fixture::new();
    let action = f.get_test_action("simple_test", &[], false);

    let tmp_dir_name = TestStrategy::get_tmp_dir_name(&action);
    // Make sure the length of tmpDirName doesn't change unexpectedy: it cannot be too long
    // because Windows and macOS have limitations on file path length.
    // Note: It's OK to update 32 to a smaller number if tmpDirName gets shorter.
    assert_eq!(tmp_dir_name.len(), 32);
}

#[test]
fn test_run_test_once() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], false);

    let expected_spawn_result = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_wall_time_in_ms(10)
        .set_runner_name("test")
        .build();
    let mock = MockSpawnStrategy::new(vec![returns(expected_spawn_result.clone())]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock,
        events.clone(),
    );

    let spawn_results = execute(&action, &ctx, &strategy);

    assert!(
        spawn_results
            .iter()
            .any(|r| sr(r) == sr(&expected_spawn_result))
    );
    let result = posted_results(&events).pop().expect("posted result");
    assert!(!result.is_cached());
    assert!(Arc::ptr_eq(result.get_test_action(), &action));
    assert!(result.get_data().test_passed());
    assert_eq!(result.get_data().exit_code(), 0);
    assert!(!result.get_data().remotely_cached());
    assert!(!result.get_data().is_remote_strategy());
    assert_eq!(result.get_data().run_duration_millis(), 10);
    assert_eq!(result.get_data().test_times, vec![10]);
    let attempts = posted_attempts(&events);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].get_execution_info().strategy, "test");
    assert_eq!(attempts[0].get_execution_info().hostname, "");
}

#[test]
fn test_run_flaky_test() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], /* flaky= */ true);

    let fail_spawn_result = SpawnResultBuilder::new()
        .set_status(Status::NonZeroExit)
        .set_exit_code(1)
        .set_failure_detail(non_zero_exit_details())
        .set_wall_time_in_ms(10)
        .set_runner_name("test")
        .build();
    let pass_spawn_result = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_wall_time_in_ms(15)
        .set_runner_name("test")
        .build();
    let mock = MockSpawnStrategy::new(vec![
        throws(fail_spawn_result.clone()),
        // XML generation
        returns(pass_spawn_result.clone()),
        returns(pass_spawn_result.clone()),
        // XML generation
        returns(pass_spawn_result.clone()),
    ]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock,
        events.clone(),
    );

    let spawn_results = execute(&action, &ctx, &strategy);

    assert_eq!(
        spawn_results.iter().map(sr).collect::<Vec<_>>(),
        vec![
            sr(&fail_spawn_result),
            sr(&pass_spawn_result),
            sr(&pass_spawn_result),
            sr(&pass_spawn_result)
        ]
    );

    let result = posted_results(&events).pop().expect("posted result");
    assert!(!result.is_cached());
    assert!(Arc::ptr_eq(result.get_test_action(), &action));
    assert_eq!(result.get_data().status(), BlazeTestStatus::Flaky);
    assert!(result.get_data().test_passed());
    assert_eq!(result.get_data().exit_code(), 0);
    assert!(!result.get_data().remotely_cached());
    assert!(!result.get_data().is_remote_strategy());
    assert_eq!(result.get_data().run_duration_millis(), 15);
    let mut times = result.get_data().test_times.clone();
    times.sort();
    assert_eq!(times, vec![10, 15]);
    let attempts = posted_attempts(&events);
    assert_eq!(attempts.len(), 2);
    let failed_attempt = &attempts[0];
    assert_eq!(failed_attempt.get_execution_info().strategy, "test");
    assert_eq!(failed_attempt.get_execution_info().hostname, "");
    assert_eq!(failed_attempt.get_status(), TestStatus::Failed);
    assert_eq!(failed_attempt.get_execution_info().exit_code, 1);
    assert!(!failed_attempt.get_execution_info().cached_remotely);
    let ok_attempt = &attempts[1];
    assert_eq!(ok_attempt.get_status(), TestStatus::Passed);
    assert_eq!(ok_attempt.get_execution_info().exit_code, 0);
    assert_eq!(ok_attempt.get_execution_info().strategy, "test");
    assert_eq!(ok_attempt.get_execution_info().hostname, "");
}

#[test]
fn test_run_test_remotely() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], false);

    let expected_spawn_result = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_wall_time_in_ms(10)
        .set_runner_name("remote")
        .set_executor_hostname("a-remote-host".to_string())
        .build();
    let mock = MockSpawnStrategy::new(vec![returns(expected_spawn_result.clone())]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock,
        events.clone(),
    );

    let spawn_results = execute(&action, &ctx, &strategy);

    assert!(
        spawn_results
            .iter()
            .any(|r| sr(r) == sr(&expected_spawn_result))
    );
    let result = posted_results(&events).pop().expect("posted result");
    assert!(!result.is_cached());
    assert!(Arc::ptr_eq(result.get_test_action(), &action));
    assert!(result.get_data().test_passed());
    assert_eq!(result.get_data().exit_code(), 0);
    assert!(!result.get_data().remotely_cached());
    assert!(result.get_data().is_remote_strategy());
    assert_eq!(result.get_data().run_duration_millis(), 10);
    assert_eq!(result.get_data().test_times, vec![10]);
    let attempts = posted_attempts(&events);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].get_status(), TestStatus::Passed);
    assert_eq!(attempts[0].get_execution_info().exit_code, 0);
    assert_eq!(attempts[0].get_execution_info().strategy, "remote");
    assert_eq!(attempts[0].get_execution_info().hostname, "a-remote-host");
}

#[test]
fn test_run_remotely_cached_test() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], false);

    let expected_spawn_result = SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_cache_hit(true)
        .set_wall_time_in_ms(10)
        .set_runner_name("remote cache")
        .build();
    let mock = MockSpawnStrategy::new(vec![returns(expected_spawn_result.clone())]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock,
        events.clone(),
    );

    let spawn_results = execute(&action, &ctx, &strategy);

    // check that the rigged SpawnResult was returned
    assert!(
        spawn_results
            .iter()
            .any(|r| sr(r) == sr(&expected_spawn_result))
    );
    let result = posted_results(&events).pop().expect("posted result");
    assert!(!result.is_cached());
    assert!(Arc::ptr_eq(result.get_test_action(), &action));
    assert!(result.get_data().test_passed());
    assert_eq!(result.get_data().exit_code(), 0);
    assert!(result.get_data().remotely_cached());
    assert!(!result.get_data().is_remote_strategy());
    assert_eq!(result.get_data().run_duration_millis(), 10);
    assert_eq!(result.get_data().test_times, vec![10]);
    let attempts = posted_attempts(&events);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].get_execution_info().strategy, "remote cache");
    assert_eq!(attempts[0].get_execution_info().hostname, "");
}

/// The answer of the two test-log tests: the test spawn writes to its test log and fails; the
/// XML generation spawn passes.
fn failing_test_with_log(check: Option<fn(&dyn Spawn)>) -> Answer {
    Box::new(move |spawn, ctx| {
        if let Some(check) = check {
            check(spawn);
        }
        if spawn.get_output_files().len() != 1 {
            let out_err = ctx.get_file_out_err();
            out_err
                .write_out(b"This will not appear in the test output: bla\n")
                .unwrap();
            out_err
                .write_out(format!("{}\n", test_log_helper::HEADER_DELIMITER).as_bytes())
                .unwrap();
            out_err
                .write_out(b"This will appear in the test output: foo\n")
                .unwrap();
            Err(ExecException::Spawn(Box::new(SpawnExecException::new(
                "Failure!!".to_string(),
                failed_test_spawn(),
                /* forcibly_run_remotely= */ false,
            ))))
        } else {
            Ok(vec![passed_test_spawn()])
        }
    })
}

fn check_test_log_and_output(action: &TestRunnerAction, out_err: &FileOutErr) {
    // check that the test log contains all the output
    let log_data = read(&action.get_test_log().get_path());
    assert!(log_data.contains("bla"));
    assert!(log_data.contains(test_log_helper::HEADER_DELIMITER));
    assert!(log_data.contains("foo"));
    // check that the test stdout contains all the expected output
    let out_data = read(out_err.get_output_path());
    assert!(out_data.contains("==================== Test output for //standalone:failing_test:"));
    assert!(!out_data.contains("bla"));
    assert!(!out_data.contains(test_log_helper::HEADER_DELIMITER));
    assert!(out_data.contains("foo"));
    assert!(out_data.contains(
        "================================================================================"
    ));
    assert!(!out_err.get_error_path().exists());
}

#[test]
fn test_that_test_log_and_output_are_returned() {
    let f = Fixture::new();
    let execution_options = with_test_output(TestOutputFormat::ERRORS);
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("failing_test", &[], false);

    let mock = MockSpawnStrategy::new(vec![failing_test_with_log(None)]);
    let out_err = f.create_temp_out_err(&tmp_dir_root);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(&f, out_err.clone(), mock, events);

    let spawn_results = execute(&action, &ctx, &strategy);

    // check that the rigged SpawnResult was returned
    assert!(
        spawn_results
            .iter()
            .any(|r| sr(r) == sr(&failed_test_spawn()))
    );
    check_test_log_and_output(&action, &out_err);
}

#[test]
fn test_that_test_log_and_output_are_returned_with_split_xml_generation() {
    let f = Fixture::new();
    let execution_options = with_test_output(TestOutputFormat::ERRORS);
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("failing_test", &["local"], false);

    fn check(spawn: &dyn Spawn) {
        // Test that both spawns have the local tag attached as a execution info
        assert!(spawn.get_execution_info().contains_key("local"));
        if spawn.get_output_files().len() == 1 {
            let test_name = "standalone/failing_test";
            assert_eq!(
                spawn
                    .get_environment()
                    .get("TEST_BINARY")
                    .map(|s| s.as_str()),
                Some(test_name)
            );
        }
    }
    let mock = MockSpawnStrategy::new(vec![failing_test_with_log(Some(check))]);
    let out_err = f.create_temp_out_err(&tmp_dir_root);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(&f, out_err.clone(), mock.clone(), events);

    let spawn_results = execute(&action, &ctx, &strategy);

    // check that the rigged SpawnResult was returned
    assert_eq!(
        spawn_results.iter().map(sr).collect::<Vec<_>>(),
        vec![sr(&failed_test_spawn()), sr(&passed_test_spawn())]
    );
    check_test_log_and_output(&action, &out_err);
    let called: Vec<(String, String)> = mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.out_err.clone())
        .collect();
    assert_eq!(called.len(), 2);
    assert_ne!(called[0], called[1]);
}

#[test]
#[ignore = "needs ResourceSetOrBuilder.buildLocalResources `resources:` overrides (unported) and exec_properties on the test owner"]
fn xml_generating_spawn_does_not_inherit_test_resource_declarations() {}

#[test]
fn test_empty_output_creates_empty_log_file() {
    let f = Fixture::new();
    let execution_options = with_test_output(TestOutputFormat::ALL);
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("empty_test", &[], false);

    let mock = MockSpawnStrategy::new(vec![returns(passed_test_spawn())]);
    let out_err = f.create_temp_out_err(&tmp_dir_root);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(&f, out_err.clone(), mock, events);

    let spawn_results = execute(&action, &ctx, &strategy);

    assert!(
        spawn_results
            .iter()
            .any(|r| sr(r) == sr(&passed_test_spawn()))
    );
    // check that the test log contains all the output
    assert_eq!(read(&action.get_test_log().get_path()), "");
    // check that the test stdout contains all the expected output
    let out_data = read(out_err.get_output_path());
    let empty_output = regex::Regex::new(
        "^==================== Test output for //standalone:empty_test:(\\s)*================================================================================(\\s)*$",
    )
    .unwrap();
    assert!(empty_output.is_match(&out_data), "{out_data:?}");
    assert!(!out_err.get_error_path().exists());
}

#[test]
fn test_append_std_err_does_not_busy_loop() {
    let f = Fixture::new();
    let execution_options = with_test_output(TestOutputFormat::ALL);
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        Arc::new(Options::<TestSummaryOptions>::get_defaults()),
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("empty_test", &[], false);

    let mock = MockSpawnStrategy::new(vec![Box::new(|_, ctx| {
        ctx.get_file_out_err().write_err(b"Foo").unwrap();
        Ok(vec![passed_test_spawn()])
    })]);
    let out_err = f.create_temp_out_err(&tmp_dir_root);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(&f, out_err.clone(), mock, events);

    execute(&action, &ctx, &strategy);

    // check that the test stdout contains all the expected output
    assert!(read(out_err.get_output_path()).contains("Foo"));
}

fn touch_xml(action: &Arc<TestRunnerAction>, exec_root: &Path) {
    // Avoid triggering split XML generation by creating an empty XML file.
    let p = action.resolve(exec_root).get_xml_output_path();
    p.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    std::fs::write(p.get_path_string(), b"").unwrap();
}

fn assert_contains_prefixed_event(events: &[Event], kind: EventKind, message: &str) {
    assert!(
        events
            .iter()
            .any(|e| e.get_kind() == kind && e.get_message().starts_with(message)),
        "no {kind:?} event starting with {message:?}"
    );
}

fn cancel_concurrent_tests_case(cancel_concurrent_tests: CancelConcurrentTests) {
    let test_on_passed = cancel_concurrent_tests == CancelConcurrentTests::ON_PASSED;
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let actions = f.get_test_actions("empty_test", &[], false, 2, cancel_concurrent_tests);
    assert_eq!(actions.len(), 2);
    let action_a = actions[0].clone();
    let action_b = actions[1].clone();
    let attempt_group = strategy.get_attempt_group(action_a.get_owner(), action_a.get_shard_num());
    assert!(Arc::ptr_eq(
        &attempt_group,
        &strategy.get_attempt_group(action_b.get_owner(), action_b.get_shard_num())
    ));

    let exec_root = f.exec_root.clone();
    let a = action_a.clone();
    let mock = MockSpawnStrategy::new(vec![
        Box::new(move |_, _| {
            touch_xml(&a, &exec_root);
            if test_on_passed {
                Ok(vec![passed_test_spawn()])
            } else {
                Err(ExecException::Spawn(Box::new(SpawnExecException::new(
                    String::new(),
                    failed_test_spawn(),
                    false,
                ))))
            }
        }),
        Box::new(|_, _| panic!("failure: this should not have been called")),
    ]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock.clone(),
        events.clone(),
    );
    let result_a = execute(&action_a, &ctx, &strategy);
    assert!(attempt_group.cancelled());
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
    assert_eq!(result_a.len(), 1);
    let posted = posted_results(&events).pop().expect("posted result");
    assert_eq!(
        posted.get_data().status(),
        if test_on_passed {
            BlazeTestStatus::Passed
        } else {
            BlazeTestStatus::Failed
        }
    );
    assert_eq!(
        posted.get_data().exit_code(),
        if test_on_passed { 0 } else { 1 }
    );
    assert_contains_prefixed_event(
        &events.get_events(),
        if test_on_passed {
            EventKind::Pass
        } else {
            EventKind::Fail
        },
        "//standalone:empty_test (run 1 of 2)",
    );

    let result_b = execute(&action_b, &ctx, &strategy);
    assert!(result_b.is_empty());
    let posted = posted_results(&events);
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].get_data().status(), BlazeTestStatus::Incomplete);
    assert!(
        events
            .get_events()
            .iter()
            .any(|e| e.get_kind() == EventKind::Cancelled
                && e.get_message() == "//standalone:empty_test (run 2 of 2)")
    );
    // Check that there are no ERROR events.
    assert!(
        events
            .get_events()
            .iter()
            .all(|e| e.get_kind() != EventKind::Error)
    );
}

#[test]
fn test_experimental_cancel_concurrent_tests_on_passed() {
    cancel_concurrent_tests_case(CancelConcurrentTests::ON_PASSED);
}

#[test]
fn test_experimental_cancel_concurrent_tests_on_failed() {
    cancel_concurrent_tests_case(CancelConcurrentTests::ON_FAILED);
}

fn does_not_trigger_case(cancel_concurrent_tests: CancelConcurrentTests, all_unexpected: bool) {
    let test_on_passed = cancel_concurrent_tests == CancelConcurrentTests::ON_PASSED;
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let actions = f.get_test_actions("empty_test", &[], false, 2, cancel_concurrent_tests);
    assert_eq!(actions.len(), 2);
    let action_a = actions[0].clone();
    let action_b = actions[1].clone();
    let attempt_group = strategy.get_attempt_group(action_a.get_owner(), action_a.get_shard_num());
    assert!(Arc::ptr_eq(
        &attempt_group,
        &strategy.get_attempt_group(action_b.get_owner(), action_b.get_shard_num())
    ));
    assert!(!attempt_group.cancelled());

    let unexpected = move |pass: bool| -> Result<Vec<SpawnResult>, ExecException> {
        if pass {
            Ok(vec![passed_test_spawn()])
        } else {
            Err(ExecException::Spawn(Box::new(SpawnExecException::new(
                String::new(),
                failed_test_spawn(),
                false,
            ))))
        }
    };
    let (exec_root_a, exec_root_b) = (f.exec_root.clone(), f.exec_root.clone());
    let (a, b) = (action_a.clone(), action_b.clone());
    // First run: the result that does not trigger cancellation.
    let first_passes = !test_on_passed;
    // Second run: the triggering result, or again the non-triggering one.
    let second_passes = if all_unexpected {
        !test_on_passed
    } else {
        test_on_passed
    };
    let mock = MockSpawnStrategy::new(vec![
        Box::new(move |_, _| {
            touch_xml(&a, &exec_root_a);
            unexpected(first_passes)
        }),
        Box::new(move |_, _| {
            touch_xml(&b, &exec_root_b);
            unexpected(second_passes)
        }),
    ]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock.clone(),
        events.clone(),
    );
    let result_a = execute(&action_a, &ctx, &strategy);
    assert!(!attempt_group.cancelled());
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
    assert_eq!(result_a.len(), 1);
    let posted = posted_results(&events).pop().expect("posted result");
    let status = |pass: bool| {
        if pass {
            BlazeTestStatus::Passed
        } else {
            BlazeTestStatus::Failed
        }
    };
    assert_eq!(posted.get_data().status(), status(first_passes));
    assert_eq!(
        posted.get_data().exit_code(),
        if first_passes { 0 } else { 1 }
    );
    let kind = |pass: bool| {
        if pass {
            EventKind::Pass
        } else {
            EventKind::Fail
        }
    };
    assert_contains_prefixed_event(
        &events.get_events(),
        kind(first_passes),
        "//standalone:empty_test (run 1 of 2)",
    );

    let result_b = execute(&action_b, &ctx, &strategy);
    assert_eq!(attempt_group.cancelled(), !all_unexpected);
    assert_eq!(result_b.len(), 1);
    let posted = posted_results(&events);
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].get_data().status(), status(second_passes));
    if !all_unexpected {
        assert_eq!(
            posted[1].get_data().exit_code(),
            if second_passes { 0 } else { 1 }
        );
    }
    assert_contains_prefixed_event(
        &events.get_events(),
        kind(second_passes),
        "//standalone:empty_test (run 2 of 2)",
    );
}

#[test]
fn test_experimental_cancel_concurrent_tests_does_not_trigger_on_unexpected_result_on_passed() {
    does_not_trigger_case(CancelConcurrentTests::ON_PASSED, false);
}

#[test]
fn test_experimental_cancel_concurrent_tests_does_not_trigger_on_unexpected_result_on_failed() {
    does_not_trigger_case(CancelConcurrentTests::ON_FAILED, false);
}

#[test]
fn test_experimental_cancel_concurrent_tests_all_unexpected_on_passed() {
    does_not_trigger_case(CancelConcurrentTests::ON_PASSED, true);
}

#[test]
fn test_experimental_cancel_concurrent_tests_all_unexpected_on_failed() {
    does_not_trigger_case(CancelConcurrentTests::ON_FAILED, true);
}

#[test]
fn missing_test_log_spawn_test_result_is_incomplete() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], false);
    let mock = MockSpawnStrategy::new(vec![returns(passed_test_spawn())]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx =
        FakeActionExecutionContext::new(&f, f.create_temp_out_err(&tmp_dir_root), mock, events);
    let spawn = strategy.create_test_runner_spawn(&action, &ctx).unwrap();

    let mut builder = TestResultData {
        test_passed: Some(true),
        ..Default::default()
    };
    builder.set_status(BlazeTestStatus::Passed);
    let result = StandaloneTestResult::new(Vec::new(), builder, ExecutionInfo::default());
    let failed_result = spawn
        .finalize_failed_test_attempt(&result as &dyn TestAttemptResult, 0)
        .unwrap();

    let data = &failed_result
        .as_any()
        .downcast_ref::<StandaloneProcessedAttemptResult>()
        .expect("StandaloneProcessedAttemptResult")
        .test_result_data;
    assert_eq!(data.status(), BlazeTestStatus::Incomplete);
    assert_eq!(data.exit_code(), 0);
}

// ------------------------------------------------------------------------------------------
// Additional checks of the spawn the strategy builds (expected values from the oracle's
// recorded re2 TestRunner spawns, see scripts/w7/replay-tests).
// ------------------------------------------------------------------------------------------

#[test]
fn test_spawn_environment_and_execution_info() {
    let f = Fixture::new();
    let (execution_options, test_summary_options) = defaults();
    let tmp_dir_root = f.tmp_dir_root(&execution_options);
    let strategy = StandaloneTestStrategy::new(
        execution_options,
        test_summary_options,
        tmp_dir_root.clone(),
    );
    let action = f.get_test_action("simple_test", &[], false);
    let mock = MockSpawnStrategy::new(vec![Box::new({
        let a = action.clone();
        let exec_root = f.exec_root.clone();
        move |_, _| {
            touch_xml(&a, &exec_root);
            Ok(vec![passed_test_spawn()])
        }
    })]);
    let events = Arc::new(StoredEventHandler::new());
    let ctx = FakeActionExecutionContext::new(
        &f,
        f.create_temp_out_err(&tmp_dir_root),
        mock.clone(),
        events,
    );
    execute(&action, &ctx, &strategy);
    let seen = mock.seen.lock().unwrap()[0].clone();
    let dir = "bazel-out/k8-fastbuild/testlogs/standalone/simple_test";
    let env = &seen.environment;
    assert_eq!(env["TEST_TARGET"], "//standalone:simple_test");
    assert_eq!(env["TEST_SIZE"], "small");
    assert_eq!(env["TEST_TIMEOUT"], "60");
    assert_eq!(env["TEST_WORKSPACE"], "_main");
    assert_eq!(env["TEST_BINARY"], "standalone/simple_test");
    assert_eq!(env["XML_OUTPUT_FILE"], format!("{dir}/test.xml"));
    assert_eq!(
        env["TEST_PREMATURE_EXIT_FILE"],
        format!("{dir}/test.exited_prematurely")
    );
    assert_eq!(
        env["RUNFILES_DIR"],
        "bazel-out/k8-fastbuild/bin/standalone/simple_test.runfiles"
    );
    assert_eq!(env["TZ"], "UTC");
    assert_eq!(env["PATH"], "/bin:/usr/bin:/usr/local/bin");
    // The fixture's tmp root (Java: getTmpRoot(rootDirectory, outputBase, ...)) is outside the
    // exec root, so TEST_TMPDIR is absolute; under the exec root it is "_tmp/<hash>".
    assert_eq!(
        env["TEST_TMPDIR"],
        tmp_dir_root
            .get_child(&TestStrategy::get_tmp_dir_name(&action))
            .get_path_string()
    );
    assert!(!env.contains_key("TEST_SHARD_INDEX"));
    assert_eq!(
        seen.execution_info.get("timeout").map(|s| s.as_str()),
        Some("60")
    );
    assert_eq!(seen.local_resources.get_memory_mb(), 20.0);
    assert_eq!(seen.output_count, 10);
}
