//! The graph dumps of `blaze dump --skyframe=...` (Java: the `dump*` methods of
//! `AbstractInMemoryMemoizingEvaluator`).
//!
//! PORT: Java iterates the graph's `ConcurrentHashMap` of node entries, whose order follows the
//! keys' `hashCode` (of `Label`, `PathFragment`, configuration checksums, ... and the map's
//! sizing history); that order is not reproduced. Every dump that lists nodes lists them sorted
//! by canonical name, and the function counts of `dumpCount` that are tied are sorted by
//! function name (Java: Guava `HashMultiset`, insertion order, i.e. the graph's order). For a
//! key's canonical name see [`canonical_name`].

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt::Write as _;

use crate::node_entry::NodeEntry as _;
use crate::{Evaluator, SkyFunctionName, SkyKey};

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
            DumpNode {
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

/// `dumpValues`: needs each value's `debugPrint` (Java `toString` of the `SkyValue`), which is
/// not ported; a dump that selects at least one node fails loudly.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:336 AbstractInMemoryMemoizingEvaluator.dumpValues
pub fn dump_values(evaluator: &Evaluator, filter: &dyn Fn(&str) -> bool) -> String {
    let entries = process_graph_for_dump_command(evaluator, filter);
    if let Some(first) = entries.first() {
        blaze_util::unported!(
            "SkyValue.debugPrint",
            "the Java toString of the value of {} (and {} more nodes)",
            first.canonical_name,
            entries.len() - 1
        );
    }
    String::new()
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
