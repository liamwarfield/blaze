//! The graph dumps of `blaze dump --skyframe=...` (Java: the `dump*` methods of
//! `AbstractInMemoryMemoizingEvaluator`).
//!
//! PORT: Java iterates the graph's `ConcurrentHashMap` of node entries, whose order follows the
//! keys' `hashCode` (of `Label`, `PathFragment`, configuration checksums, ... and the map's
//! sizing history); that order is not reproduced. Every dump that lists nodes lists them sorted
//! by canonical name, and the function counts of `dumpCount` that are tied are sorted by
//! function name (Java: Guava `HashMultiset`, insertion order, i.e. the graph's order). For a
//! key's canonical name see [`canonical_name`].

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::{LazyLock, Once};

use parking_lot::RwLock;

use crate::node_entry::NodeEntry as _;
use crate::{Evaluator, SkyFunctionName, SkyKey, SkyValue};

/// One node of the graph as the dumps read it.
struct DumpNode {
    key: SkyKey,
    canonical_name: String,
    done: bool,
    keeps_edges: bool,
    /// The direct dependencies of a done node that keeps edges, in groups.
    direct_dep_groups: Vec<Vec<SkyKey>>,
    /// The reverse dependencies of a done node that keeps edges.
    reverse_deps: Vec<SkyKey>,
    /// `entry.getValue()` of a done node (`null` for a node that holds only an error).
    value: Option<SkyValue>,
}

/// One value type's `SkyValue.debugPrint`: the text it writes to the stream, newlines included.
type DebugPrintFn = Box<dyn Fn(&(dyn Any + Send + Sync)) -> String + Send + Sync>;

/// `SkyValue.debugPrint` by value type. Java dispatches on the value's class; the Rust values are
/// `Arc<dyn Any>`, so each crate that defines a value type whose Java `toString` (or
/// `debugPrint` override) is deterministic registers it here with [`register_to_string`] or
/// [`register_debug_print`]. A value type that is not registered fails loudly in
/// [`dump_values`]: either its Java text is not ported yet, or it holds an identity hash code
/// (`Object.toString`), which no port can reproduce.
static DEBUG_PRINTS: LazyLock<RwLock<HashMap<TypeId, DebugPrintFn>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Registers `debug_print` as the `SkyValue.debugPrint` override of `T`: the whole text the
/// override writes, its newlines included.
pub fn register_debug_print<T: Any + Send + Sync>(debug_print: fn(&T) -> String) {
    DEBUG_PRINTS.write().insert(
        TypeId::of::<T>(),
        Box::new(move |v| debug_print(v.downcast_ref::<T>().expect("registered by TypeId"))),
    );
}

/// Registers `to_string` as the Java `toString()` of `T`, printed by the default
/// `SkyValue.debugPrint` (`out.println(this)`).
// Java: com/google/devtools/build/skyframe/SkyValue.java:27 SkyValue.debugPrint
pub fn register_to_string<T: Any + Send + Sync>(to_string: fn(&T) -> String) {
    DEBUG_PRINTS.write().insert(
        TypeId::of::<T>(),
        Box::new(move |v| {
            let mut out = to_string(v.downcast_ref::<T>().expect("registered by TypeId"));
            out.push('\n');
            out
        }),
    );
}

/// The value types of the graph's own nodes.
fn register_skyframe_values() {
    // Java: com/google/devtools/build/skyframe/ErrorTransienceValue.java:64 ErrorTransienceValue.toString
    register_to_string::<crate::evaluator::ErrorTransienceValue>(|_| {
        "ErrorTransienceValue".to_owned()
    });
    // A record: `EnvironmentVariableValue[value=...]`, `null` for an unset variable. The value of
    // CLIENT_ENVIRONMENT_VARIABLE and REPOSITORY_ENVIRONMENT_VARIABLE.
    // Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
    register_to_string::<crate::precomputed::EnvironmentVariableValue>(|v| {
        format!(
            "EnvironmentVariableValue[value={}]",
            v.value().unwrap_or("null")
        )
    });
}

/// `SkyKey.getCanonicalName()`: `functionName() + ":" + argument()`.
///
/// PORT: a key's `Debug` is Java's `toString()`. For an `AbstractSkyKey` that is already
/// `functionName + ":" + arg`; a key that is its own argument (`argument()` is `this`:
/// `GlobsValue.Key`, `TargetPatternKey`, ...) prints only itself, and gets the function name
/// prepended.
fn canonical_name(key: &SkyKey) -> String {
    let debug = format!("{key:?}");
    let prefix = format!("{}:", key.function_name().0);
    if debug.starts_with(&prefix) {
        debug
    } else {
        format!("{prefix}{debug}")
    }
}

/// `getInMemoryGraph().getAllNodeEntries()`, sorted by canonical name (see the module
/// documentation).
fn all_node_entries(evaluator: &Evaluator) -> Vec<DumpNode> {
    let mut nodes: Vec<DumpNode> = evaluator
        .graph
        .get_all_node_entries()
        .into_iter()
        .map(|(key, node)| {
            let mut state = node.lock();
            let entry = &mut state.entry;
            let done = entry.is_done();
            let keeps_edges = entry.keeps_edges();
            let (direct_dep_groups, reverse_deps) = if done && keeps_edges {
                (
                    entry
                        .get_compressed_direct_deps_for_done_entry()
                        .groups()
                        .to_vec(),
                    entry.get_reverse_deps_for_done_entry(),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            let value = if done { entry.get_value() } else { None };
            DumpNode {
                value,
                canonical_name: canonical_name(&key),
                key,
                done,
                keeps_edges,
                direct_dep_groups,
                reverse_deps,
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
    nodes
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:293 AbstractInMemoryMemoizingEvaluator.dumpSummary
pub fn dump_summary(evaluator: &Evaluator) -> String {
    let mut nodes: i64 = 0;
    let mut edges: i64 = 0;
    for entry in all_node_entries(evaluator) {
        nodes += 1;
        if entry.done && entry.keeps_edges {
            edges += entry
                .direct_dep_groups
                .iter()
                .map(|g| g.len() as i64)
                .sum::<i64>();
        }
    }
    let mut out = String::new();
    writeln!(out, "Node count: {nodes}").unwrap();
    writeln!(out, "Edge count: {edges}").unwrap();
    out
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:307 AbstractInMemoryMemoizingEvaluator.dumpCount
pub fn dump_count(evaluator: &Evaluator) -> String {
    let mut counter: BTreeMap<String, i64> = BTreeMap::new();
    for entry in all_node_entries(evaluator) {
        *counter
            .entry(entry.key.function_name().0.to_owned())
            .or_insert(0) += 1;
    }
    // Multisets.copyHighestCountFirst: by count, highest first (ties by name, see the module
    // documentation).
    let mut entries: Vec<(String, i64)> = counter.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out = String::new();
    for (element, count) in entries {
        // \t is spreadsheet-friendly.
        writeln!(out, "{element}\t{count}").unwrap();
    }
    out
}

/// `processGraphForDumpCommand`: the done entries whose canonical name passes `filter`.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:317 AbstractInMemoryMemoizingEvaluator.processGraphForDumpCommand
fn process_graph_for_dump_command(
    evaluator: &Evaluator,
    filter: &dyn Fn(&str) -> bool,
) -> Vec<DumpNode> {
    // PORT: Java checks Thread.interrupted() per entry and prints "aborting"; the dump here runs
    // over a snapshot of the graph.
    all_node_entries(evaluator)
        .into_iter()
        .filter(|entry| filter(&entry.canonical_name) && entry.done)
        .collect()
}

/// `dumpValues`: each done node's canonical name, its value's `debugPrint`, and an empty line.
///
/// PORT: a value type without a registered `debugPrint` (see [`register_to_string`]) fails
/// loudly, naming every such type among the selected nodes; Java prints them with their own
/// `toString`, which for the types not registered is either not ported yet or
/// `Object.toString` (class name and identity hash code).
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:336 AbstractInMemoryMemoizingEvaluator.dumpValues
pub fn dump_values(evaluator: &Evaluator, filter: &dyn Fn(&str) -> bool) -> String {
    static SKYFRAME_VALUES: Once = Once::new();
    SKYFRAME_VALUES.call_once(register_skyframe_values);
    let entries = process_graph_for_dump_command(evaluator, filter);
    let prints = DEBUG_PRINTS.read();
    let mut missing: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for entry in &entries {
        if let Some(value) = &entry.value {
            let value: &(dyn Any + Send + Sync) = value.as_ref();
            if !prints.contains_key(&value.type_id()) {
                let function = entry.key.function_name().0.to_owned();
                missing
                    .entry(function)
                    .or_insert_with(|| (entry.canonical_name.clone(), 0))
                    .1 += 1;
            }
        }
    }
    if !missing.is_empty() {
        let list: Vec<String> = missing
            .iter()
            .map(|(function, (first, n))| format!("{function} ({n} nodes, first {first})"))
            .collect();
        blaze_util::unported!(
            "SkyValue.debugPrint",
            "the Java toString of the values of {}",
            list.join(", ")
        );
    }
    let mut out = String::new();
    for entry in &entries {
        out.push_str(&entry.canonical_name);
        out.push('\n');
        // Java: entry.getValue().debugPrint(out); a node holding only an error has a null value.
        let Some(value) = &entry.value else {
            panic!(
                "java.lang.NullPointerException: the value of {} (an error) in dumpValues",
                entry.canonical_name
            );
        };
        let value: &(dyn Any + Send + Sync) = value.as_ref();
        out.push_str(&prints[&value.type_id()](value));
        out.push('\n');
    }
    out
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:349 AbstractInMemoryMemoizingEvaluator.dumpDeps
pub fn dump_deps(evaluator: &Evaluator, filter: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    for entry in process_graph_for_dump_command(evaluator, filter) {
        out.push_str(&entry.canonical_name);
        out.push('\n');
        if entry.keeps_edges {
            for (i, group) in entry.direct_dep_groups.iter().enumerate() {
                writeln!(out, "  Group {}:", i + 1).unwrap();
                for dep in group {
                    out.push_str("    ");
                    out.push_str(&canonical_name(dep));
                    out.push('\n');
                    out.push('\n'); // newline for readability
                }
            }
        } else {
            out.push_str("  (direct deps not stored)\n");
        }
        out.push('\n');
    }
    out
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:375 AbstractInMemoryMemoizingEvaluator.dumpFunctionGraph
pub fn dump_function_graph(evaluator: &Evaluator, filter: &dyn Fn(&str) -> bool) -> String {
    let mut seen: HashSet<(SkyFunctionName, SkyFunctionName)> = HashSet::new();
    let mut out = String::from("digraph {\n");
    for entry in process_graph_for_dump_command(evaluator, filter) {
        if entry.keeps_edges {
            let source = entry.key.function_name();
            // entry.getDirectDeps(): the groups flattened.
            for dep in entry.direct_dep_groups.iter().flatten() {
                let dest = dep.function_name();
                if !seen.insert((source, dest)) {
                    continue;
                }
                writeln!(out, "  \"{}\" -> \"{}\"", source.0, dest.0).unwrap();
            }
        }
    }
    out.push_str("}\n");
    out
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:398 AbstractInMemoryMemoizingEvaluator.dumpRdeps
pub fn dump_rdeps(evaluator: &Evaluator, filter: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    for entry in process_graph_for_dump_command(evaluator, filter) {
        out.push_str(&entry.canonical_name);
        out.push('\n');
        if entry.keeps_edges {
            // PORT: Java's order of the reverse deps is the entry's (insertion order of the
            // parents that registered); they are sorted by canonical name here.
            let mut rdeps: Vec<String> = entry.reverse_deps.iter().map(canonical_name).collect();
            rdeps.sort();
            for rdep in rdeps {
                out.push_str("    ");
                out.push_str(&rdep);
                out.push('\n');
                out.push('\n');
            }
        } else {
            out.push_str("  (rdeps not stored)\n");
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EvaluationContext, EvaluatorBuilder, FunctionHermeticity, Key};

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Printed(u32);
    struct PrintedValue(u32);
    impl Key for Printed {
        type Value = PrintedValue;
        const FUNCTION: SkyFunctionName = SkyFunctionName("PRINTED");
        const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Unprinted(u32);
    struct UnprintedValue;
    impl Key for Unprinted {
        type Value = UnprintedValue;
        const FUNCTION: SkyFunctionName = SkyFunctionName("UNPRINTED");
        const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    }

    fn evaluator() -> Evaluator {
        register_to_string::<PrintedValue>(|v| format!("printed {}", v.0));
        let ev = EvaluatorBuilder::new()
            .typed_function::<Printed, _>(|k: &Printed, _env| Ok(Some(PrintedValue(k.0 * 10))))
            .typed_function::<Unprinted, _>(|_: &Unprinted, _env| Ok(Some(UnprintedValue)))
            .build();
        ev.evaluate(
            &[
                SkyKey::new(Printed(2)),
                SkyKey::new(Printed(1)),
                SkyKey::new(Unprinted(1)),
            ],
            &EvaluationContext {
                parallelism: 2,
                thread_stack_size: 16 << 20,
                ..EvaluationContext::default()
            },
        )
        .expect("not interrupted");
        ev
    }

    #[test]
    fn dump_values_prints_name_value_and_empty_line() {
        let ev = evaluator();
        assert_eq!(
            dump_values(&ev, &|name| name.starts_with("PRINTED:")),
            "PRINTED:Printed(1)\nprinted 10\n\nPRINTED:Printed(2)\nprinted 20\n\n"
        );
        assert_eq!(
            dump_values(&ev, &|name| name.starts_with("NONE:")),
            "",
            "no node selected"
        );
    }

    #[test]
    #[should_panic(expected = "SkyValue.debugPrint")]
    fn dump_values_of_an_unregistered_type_fails_loudly() {
        let ev = evaluator();
        dump_values(&ev, &|_| true);
    }
}
