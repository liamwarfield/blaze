//! Port of src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java.
//!
//! PORT: Java's module keeps `localOptions`, `ignoreLocalSignals`, `verboseFailures` and the
//! reporter from `beforeCommand`; Blaze passes them to `get_local_strategies` and builds the
//! `ModuleIgnoreFailureCheck` from them, so `beforeCommand` with a mocked environment is the
//! construction of that check.

mod dynamic_common;

use std::sync::Arc;

use blaze_events::{EventHandler, StoredEventHandler};
use blaze_exec::dynamic::dynamic_execution_module::{
    ModuleIgnoreFailureCheck, get_local_strategies,
};
use blaze_exec::dynamic::dynamic_spawn_strategy::IgnoreFailureCheck;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_options::Options;
use dynamic_common::*;
use indexmap::IndexMap;

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:61 DynamicExecutionModuleTest.setUp
fn options() -> DynamicExecutionOptions {
    let mut options = Options::<DynamicExecutionOptions>::get_defaults();
    options.dynamic_local_strategy = Vec::new(); // default
    options.dynamic_remote_strategy = Vec::new(); // default
    options
}

/// `parseStrategies("key=a,b", "c")` (`StringToStringListConverter`: no `=` is the key "").
fn parse_strategies(strategies: &[&str]) -> IndexMap<String, Vec<String>> {
    let mut result = IndexMap::new();
    for s in strategies {
        let (key, values) = match s.split_once('=') {
            Some((k, v)) => (k, v),
            None => ("", *s),
        };
        result.insert(
            key.to_string(),
            values.split(',').map(str::to_string).collect::<Vec<_>>(),
        );
    }
    result
}

fn parse_strategies_to_options(strategies: &[&str]) -> Vec<(String, Vec<String>)> {
    parse_strategies(strategies).into_iter().collect()
}

/// `Map.equals`: the same entries, in any order.
fn assert_same_map(a: &IndexMap<String, Vec<String>>, b: &IndexMap<String, Vec<String>>) {
    assert_eq!(a.len(), b.len(), "{a:?} != {b:?}");
    for (k, v) in a {
        assert_eq!(b.get(k), Some(v), "{a:?} != {b:?}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:69 DynamicExecutionModuleTest.testGetLocalStrategies_getsDefaultWithNoOptions
#[test]
fn test_get_local_strategies_gets_default_with_no_options() {
    let options = options();
    assert_same_map(
        &get_local_strategies(&options, None, true).ok().unwrap(),
        &parse_strategies(&["worker,sandboxed"]),
    );
    assert_same_map(
        &get_local_strategies(&options, None, false).ok().unwrap(),
        &parse_strategies(&["worker"]),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:78 DynamicExecutionModuleTest.testGetLocalStrategies_genericOptionOverridesFallbacks
#[test]
fn test_get_local_strategies_generic_option_overrides_fallbacks() {
    let mut options = options();
    options.dynamic_local_strategy = parse_strategies_to_options(&["local,worker"]);
    assert_same_map(
        &get_local_strategies(&options, None, true).ok().unwrap(),
        &parse_strategies(&["local,worker"]),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:86 DynamicExecutionModuleTest.testGetLocalStrategies_specificOptionKeepsFallbacks
#[test]
fn test_get_local_strategies_specific_option_keeps_fallbacks() {
    let mut options = options();
    options.dynamic_local_strategy = parse_strategies_to_options(&["Foo=local,worker"]);
    assert_same_map(
        &get_local_strategies(&options, None, true).ok().unwrap(),
        &parse_strategies(&["Foo=local,worker", "worker,sandboxed"]),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:94 DynamicExecutionModuleTest.testGetLocalStrategies_canMixSpecificsAndGenericOptions
#[test]
fn test_get_local_strategies_can_mix_specifics_and_generic_options() {
    let mut options = options();
    options.dynamic_local_strategy = parse_strategies_to_options(&["Foo=local,worker", "worker"]);
    assert_same_map(
        &get_local_strategies(&options, None, true).ok().unwrap(),
        &parse_strategies(&["Foo=local,worker", "worker"]),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicExecutionModuleTest.java:102 DynamicExecutionModuleTest.canIgnoreFailure_simpleCases
#[test]
fn can_ignore_failure_simple_cases() {
    let spawn = spawn(&[], "Mnemonic", "output", false);
    let context = TestContext::with_strategies(
        MockStrategy::new("local", true),
        MockStrategy::new("remote", true),
    );
    // module.beforeCommand(env) with the given ignoreLocalSignals.
    let check = |signals: &[i32]| ModuleIgnoreFailureCheck {
        ignore_local_signals: signals.to_vec(),
        verbose_failures: false,
        reporter: Arc::new(StoredEventHandler::new()) as Arc<dyn EventHandler>,
    };

    let module = check(&[]);
    assert!(!module.can_ignore_failure(&*spawn, &context, 130, "Failed", None, true));

    let module = check(&[9]);
    assert!(!module.can_ignore_failure(&*spawn, &context, 130, "Failed", None, true));

    let module = check(&[2, 9]);
    assert!(!module.can_ignore_failure(&*spawn, &context, 130, "Failed", None, false));
    assert!(!module.can_ignore_failure(&*spawn, &context, 0, "Failed", None, true));
    assert!(module.can_ignore_failure(&*spawn, &context, 130, "Failed", None, true));
    assert!(module.can_ignore_failure(&*spawn, &context, 137, "Failed", None, true));
}
