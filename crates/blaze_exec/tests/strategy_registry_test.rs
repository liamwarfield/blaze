//! Port of `src/test/java/com/google/devtools/build/lib/exec/SpawnStrategyRegistryTest.java`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::spawn_strategy::{
    SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns,
};
use blaze_exec::actions::{
    ExecException, FakeOwner, PlatformInfo, ResourceSet, SimpleSpawn, Spawn, SpawnResult,
};
use blaze_exec::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::spawn_runner::{SpawnExecutionContext, SpawnRunner};
use blaze_exec::exec::spawn_strategy_registry::{DynamicMode, SpawnStrategyRegistry};
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::common::regex_filter::RegexFilter;
use blaze_protos::blaze::strategy_policy::{MnemonicPolicy, StrategyPolicy};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;
use regex::Regex;

fn ello_matcher() -> RegexFilter {
    RegexFilter::new(&["ello".to_string()], &[]).unwrap()
}

fn llo_matcher() -> RegexFilter {
    RegexFilter::new(&["llo".to_string()], &[]).unwrap()
}

/// `PlatformInfo.EMPTY_PLATFORM_INFO`.
fn empty_platform_info() -> PlatformInfo {
    PlatformInfo {
        label: Label::parse_canonical_unchecked("@bazel_tools//tools:internal_platform"),
        exec_properties: IndexMap::new(),
        remote_execution_properties: String::new(),
    }
}

fn create_spawn_with_mnemonic_and_description(mnemonic: &str, description: &str) -> SimpleSpawn {
    let mut owner = FakeOwner::new(
        mnemonic,
        Some(description),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    owner.platform = Some(empty_platform_info());
    SimpleSpawn::new_simple(
        Arc::new(owner),
        vec![],
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        vec![],
        Arc::new(ResourceSet::zero()),
    )
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

struct NoopStrategy {
    name: String,
    sandboxed: bool,
    used_called: AtomicI32,
}

impl NoopStrategy {
    fn new(name: &str) -> Arc<NoopStrategy> {
        Arc::new(NoopStrategy {
            name: name.to_string(),
            sandboxed: false,
            used_called: AtomicI32::new(0),
        })
    }
    fn new_sandboxed(name: &str) -> Arc<NoopStrategy> {
        Arc::new(NoopStrategy {
            name: name.to_string(),
            sandboxed: true,
            used_called: AtomicI32::new(0),
        })
    }
    fn used_called(&self) -> i32 {
        self.used_called.load(Ordering::SeqCst)
    }
}

impl fmt::Display for NoopStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "strategy{}", self.name)
    }
}

impl SpawnStrategy for NoopStrategy {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _ctx: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        panic!("UnsupportedOperationException")
    }
    fn can_exec(&self, _spawn: &dyn Spawn, _registry: &dyn ActionContextRegistry) -> bool {
        false
    }
    fn used_context(&self, _registry: Option<&dyn ActionContextRegistry>) {
        self.used_called.fetch_add(1, Ordering::SeqCst);
    }
    fn get_simple_class_name(&self) -> String {
        if self.sandboxed {
            "NoopSandboxedStrategy".to_string()
        } else {
            "NoopStrategy".to_string()
        }
    }
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        if self.sandboxed { Some(self) } else { None }
    }
}

impl SandboxedSpawnStrategy for NoopStrategy {
    fn exec_with_stop_concurrent_spawns(
        &self,
        _spawn: &dyn Spawn,
        _ctx: &dyn ActionExecutionContext,
        _stop: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        panic!("UnsupportedOperationException")
    }
}

struct NullRunner;

impl SpawnRunner for NullRunner {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        panic!("NullPointerException")
    }
    fn can_exec(&self, _spawn: &dyn Spawn) -> bool {
        panic!("NullPointerException")
    }
    fn handles_caching(&self) -> bool {
        false
    }
    fn get_name(&self) -> String {
        "null".to_string()
    }
}

struct NoopAbstractStrategy {
    inner: AbstractSpawnStrategy,
    name: String,
    used_called: AtomicI32,
}

impl NoopAbstractStrategy {
    fn new(name: &str) -> Arc<NoopAbstractStrategy> {
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        Arc::new(NoopAbstractStrategy {
            inner: AbstractSpawnStrategy::new(
                Path::create_from_str("/execroot", &fs),
                Arc::new(NullRunner),
                Arc::new(blaze_options::Options::<ExecutionOptions>::get_defaults()),
                name,
            ),
            name: name.to_string(),
            used_called: AtomicI32::new(0),
        })
    }
    fn used_called(&self) -> i32 {
        self.used_called.load(Ordering::SeqCst)
    }
}

impl fmt::Display for NoopAbstractStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "strategy{}", self.name)
    }
}

impl SpawnStrategy for NoopAbstractStrategy {
    fn exec(
        &self,
        spawn: &dyn Spawn,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.inner.exec(spawn, ctx)
    }
    fn can_exec(&self, spawn: &dyn Spawn, registry: &dyn ActionContextRegistry) -> bool {
        self.inner.can_exec(spawn, registry)
    }
    fn used_context(&self, _registry: Option<&dyn ActionContextRegistry>) {
        self.used_called.fetch_add(1, Ordering::SeqCst);
    }
    fn get_simple_class_name(&self) -> String {
        "NoopAbstractStrategy".to_string()
    }
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        Some(&self.inner)
    }
    fn as_abstract_spawn_strategy(&self) -> Option<&AbstractSpawnStrategy> {
        Some(&self.inner)
    }
}

fn ptr<T: ?Sized>(a: &Arc<T>) -> *const () {
    Arc::as_ptr(a) as *const ()
}

/// Truth `containsExactly(...)` (any order).
fn assert_contains_exactly(actual: &[Arc<dyn SpawnStrategy>], expected: &[*const ()]) {
    let mut a: Vec<*const ()> = actual.iter().map(ptr).collect();
    let mut e: Vec<*const ()> = expected.to_vec();
    a.sort();
    e.sort();
    assert_eq!(
        a,
        e,
        "actual: {}",
        blaze_exec::actions::spawn_strategy::strategies_to_string(actual)
    );
}

fn err_matches(message: &str, pattern: &str) {
    assert!(
        Regex::new(pattern).unwrap().is_match(message),
        "message {message:?} does not match {pattern:?}"
    );
}

#[test]
fn test_registration() {
    let strategy = NoopStrategy::new("");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy.clone(), &["foo"])
        .set_default_strategies(&strings(&["foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy)],
    );
}

#[test]
fn test_mnemonic_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_mnemonic_filter("mnem", &strings(&["bar", "foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            None,
        ),
        &[ptr(&strategy2), ptr(&strategy1)],
    );
}

#[test]
fn test_strategy_policy_applied_to_per_mnemonic_strategies() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_policy_proto = StrategyPolicy {
        mnemonic_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["foo"]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let strategy_registry = SpawnStrategyRegistry::builder_with_policy(&strategy_policy_proto)
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_mnemonic_filter("some-mnemonic", &strings(&["bar", "foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("some-mnemonic", ""),
            None,
        ),
        &[ptr(&strategy1)],
    );
}

#[test]
fn strategy_policy_applied_to_per_defaulttrategies() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_policy_proto = StrategyPolicy {
        mnemonic_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["foo"]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let strategy_registry = SpawnStrategyRegistry::builder_with_policy(&strategy_policy_proto)
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .build()
        .unwrap();

    let strategies = strategy_registry.get_strategies(
        &create_spawn_with_mnemonic_and_description("some-mnemonic", ""),
        None,
    );

    assert_contains_exactly(&strategies, &[ptr(&strategy1)]);
}

#[test]
fn strategy_policy_applied_to_regexp_filter_sanitize_strategy() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_policy_proto = StrategyPolicy {
        mnemonic_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["foo"]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let strategy_registry = SpawnStrategyRegistry::builder_with_policy(&strategy_policy_proto)
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_description_filter(ello_matcher(), &strings(&["foo", "bar"]))
        .build()
        .unwrap();

    let strategies = strategy_registry.get_strategies(
        &create_spawn_with_mnemonic_and_description("regex-mnemonic", "hello"),
        None,
    );

    assert_contains_exactly(&strategies, &[ptr(&strategy1)]);
}

#[test]
fn strategy_policy_applied_to_regexp_filter_fallback_to_default_strategy() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy3 = NoopStrategy::new("3");
    let strategy_policy_proto = StrategyPolicy {
        mnemonic_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["foo", "baz"]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let strategy_registry = SpawnStrategyRegistry::builder_with_policy(&strategy_policy_proto)
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .register_strategy(strategy3.clone(), &["baz"])
        .add_description_filter(ello_matcher(), &strings(&["foo"]))
        .add_description_filter(llo_matcher(), &strings(&["bar"]))
        .add_mnemonic_filter("regex-mnemonic", &strings(&["baz"]))
        .build()
        .unwrap();

    let strategies = strategy_registry.get_strategies(
        &create_spawn_with_mnemonic_and_description("regex-mnemonic", "hello"),
        None,
    );

    assert_contains_exactly(&strategies, &[ptr(&strategy3)]);
}

#[test]
fn test_later_strategy_overrides_earlier() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["foo"])
        .add_mnemonic_filter("mnem", &strings(&["foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

#[test]
fn test_description_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_description_filter(ello_matcher(), &strings(&["bar", "foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("", "hello"),
            None,
        ),
        &[ptr(&strategy2), ptr(&strategy1)],
    );
}

#[test]
fn test_description_has_precedence_over_mnemonic() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_mnemonic_filter("mnem", &strings(&["foo"]))
        .add_description_filter(ello_matcher(), &strings(&["bar"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("mnem", "hello"),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

#[test]
fn test_multiple_mnemonic_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_mnemonic_filter("mnem", &strings(&["foo"]))
        .add_mnemonic_filter("mnem", &strings(&["bar"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

/// If an action matches multiple filters, the latter one gets the priority.
#[test]
fn test_multiple_description_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_description_filter(ello_matcher(), &strings(&["foo"]))
        .add_description_filter(llo_matcher(), &strings(&["bar"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("", "hello"),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

/// This demonstrate that the latter description filter overrides preceding one of same regexp.
/// filter=val_1 filter=val_2 is equivalent to filter=val_2
#[test]
fn test_duplicated_description_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_description_filter(ello_matcher(), &strings(&["foo"]))
        .add_description_filter(ello_matcher(), &strings(&["bar"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("", "hello"),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

#[test]
fn test_platform_filter() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_exec_platform_filter(empty_platform_info().label, &strings(&["foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy1)],
    );
}

/// Tests that platform filters not affect the strategy ordering.
#[test]
fn test_platform_filter_order() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_exec_platform_filter(empty_platform_info().label, &strings(&["bar", "foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy1), ptr(&strategy2)],
    );
}

#[test]
fn test_multiple_default_strategies() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy3 = NoopStrategy::new("3");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .register_strategy(strategy3.clone(), &["baz"])
        .set_default_strategies(&strings(&["foo", "baz"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy1), ptr(&strategy3)],
    );
}

#[test]
fn test_default_strategies_independent_of_filters() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy3 = NoopStrategy::new("3");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .register_strategy(strategy3.clone(), &["baz"])
        .add_mnemonic_filter("mnem", &strings(&["bar"]))
        .set_default_strategies(&strings(&["foo", "baz"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy1), ptr(&strategy3)],
    );

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            None,
        ),
        &[ptr(&strategy2)],
    );
}

#[test]
fn test_implicit_default() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry
            .get_strategies(&create_spawn_with_mnemonic_and_description("", ""), None),
        &[ptr(&strategy1), ptr(&strategy2)],
    );
}

#[test]
fn test_mnemonic_strategy_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_mnemonic_filter("mnem", &strings(&["bar", "foo"]))
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "bar.*Valid.*foo");
}

/// Don't throw an error if any of the replaced strategies was not registered.
#[test]
fn test_description_strategy_replaced_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_description_filter(ello_matcher(), &strings(&["bar", "foo"]))
        .add_description_filter(ello_matcher(), &strings(&["foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_strategies(
            &create_spawn_with_mnemonic_and_description("", "hello"),
            None,
        ),
        &[ptr(&strategy1)],
    );
}

/// Throw error when some of strategies were not registered.
#[test]
fn test_description_strategy_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_description_filter(ello_matcher(), &strings(&["bar", "foo"]))
        .build()
        .unwrap_err();

    err_matches(
        exception.get_message(),
        "'bar' was requested.*Valid values are: \\[foo\\]",
    );
}

#[test]
fn test_description_strategy_all_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_description_filter(ello_matcher(), &strings(&["bar", "food"]))
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "bar.*Valid.*foo");
}

#[test]
fn test_default_strategy_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .set_default_strategies(&strings(&["bar"]))
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "bar.*Valid.*foo");
}

fn mnemonic_map(mnemonic: &str, ids: &[&str]) -> IndexMap<String, Vec<String>> {
    let mut m = IndexMap::new();
    m.insert(mnemonic.to_string(), strings(ids));
    m
}

#[test]
fn test_dynamic_strategies() {
    let strategy1 = NoopStrategy::new_sandboxed("1");
    let strategy2 = NoopStrategy::new_sandboxed("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .add_dynamic_local_strategies(&mnemonic_map("mnem", &["bar"]))
        .add_dynamic_remote_strategies(&mnemonic_map("mnem", &["foo"]))
        .build()
        .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_dynamic_spawn_action_contexts(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            DynamicMode::Remote,
        ),
        &[ptr(&strategy1)],
    );
    assert_contains_exactly(
        &strategy_registry.get_dynamic_spawn_action_contexts(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            DynamicMode::Local,
        ),
        &[ptr(&strategy2)],
    );
}

#[test]
fn test_dynamic_strategy_not_present() {
    let strategy1 = NoopStrategy::new_sandboxed("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_dynamic_local_strategies(&mnemonic_map("mnem", &["bar"]))
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "bar.*Valid.*foo");
}

#[test]
fn test_dynamic_strategy_not_sandboxed() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .add_dynamic_local_strategies(&mnemonic_map("mnem", &["foo"]))
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "sandboxed strategy");
}

#[test]
fn test_dynamic_strategies_honor_strategy_policy() {
    let remote_strategy = NoopStrategy::new_sandboxed("remote");
    let local_strategy = NoopStrategy::new_sandboxed("local");
    let strategy_registry = SpawnStrategyRegistry::builder_with_policy(&StrategyPolicy {
        dynamic_remote_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["remote"]),
            ..Default::default()
        }),
        dynamic_local_policy: Some(MnemonicPolicy {
            default_allowlist: strings(&["local"]),
            ..Default::default()
        }),
        ..Default::default()
    })
    .register_strategy(remote_strategy.clone(), &["remote"])
    .register_strategy(local_strategy.clone(), &["local"])
    // Pointlessly register both strategies in order to test that policy filters them.
    .add_dynamic_local_strategies(&mnemonic_map("mnem", &["remote", "local"]))
    .add_dynamic_remote_strategies(&mnemonic_map("mnem", &["remote", "local"]))
    .build()
    .unwrap();

    assert_contains_exactly(
        &strategy_registry.get_dynamic_spawn_action_contexts(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            DynamicMode::Remote,
        ),
        &[ptr(&remote_strategy)],
    );
    assert_contains_exactly(
        &strategy_registry.get_dynamic_spawn_action_contexts(
            &create_spawn_with_mnemonic_and_description("mnem", ""),
            DynamicMode::Local,
        ),
        &[ptr(&local_strategy)],
    );
}

#[test]
fn test_remote_local_fallback() {
    let strategy1 = NoopAbstractStrategy::new("1");
    let strategy2 = NoopAbstractStrategy::new("2");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .register_strategy(strategy2.clone(), &["bar"])
        .set_remote_local_fallback_strategy_identifier("bar")
        .build()
        .unwrap();

    let fallback = strategy_registry
        .get_remote_local_fallback_strategy(&create_spawn_with_mnemonic_and_description("", ""))
        .unwrap();
    assert_eq!(ptr(&fallback), ptr(&strategy2));
}

#[test]
fn test_remote_local_fallback_not_present() {
    let strategy1 = NoopStrategy::new("1");
    let exception = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .set_remote_local_fallback_strategy_identifier("bar")
        .build()
        .unwrap_err();

    err_matches(exception.get_message(), "bar.*Valid.*foo");
}

#[test]
fn test_remote_local_fallback_not_registered() {
    let strategy1 = NoopStrategy::new("1");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["foo"])
        .build()
        .unwrap();

    assert!(
        strategy_registry
            .get_remote_local_fallback_strategy(&create_spawn_with_mnemonic_and_description("", ""))
            .is_none()
    );
}

#[test]
fn test_notify_used() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy3 = NoopStrategy::new("3");
    let strategy4 = NoopAbstractStrategy::new("4");
    let strategy5 = NoopStrategy::new_sandboxed("5");
    let strategy6 = NoopStrategy::new_sandboxed("6");
    let strategy7 = NoopStrategy::new("7");
    let strategy8 = NoopStrategy::new("8");
    let strategy9 = NoopStrategy::new("9");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["1"])
        .register_strategy(strategy2.clone(), &["2"])
        .register_strategy(strategy3.clone(), &["3"])
        .register_strategy(strategy7.clone(), &["4"]) // no notification: identifier is overridden
        .register_strategy(strategy4.clone(), &["4"])
        .register_strategy(strategy5.clone(), &["5"]) // no notification: dynamic strategies are separate
        .register_strategy(strategy6.clone(), &["6"]) // no notification: dynamic strategies are separate
        .register_strategy(strategy8.clone(), &["8"]) // no notification: never referenced
        .register_strategy(strategy9.clone(), &["9"]) // no notification: reference overridden
        .add_mnemonic_filter("mnem", &strings(&["1"]))
        .add_description_filter(ello_matcher(), &strings(&["2"]))
        .set_default_strategies(&strings(&["9"]))
        .set_default_strategies(&strings(&["3"]))
        .set_remote_local_fallback_strategy_identifier("4")
        .add_dynamic_local_strategies(&mnemonic_map("oy", &["5"]))
        .add_dynamic_remote_strategies(&mnemonic_map("oy", &["6"]))
        .build()
        .unwrap();

    strategy_registry.notify_used(None);

    assert_eq!(strategy1.used_called(), 1);
    assert_eq!(strategy2.used_called(), 1);
    assert_eq!(strategy3.used_called(), 1);
    assert_eq!(strategy4.used_called(), 1);

    assert_eq!(strategy5.used_called(), 0);
    assert_eq!(strategy6.used_called(), 0);
    assert_eq!(strategy7.used_called(), 0);
    assert_eq!(strategy8.used_called(), 0);
    assert_eq!(strategy9.used_called(), 0);
}

#[test]
fn test_notify_used_dynamic() {
    let strategy1 = NoopStrategy::new("1");
    let strategy2 = NoopStrategy::new("2");
    let strategy3 = NoopStrategy::new("3");
    let strategy4 = NoopAbstractStrategy::new("4");
    let strategy5 = NoopStrategy::new_sandboxed("5");
    let strategy6 = NoopStrategy::new_sandboxed("6");
    let strategy7 = NoopStrategy::new("7");
    let strategy_registry = SpawnStrategyRegistry::builder()
        .register_strategy(strategy1.clone(), &["1"]) // no notification: regular strategies are separate
        .register_strategy(strategy2.clone(), &["2"]) // no notification: regular strategies are separate
        .register_strategy(strategy3.clone(), &["3"]) // no notification: regular strategies are separate
        .register_strategy(strategy4.clone(), &["4"]) // no notification: regular strategies are separate
        .register_strategy(strategy5.clone(), &["5"])
        .register_strategy(strategy6.clone(), &["6"])
        .register_strategy(strategy7.clone(), &["7"]) // no notification: reference overridden
        .add_mnemonic_filter("mnem", &strings(&["1"]))
        .add_description_filter(ello_matcher(), &strings(&["2"]))
        .set_default_strategies(&strings(&["3"]))
        .set_remote_local_fallback_strategy_identifier("4")
        .add_dynamic_local_strategies(&mnemonic_map("oy", &["7"]))
        .add_dynamic_local_strategies(&mnemonic_map("oy", &["5"]))
        .add_dynamic_remote_strategies(&mnemonic_map("oy", &["6"]))
        .build()
        .unwrap();

    strategy_registry.notify_used_dynamic(None);

    assert_eq!(strategy1.used_called(), 0);
    assert_eq!(strategy2.used_called(), 0);
    assert_eq!(strategy3.used_called(), 0);
    assert_eq!(strategy4.used_called(), 0);

    assert_eq!(strategy5.used_called(), 1);
    assert_eq!(strategy6.used_called(), 1);

    assert_eq!(strategy7.used_called(), 0);
}
