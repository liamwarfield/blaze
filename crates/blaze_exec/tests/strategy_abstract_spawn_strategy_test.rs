//! Port of `src/test/java/com/google/devtools/build/lib/exec/AbstractSpawnStrategyTest.java`
//! (plus `SpawnStrategyResolver` checks).

use std::any::Any;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Clock, ExtendedEventHandler, StoredEventHandler};
use blaze_exec::actions::spawn_strategy::{
    SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns,
};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ExecException, FakeOwner, FileArtifactValue,
    FilesetOutputTree, InputMapping, InputMetadataProvider, PlatformInfo, ResourceSet,
    RunfilesArtifactValue, SimpleSpawn, Spawn, SpawnResult, SpawnResultBuilder, Status,
    TreeArtifactValue,
};
use blaze_exec::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use blaze_exec::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, CacheHandle, SpawnCache, SpawnCacheError,
    SpawnExecutedEvent, no_cache, success,
};
use blaze_exec::exec::spawn_log_context::{SpawnLogAction, SpawnLogContext};
use blaze_exec::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::exec::spawn_strategy_resolver::SpawnStrategyResolver;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn::Code};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;

fn non_zero_exit_details() -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
            code: Code::NonZeroExit as i32,
            ..Default::default()
        })),
    }
}

fn empty_platform_info() -> PlatformInfo {
    PlatformInfo {
        label: Label::parse_canonical_unchecked("@bazel_tools//tools:internal_platform"),
        exec_properties: IndexMap::new(),
        remote_execution_properties: String::new(),
    }
}

/// `new SpawnBuilder("/bin/echo", "Hi!").withEnvironment("VARIABLE", "value").build()`.
fn simple_spawn() -> SimpleSpawn {
    let mut owner = FakeOwner::new(
        "Mnemonic",
        Some("progress message"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    owner.platform = Some(empty_platform_info());
    let mut env = IndexMap::new();
    env.insert("VARIABLE".to_string(), "value".to_string());
    SimpleSpawn::new_simple(
        Arc::new(owner),
        vec!["/bin/echo".to_string(), "Hi!".to_string()],
        env,
        IndexMap::new(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        vec![],
        Arc::new(ResourceSet::zero()),
    )
}

fn success_result() -> SpawnResult {
    SpawnResultBuilder::new()
        .set_status(Status::Success)
        .set_runner_name("test")
        .build()
}

fn non_zero_result() -> SpawnResult {
    SpawnResultBuilder::new()
        .set_status(Status::NonZeroExit)
        .set_exit_code(1)
        .set_failure_detail(non_zero_exit_details())
        .set_runner_name("test")
        .build()
}

/// `ManualClock`.
#[derive(Default)]
struct ManualClock {
    current_time_millis: AtomicI64,
}

impl ManualClock {
    fn advance_millis(&self, time: i64) {
        self.current_time_millis.fetch_add(time, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst)
    }
    fn nano_time(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst) * 1_000_000
    }
}

/// The mocked `SpawnRunner`.
struct MockSpawnRunner {
    result: SpawnResult,
    handles_caching: bool,
    exec_calls: AtomicI32,
    clock: Option<Arc<ManualClock>>,
}

impl MockSpawnRunner {
    fn new(result: SpawnResult) -> Arc<MockSpawnRunner> {
        Arc::new(MockSpawnRunner {
            result,
            handles_caching: false,
            exec_calls: AtomicI32::new(0),
            clock: None,
        })
    }
}

impl SpawnRunner for MockSpawnRunner {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(clock) = &self.clock {
            clock.advance_millis(1);
        }
        Ok(self.result.clone())
    }
    fn can_exec(&self, _spawn: &dyn Spawn) -> bool {
        false
    }
    fn handles_caching(&self) -> bool {
        self.handles_caching
    }
    fn get_name(&self) -> String {
        // Mockito returns null for unstubbed getName().
        String::new()
    }
}

/// An `InputMetadataProvider` that knows nothing.
struct EmptyInputMetadataProvider;

impl InputMetadataProvider for EmptyInputMetadataProvider {
    fn get_input_metadata(&self, _input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
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

/// The mocked `ActionExecutionContext`.
struct TestActionExecutionContext {
    exec_root: Path,
    event_handler: StoredEventHandler,
    clock: Arc<ManualClock>,
    spawn_cache: Option<Arc<dyn SpawnCache>>,
    spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    spawn_strategy_registry: Option<Arc<SpawnStrategyRegistry>>,
    action_file_system: Option<FileSystemRef>,
    input_metadata_provider: EmptyInputMetadataProvider,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    spawn_cache_lookups: AtomicI32,
}

impl TestActionExecutionContext {
    fn new() -> TestActionExecutionContext {
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        TestActionExecutionContext {
            exec_root: Path::create_from_str("/execroot", &fs),
            event_handler: StoredEventHandler::new(),
            clock: Arc::new(ManualClock::default()),
            spawn_cache: None,
            spawn_log_context: None,
            spawn_strategy_registry: None,
            action_file_system: None,
            input_metadata_provider: EmptyInputMetadataProvider,
            file_out_err: FileOutErr::new(
                Path::create_from_str("/out/stdout", &fs),
                Path::create_from_str("/out/stderr", &fs),
            ),
            client_env: IndexMap::new(),
            spawn_cache_lookups: AtomicI32::new(0),
        }
    }
}

impl ActionContextRegistry for TestActionExecutionContext {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        self.spawn_strategy_registry.clone()
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        self.spawn_log_context.clone()
    }
    fn get_spawn_cache(&self) -> Option<Arc<dyn SpawnCache>> {
        self.spawn_cache_lookups.fetch_add(1, Ordering::SeqCst);
        self.spawn_cache.clone()
    }
}

impl ActionExecutionContext for TestActionExecutionContext {
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &self.input_metadata_provider
    }
    fn get_action_file_system(&self) -> Option<FileSystemRef> {
        self.action_file_system.clone()
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        &self.event_handler
    }
    fn get_clock(&self) -> &dyn Clock {
        &*self.clock
    }
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {}
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {}
}

/// A mocked `SpawnCache` returning a mocked `CacheHandle`.
struct MockSpawnCache {
    useful_in_dynamic_execution: bool,
    handle_result: Option<SpawnResult>,
    stored: Arc<Mutex<Vec<SpawnResult>>>,
    lookups: AtomicI32,
    useful_calls: AtomicI32,
}

impl MockSpawnCache {
    fn new(handle_result: Option<SpawnResult>, useful: bool) -> Arc<MockSpawnCache> {
        Arc::new(MockSpawnCache {
            useful_in_dynamic_execution: useful,
            handle_result,
            stored: Arc::new(Mutex::new(Vec::new())),
            lookups: AtomicI32::new(0),
            useful_calls: AtomicI32::new(0),
        })
    }
}

struct MockCacheHandle {
    stored: Arc<Mutex<Vec<SpawnResult>>>,
}

impl CacheHandle for MockCacheHandle {
    fn has_result(&self) -> bool {
        false
    }
    fn get_result(&self) -> SpawnResult {
        panic!("not stubbed")
    }
    fn will_store(&self) -> bool {
        true
    }
    fn store(&self, result: &SpawnResult) -> Result<(), SpawnCacheError> {
        self.stored.lock().push(result.clone());
        Ok(())
    }
}

impl SpawnCache for MockSpawnCache {
    fn lookup(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle>, SpawnCacheError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        match &self.handle_result {
            Some(r) => Ok(success(r.clone())),
            None => Ok(Box::new(MockCacheHandle {
                stored: self.stored.clone(),
            })),
        }
    }
    fn useful_in_dynamic_execution(&self) -> bool {
        self.useful_calls.fetch_add(1, Ordering::SeqCst);
        self.useful_in_dynamic_execution
    }
}

struct NoopStopConcurrentSpawns;

impl StopConcurrentSpawns for NoopStopConcurrentSpawns {
    fn stop(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: &FileOutErr,
    ) -> Result<(), ExecException> {
        Ok(())
    }
}

fn tested_spawn_strategy(spawn_runner: Arc<dyn SpawnRunner>) -> AbstractSpawnStrategy {
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    AbstractSpawnStrategy::new(
        Path::create_from_str("/execroot", &fs),
        spawn_runner,
        Arc::new(blaze_options::Options::<ExecutionOptions>::get_defaults()),
        "TestedSpawnStrategy",
    )
}

fn assert_same_result(a: &SpawnResult, b: &SpawnResult) {
    assert_eq!(a.status(), b.status());
    assert_eq!(a.exit_code(), b.exit_code());
    assert_eq!(a.get_runner_name(), b.get_runner_name());
    assert_eq!(a.failure_detail(), b.failure_detail());
}

#[test]
fn test_zero_exit() {
    let mut ctx = TestActionExecutionContext::new();
    ctx.spawn_cache = Some(no_cache());
    let spawn_result = success_result();
    let spawn_runner = MockSpawnRunner::new(spawn_result.clone());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_event_posting() {
    let mut ctx = TestActionExecutionContext::new();
    ctx.spawn_cache = Some(no_cache());
    let spawn_result = success_result();
    let before_time = UNIX_EPOCH + Duration::from_millis(ctx.clock.current_time_millis() as u64);
    let spawn_runner = Arc::new(MockSpawnRunner {
        result: spawn_result.clone(),
        handles_caching: false,
        exec_calls: AtomicI32::new(0),
        clock: Some(ctx.clock.clone()),
    });

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);
    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    let posts = ctx.event_handler.get_posts();
    assert_eq!(posts.len(), 1);
    let event = (posts[0].as_ref() as &dyn Any)
        .downcast_ref::<SpawnExecutedEvent>()
        .expect("SpawnExecutedEvent");
    assert_eq!(event.get_start_time_instant(), before_time);
    assert_same_result(event.get_spawn_result(), &spawn_result);
    assert_eq!(event.get_exit_code(), 0);
}

#[test]
fn test_non_zero_exit() {
    let mut ctx = TestActionExecutionContext::new();
    ctx.spawn_cache = Some(no_cache());
    let result = non_zero_result();
    let spawn_runner = MockSpawnRunner::new(result.clone());

    let e = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap_err();
    let ExecException::Spawn(e) = e else {
        panic!("expected SpawnExecException, got {e:?}");
    };
    assert_same_result(e.get_spawn_result(), &result);
    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_cache_hit() {
    let mut ctx = TestActionExecutionContext::new();
    let spawn_result = success_result();
    let cache = MockSpawnCache::new(Some(spawn_result.clone()), true);
    ctx.spawn_cache = Some(cache.clone());
    let spawn_runner = MockSpawnRunner::new(success_result());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();
    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn test_cache_miss() {
    let mut ctx = TestActionExecutionContext::new();
    let cache = MockSpawnCache::new(None, true);
    ctx.spawn_cache = Some(cache.clone());
    let spawn_result = success_result();
    let spawn_runner = MockSpawnRunner::new(spawn_result.clone());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    let stored = cache.stored.lock();
    assert_eq!(stored.len(), 1);
    assert_same_result(&stored[0], &spawn_result);
}

#[test]
fn test_exec_when_local_caches_uses_no_cache() {
    let mut ctx = TestActionExecutionContext::new();
    let cache = MockSpawnCache::new(None, true);
    ctx.spawn_cache = Some(cache.clone());
    let spawn_result = success_result();
    let spawn_runner = Arc::new(MockSpawnRunner {
        result: spawn_result.clone(),
        handles_caching: true,
        exec_calls: AtomicI32::new(0),
        clock: None,
    });

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    // verifyNoInteractions(cache)
    assert_eq!(cache.lookups.load(Ordering::SeqCst), 0);
    assert_eq!(cache.useful_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ctx.spawn_cache_lookups.load(Ordering::SeqCst), 0);
}

#[test]
fn test_exec_useful_cache_in_dynamic_execution() {
    let mut ctx = TestActionExecutionContext::new();
    let cache = MockSpawnCache::new(None, true);
    ctx.spawn_cache = Some(cache.clone());
    let spawn_result = success_result();
    let spawn_runner = MockSpawnRunner::new(spawn_result.clone());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec_with_stop_concurrent_spawns(
            &simple_spawn(),
            &ctx,
            Some(&NoopStopConcurrentSpawns as &dyn StopConcurrentSpawns),
        )
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    let stored = cache.stored.lock();
    assert_eq!(stored.len(), 1);
    assert_same_result(&stored[0], &spawn_result);
}

#[test]
fn test_exec_non_useful_cache_in_dynamic_execution() {
    let mut ctx = TestActionExecutionContext::new();
    let cache = MockSpawnCache::new(None, false);
    ctx.spawn_cache = Some(cache.clone());
    let spawn_result = success_result();
    let spawn_runner = MockSpawnRunner::new(spawn_result.clone());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec_with_stop_concurrent_spawns(
            &simple_spawn(),
            &ctx,
            Some(&NoopStopConcurrentSpawns as &dyn StopConcurrentSpawns),
        )
        .unwrap();

    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    assert_eq!(cache.useful_calls.load(Ordering::SeqCst), 1);
    // verifyNoMoreInteractions(cache)
    assert_eq!(cache.lookups.load(Ordering::SeqCst), 0);
}

#[test]
fn test_cache_miss_with_non_zero_exit() {
    let mut ctx = TestActionExecutionContext::new();
    let cache = MockSpawnCache::new(None, true);
    ctx.spawn_cache = Some(cache.clone());
    let result = non_zero_result();
    let spawn_runner = MockSpawnRunner::new(result.clone());

    let e = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap_err();
    let ExecException::Spawn(e) = e else {
        panic!("expected SpawnExecException, got {e:?}");
    };
    assert_same_result(e.get_spawn_result(), &result);
    // Must only be called exactly once.
    assert_eq!(spawn_runner.exec_calls.load(Ordering::SeqCst), 1);
    let stored = cache.stored.lock();
    assert_eq!(stored.len(), 1);
    assert_same_result(&stored[0], &result);
}

/// The mocked `SpawnLogContext`: records the arguments of `logSpawn`.
#[derive(Default)]
struct MockSpawnLogContext {
    calls: Mutex<Vec<(String, Duration, Status, usize)>>,
}

impl SpawnLogContext for MockSpawnLogContext {
    fn log_spawn(
        &self,
        spawn: &dyn Spawn,
        _input_metadata_provider: &dyn InputMetadataProvider,
        _input_map: &dyn Fn() -> Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException> {
        self.calls.lock().push((
            spawn.get_mnemonic(),
            timeout,
            result.status(),
            Arc::as_ptr(file_system) as *const () as usize,
        ));
        Ok(())
    }
    fn log_symlink_action(&self, _action: &dyn SpawnLogAction) -> io::Result<()> {
        Ok(())
    }
    fn close(&self) -> io::Result<()> {
        Ok(())
    }
    fn should_publish(&self) -> bool {
        false
    }
}

#[test]
fn test_exec_calls_log_spawn() {
    let mut ctx = TestActionExecutionContext::new();
    let action_fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let spawn_log_context = Arc::new(MockSpawnLogContext::default());
    ctx.spawn_cache = Some(no_cache());
    ctx.spawn_log_context = Some(spawn_log_context.clone());
    ctx.action_file_system = Some(action_fs.clone());

    let spawn_result = success_result();
    let spawn_runner = MockSpawnRunner::new(spawn_result.clone());

    let spawn_results = tested_spawn_strategy(spawn_runner.clone())
        .exec(&simple_spawn(), &ctx)
        .unwrap();
    assert_eq!(spawn_results.len(), 1);
    assert_same_result(&spawn_results[0], &spawn_result);

    let calls = spawn_log_context.calls.lock();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "Mnemonic");
    assert_eq!(calls[0].1, Duration::ZERO);
    assert_eq!(calls[0].2, Status::Success);
    assert_eq!(calls[0].3, Arc::as_ptr(&action_fs) as *const () as usize);
}

/// A strategy whose `canExec` is fixed (for the resolver).
struct FixedStrategy {
    name: &'static str,
    can_exec: bool,
}

impl std::fmt::Display for FixedStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl SpawnStrategy for FixedStrategy {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _ctx: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        Ok(vec![success_result()])
    }
    fn can_exec(&self, _spawn: &dyn Spawn, _registry: &dyn ActionContextRegistry) -> bool {
        self.can_exec
    }
    fn get_simple_class_name(&self) -> String {
        "FixedStrategy".to_string()
    }
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        None
    }
}

/// Not in the Java test suite: checks `SpawnStrategyResolver.resolve`'s error text, copied from
/// its format string at the pin.
#[test]
fn resolver_no_usable_strategy_message() {
    let mut ctx = TestActionExecutionContext::new();
    let registry = SpawnStrategyRegistry::builder()
        .register_strategy(
            Arc::new(FixedStrategy {
                name: "linux-sandbox",
                can_exec: false,
            }),
            &["sandboxed"],
        )
        .register_strategy(
            Arc::new(FixedStrategy {
                name: "local",
                can_exec: false,
            }),
            &["local"],
        )
        .set_default_strategies(&["sandboxed".to_string(), "local".to_string()])
        .build()
        .unwrap();
    ctx.spawn_strategy_registry = Some(Arc::new(registry));

    let e = SpawnStrategyResolver::new()
        .resolve(&simple_spawn(), &ctx)
        .err()
        .unwrap();
    // Expected text: SpawnStrategyResolver.resolve's message format at the pin.
    assert_eq!(
        e.get_message(),
        "Mnemonic spawn cannot be executed with any of the available strategies: [linux-sandbox, local]. Your --spawn_strategy, --genrule_strategy, --strategy and/or --allowed_strategies_by_exec_platform flags are probably too strict. Visit https://github.com/bazelbuild/bazel/issues/7480 for advice.\n"
    );
}

/// Not in the Java test suite: `resolve` keeps the registry order and drops strategies whose
/// `canExec` is false.
#[test]
fn resolver_picks_first_execable_strategy() {
    let mut ctx = TestActionExecutionContext::new();
    let registry = SpawnStrategyRegistry::builder()
        .register_strategy(
            Arc::new(FixedStrategy {
                name: "worker",
                can_exec: false,
            }),
            &["worker"],
        )
        .register_strategy(
            Arc::new(FixedStrategy {
                name: "linux-sandbox",
                can_exec: true,
            }),
            &["sandboxed"],
        )
        .register_strategy(
            Arc::new(FixedStrategy {
                name: "local",
                can_exec: true,
            }),
            &["local"],
        )
        .set_default_strategies(&[
            "worker".to_string(),
            "sandboxed".to_string(),
            "local".to_string(),
        ])
        .build()
        .unwrap();
    ctx.spawn_strategy_registry = Some(Arc::new(registry));

    let strategies = SpawnStrategyResolver::new()
        .resolve(&simple_spawn(), &ctx)
        .unwrap();
    let names: Vec<String> = strategies.iter().map(|s| s.to_string()).collect();
    assert_eq!(names, vec!["linux-sandbox", "local"]);
}
