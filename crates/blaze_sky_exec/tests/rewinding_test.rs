//! Port of `RewindingTest` (src/test/java/com/google/devtools/build/lib/skyframe/rewinding/
//! RewindingTest.java, with its `RewindingTestsHelper`): every case is a
//! `BuildIntegrationTestCase` whose spawns the test controls (a `ControllableActionStrategyModule`
//! wrapping the `remote` and `standalone` strategies loses inputs on cue) and whose Skyframe graph
//! it watches (a `NotifyingHelper` listener collecting the keys marked dirty for rewinding).
//!
//! Java runs each case with every combination of `--track_incremental_state`, `--keep_going` and
//! `--experimental_merged_skyframe_analysis_execution` (`@TestParameter`s of the class); here each
//! test function runs its case for the eight combinations in turn, each on a new fixture.
//!
//! The cases use Bazel's remote worker (built from the pinned source, the files
//! `scripts/remote/worker` runs), as Java's `WorkerInstance` rule: one worker per test function,
//! on a free port, as a systemd user unit that the test stops and that ends by itself after
//! `WORKER_RUNTIME_MAX_SEC`.
//!
//! Covered elsewhere as well: `generatedHeaderRewound_lostInInputDiscovery` and
//! `ineffectiveRewindingResultsInLostInputTooManyTimes` in the `ActionExecutionFunction` fixture
//! (`action_execution_function.rs`, same names), the engine cases in blaze_skyframe
//! (`rewinding_evaluator_test.rs`), and the end-to-end tests with a lost remote blob in
//! `crates/blaze/tests/rewind_lost_inputs_test.rs`.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use blaze_buildtool_testing::BuildIntegrationTestCase;
use blaze_buildtool_testing::action_event_recorder::ActionEventRecorder;
use blaze_buildtool_testing::spawn_controller::{
    ControllableActionStrategyModule, ExecResult, SpawnController,
};
use blaze_events::EventKind;
use blaze_exec::actions::{ActionInputRef, LostArtifacts, LostInputsExecException, Spawn};
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_sky_exec::keys::{ActionExecutionKey, ArtifactNestedSetKey};
use blaze_skyframe::graph_listener::{Context, EventType, Order};
use blaze_skyframe::{DirtyType, SkyKey};
use parking_lot::Mutex;

// -------------------------------------------------------------------------------------------
// The remote worker (Java: `@ClassRule WorkerInstance worker = IntegrationTestUtils.createWorker()`).

/// The oracle's embedded JDK and Bazel's remote worker built from the pinned source (the same
/// files `scripts/remote/worker` runs).
fn worker_command() -> (PathBuf, PathBuf) {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    (
        home.join(
            ".cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9/embedded_tools/jdk/bin/java",
        ),
        home.join(".cache/blaze/work/upstream-src/bazel-bin/src/tools/remote/worker_deploy.jar"),
    )
}

/// A running remote worker: the transient systemd user unit `blaze-remote-worker-<name>`,
/// stopped (and its directory deleted) when dropped, also when the test fails or panics. The unit
/// runs in the agents' slice and ends by itself after `RuntimeMaxSec`, so one that outlives a
/// killed test process (no drop) does not stay.
struct Worker {
    unit: String,
    dir: PathBuf,
    port: u16,
}

/// The longest a worker may run (one test function runs its case for eight combinations).
const WORKER_RUNTIME_MAX_SEC: u32 = 900;

impl Worker {
    fn start() -> Worker {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let name = format!("rewinding-test-{port}");
        let unit = format!("blaze-remote-worker-{name}");
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let dir = home
            .join(".cache/blaze/work/config-cluster/workers")
            .join(format!("worker-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::create_dir_all(dir.join("cas")).unwrap();
        // ExecutionServer resolves its (macOS-only) xcode-locator through runfiles; a directory
        // is enough.
        std::fs::create_dir_all(dir.join(
            "runfiles/io_bazel/src/tools/remote/src/main/java/com/google/devtools/build/remote/worker",
        ))
        .unwrap();
        // The guard exists before the unit does, so a failed start is cleaned up too.
        let worker = Worker { unit, dir, port };
        let (java, jar) = worker_command();
        let d = |p: &str| worker.dir.join(p).display().to_string();
        let status = Command::new("systemd-run")
            .args([
                "--user",
                "--collect",
                "--quiet",
                &format!("--unit={}", worker.unit),
                "--slice=app-t3code-agents-jobs.slice",
                "-p",
                "MemoryMax=8G",
                "-p",
                "MemorySwapMax=0",
                "-p",
                &format!("RuntimeMaxSec={WORKER_RUNTIME_MAX_SEC}"),
                &format!("--setenv=HOME={}", home.display()),
                "--setenv=PATH=/usr/bin:/bin",
                &format!("--setenv=RUNFILES_DIR={}", d("runfiles")),
                "-p",
                &format!("StandardOutput=file:{}", d("stdout")),
                "-p",
                &format!("StandardError=file:{}", d("stderr")),
            ])
            .arg(&java)
            .arg("-jar")
            .arg(&jar)
            .args([
                format!("--work_path={}", d("work")),
                format!("--cas_path={}", d("cas")),
                format!("--pid_file={}", d("pid")),
                format!("--listen_port={port}"),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "systemd-run of {}", worker.unit);
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return worker;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!("worker {} did not start; see {}", worker.unit, d("stderr"));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &self.unit])
            .status();
        // The worker leaves read-only files in its CAS and work directories.
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+w")
            .arg(&self.dir)
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// -------------------------------------------------------------------------------------------
// RewindingTest's setup.

/// One test instance: Java's `RewindingTest` object for one combination of the parameters.
struct RewindingTest {
    test_case: BuildIntegrationTestCase,
    helper: RewindingTestsHelper,
}

/// Runs `case` for every combination of Java's three `@TestParameter`s, each on a new fixture.
fn for_each_parameter_combination(worker: &Worker, case: impl Fn(&mut RewindingTest)) {
    for track_incremental_state in [false, true] {
        for keep_going in [false, true] {
            for skymeld in [false, true] {
                eprintln!(
                    "RewindingTest[trackIncrementalState={track_incremental_state}, \
                     keepGoing={keep_going}, skymeld={skymeld}]"
                );
                let mut test =
                    RewindingTest::new(worker, track_incremental_state, keep_going, skymeld);
                case(&mut test);
            }
        }
    }
}

impl RewindingTest {
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:70 RewindingTest.getRuntimeBuilder
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:103 RewindingTest.setupOptions
    fn new(
        worker: &Worker,
        track_incremental_state: bool,
        keep_going: bool,
        skymeld: bool,
    ) -> Self {
        let spawn_controller = SpawnController::new();
        let recorder = ActionEventRecorder::new();
        // PORT: RemoteModule, BlockWaitingModule, IncludeScanningModule and CredentialModule are
        // among Bazel's modules here; the lost-outputs module (top-level output rewinding) is not
        // ported yet; external repositories stay enabled: Java's anonymous module turns them off
        // in workspaceInit (WorkspaceBuilder.allowExternalRepositories(false)) to keep
        // RepoMappingManifestAction out of the action graph, and Blaze has no such switch yet.
        // The runfiles cases whose rewound keys count that action are ignored for it.
        let mut test_case = BuildIntegrationTestCase::new_with_modules(vec![Arc::new(
            ControllableActionStrategyModule::new(
                spawn_controller.clone(),
                &["remote", "standalone"],
            ),
        )]);
        test_case.add_options(&[
            "--enable_runfiles",
            "--spawn_strategy=remote",
            &format!("--remote_executor=grpc://localhost:{}", worker.port),
            "--remote_download_regex=.*\\.inlined$",
            "--noexperimental_merged_skyframe_analysis_execution",
            "--rewind_lost_inputs",
            "--features=cc_include_scanning",
            "--experimental_remote_include_extraction_size_threshold=0",
            "--experimental_inmemory_dotincludes_files",
            "--experimental_remote_cache_eviction_retries=0",
            &format!("--track_incremental_state={track_incremental_state}"),
            &format!("--keep_going={keep_going}"),
            &format!("--experimental_merged_skyframe_analysis_execution={skymeld}"),
        ]);
        test_case.register_subscriber(recorder.clone());
        RewindingTest {
            test_case,
            helper: RewindingTestsHelper {
                spawn_controller,
                recorder,
            },
        }
    }
}

// -------------------------------------------------------------------------------------------
// RewindingTestsHelper.

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:153 RewindingTestsHelper
struct RewindingTestsHelper {
    spawn_controller: Arc<SpawnController>,
    recorder: Arc<ActionEventRecorder>,
}

/// A file digest as a hex string with its size (`hash/size`).
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:177 RewindingTestsHelper.toHex
fn to_hex(digest: &[u8], size: i64) -> String {
    let mut hex = String::new();
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex.push('/');
    hex.push_str(&size.to_string());
    hex
}

/// `SpawnInputUtils.getInputWithName(spawn, name)`: the first input whose exec path contains
/// `name`.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnInputUtils.java:33 SpawnInputUtils.getInputWithName
fn get_input_with_name(spawn: &dyn Spawn, name: &str) -> ActionInputRef {
    spawn
        .get_input_files()
        .to_list()
        .iter()
        .find(|input| input.0.get_exec_path_string().contains(name))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "No spawn input with name {name} found in spawn {}",
                spawn.get_resource_owner().describe()
            )
        })
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:252 RewindingTestsHelper.getHexDigest
fn get_hex_digest(input: &ActionInputRef, context: &dyn ActionExecutionContext) -> String {
    let metadata = context
        .get_input_metadata_provider()
        .get_input_metadata(input)
        .unwrap_or_else(|e| panic!("IOException: {e}"))
        .unwrap_or_else(|| panic!("no metadata for {}", input.0.get_exec_path_string()));
    to_hex(metadata.get_digest().expect("digest"), metadata.get_size())
}

/// `createLostInputsExecException(context, lostInputs)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:243 RewindingTestsHelper.createLostInputsExecException
fn create_lost_inputs_exec_exception(
    context: &dyn ActionExecutionContext,
    lost_inputs: &[ActionInputRef],
) -> ExecResult {
    let mut lost = LostArtifacts::default();
    for lost_input in lost_inputs {
        lost.by_digest
            .entry(get_hex_digest(lost_input, context))
            .or_default()
            .insert(lost_input.clone());
    }
    ExecResult::of_exception(LostInputsExecException::create(lost))
}

/// `createLostInputsExecException(spawn, context, lostInputNames...)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:229 RewindingTestsHelper.createLostInputsExecException
fn create_lost_inputs_exec_exception_named(
    spawn: &dyn Spawn,
    context: &dyn ActionExecutionContext,
    lost_input_names: &[&str],
) -> ExecResult {
    let inputs: Vec<ActionInputRef> = lost_input_names
        .iter()
        .map(|name| get_input_with_name(spawn, name))
        .collect();
    create_lost_inputs_exec_exception(context, &inputs)
}

/// `latin1StringFromActionInput(context, input)`: only inputs ending in `.inlined` are fetched
/// eagerly (`--remote_download_regex`), so only they are guaranteed readable.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:320 RewindingTestsHelper.latin1StringFromActionInput
fn latin1_string_from_action_input(
    context: &dyn ActionExecutionContext,
    input: &ActionInputRef,
) -> String {
    let exec_path = input.0.get_exec_path_string();
    assert!(
        exec_path.ends_with(".inlined"),
        "Only inputs ending in .inlined are guaranteed readable. Tried to read: {exec_path}"
    );
    let path = context.get_input_path(&*input.0);
    let bytes = std::fs::read(path.get_path_string()).unwrap();
    bytes.iter().map(|b| *b as char).collect()
}

impl RewindingTestsHelper {
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:213 RewindingTestsHelper.getExecutedSpawnDescriptions
    fn get_executed_spawn_descriptions(&self) -> Vec<String> {
        self.spawn_controller.get_executed_spawn_descriptions()
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:221 RewindingTestsHelper.addSpawnShim
    fn add_spawn_shim(
        &self,
        spawn_description: &str,
        shim: impl FnOnce(&dyn Spawn, &dyn ActionExecutionContext) -> ExecResult + Send + 'static,
    ) {
        self.spawn_controller.add_spawn_shim(
            spawn_description,
            Box::new(
                move |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
                    Ok(shim(spawn, context))
                },
            ),
        );
    }

    /// Injects a listener that collects keys rewound by rewinding into the returned list,
    /// starting with the next build. `ArtifactNestedSetKey` is not collected (its nested set may
    /// contain several paths to a lost artifact, any of which is a correct chain for rewinding).
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:267 RewindingTestsHelper.collectOrderedRewoundKeys
    fn collect_ordered_rewound_keys(
        &self,
        test_case: &BuildIntegrationTestCase,
    ) -> Arc<Mutex<Vec<SkyKey>>> {
        let rewound_keys: Arc<Mutex<Vec<SkyKey>>> = Arc::new(Mutex::new(Vec::new()));
        let collected = rewound_keys.clone();
        test_case.inject_listener_at_start_of_next_build(Arc::new(
            move |key: &SkyKey, event_type: EventType, order: Order, context: &Context| {
                if event_type == EventType::MarkDirty
                    && order == Order::After
                    && let Context::MarkDirtyAfter(after) = context
                    && after.dirty_type == DirtyType::Rewind
                    && after.actually_dirtied
                    // Ignore ArtifactNestedSetKey. See method doc.
                    && key.downcast::<ArtifactNestedSetKey>().is_none()
                {
                    collected.lock().push(key.clone());
                }
            },
        ));
        rewound_keys
    }

    /// Builds the genrule `//{pkg}:consume_output`, which must have `//{pkg}:output.inlined` as a
    /// `srcs` dep, and returns the contents of `output.inlined` read during its execution.
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:341 RewindingTestsHelper.buildAndGetOutput
    fn build_and_get_output(&self, pkg: &str, test_case: &mut BuildIntegrationTestCase) -> String {
        let invocation_output: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let output = invocation_output.clone();
        self.add_spawn_shim(
            &format!("Executing genrule //{pkg}:consume_output"),
            move |spawn, context| {
                let action_input = get_input_with_name(spawn, "output.inlined");
                *output.lock() = Some(latin1_string_from_action_input(context, &action_input));
                ExecResult::delegate()
            },
        );
        let result = test_case.build_target(&[&format!("//{pkg}:consume_output")]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );
        invocation_output.lock().take().expect("consume_output ran")
    }
}

/// `assertOnlyActionsRewound(rewoundKeys)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:285 RewindingTestsHelper.assertOnlyActionsRewound
fn assert_only_actions_rewound(rewound_keys: &[SkyKey]) {
    for key in rewound_keys {
        if key.downcast::<ArtifactNestedSetKey>().is_none() {
            assert!(
                key.downcast::<ActionExecutionKey>().is_some(),
                "{key:?} is not an action key"
            );
        }
    }
}

/// `rewoundArtifactOwnerLabels(rewoundKeys)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:293 RewindingTestsHelper.rewoundArtifactOwnerLabels
fn rewound_artifact_owner_labels(rewound_keys: &[SkyKey]) -> Vec<String> {
    rewound_keys
        .iter()
        .filter_map(|k| k.downcast::<ActionExecutionKey>())
        .map(|k| k.0.get_label().expect("label").get_canonical_form())
        .collect()
}

/// Truth's `containsExactly(expected...)` (as multisets, any order).
fn assert_contains_exactly(actual: &[String], expected: &[&str]) {
    let mut actual_sorted: Vec<&str> = actual.iter().map(String::as_str).collect();
    let mut expected_sorted: Vec<&str> = expected.to_vec();
    actual_sorted.sort();
    expected_sorted.sort();
    assert_eq!(actual_sorted, expected_sorted, "actual: {actual:?}");
}

/// Truth's `containsExactly(expected...).inOrder()`.
fn assert_contains_exactly_in_order(actual: &[String], expected: &[&str]) {
    assert_eq!(actual, expected);
}

/// `assertContainsError(expected)`: an error event containing `expected`.
// Java: src/test/java/com/google/devtools/build/lib/buildtool/util/BuildIntegrationTestCase.java:1250 BuildIntegrationTestCase.assertContainsError
fn assert_contains_error(test_case: &BuildIntegrationTestCase, expected: &str) {
    let events = test_case.events();
    assert!(
        events
            .iter()
            .any(|e| e.get_kind() == EventKind::Error && e.get_message().contains(expected)),
        "no error containing {expected:?} among {:?}",
        events.iter().map(|e| e.get_message()).collect::<Vec<_>>()
    );
}

/// `assertActionKey(skyKey, label, index)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:305 RewindingTestsHelper.assertActionKey
fn assert_action_key(sky_key: &SkyKey, label: &str, index: i32) {
    let key = sky_key
        .downcast::<ActionExecutionKey>()
        .unwrap_or_else(|| panic!("{sky_key:?} is not an action key"));
    assert_eq!(
        key.0.get_label().expect("label").get_canonical_form(),
        label
    );
    assert_eq!(key.0.get_action_index(), index);
}

/// `setUpTreeArtifactPackage(testCase)`: a Starlark rule `make_cc` that makes a tree artifact
/// and a `cc_library` over it (one compile per file of the tree, then the links).
///
/// PORT: Java's mock module setup makes `@rules_cc` visible; here the workspace's `MODULE.bazel`
/// adds the real module (from the shared repository cache) next to the fixture's `platforms`.
/// Java's mock C++ toolchain (`BazelMockCcSupport`) is replaced by the real autoconfigured one,
/// which looks for the compiler on the repository environment's `PATH`; the fixture's command
/// has no client environment (Java's tests have none either), so the setup passes one.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1448 RewindingTestsHelper.setUpTreeArtifactPackage
fn set_up_tree_artifact_package(test_case: &mut BuildIntegrationTestCase) {
    test_case.write(
        "MODULE.bazel",
        &[
            "bazel_dep(name = \"platforms\", version = \"1.0.0\")",
            "bazel_dep(name = \"rules_cc\", version = \"0.2.17\")",
        ],
    );
    test_case.write(
        "tree/tree.bzl",
        &[
            "def _tree_impl(ctx):",
            "    tree_artifact = ctx.actions.declare_directory(ctx.attr.name + \"_dir.cc\")",
            "    ctx.actions.run_shell(",
            "        inputs = ctx.files.srcs,",
            "        outputs = [tree_artifact],",
            "        command = \"touch $1/file1.cc && touch $1/file2.cc\",",
            "        arguments = [tree_artifact.path],",
            "    )",
            "    return DefaultInfo(files = depset(direct = [tree_artifact]))",
            "",
            "tree = rule(",
            "    implementation = _tree_impl,",
            "    attrs = {\"srcs\": attr.label_list(allow_files = True)},",
            ")",
        ],
    );
    test_case.write(
        "tree/BUILD",
        &[
            "load(\"@rules_cc//cc:cc_library.bzl\", \"cc_library\")",
            "load(\":tree.bzl\", \"tree\")",
            "",
            "tree(",
            "    name = \"make_cc\",",
            "    srcs = [\"source_1.txt\"],",
            ")",
            "",
            "cc_library(",
            "    name = \"consumes_tree\",",
            "    srcs = [",
            "        \"source_2.cc\",",
            "        \":make_cc\",",
            "    ],",
            ")",
        ],
    );
    test_case.write("tree/source_1.txt", &["source_1"]);
    test_case.write("tree/source_2.cc", &["#define FOO"]);
    // Don't want to have to track inclusion extraction for tree file artifacts.
    test_case.add_options(&["--features=-cc_include_scanning"]);
    // PORT: the real toolchain's autoconfiguration (see above).
    test_case.add_options(&["--repo_env=PATH=/usr/bin:/bin"]);
}

/// `SpawnInputUtils.getTreeArtifactWithName(spawn, name)`.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnInputUtils.java:76 SpawnInputUtils.getTreeArtifactWithName
fn get_tree_artifact_with_name(spawn: &dyn Spawn, name: &str) -> ActionInputRef {
    let input = get_input_with_name(spawn, name);
    assert!(
        input.0.as_artifact().is_some_and(|a| a.is_tree_artifact()),
        "Expected spawn {} to have tree artifact input with name {name}, but it is: {}",
        spawn.get_resource_owner().describe(),
        input.0.get_exec_path_string()
    );
    input
}

/// `SpawnInputUtils.getExpandedToArtifact(name, expandableArtifact, spawn, context)`.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnInputUtils.java:87 SpawnInputUtils.getExpandedToArtifact
fn get_expanded_to_artifact(
    name: &str,
    expandable_artifact: &ActionInputRef,
    spawn: &dyn Spawn,
    context: &dyn ActionExecutionContext,
) -> ActionInputRef {
    context
        .get_input_metadata_provider()
        .get_tree_metadata(expandable_artifact)
        .expect("tree metadata")
        .get_children()
        .into_iter()
        .find(|artifact| artifact.0.get_exec_path_string().contains(name))
        .unwrap_or_else(|| {
            panic!(
                "No artifact expanded from {} with name {name} found in spawn {}",
                expandable_artifact.0.get_exec_path_string(),
                spawn.get_resource_owner().describe()
            )
        })
}

/// `assertArtifactKey(skyKey, path)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:300 RewindingTestsHelper.assertArtifactKey
fn assert_artifact_key(sky_key: &SkyKey, path: &str) {
    let key = sky_key
        .downcast::<blaze_sky_exec::keys::ArtifactKey>()
        .unwrap_or_else(|| panic!("{sky_key:?} is not an artifact key"));
    assert_eq!(key.0.get_root_relative_path_string(), path);
}

/// `runTreeArtifactRewoundWhenTreeFilesLost` with its spawn-failed shim.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1620 RewindingTestsHelper.runTreeArtifactRewoundWhenTreeFilesLost
fn run_tree_artifact_rewound_when_tree_files_lost(
    t: &mut RewindingTest,
    lost_tree_file_artifact_names: &'static [&'static str],
) {
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1595 RewindingTestsHelper.getTreeArtifactRewoundWhenTreeFilesLostSpawnFailedShim
    let shim = move |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1606 RewindingTestsHelper.getTreeArtifactRewoundWhenTreeFilesLostTree
        let tree_artifact = get_tree_artifact_with_name(spawn, "make_cc_dir");
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1610 RewindingTestsHelper.getTreeArtifactRewoundWhenTreeFilesLostInputs
        let lost_tree_file_artifacts: Vec<ActionInputRef> = lost_tree_file_artifact_names
            .iter()
            .map(|n| get_expanded_to_artifact(n, &tree_artifact, spawn, context))
            .collect();
        create_lost_inputs_exec_exception(context, &lost_tree_file_artifacts)
    };
    set_up_tree_artifact_package(&mut t.test_case);

    t.helper
        .add_spawn_shim("Linking tree/libconsumes_tree.so", shim);

    let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
    let result = t.test_case.build_target(&["//tree:consumes_tree"]);
    assert!(
        result.is_success(),
        "build failed: {:?}",
        result.get_failure_detail()
    );
    t.helper.spawn_controller.verify_all_shims_consumed();
    assert_contains_exactly(
        &t.helper.get_executed_spawn_descriptions(),
        &[
            "Action tree/make_cc_dir.cc",
            "Compiling tree/make_cc_dir.cc/file1.cc",
            "Compiling tree/make_cc_dir.cc/file2.cc",
            "Compiling tree/source_2.cc",
            "Linking tree/libconsumes_tree.so",
            "Compiling tree/make_cc_dir.cc/file1.cc",
            "Compiling tree/make_cc_dir.cc/file2.cc",
            "Linking tree/libconsumes_tree.so",
            "Linking tree/libconsumes_tree.a",
        ],
    );

    t.helper.recorder.assert_events(
        /* run_once= */
        &[
            "Action tree/make_cc_dir.cc",
            "Linking tree/libconsumes_tree.a",
        ],
        /* completed_rewound= */
        &[
            "Compiling tree/make_cc_dir.cc/file1.cc",
            "Compiling tree/make_cc_dir.cc/file2.cc",
        ],
        /* failed_rewound= */ &["Linking tree/libconsumes_tree.so"],
        /* action_rewinding_post_lost_input_counts= */
        &[lost_tree_file_artifact_names.len()],
    );

    let rewound_keys = rewound_keys.lock().clone();
    assert_eq!(rewound_keys.len(), 3, "{rewound_keys:?}");
    let mut tree_action_indices: std::collections::HashSet<i32> = [0, 1].into_iter().collect();
    for key in &rewound_keys[..2] {
        let key = key
            .downcast::<ActionExecutionKey>()
            .unwrap_or_else(|| panic!("{key:?} is not an action key"));
        assert_eq!(
            key.0.get_label().expect("label").get_canonical_form(),
            "//tree:consumes_tree"
        );
        assert!(tree_action_indices.remove(&key.0.get_action_index()));
    }
    assert_artifact_key(&rewound_keys[2], "tree/_pic_objs/consumes_tree/make_cc_dir");
}

/// `SpawnInputUtils.getRunfilesArtifactWithName(spawn, context, name)`.
///
/// PORT: Java reads the runfiles tree's `getArtifacts()`; Blaze's `RunfilesTree` has the
/// mapping, whose values are those artifacts.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnInputUtils.java:100 SpawnInputUtils.getRunfilesArtifactWithName
fn get_runfiles_artifact_with_name(
    spawn: &dyn Spawn,
    context: &dyn ActionExecutionContext,
    name: &str,
) -> ActionInputRef {
    spawn
        .get_input_files()
        .to_list()
        .iter()
        .filter(|i| i.0.as_artifact().is_some_and(|a| a.is_runfiles_tree()))
        .filter_map(|i| {
            context
                .get_input_metadata_provider()
                .get_runfiles_metadata(i)
        })
        .flat_map(|t| {
            t.get_runfiles_tree()
                .get_mapping()
                .into_iter()
                .filter_map(|(_, artifact)| artifact)
                .collect::<Vec<_>>()
        })
        .find(|artifact| artifact.0.get_exec_path_string().contains(name))
        .unwrap_or_else(|| {
            panic!(
                "No runfiles artifact with name {name} found in spawn {}",
                spawn.get_resource_owner().describe()
            )
        })
}

/// `mockFooBinary(relativePath)`: an executable rule whose runfiles are its srcs and data.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2101 RewindingTestsHelper.mockFooBinary
fn mock_foo_binary(test_case: &BuildIntegrationTestCase, relative_path: &str) {
    test_case.write(
        relative_path,
        &[
            "def _impl(ctx):",
            "  symlink = ctx.actions.declare_file(ctx.label.name)",
            "  ctx.actions.symlink(output = symlink, target_file = ctx.files.srcs[0],",
            "    is_executable = True)",
            "  files = depset(ctx.files.srcs)",
            "  return [DefaultInfo(files = files, executable = symlink,",
            "     runfiles = ctx.runfiles(transitive_files = files, collect_default = True))]",
            "foo_binary = rule(",
            "  implementation = _impl,",
            "  executable = True,",
            "  attrs = {",
            "    \"srcs\": attr.label_list(allow_files=True),",
            "    \"deps\": attr.label_list(),",
            "    \"data\": attr.label_list(allow_files=True),",
            "  },",
            ")",
        ],
    );
}

/// `buildRunfileManifests()`: `CoreOptions.buildRunfileManifests` of the fixture's options
/// (default true; no case here sets it).
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:3650 RewindingTestsHelper.buildRunfileManifests
fn build_runfile_manifests(test_case: &BuildIntegrationTestCase) -> bool {
    let mut value = true;
    for option in test_case.get_options() {
        match option.as_str() {
            "--build_runfile_manifests" | "--build_runfile_manifests=true" => value = true,
            "--nobuild_runfile_manifests" | "--build_runfile_manifests=false" => value = false,
            _ => {}
        }
    }
    value
}

/// `runGeneratedRunfilesRewound(lostRunfiles, shim)` with the spawn-failed shim.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2124 RewindingTestsHelper.runGeneratedRunfilesRewound
fn run_generated_runfiles_rewound(t: &mut RewindingTest, lost_runfiles: &'static [&'static str]) {
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2086 RewindingTestsHelper.getGeneratedRunfilesRewoundSpawnFailedShim
    let shim = move |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2094 RewindingTestsHelper.getGeneratedRunfilesRewoundLostRunfiles
        let lost_runfile_artifacts: Vec<ActionInputRef> = lost_runfiles
            .iter()
            .map(|n| get_runfiles_artifact_with_name(spawn, context, n))
            .collect();
        create_lost_inputs_exec_exception(context, &lost_runfile_artifacts)
    };
    mock_foo_binary(&t.test_case, "middle/foo_binary.bzl");
    t.test_case.write(
        "middle/BUILD",
        &[
            "load(\":foo_binary.bzl\", \"foo_binary\")",
            "genrule(",
            "    name = \"gen1\",",
            "    srcs = [],",
            "    outs = [\"gen1.dat\"],",
            "    cmd = 'echo \"made by gen1\" > $@',",
            ")",
            "",
            "genrule(",
            "    name = \"gen2\",",
            "    srcs = [],",
            "    outs = [\"gen2.dat\"],",
            "    cmd = 'echo \"made by gen2\" > $@',",
            ")",
            "",
            "foo_binary(",
            "    name = \"tool\",",
            "    srcs = [\"tool.sh\"],",
            "    data = [",
            "        \"gen1.dat\",",
            "        \"gen2.dat\",",
            "        \"source_1.txt\",",
            "    ],",
            ")",
            "",
            "genrule(",
            "    name = \"tool_user\",",
            "    srcs = [],",
            "    outs = [\"tool_user.out\"],",
            "    cmd = \"touch $(OUTS)\",",
            "    tools = [\"tool\"],",
            ")",
        ],
    );
    let tool = t.test_case.write("middle/tool.sh", &["#!/bin/bash"]);
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    t.test_case.write("middle/source_1.txt", &["source_1"]);

    t.helper
        .add_spawn_shim("Executing genrule //middle:tool_user", shim);

    let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
    let result = t.test_case.build_target(&["//middle:tool_user"]);
    assert!(
        result.is_success(),
        "build failed: {:?}",
        result.get_failure_detail()
    );
    t.helper.spawn_controller.verify_all_shims_consumed();
    assert_contains_exactly(
        &t.helper.get_executed_spawn_descriptions(),
        &[
            "Executing genrule //middle:gen1 [for tool]",
            "Executing genrule //middle:gen2 [for tool]",
            "Executing genrule //middle:tool_user",
            "Executing genrule //middle:gen1 [for tool]",
            "Executing genrule //middle:gen2 [for tool]",
            "Executing genrule //middle:tool_user",
        ],
    );

    t.helper.recorder.assert_events(
        /* run_once= */ &[],
        /* completed_rewound= */
        &[
            "Executing genrule //middle:gen1 [for tool]",
            "Executing genrule //middle:gen2 [for tool]",
        ],
        /* failed_rewound= */ &["Executing genrule //middle:tool_user"],
        /* action_rewinding_post_lost_input_counts= */ &[lost_runfiles.len()],
    );

    let rewound_keys = rewound_keys.lock().clone();
    let action_key = |key: &SkyKey| {
        key.downcast::<ActionExecutionKey>()
            .unwrap_or_else(|| panic!("{key:?} is not an action key"))
            .0
            .clone()
    };
    if build_runfile_manifests(&t.test_case) {
        assert_eq!(rewound_keys.len(), 6, "{rewound_keys:?}");
        let mut expected_rewound_genrules: std::collections::HashSet<&str> =
            ["//middle:gen1", "//middle:gen2"].into_iter().collect();
        let mut i = 0;
        let mut source_manifest_action_seen = false;
        while i < 5 {
            let key = action_key(&rewound_keys[i]);
            let action_label = key.get_label().expect("label").get_canonical_form();
            i += 1;
            if action_label == "//middle:tool" {
                match key.get_action_index() {
                    // SymlinkAction
                    0 => {}
                    1 => source_manifest_action_seen = true,
                    // SymlinkTreeAction
                    2 => assert!(source_manifest_action_seen),
                    _ => panic!(
                        "Unexpected action index. actionKey: {key:?}, rewoundKeys: {rewound_keys:?}"
                    ),
                }
            } else {
                assert!(expected_rewound_genrules.remove(action_label.as_str()));
            }
        }
        assert_action_key(&rewound_keys[i], "//middle:tool", /* index= */ 3);
    } else {
        assert_eq!(rewound_keys.len(), 4, "{rewound_keys:?}");
        let mut expected_rewound_genrules: std::collections::HashSet<&str> =
            ["//middle:gen1", "//middle:gen2"].into_iter().collect();
        let mut i = 0;
        while i < 3 {
            let key = action_key(&rewound_keys[i]);
            let action_label = key.get_label().expect("label").get_canonical_form();
            i += 1;
            if action_label == "//middle:tool" {
                assert_eq!(key.get_action_index(), 0);
            } else {
                assert!(expected_rewound_genrules.remove(action_label.as_str()));
            }
        }
        assert_action_key(&rewound_keys[i], "//middle:tool", /* index= */ 1);
    }
}

/// `write(path, text)` for a text block: one line per line of `text`.
fn write_text(test_case: &BuildIntegrationTestCase, relative_path: &str, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    test_case.write(relative_path, &lines);
}

/// `ensureMinimumJobs(minJobs)`: `--jobs` at least `min_jobs` (the default is the host's CPUs).
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:3636 RewindingTestsHelper.ensureMinimumJobs
fn ensure_minimum_jobs(test_case: &mut BuildIntegrationTestCase, min_jobs: usize) {
    // JobsConverter().convert("auto"): HOST_CPUS.
    let auto_jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if auto_jobs < min_jobs {
        test_case.add_options(&[&format!("--jobs={min_jobs}")]);
    }
}

/// The number of actions that consume the tree artifact populated by an action template
/// expansion concurrently in `actionTemplateExpansionRewound_notConcurrentWithTreeConsumers`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1669 RewindingTestsHelper.TREE_CONSUMER_COUNT
const TREE_CONSUMER_COUNT: usize = 8;

/// A tool that copies its second argument to its first (the non-Windows script).
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1677 RewindingTestsHelper.COPY_TOOL_SCRIPT
const COPY_TOOL_SCRIPT: &str = "#!/bin/bash\ncp \"$2\" \"$1\"\n";

/// A subscriber counting the spawns of `Consuming //foo:consumer_*` as they finish.
struct ConsumerSpawnCounter {
    consumers_reading_tree: Arc<std::sync::atomic::AtomicI64>,
}

impl blaze_events::EventBus for ConsumerSpawnCounter {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        let Some(event) = blaze_sky_exec::events::downcast_event::<
            blaze_exec::exec::action_execution_context::SpawnExecutedEvent,
        >(&event) else {
            return;
        };
        // event.getActionMetadata().describe(): the progress message, else "Executing " +
        // the mnemonic (ActionExecutionMetadata.describe).
        let describe = event
            .get_action_progress_message()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Executing {}", event.get_action_mnemonic()));
        if describe.starts_with("Consuming //foo:consumer_") {
            self.consumers_reading_tree
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// The `foo/defs.bzl` of the template expansion cases.
const DEFS: &str = r#"def _copy_tool_impl(ctx):
    tool = ctx.actions.declare_file(ctx.attr.name + ".bat")
    ctx.actions.write(tool, r"""COPY_TOOL_SCRIPT""", is_executable = True)
    return DefaultInfo(files = depset([tool]), executable = tool)

copy_tool = rule(implementation = _copy_tool_impl, executable = True)

def _map_impl(template_ctx, input_directories, output_directories, tools, **kwargs):
    for child in input_directories["seed"].children:
        # The two actions form a chain within the expansion, so the second one consumes a
        # file of the very tree artifact it populates.
        mid = template_ctx.declare_file(
            child.basename + ".mid",
            directory = output_directories["mapped"],
        )
        args = template_ctx.args()
        args.add_all([mid, child])
        template_ctx.run(
            inputs = [child],
            outputs = [mid],
            executable = tools["copy_tool"],
            arguments = [args],
            progress_message = "Mapping foo/mapped_dir (1)",
        )
        out = template_ctx.declare_file(
            child.basename + ".out",
            directory = output_directories["mapped"],
        )
        args = template_ctx.args()
        args.add_all([out, mid])
        template_ctx.run(
            inputs = [mid],
            outputs = [out],
            executable = tools["copy_tool"],
            arguments = [args],
            progress_message = "Mapping foo/mapped_dir (2)",
        )

def _mapped_tree_impl(ctx):
    seed = ctx.actions.declare_directory("seed_dir")
    ctx.actions.run_shell(
        outputs = [seed],
        command = "echo seed > $1/f1",
        arguments = [seed.path],
        progress_message = "Seeding foo/seed_dir",
    )
    mapped = ctx.actions.declare_directory("mapped_dir")
    ctx.actions.map_directory(
        implementation = _map_impl,
        input_directories = {"seed": seed},
        output_directories = {"mapped": mapped},
        tools = {"copy_tool": ctx.attr._copy_tool.files_to_run},
        # Ensure that the rewound expansion action re-executes its spawn instead of
        # picking up the result of its first execution from the cache.
        execution_requirements = {"no-cache": "1"},
    )
    return DefaultInfo(files = depset([mapped]))

mapped_tree = rule(
    implementation = _mapped_tree_impl,
    attrs = {
        "_copy_tool": attr.label(
            default = ":copy_tool",
            executable = True,
            cfg = "exec",
        ),
    },
)

def _consumer_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.name + ".out")
    ctx.actions.run_shell(
        inputs = ctx.files.srcs,
        outputs = [out],
        command = "echo consumed > $1",
        arguments = [out.path],
        progress_message = "Consuming //foo:" + ctx.attr.name,
    )
    return DefaultInfo(files = depset([out]))

consumer = rule(
    implementation = _consumer_impl,
    attrs = {"srcs": attr.label_list(allow_files = True)},
)
"#;

/// The `foo/BUILD` of `notConcurrentWithTreeConsumers` before its consumers.
const BUILD: &str = r#"load(":defs.bzl", "consumer", "copy_tool", "mapped_tree")

copy_tool(name = "copy_tool")

mapped_tree(name = "mapped_tree")

genrule(
    name = "warmup_gen",
    outs = ["warmup.out"],
    cmd = "echo warmup > $@",
    tags = ["no-cache"],
)

genrule(
    name = "warmup_consumer",
    srcs = ["warmup.out"],
    outs = ["warmup_consumed.out"],
    cmd = "cp $< $@",
)

consumer(
    name = "losing_consumer",
    srcs = [
        "warmup_consumed.out",
        ":mapped_tree",
    ],
)
"#;

/// One `consumer_{i}` target.
const CONSUMER: &str = r#"consumer(
    name = "consumer_{i}",
    srcs = [
        "warmup_consumed.out",
        ":mapped_tree",
    ],
)
"#;

/// The `foo/defs.bzl` of `siblingActionsReExecuteConcurrently`.
const SIBLING_DEFS: &str = r#"def _copy_tool_impl(ctx):
    tool = ctx.actions.declare_file(ctx.attr.name + ".bat")
    ctx.actions.write(tool, r"""COPY_TOOL_SCRIPT""", is_executable = True)
    return DefaultInfo(files = depset([tool]), executable = tool)

copy_tool = rule(implementation = _copy_tool_impl, executable = True)

def _map_impl(template_ctx, input_directories, output_directories, tools, **kwargs):
    for child in input_directories["seed"].children:
        out = template_ctx.declare_file(
            child.basename + ".out",
            directory = output_directories["mapped"],
        )
        args = template_ctx.args()
        args.add_all([out, child])
        template_ctx.run(
            inputs = [child],
            outputs = [out],
            executable = tools["copy_tool"],
            arguments = [args],
            progress_message = "Mapping foo/mapped_dir " + child.basename,
        )

def _mapped_tree_impl(ctx):
    seed = ctx.actions.declare_directory("seed_dir")
    ctx.actions.run_shell(
        outputs = [seed],
        command = "SEED_COMMAND",
        arguments = [seed.path],
        progress_message = "Seeding foo/seed_dir",
    )
    mapped = ctx.actions.declare_directory("mapped_dir")
    ctx.actions.map_directory(
        implementation = _map_impl,
        input_directories = {"seed": seed},
        output_directories = {"mapped": mapped},
        tools = {"copy_tool": ctx.attr._copy_tool.files_to_run},
        # Ensure that the rewound expansion actions re-execute their spawns instead of
        # picking up the results of their first executions from the cache.
        execution_requirements = {"no-cache": "1"},
    )
    return DefaultInfo(files = depset([mapped]))

mapped_tree = rule(
    implementation = _mapped_tree_impl,
    attrs = {
        "_copy_tool": attr.label(
            default = ":copy_tool",
            executable = True,
            cfg = "exec",
        ),
    },
)
"#;

/// The `foo/BUILD` of `siblingActionsReExecuteConcurrently`.
const SIBLING_BUILD: &str = r#"load(":defs.bzl", "copy_tool", "mapped_tree")

copy_tool(name = "copy_tool")

mapped_tree(name = "mapped_tree")

genrule(
    name = "losing_consumer",
    srcs = [":mapped_tree"],
    outs = ["consumed.out"],
    cmd = "echo consumed > $@",
)
"#;

/// `writeTwoGenrulePackage(testCase)`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:558 RewindingTestsHelper.writeTwoGenrulePackage
fn write_two_genrule_package(test_case: &BuildIntegrationTestCase) {
    test_case.write(
        "test/BUILD",
        &[
            "genrule(",
            "    name = \"rule1\",",
            "    srcs = [\"source_1.txt\"],",
            "    outs = [\"intermediate.txt\"],",
            "    cmd = \"(cat $< && echo from rule1) > $@\",",
            ")",
            "",
            "genrule(",
            "    name = \"rule2\",",
            "    srcs = [",
            "        \"intermediate.txt\",",
            "        \"source_2.txt\",",
            "    ],",
            "    outs = [\"output.inlined\"],",
            "    cmd = \"(cat $(SRCS) && echo from rule2) > $@\",",
            ")",
            "",
            "genrule(",
            "    name = \"consume_output\",",
            "    srcs = [\":output.inlined\"],",
            "    outs = [\"dummy.out\"],",
            "    cmd = \"touch $@\",",
            ")",
        ],
    );
    test_case.write("test/source_1.txt", &["source_1"]);
    test_case.write("test/source_2.txt", &["source_2"]);
}

/// `assertOutputForStopBeforeRewoundReexecution()`.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:809 RewindingTestsHelper.assertOutputForStopBeforeRewoundReexecution
fn assert_output_for_stop_before_rewound_reexecution(helper: &RewindingTestsHelper) {
    assert_contains_exactly_in_order(
        &helper.get_executed_spawn_descriptions(),
        &[
            "Executing genrule //test:rule1",
            "Executing genrule //test:rule2",
            "Executing genrule //test:rule1",
        ],
    );
    helper.recorder.assert_events(
        /* run_once= */ &["Executing genrule //test:rule1"],
        /* completed_rewound= */ &[],
        /* failed_rewound= */ &[],
        /* action_rewinding_post_lost_input_counts= */ &[1],
    );
    let rule2 = "Executing genrule //test:rule2";
    let count = |events: Vec<String>| events.iter().filter(|d| *d == rule2).count();
    assert_eq!(count(helper.recorder.get_action_started_events()), 1);
    assert_eq!(count(helper.recorder.get_action_completion_events()), 0);
    assert_eq!(count(helper.recorder.get_action_executed_events()), 0);
    assert_eq!(
        count(helper.recorder.get_action_result_received_events()),
        0
    );
    assert_eq!(count(helper.recorder.get_action_rewound_events()), 1);
}

/// `assertOutputForRule2NotCreated()`: the file to build of `//test:rule2` (`output.inlined`)
/// does not exist.
///
/// PORT: Java finds the artifact through `getExistingConfiguredTarget("//test:rule2")`; the
/// fixture has no analysis access, so no `bazel-out/*/bin/test/output.inlined` may exist in any
/// configuration of the exec root (which includes Java's one).
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:192 RewindingTest.assertOutputForRule2NotCreated
fn assert_output_for_rule2_not_created(test_case: &BuildIntegrationTestCase) {
    let bazel_out = test_case.get_output_base().join("execroot/_main/bazel-out");
    let Ok(configurations) = std::fs::read_dir(&bazel_out) else {
        return;
    };
    for configuration in configurations.flatten() {
        let output = configuration.path().join("bin/test/output.inlined");
        assert!(!output.exists(), "{} exists", output.display());
    }
}

// -------------------------------------------------------------------------------------------
// The cases.

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:155 RewindingTest.noLossSmokeTest
#[test]
fn no_loss_smoke_test() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:354 RewindingTestsHelper.runNoLossSmokeTest
        t.test_case.write(
            "test/BUILD",
            &[
                "genrule(",
                "    name = \"rule1\",",
                "    srcs = [\"source.txt\"],",
                "    outs = [\"intermediate.txt\"],",
                "    cmd = \"(cat $< && echo from rule1) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [\"intermediate.txt\"],",
                "    outs = [\"output.inlined\"],",
                "    cmd = \"(cat $< && echo from rule2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"consume_output\",",
                "    srcs = [\":output.inlined\"],",
                "    outs = [\"dummy.out\"],",
                "    cmd = \"touch $@\",",
                ")",
            ],
        );
        t.test_case.write("test/source.txt", &["source"]);

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let output_file_content = t.helper.build_and_get_output("test", &mut t.test_case);

        assert_eq!(output_file_content, "source\nfrom rule1\nfrom rule2\n");
        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:consume_output",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
            ],
            /* completed_rewound= */ &[],
            /* failed_rewound= */ &[],
            /* action_rewinding_post_lost_input_counts= */ &[0],
        );

        assert!(rewound_keys.lock().is_empty());
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:160 RewindingTest.lostInputWithRewindingDisabled
#[test]
fn lost_input_with_rewinding_disabled() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:402 RewindingTestsHelper.runLostInputWithRewindingDisabled
        t.test_case.write(
            "foo/BUILD",
            &[
                "genrule(name = 'top', outs = ['top.out'], srcs = [':dep'], cmd = 'cp $< $@')",
                "genrule(name = 'dep', outs = ['dep.out'], cmd = 'touch $@')",
            ],
        );
        t.test_case.add_options(&["--norewind_lost_inputs"]);
        t.helper
            .add_spawn_shim("Executing genrule //foo:top", |spawn, context| {
                create_lost_inputs_exec_exception_named(spawn, context, &["dep.out"])
            });

        let result = t.test_case.build_target(&["//foo:top"]);
        assert!(!result.is_success());
        match result
            .get_failure_detail()
            .and_then(|f| f.category.as_ref())
        {
            Some(blaze_protos::failure_details::failure_detail::Category::ActionRewinding(r)) => {
                assert_eq!(
                    r.code,
                    blaze_protos::failure_details::action_rewinding::Code::LostInputRewindingDisabled
                        as i32
                );
            }
            other => panic!("not an ActionRewinding failure: {other:?}"),
        }
        assert_contains_error(
            &t.test_case,
            "Executing genrule //foo:top failed: Unexpected lost inputs (pass \
             --rewind_lost_inputs to enable recovery): foo/dep.out",
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:165 RewindingTest.buildingParentFoundUndoneChildNotToleratedWithoutRewinding
#[test]
#[ignore = "needs testCase.setCustomBugReporterAndReinitialize (a BugReporter mock) and a GET_BATCH event with Reason.PREFETCH, which Blaze's graph does not emit"]
fn building_parent_found_undone_child_not_tolerated_without_rewinding() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:170 RewindingTest.dependentActionsReevaluated
#[test]
fn dependent_actions_reevaluated() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:470 RewindingTestsHelper.runDependentActionsReevaluated_spawnFailed
        // The first time rule2 is executed, the execution strategy fails, saying that rule2's two
        // input files are missing.
        let shim = |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
            // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:478 RewindingTestsHelper.getIntermediate1And2LostInputs
            let lost = [
                get_input_with_name(spawn, "intermediate_1.txt"),
                get_input_with_name(spawn, "intermediate_2.txt"),
            ];
            create_lost_inputs_exec_exception(context, &lost)
        };
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:484 RewindingTestsHelper.runDependentActionsReevaluated
        // This test sets up a genrule, rule2, that consumes the outputs of two other genrules.
        t.test_case.write(
            "test/BUILD",
            &[
                "genrule(",
                "    name = \"rule1_1\",",
                "    srcs = [\"source_1.txt\"],",
                "    outs = [\"intermediate_1.txt\"],",
                "    cmd = \"(cat $< && echo from rule1_1) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule1_2\",",
                "    srcs = [\"source_2.txt\"],",
                "    outs = [\"intermediate_2.txt\"],",
                "    cmd = \"(cat $< && echo from rule1_2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [",
                "        \"intermediate_1.txt\",",
                "        \"intermediate_2.txt\",",
                "        \"source_3.txt\",",
                "    ],",
                "    outs = [\"output.inlined\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"consume_output\",",
                "    srcs = [\":output.inlined\"],",
                "    outs = [\"dummy.out\"],",
                "    cmd = \"touch $@\",",
                ")",
            ],
        );
        t.test_case.write("test/source_1.txt", &["source_1"]);
        t.test_case.write("test/source_2.txt", &["source_2"]);
        t.test_case.write("test/source_3.txt", &["source_3"]);

        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", shim);

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let output_file_contents = t.helper.build_and_get_output("test", &mut t.test_case);

        // The evaluation succeeds, producing the expected output, after re-executing rule1_1's
        // and rule1_2's actions.
        assert_eq!(
            output_file_contents,
            "source_1\nfrom rule1_1\nsource_2\nfrom rule1_2\nsource_3\nfrom rule2\n"
        );

        assert_contains_exactly(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1_1",
                "Executing genrule //test:rule1_2",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule1_1",
                "Executing genrule //test:rule1_2",
                "Executing genrule //test:rule2",
                "Executing genrule //test:consume_output",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &[],
            /* completed_rewound= */
            &[
                "Executing genrule //test:rule1_1",
                "Executing genrule //test:rule1_2",
            ],
            /* failed_rewound= */ &["Executing genrule //test:rule2"],
            /* action_rewinding_post_lost_input_counts= */ &[2],
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_only_actions_rewound(&rewound_keys);
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:rule1_1", "//test:rule1_2"],
        );
    });
}

/// `writeNGenrulePackages(n)`: `n` genrules over a source file each, `n` genrules of which the
/// `i`-th consumes the outputs of the first `i`, and `consume_output` over all consumers.
// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:704 RewindingTestsHelper.writeNGenrulePackages
fn write_n_genrule_packages(test_case: &BuildIntegrationTestCase, n: usize) {
    let mut lines: Vec<String> = Vec::new();
    for i in 1..=n {
        test_case.write(&format!("test/source_{i}.txt"), &[&format!("source_{i}")]);
        lines.push("genrule(".to_owned());
        lines.push(format!("    name = 'rule{i}',"));
        lines.push(format!("    srcs = ['source_{i}.txt'],"));
        lines.push(format!("    outs = ['out_{i}.txt'],"));
        lines.push(format!(
            "    cmd = '(cat $(SRCS) && echo from rule{i}) > $@') "
        ));
        lines.push(String::new());
    }
    let mut outs = String::new();
    for i in 1..=n {
        let out = format!(" 'consume_{i}.out', ");
        outs.push_str(&out);
        let mut entries = String::new();
        for e in 1..=i {
            entries.push_str(&format!("':out_{e}.txt', "));
        }
        lines.push("genrule(".to_owned());
        lines.push(format!("    name = 'consume_{i}',"));
        lines.push(format!("    srcs = [{entries}],"));
        lines.push(format!("    outs = [{out}],"));
        lines.push(format!(
            "    cmd = '(cat $(SRCS) && echo from consume_{i}) > $@') "
        ));
        lines.push(String::new());
    }
    lines.push("genrule(".to_owned());
    lines.push("    name = 'consume_output',".to_owned());
    lines.push(format!("    srcs = [{outs}],"));
    lines.push("    outs = ['output.inlined'],".to_owned());
    lines.push("    cmd = 'touch $@')".to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    test_case.write("test/BUILD", &lines);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:175 RewindingTest.multipleLostInputsForRewindPlan
#[test]
fn multiple_lost_inputs_for_rewind_plan() {
    use blaze_sky_exec::rewinding::action_rewind_strategy::MAX_ACTION_REWIND_EVENTS;
    let worker = Worker::start();
    for producer_strategy in ["standalone", "remote"] {
        for consumer_strategy in ["standalone", "remote"] {
            eprintln!("producerStrategy={producer_strategy}, consumerStrategy={consumer_strategy}");
            for_each_parameter_combination(&worker, |t| {
                // Java adds --remote_cache_async=false only when the test does not run on Bazel.
                t.test_case.add_options(&[
                    &format!("--strategy_regexp=.*//test:rule.*={producer_strategy}"),
                    &format!("--strategy_regexp=.*//test:consume.*={consumer_strategy}"),
                ]);
                // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:751 RewindingTestsHelper.runMultipleLostInputsForRewindPlan
                write_n_genrule_packages(&t.test_case, MAX_ACTION_REWIND_EVENTS + 1);
                for target in 1..=MAX_ACTION_REWIND_EVENTS + 1 {
                    t.helper.add_spawn_shim(
                        &format!("Executing genrule //test:consume_{target}"),
                        move |spawn, context| {
                            let lost_inputs: Vec<ActionInputRef> = (1..=target)
                                .map(|e| get_input_with_name(spawn, &format!("out_{e}.txt")))
                                .collect();
                            create_lost_inputs_exec_exception(context, &lost_inputs)
                        },
                    );
                }
                let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
                let targets: Vec<String> = (1..=MAX_ACTION_REWIND_EVENTS + 1)
                    .map(|i| format!("//test:consume_{i}"))
                    .collect();
                let targets: Vec<&str> = targets.iter().map(String::as_str).collect();
                let result = t.test_case.build_target(&targets);
                assert!(
                    result.is_success(),
                    "build failed: {:?}",
                    result.get_failure_detail()
                );
                assert_only_actions_rewound(&rewound_keys.lock());
                t.helper.spawn_controller.verify_all_shims_consumed();
                t.helper
                    .recorder
                    .assert_total_lost_input_counts_from_stats(&[(MAX_ACTION_REWIND_EVENTS + 1)
                        * (MAX_ACTION_REWIND_EVENTS + 2)
                        / 2]);
            });
        }
    }
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:190 RewindingTest.ineffectiveRewindingResultsInLostInputTooManyTimes
#[test]
#[ignore = "asserts the bug report of ActionRewindStrategy (bugReporter.sendBugReport, not ported in blaze_sky_exec) through testCase.recordBugReportsAndReinitialize, which the fixture lacks"]
fn ineffective_rewinding_results_in_lost_input_too_many_times() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:196 RewindingTest.interruptedDuringRewindStopsNormally
#[test]
fn interrupted_during_rewind_stops_normally() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:779 RewindingTestsHelper.runInterruptedDuringRewindStopsNormally
        // This test sets up two genrules, and makes the first execution of rule2 fail, saying
        // that the file produced by rule1 is missing. Before rule1 is re-executed, the test
        // interrupts the build. The build should stop with an interrupt normally (and not crash).
        write_two_genrule_package(&t.test_case);

        let controller = t.helper.spawn_controller.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", move |spawn, context| {
                controller.add_spawn_shim(
                    "Executing genrule //test:rule1",
                    Box::new(|_: &dyn Spawn, _: &dyn ActionExecutionContext| {
                        // Thread.currentThread().interrupt()
                        blaze_shell::thread_interrupt::interrupt_current();
                        Ok(ExecResult::delegate())
                    }),
                );
                let lost_inputs = [get_input_with_name(spawn, "intermediate.txt")];
                create_lost_inputs_exec_exception(context, &lost_inputs)
            });

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        // assertThrows(InterruptedException.class, () -> testCase.buildTarget("//test:rule2")):
        // the command ends interrupted.
        let result = t.test_case.build_target(&["//test:rule2"]);
        assert_eq!(
            result.get_exit_code().get_numeric_exit_code(),
            blaze_util::exit_code::ExitCode::INTERRUPTED.get_numeric_exit_code(),
            "{:?}",
            result.get_failure_detail()
        );

        assert_output_for_stop_before_rewound_reexecution(&t.helper);

        let rewound_keys = rewound_keys.lock().clone();
        assert_only_actions_rewound(&rewound_keys);
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:rule1"],
        );
        assert_output_for_rule2_not_created(&t.test_case);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:202 RewindingTest.failureDuringRewindStopsNormally
#[test]
#[ignore = "asserts BuildFailedException.getMessage() (null with --keep_going), which the fixture's command result does not carry, and the output of //test:rule2 through getExistingConfiguredTarget, which the fixture lacks"]
fn failure_during_rewind_stops_normally() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:219 RewindingTest.intermediateActionRewound
#[test]
fn intermediate_action_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:908 RewindingTestsHelper.runIntermediateActionRewound
        // This test sets up three genrules, and makes the first execution of rule3 fail, saying
        // that the file produced by rule2 is missing but the file produced by rule1 is not.
        // Rule2 executes twice, consuming the file output from rule1 both times. Rule1 is not
        // executed a second time.
        t.test_case.write(
            "test/BUILD",
            &[
                "genrule(",
                "    name = \"rule1\",",
                "    srcs = [\"source_1.txt\"],",
                "    outs = [\"intermediate_1.txt\"],",
                "    cmd = \"(cat $< && echo from rule1) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [",
                "        \"intermediate_1.txt\",",
                "        \"source_2.txt\",",
                "    ],",
                "    outs = [\"intermediate_2.txt\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule3\",",
                "    srcs = [",
                "        \"intermediate_1.txt\",",
                "        \"intermediate_2.txt\",",
                "        \"source_3.txt\",",
                "    ],",
                "    outs = [\"output.inlined\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule3) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"consume_output\",",
                "    srcs = [\":output.inlined\"],",
                "    outs = [\"dummy.out\"],",
                "    cmd = \"touch $@\",",
                ")",
            ],
        );
        t.test_case.write("test/source_1.txt", &["source_1"]);
        t.test_case.write("test/source_2.txt", &["source_2"]);
        t.test_case.write("test/source_3.txt", &["source_3"]);

        t.helper
            .add_spawn_shim("Executing genrule //test:rule3", |spawn, context| {
                create_lost_inputs_exec_exception_named(spawn, context, &["intermediate_2.txt"])
            });

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let output_file_contents = t.helper.build_and_get_output("test", &mut t.test_case);

        assert_eq!(
            output_file_contents,
            "source_1\nfrom rule1\nsource_1\nfrom rule1\nsource_2\nfrom rule2\nsource_3\nfrom rule3\n"
        );

        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
                "Executing genrule //test:consume_output",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &["Executing genrule //test:rule1"],
            /* completed_rewound= */ &["Executing genrule //test:rule2"],
            /* failed_rewound= */ &["Executing genrule //test:rule3"],
            /* action_rewinding_post_lost_input_counts= */ &[1],
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_only_actions_rewound(&rewound_keys);
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:rule2"],
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:224 RewindingTest.chainOfActionsRewound
#[test]
fn chain_of_actions_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:996 RewindingTestsHelper.runChainOfActionsRewound
        // This test exercises recursive rewinding. It sets up three genrules that depend on each
        // other in a chain. Rule1 and rule2 execute successfully their first time. When rule3
        // executes, it fails, saying that the file produced by rule2 is missing. When rule2 is
        // executed for the second time, it fails, saying that the file produced by rule1 is
        // missing. Thereafter, all executions succeed.
        t.test_case.write(
            "test/BUILD",
            &[
                "genrule(",
                "    name = \"rule1\",",
                "    srcs = [\"source_1.txt\"],",
                "    outs = [\"intermediate_1.txt\"],",
                "    cmd = \"(cat $< && echo from rule1) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [",
                "        \"intermediate_1.txt\",",
                "        \"source_2.txt\",",
                "    ],",
                "    outs = [\"intermediate_2.txt\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule3\",",
                "    srcs = [",
                "        \"intermediate_2.txt\",",
                "        \"source_3.txt\",",
                "    ],",
                "    outs = [\"output.inlined\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule3) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"consume_output\",",
                "    srcs = [\":output.inlined\"],",
                "    outs = [\"dummy.out\"],",
                "    cmd = \"touch $@\",",
                ")",
            ],
        );
        t.test_case.write("test/source_1.txt", &["source_1"]);
        t.test_case.write("test/source_2.txt", &["source_2"]);
        t.test_case.write("test/source_3.txt", &["source_3"]);

        let controller = t.helper.spawn_controller.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule3", move |spawn, context| {
                controller.add_spawn_shim(
                    "Executing genrule //test:rule2",
                    Box::new(
                        |other_spawn: &dyn Spawn, other_context: &dyn ActionExecutionContext| {
                            Ok(create_lost_inputs_exec_exception_named(
                                other_spawn,
                                other_context,
                                &["intermediate_1.txt"],
                            ))
                        },
                    ),
                );
                create_lost_inputs_exec_exception_named(spawn, context, &["intermediate_2.txt"])
            });

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let output_file_contents = t.helper.build_and_get_output("test", &mut t.test_case);

        assert_eq!(
            output_file_contents,
            "source_1\nfrom rule1\nsource_2\nfrom rule2\nsource_3\nfrom rule3\n"
        );

        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
                "Executing genrule //test:consume_output",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &[],
            /* completed_rewound= */
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
            ],
            /* failed_rewound= */ &["Executing genrule //test:rule3"],
            /* action_rewinding_post_lost_input_counts= */ &[2],
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_only_actions_rewound(&rewound_keys);
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:rule2", "//test:rule1"],
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:229 RewindingTest.nondeterministicActionRewound
#[test]
fn nondeterministic_action_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1085 RewindingTestsHelper.runNondeterministicActionRewound
        // This test demonstrates that rewinding works when rewound actions are nondeterministic.
        //
        // A nondeterministic genrule, rule1, produces output which is lost. The genrule rule2
        // uses this output and fails. The rewound nondeterministic action generates a new
        // output. This test asserts that rule2 uses the new output on its second try, by checking
        // rule2's output when it's used by rule3.
        t.test_case.write(
            "test/BUILD",
            &[
                "genrule(",
                "    name = \"rule1\",",
                "    srcs = [\"source_1.txt\"],",
                "    outs = [\"intermediate_1.inlined\"],",
                "    cmd = \"(cat $(location source_1.txt) && echo $$RANDOM) > $@\",",
                "    tags = [\"no-cache\"],",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [",
                "        \"source_2.txt\",",
                "        \"intermediate_1.inlined\",",
                "    ],",
                "    outs = [\"intermediate_2.inlined\"],",
                "    cmd = \"(cat $(SRCS) && echo from rule2) > $@\",",
                ")",
                "",
                "genrule(",
                "    name = \"rule3\",",
                "    srcs = [\"intermediate_2.inlined\"],",
                "    outs = [\"output.txt\"],",
                "    cmd = \"(cat $< && echo from rule3) > $@\",",
                ")",
            ],
        );
        t.test_case.write("test/source_1.txt", &["source_1"]);
        t.test_case.write("test/source_2.txt", &["source_2"]);

        let intermediate_1_first_content: Arc<Mutex<Option<String>>> = Arc::default();
        let first = intermediate_1_first_content.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", move |spawn, context| {
                let intermediate_1 = get_input_with_name(spawn, "intermediate_1.inlined");
                *first.lock() = Some(latin1_string_from_action_input(context, &intermediate_1));
                create_lost_inputs_exec_exception(context, &[intermediate_1])
            });

        let intermediate_1_second_content: Arc<Mutex<Option<String>>> = Arc::default();
        let second = intermediate_1_second_content.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", move |spawn, context| {
                let intermediate_1 = get_input_with_name(spawn, "intermediate_1.inlined");
                *second.lock() = Some(latin1_string_from_action_input(context, &intermediate_1));
                ExecResult::delegate()
            });

        let intermediate_2_content: Arc<Mutex<Option<String>>> = Arc::default();
        let content_2 = intermediate_2_content.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule3", move |spawn, context| {
                let intermediate_2 = get_input_with_name(spawn, "intermediate_2.inlined");
                *content_2.lock() = Some(latin1_string_from_action_input(context, &intermediate_2));
                ExecResult::delegate()
            });

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let result = t.test_case.build_target(&["//test:rule3"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        let first_content = intermediate_1_first_content.lock().clone();
        let second_content = intermediate_1_second_content.lock().clone();
        assert_ne!(second_content, first_content);
        assert_eq!(
            intermediate_2_content.lock().clone(),
            Some(format!(
                "source_2\n{}from rule2\n",
                second_content.unwrap_or_default()
            ))
        );
        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule1",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &["Executing genrule //test:rule3"],
            /* completed_rewound= */ &["Executing genrule //test:rule1"],
            /* failed_rewound= */ &["Executing genrule //test:rule2"],
            /* action_rewinding_post_lost_input_counts= */ &[1],
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_only_actions_rewound(&rewound_keys);
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:rule1"],
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:234 RewindingTest.parallelTrackSharedActionsRewound
#[test]
#[ignore = "its listener orders the shared actions on ADD_EXTERNAL_DEP events, which blaze_skyframe does not emit (graph_listener module docs)"]
fn parallel_track_shared_actions_rewound() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:239 RewindingTest.treeFileArtifactRewound
#[test]
fn tree_file_artifact_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1494 RewindingTestsHelper.runTreeFileArtifactRewound_spawnFailed
        let shim = |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
            // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1502 RewindingTestsHelper.getTreeFileArtifactRewoundLostInputs
            let lost_inputs = [get_input_with_name(spawn, "make_cc_dir.cc/file1.cc")];
            create_lost_inputs_exec_exception(context, &lost_inputs)
        };
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1506 RewindingTestsHelper.runTreeFileArtifactRewound
        // This test demonstrates that rewinding works when an action fails due to a lost input
        // which is a generated TreeFileArtifact that is directly depended on: the failed action
        // directly depends on a file *contained in the tree*, and does *not* directly depend on
        // the tree itself.
        //
        // The compilation action "Compiling tree/make_cc_dir.cc/file1.cc" fails, saying that
        // "make_cc_dir.cc/file1.cc", one of the output files in the tree outputted by the
        // "make_cc" rule, is lost. The action that generated that tree, "Action
        // tree/make_cc_dir.cc", is rewound along with the failed compilation action.
        set_up_tree_artifact_package(&mut t.test_case);

        t.helper
            .add_spawn_shim("Compiling tree/make_cc_dir.cc/file1.cc", shim);

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let result = t.test_case.build_target(&["//tree:consumes_tree"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_contains_exactly(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Action tree/make_cc_dir.cc",
                "Compiling tree/make_cc_dir.cc/file1.cc",
                "Action tree/make_cc_dir.cc",
                "Compiling tree/make_cc_dir.cc/file1.cc",
                "Compiling tree/make_cc_dir.cc/file2.cc",
                "Compiling tree/source_2.cc",
                "Linking tree/libconsumes_tree.so",
                "Linking tree/libconsumes_tree.a",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */
            &[
                "Compiling tree/make_cc_dir.cc/file2.cc",
                "Linking tree/libconsumes_tree.so",
                "Linking tree/libconsumes_tree.a",
            ],
            /* completed_rewound= */ &["Action tree/make_cc_dir.cc"],
            /* failed_rewound= */ &["Compiling tree/make_cc_dir.cc/file1.cc"],
            /* action_rewinding_post_lost_input_counts= */ &[1],
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_eq!(rewound_keys.len(), 1, "{rewound_keys:?}");
        assert_action_key(&rewound_keys[0], "//tree:make_cc", /* index= */ 0);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:245 RewindingTest.treeArtifactRewound_allFilesLost
#[test]
fn tree_artifact_rewound_all_files_lost() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1553 RewindingTestsHelper.runTreeArtifactRewound_allFilesLost_spawnFailed
        // The linking action "Linking tree/libconsumes_tree.so" fails, saying that the "*.pic.o"
        // files produced by the compilation actions are lost. The linking action which failed is
        // reset along with those compilation actions.
        run_tree_artifact_rewound_when_tree_files_lost(
            t,
            &["make_cc_dir/file1.pic.o", "make_cc_dir/file2.pic.o"],
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:251 RewindingTest.treeArtifactRewound_oneFileLost
#[test]
fn tree_artifact_rewound_one_file_lost() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1575 RewindingTestsHelper.runTreeArtifactRewound_oneFileLost_spawnFailed
        // Like the all-files case, but only one of the tree's files is lost; *all* the
        // compilation actions whose outputs are included by the tree are rewound.
        run_tree_artifact_rewound_when_tree_files_lost(t, &["make_cc_dir/file1.pic.o"]);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:257 RewindingTest.actionTemplateExpansionRewound_notConcurrentWithTreeConsumers
#[test]
#[ignore = "ctx.actions.map_directory stops at unported!(StarlarkActionFactory.mapDirectory) in blaze_analysis (ported case kept for when it lands)"]
fn action_template_expansion_rewound_not_concurrent_with_tree_consumers() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1703 RewindingTestsHelper.runActionTemplateExpansionRewound_notConcurrentWithTreeConsumers
        // An action generated by an action template must not prepare for its re-execution while
        // a consumer of the tree artifact it populates is running: a rewound expansion action
        // synchronizes on the template rather than on itself. The rewound action also consumes
        // the output of another action of the same expansion, which must not deadlock.
        //
        // All consumers and the action that reports the lost input have to run concurrently for
        // the expansion to be rewound while the tree artifact is being read.
        ensure_minimum_jobs(&mut t.test_case, TREE_CONSUMER_COUNT + 1);
        t.test_case
            .add_options(&["--experimental_allow_map_directory"]);
        write_text(
            &t.test_case,
            "foo/defs.bzl",
            &DEFS.replace("COPY_TOOL_SCRIPT", COPY_TOOL_SCRIPT),
        );
        let consumers: Vec<String> = (0..TREE_CONSUMER_COUNT)
            .map(|i| CONSUMER.replace("{i}", &i.to_string()))
            .collect();
        write_text(
            &t.test_case,
            "foo/BUILD",
            &format!("{BUILD}{}", consumers.join("\n")),
        );

        // The first rewound action of a build waits for all in-flight actions to finish before
        // it prepares for its re-execution, which would mask the behavior under test. Rewind an
        // unrelated action first; all consumers depend on its output and thus can't start any
        // earlier.
        t.helper.add_spawn_shim(
            "Executing genrule //foo:warmup_consumer",
            |spawn, context| {
                create_lost_inputs_exec_exception_named(spawn, context, &["warmup.out"])
            },
        );

        // A consumer holds the read lock on the tree artifact from before its spawn shim runs
        // until after its spawn has been executed, so the count is a lower bound on the number
        // of consumers that are reading the tree artifact.
        let consumers_reading_tree = Arc::new(AtomicI64::new(0));
        let max_consumers_reading_during_expansion = Arc::new(AtomicI64::new(0));
        t.test_case
            .register_subscriber(Arc::new(ConsumerSpawnCounter {
                consumers_reading_tree: consumers_reading_tree.clone(),
            }));
        for i in 0..TREE_CONSUMER_COUNT {
            let reading = consumers_reading_tree.clone();
            t.helper
                .add_spawn_shim(&format!("Consuming //foo:consumer_{i}"), move |_, _| {
                    reading.fetch_add(1, Ordering::SeqCst);
                    ExecResult::delegate()
                });
        }
        // The expansion actions are the only writers, so they must never observe a reader. Both
        // of them run twice: once initially and once after the expansion has been rewound.
        for expansion_action in ["Mapping foo/mapped_dir (1)", "Mapping foo/mapped_dir (2)"] {
            for _run in 0..2 {
                let reading = consumers_reading_tree.clone();
                let max = max_consumers_reading_during_expansion.clone();
                t.helper.add_spawn_shim(expansion_action, move |_, _| {
                    max.fetch_max(reading.load(Ordering::SeqCst), Ordering::SeqCst);
                    ExecResult::delegate()
                });
            }
        }
        // Fails without executing a spawn, so that the expansion is rewound while the other
        // consumers are as likely as possible to still be reading the tree artifact.
        t.helper
            .add_spawn_shim("Consuming //foo:losing_consumer", |spawn, context| {
                let mapped_tree = get_tree_artifact_with_name(spawn, "mapped_dir");
                let lost = get_expanded_to_artifact("f1.out", &mapped_tree, spawn, context);
                create_lost_inputs_exec_exception(context, &[lost])
            });

        let result = t.test_case.build_target(&["//foo:all"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_eq!(
            max_consumers_reading_during_expansion.load(Ordering::SeqCst),
            0,
            "the rewound action template expansion prepared for its re-execution while consumers \
             were still reading the tree artifact it populates"
        );
        // Rewinding an expanded action re-expands the template, so both actions of the chain
        // re-run.
        let executed_spawns = t.helper.get_executed_spawn_descriptions();
        let count = |d: &str| executed_spawns.iter().filter(|s| *s == d).count();
        assert_eq!(
            count("Mapping foo/mapped_dir (1)"),
            2,
            "{executed_spawns:?}"
        );
        assert_eq!(
            count("Mapping foo/mapped_dir (2)"),
            2,
            "{executed_spawns:?}"
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:262 RewindingTest.actionTemplateExpansionRewound_siblingActionsReExecuteConcurrently
#[test]
#[ignore = "ctx.actions.map_directory stops at unported!(StarlarkActionFactory.mapDirectory) in blaze_analysis (ported case kept for when it lands)"]
fn action_template_expansion_rewound_sibling_actions_re_execute_concurrently() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:1922 RewindingTestsHelper.runActionTemplateExpansionRewound_siblingActionsReExecuteConcurrently
        let concurrent_actions = 8;
        // All re-executed sibling actions have to run concurrently for the rendezvous below to
        // complete.
        ensure_minimum_jobs(&mut t.test_case, concurrent_actions);
        t.test_case
            .add_options(&["--experimental_allow_map_directory"]);
        let children: Vec<String> = (1..=concurrent_actions).map(|i| format!("f{i}")).collect();
        let seed_command = children
            .iter()
            .map(|child| format!("echo seed > $1/{child}"))
            .collect::<Vec<_>>()
            .join(" && ");
        write_text(
            &t.test_case,
            "foo/defs.bzl",
            &SIBLING_DEFS
                .replace("COPY_TOOL_SCRIPT", COPY_TOOL_SCRIPT)
                .replace("SEED_COMMAND", &seed_command),
        );
        write_text(&t.test_case, "foo/BUILD", SIBLING_BUILD);

        // The initial executions of the expansion actions pass through unmodified; per
        // description, shims are consumed in the order in which they were added.
        for child in &children {
            t.helper
                .add_spawn_shim(&format!("Mapping foo/mapped_dir {child}"), |_, _| {
                    ExecResult::delegate()
                });
        }
        // Each re-executed sibling waits for all other ones to start before running its spawn,
        // which can only succeed if all of them run concurrently.
        // (TestUtils.WAIT_TIMEOUT_SECONDS is Long.MAX_VALUE / 1000: no timeout.)
        let all_siblings_re_executing = Arc::new(std::sync::Barrier::new(concurrent_actions));
        for child in &children {
            let barrier = all_siblings_re_executing.clone();
            t.helper
                .add_spawn_shim(&format!("Mapping foo/mapped_dir {child}"), move |_, _| {
                    barrier.wait();
                    ExecResult::delegate()
                });
        }
        // Report all files of the tree artifact as lost so that every sibling action has to
        // re-execute regardless of how precisely rewinding translates lost files into rewound
        // expanded actions.
        let lost_children = children.clone();
        t.helper.add_spawn_shim(
            "Executing genrule //foo:losing_consumer",
            move |spawn, context| {
                let mapped_tree = get_tree_artifact_with_name(spawn, "mapped_dir");
                let lost: Vec<ActionInputRef> = lost_children
                    .iter()
                    .map(|child| {
                        get_expanded_to_artifact(
                            &format!("{child}.out"),
                            &mapped_tree,
                            spawn,
                            context,
                        )
                    })
                    .collect();
                create_lost_inputs_exec_exception(context, &lost)
            },
        );

        let result = t.test_case.build_target(&["//foo:losing_consumer"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        t.helper.spawn_controller.verify_all_shims_consumed();
        let executed_spawns = t.helper.get_executed_spawn_descriptions();
        let count = |d: &str| executed_spawns.iter().filter(|s| *s == d).count();
        for child in &children {
            assert_eq!(
                count(&format!("Mapping foo/mapped_dir {child}")),
                2,
                "{executed_spawns:?}"
            );
        }
        assert_eq!(
            count("Executing genrule //foo:losing_consumer"),
            2,
            "{executed_spawns:?}"
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:268 RewindingTest.generatedRunfilesRewound_allFilesLost
#[test]
#[ignore = "Java's RewindingTest turns external repositories off (workspaceInit: WorkspaceBuilder.allowExternalRepositories(false)) so the executable rule registers no RepoMappingManifestAction; Blaze has no such switch (allowExternalRepositories is fixed to true in ConfiguredTargetFunction and AspectFunction registration and in blaze_sky_top incremental.rs), so the tool has 5 actions as in the oracle and the rewound keys are one more than the Java assertions expect"]
fn generated_runfiles_rewound_all_files_lost() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2059 RewindingTestsHelper.runGeneratedRunfilesRewound_allFilesLost_spawnFailed
        // Rewinding must propagate across the runfiles tree artifacts and actions associated
        // with the runfiles.
        run_generated_runfiles_rewound(t, &["gen1.dat", "gen2.dat"]);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:273 RewindingTest.generatedRunfilesRewound_oneFileLost
#[test]
#[ignore = "Java's RewindingTest turns external repositories off (workspaceInit: WorkspaceBuilder.allowExternalRepositories(false)) so the executable rule registers no RepoMappingManifestAction; Blaze has no such switch (allowExternalRepositories is fixed to true in ConfiguredTargetFunction and AspectFunction registration and in blaze_sky_top incremental.rs), so the tool has 5 actions as in the oracle and the rewound keys are one more than the Java assertions expect"]
fn generated_runfiles_rewound_one_file_lost() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2071 RewindingTestsHelper.runGeneratedRunfilesRewound_oneFileLost_spawnFailed
        run_generated_runfiles_rewound(t, &["gen1.dat"]);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:278 RewindingTest.dupeDirectAndRunfilesDependencyRewound
#[test]
#[ignore = "Java's RewindingTest turns external repositories off (workspaceInit: WorkspaceBuilder.allowExternalRepositories(false)) so the executable rule registers no RepoMappingManifestAction; Blaze has no such switch (allowExternalRepositories is fixed to true in ConfiguredTargetFunction and AspectFunction registration and in blaze_sky_top incremental.rs), so the tool has 5 actions as in the oracle and the rewound keys are one more than the Java assertions expect"]
fn dupe_direct_and_runfiles_dependency_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2239 RewindingTestsHelper.runDupeDirectAndRunfilesDependencyRewound_spawnFailed
        let intermediate_1_first_content: Arc<Mutex<Option<String>>> = Arc::default();
        let first = intermediate_1_first_content.clone();
        let shim = move |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
            // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2251 RewindingTestsHelper.getDupeDirectAndRunfilesDependencyRewoundLostInput
            let lost_input =
                get_runfiles_artifact_with_name(spawn, context, "intermediate_1.inlined");
            *first.lock() = Some(latin1_string_from_action_input(context, &lost_input));
            create_lost_inputs_exec_exception(context, &[lost_input])
        };
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2267 RewindingTestsHelper.runDupeDirectAndRunfilesDependencyRewound
        // Rewinding works when a lost input is both directly depended on and, via runfiles,
        // indirectly depended on by an action; it must invalidate both paths from the failed
        // action to the rewound generating action, the runfiles nodes after the artifact of the
        // lost input.
        mock_foo_binary(&t.test_case, "test/foo_binary.bzl");
        t.test_case.write(
            "test/BUILD",
            &[
                "load(\":foo_binary.bzl\", \"foo_binary\")",
                "genrule(",
                "    name = \"rule1\",",
                "    srcs = [],",
                "    outs = [\"intermediate_1.inlined\"],",
                "    cmd = \"echo $$RANDOM > $@\",",
                "    tags = [\"no-cache\"],",
                ")",
                "",
                "foo_binary(",
                "    name = \"tool\",",
                "    srcs = [\"tool.sh\"],",
                "    data = [\"intermediate_1.inlined\"],",
                ")",
                "",
                "genrule(",
                "    name = \"rule2\",",
                "    srcs = [],",
                "    outs = [\"intermediate_2.inlined\"],",
                "    cmd = \"($(location tool) && cat $(location intermediate_1.inlined) && \" +",
                "          \"echo from rule2) > $@\",",
                "    tools = [",
                "        \"intermediate_1.inlined\",",
                "        \"tool\",",
                "    ],",
                ")",
                "",
                "genrule(",
                "    name = \"rule3\",",
                "    srcs = [\"intermediate_2.inlined\"],",
                "    outs = [\"output.txt\"],",
                "    cmd = \"(cat $< && echo from rule3) > $@\",",
                ")",
            ],
        );
        // TestConstants.WORKSPACE_NAME is _main on Bazel.
        let tool = t.test_case.write(
            "test/tool.sh",
            &[
                "#!/bin/bash",
                "cat ${0}.runfiles/_main/test/intermediate_1.inlined",
                "echo 'from tool'",
            ],
        );
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", shim);

        let intermediate_1_second_content: Arc<Mutex<Option<String>>> = Arc::default();
        let second = intermediate_1_second_content.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule2", move |spawn, context| {
                let intermediate_1 =
                    get_runfiles_artifact_with_name(spawn, context, "intermediate_1.inlined");
                *second.lock() = Some(latin1_string_from_action_input(context, &intermediate_1));
                ExecResult::delegate()
            });

        let intermediate_2_content: Arc<Mutex<Option<String>>> = Arc::default();
        let content_2 = intermediate_2_content.clone();
        t.helper
            .add_spawn_shim("Executing genrule //test:rule3", move |spawn, context| {
                let intermediate_2 = get_input_with_name(spawn, "intermediate_2.inlined");
                *content_2.lock() = Some(latin1_string_from_action_input(context, &intermediate_2));
                ExecResult::delegate()
            });

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let result = t.test_case.build_target(&["//test:rule3"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        let first_content = intermediate_1_first_content.lock().clone();
        let second_content = intermediate_1_second_content.lock().clone();
        assert_ne!(second_content, first_content);
        let second_content = second_content.unwrap_or_default();
        assert_eq!(
            intermediate_2_content.lock().clone(),
            Some(format!(
                "{second_content}from tool\n{second_content}from rule2\n"
            ))
        );
        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:rule1 [for tool]",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule1 [for tool]",
                "Executing genrule //test:rule2",
                "Executing genrule //test:rule3",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &["Executing genrule //test:rule3"],
            /* completed_rewound= */ &["Executing genrule //test:rule1 [for tool]"],
            /* failed_rewound= */ &["Executing genrule //test:rule2"],
            /* action_rewinding_post_lost_input_counts= */ &[1],
        );

        let rewound_keys = rewound_keys.lock().clone();
        let action_key = |key: &SkyKey| {
            key.downcast::<ActionExecutionKey>()
                .unwrap_or_else(|| panic!("{key:?} is not an action key"))
                .0
                .clone()
        };
        if build_runfile_manifests(&t.test_case) {
            assert_eq!(rewound_keys.len(), 5, "{rewound_keys:?}");
            let mut source_manifest_action_seen = false;
            for key in &rewound_keys[..4] {
                let key = action_key(key);
                let action_label = key.get_label().expect("label").get_canonical_form();
                if action_label == "//test:tool" {
                    match key.get_action_index() {
                        // SymlinkAction
                        0 => {}
                        1 => source_manifest_action_seen = true,
                        // SymlinkTreeAction
                        2 => assert!(source_manifest_action_seen),
                        _ => panic!(
                            "Unexpected action index. actionKey: {key:?}, rewoundKeys: {rewound_keys:?}"
                        ),
                    }
                } else {
                    assert_eq!(action_label, "//test:rule1");
                }
            }
            assert_action_key(&rewound_keys[4], "//test:tool", /* index= */ 3);
        } else {
            assert_eq!(rewound_keys.len(), 3, "{rewound_keys:?}");
            for key in &rewound_keys[..2] {
                let key = action_key(key);
                let action_label = key.get_label().expect("label").get_canonical_form();
                if action_label == "//test:tool" {
                    assert_eq!(key.get_action_index(), 0);
                } else {
                    assert_eq!(action_label, "//test:rule1");
                }
            }
            assert_action_key(&rewound_keys[2], "//test:tool", /* index= */ 1);
        }
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:283 RewindingTest.treeInRunfilesRewound
#[test]
#[ignore = "Java's RewindingTest turns external repositories off (workspaceInit: WorkspaceBuilder.allowExternalRepositories(false)) so the executable rule registers no RepoMappingManifestAction; Blaze has no such switch (allowExternalRepositories is fixed to true in ConfiguredTargetFunction and AspectFunction registration and in blaze_sky_top incremental.rs), so the tool has 5 actions as in the oracle and the rewound keys are one more than the Java assertions expect"]
fn tree_in_runfiles_rewound() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2408 RewindingTestsHelper.runTreeInRunfilesRewound_spawnFailed
        let shim = |spawn: &dyn Spawn, context: &dyn ActionExecutionContext| {
            // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2420 RewindingTestsHelper.getTreeInRunfilesRewoundTree
            let tree_artifact = get_runfiles_artifact_with_name(spawn, context, "gen_tree");
            // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2424 RewindingTestsHelper.getTreeInRunfilesRewoundLostInputs
            let lost_inputs = [
                get_expanded_to_artifact("gen1.out", &tree_artifact, spawn, context),
                get_expanded_to_artifact("gen2.out", &tree_artifact, spawn, context),
            ];
            create_lost_inputs_exec_exception(context, &lost_inputs)
        };
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2431 RewindingTestsHelper.runTreeInRunfilesRewound
        t.test_case.write(
            "middle/tree.bzl",
            &[
                "def _tree_impl(ctx):",
                "    tree_artifact = ctx.actions.declare_directory(ctx.attr.name + \"_dir\")",
                "    ctx.actions.run_shell(",
                "        inputs = ctx.files.srcs,",
                "        outputs = [tree_artifact],",
                "        command = '(echo \"tree1\" > $1/gen1.out) && (echo \"tree2\" > $1/gen2.out)',",
                "        arguments = [tree_artifact.path],",
                "    )",
                "    return DefaultInfo(",
                "        files = depset(direct = [tree_artifact]),",
                "        runfiles = ctx.runfiles(files = [tree_artifact]),",
                "    )",
                "",
                "tree = rule(",
                "    implementation = _tree_impl,",
                "    attrs = {\"srcs\": attr.label_list(allow_files = True)},",
                ")",
            ],
        );
        mock_foo_binary(&t.test_case, "middle/foo_binary.bzl");
        t.test_case.write(
            "middle/BUILD",
            &[
                "load(\":tree.bzl\", \"tree\")",
                "load(\":foo_binary.bzl\", \"foo_binary\")",
                "",
                "tree(",
                "    name = \"gen_tree\",",
                "    srcs = [\"source_1.txt\"],",
                ")",
                "",
                "foo_binary(",
                "    name = \"tool\",",
                "    srcs = [\"tool.sh\"],",
                "    data = [",
                "        \"source_2.txt\",",
                "        \":gen_tree\",",
                "    ],",
                ")",
                "",
                "genrule(",
                "    name = \"tool_user\",",
                "    srcs = [],",
                "    outs = [\"tool_user.out\"],",
                "    cmd = \"touch $(OUTS)\",",
                "    tools = [\"tool\"],",
                ")",
            ],
        );
        let tool = t.test_case.write("middle/tool.sh", &["#!/bin/bash"]);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        t.test_case.write("middle/source_1.txt", &["source_1"]);
        t.test_case.write("middle/source_2.txt", &["source_2"]);

        t.helper
            .add_spawn_shim("Executing genrule //middle:tool_user", shim);

        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);
        let result = t.test_case.build_target(&["//middle:tool_user"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );
        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Action middle/gen_tree_dir [for tool]",
                "Executing genrule //middle:tool_user",
                "Action middle/gen_tree_dir [for tool]",
                "Executing genrule //middle:tool_user",
            ],
        );

        t.helper.recorder.assert_events(
            /* run_once= */ &[],
            /* completed_rewound= */ &["Action middle/gen_tree_dir [for tool]"],
            /* failed_rewound= */ &["Executing genrule //middle:tool_user"],
            /* action_rewinding_post_lost_input_counts= */ &[2],
        );

        let rewound_keys = rewound_keys.lock().clone();
        let action_key = |key: &SkyKey| {
            key.downcast::<ActionExecutionKey>()
                .unwrap_or_else(|| panic!("{key:?} is not an action key"))
                .0
                .clone()
        };
        let (expected_len, loop_end, tool_index) = if build_runfile_manifests(&t.test_case) {
            (6, 5, 3)
        } else {
            (4, 3, 1)
        };
        assert_eq!(rewound_keys.len(), expected_len, "{rewound_keys:?}");
        let mut i = 0;
        let mut source_manifest_action_seen = false;
        while i < loop_end {
            let key = action_key(&rewound_keys[i]);
            let action_label = key.get_label().expect("label").get_canonical_form();
            i += 1;
            if action_label == "//middle:tool" {
                if expected_len == 6 {
                    match key.get_action_index() {
                        // SymlinkAction
                        0 => {}
                        1 => source_manifest_action_seen = true,
                        // SymlinkTreeAction
                        2 => assert!(source_manifest_action_seen),
                        _ => panic!(
                            "Unexpected action index. actionKey: {key:?}, rewoundKeys: {rewound_keys:?}"
                        ),
                    }
                } else {
                    assert_eq!(key.get_action_index(), 0);
                }
            } else {
                assert_eq!(action_label, "//middle:gen_tree");
                assert_artifact_key(&rewound_keys[i], "middle/gen_tree_dir");
                i += 1;
            }
        }
        assert_action_key(&rewound_keys[i], "//middle:tool", tool_index);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:288 RewindingTest.inputsFromSameGeneratingActionSplitAmongNestedSetChildren
#[test]
fn inputs_from_same_generating_action_split_among_nested_set_children() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:2565 RewindingTestsHelper.runInputsFromSameGeneratingActionSplitAmongNestedSetChildren
        t.test_case.write(
            "test/defs.bzl",
            &[
                "def _consumer_impl(ctx):",
                "    in1, in2, in3 = ctx.attr.three_output_genrule.files.to_list()",
                "    out = ctx.actions.declare_file(\"consumer.out\")",
                "    ctx.actions.run_shell(",
                "        outputs = [out],",
                "        # Arrange the inputs such that they are split among the depset's children.",
                "        inputs = depset([in1], transitive = [depset([in2, in3])]),",
                "        command = \"touch %s\" % out.path,",
                "        progress_message = \"Running consumer\",",
                "    )",
                "    return DefaultInfo(files = depset([out]))",
                "",
                "consumer = rule(",
                "    implementation = _consumer_impl,",
                "    attrs = {\"three_output_genrule\": attr.label(mandatory = True)},",
                ")",
            ],
        );
        t.test_case.write(
            "test/BUILD",
            &[
                "load(\":defs.bzl\", \"consumer\")",
                "",
                "genrule(",
                "    name = \"gen\",",
                "    outs = [",
                "        \"gen.out1\",",
                "        \"gen.out2\",",
                "        \"gen.out3\",",
                "    ],",
                "    cmd = \"touch $(OUTS)\",",
                ")",
                "",
                "consumer(",
                "    name = \"consumer\",",
                "    three_output_genrule = \":gen\",",
                ")",
            ],
        );

        t.helper
            .add_spawn_shim("Running consumer", |spawn, context| {
                create_lost_inputs_exec_exception_named(spawn, context, &["gen.out1"])
            });
        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);

        let result = t.test_case.build_target(&["//test:consumer"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        let rewound_keys = rewound_keys.lock().clone();
        assert_eq!(rewound_keys.len(), 1, "{rewound_keys:?}");
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys),
            &["//test:gen"],
        );
        assert_contains_exactly_in_order(
            &t.helper.get_executed_spawn_descriptions(),
            &[
                "Executing genrule //test:gen",
                "Running consumer",
                "Executing genrule //test:gen",
                "Running consumer",
            ],
        );
        t.helper.recorder.assert_events(
            /* run_once= */ &[],
            /* completed_rewound= */ &["Executing genrule //test:gen"],
            /* failed_rewound= */ &["Running consumer"],
            /* action_rewinding_post_lost_input_counts= */ &[1],
        );
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:293 RewindingTest.generatedHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "skipped on Bazel in Java (skipIfBazel: BuildIntegrationTestCase supports no include scanning or header modules on Bazel)"]
fn generated_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:299 RewindingTest.generatedHeaderRewound_lostInActionExecution
#[test]
#[ignore = "its expected spawns come from Java's mock C++ tools (Compiling tools/cpp/malloc.cc, tools/cpp/linkextra.cc); the fixture builds with the real @bazel_tools C++ toolchain"]
fn generated_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:305 RewindingTest.generatedTransitiveHeaderRewound_lostInInputDiscovery
#[test]
#[ignore = "skipped on Bazel in Java (skipIfBazel: BuildIntegrationTestCase supports no include scanning or header modules on Bazel)"]
fn generated_transitive_header_rewound_lost_in_input_discovery() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:311 RewindingTest.generatedTransitiveHeaderRewound_lostInActionExecution
#[test]
#[ignore = "its expected spawns come from Java's mock C++ tools (Compiling tools/cpp/malloc.cc, tools/cpp/linkextra.cc); the fixture builds with the real @bazel_tools C++ toolchain"]
fn generated_transitive_header_rewound_lost_in_action_execution() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:317 RewindingTest.doneToDirtyDepForNodeInError
#[test]
#[ignore = "its listener waits on ADD_REVERSE_DEP events, which blaze_skyframe emits only in its own unit tests (cfg(test) blocks in evaluator.rs)"]
fn done_to_dirty_dep_for_node_in_error() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:322 RewindingTest.flakyActionFailsAfterRewind_raceWithIndirectConsumer_undoneDuringInputChecking
#[test]
#[ignore = "its listener waits on GET_BATCH events with Reason.PREFETCH, which blaze_skyframe does not emit (only PRE_OR_POST_EVALUATION)"]
fn flaky_action_fails_after_rewind_race_with_indirect_consumer_undone_during_input_checking() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:328 RewindingTest.discoveredCppModuleLost
#[test]
#[ignore = "skipped on Bazel in Java (skipIfBazel: BuildIntegrationTestCase supports no include scanning or header modules on Bazel)"]
fn discovered_cpp_module_lost() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:334 RewindingTest.multipleLostInputsWithSameDigest_rewoundTogether
#[test]
fn multiple_lost_inputs_with_same_digest_rewound_together() {
    let worker = Worker::start();
    for_each_parameter_combination(&worker, |t| {
        // Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTestsHelper.java:3191 RewindingTestsHelper.runMultipleLostInputsWithSameDigest_rewoundTogether
        t.test_case.write(
            "foo/BUILD",
            &[
                "genrule(name = \"top\", srcs = [\":dep1\", \":dep2\"], outs = [\"top.out\"], cmd = \"echo top >$@\")",
                "genrule(name = \"dep1\", outs = [\"dep1.out\"], cmd = \"echo dep > $@\")",
                "genrule(name = \"dep2\", outs = [\"dep2.out\"], cmd = \"echo dep > $@\")",
            ],
        );
        t.helper
            .add_spawn_shim("Executing genrule //foo:top", |spawn, context| {
                create_lost_inputs_exec_exception_named(spawn, context, &["dep1.out", "dep2.out"])
            });
        let rewound_keys = t.helper.collect_ordered_rewound_keys(&t.test_case);

        let result = t.test_case.build_target(&["//foo:top"]);
        assert!(
            result.is_success(),
            "build failed: {:?}",
            result.get_failure_detail()
        );

        t.helper.spawn_controller.verify_all_shims_consumed();
        assert_contains_exactly(
            &rewound_artifact_owner_labels(&rewound_keys.lock()),
            &["//foo:dep1", "//foo:dep2"],
        );
        assert_eq!(t.helper.recorder.get_action_rewound_events().len(), 1);
    });
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:339 RewindingTest.lostTopLevelOutputWithRewindingDisabled
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn lost_top_level_output_with_rewinding_disabled() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:344 RewindingTest.topLevelOutputRewound_regularFile
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:349 RewindingTest.topLevelOutputRewound_aspectOwned
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_aspect_owned() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:354 RewindingTest.topLevelOutputRewound_fileInTreeArtifact
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:359 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_regularFile
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_partially_built_target_regular_file() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:364 RewindingTest.topLevelOutputRewound_partiallyBuiltTarget_fileInTreeArtifact
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_partially_built_target_file_in_tree_artifact() {}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/rewinding/RewindingTest.java:369 RewindingTest.topLevelOutputRewound_ineffectiveRewinding
#[test]
#[ignore = "needs LostImportantOutputHandlerModule (lost top-level outputs reported by the important-output handler), not ported onto the fixture"]
fn top_level_output_rewound_ineffective_rewinding() {}
