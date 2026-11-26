//! `blaze dump`: dumps the internal state of the server process.
//!
//! PORT: the state `dump` reads is the server's: the Skyframe graph left by earlier commands, the
//! loaded action cache, the rule class provider. Blaze keeps no Skyframe graph between commands
//! (docs/research/08-server-and-incrementality.md, decision D2: Bazel's
//! `--notrack_incremental_state` behaviour), so the graph a command that does not analyze starts
//! with is empty: [`server_evaluator`] is that graph. The dumps of the graph are ported for it;
//! a dump that would have to iterate graph entries (their order is the one of Java's
//! `ConcurrentHashMap`, their edges and `debugPrint` are not exposed by `blaze_skyframe`) fails
//! loudly. `--memory` (`SkyframeMemoryDumper`) measures JVM objects and is not ported.

use std::sync::Arc;

use blaze_events::{Event, EventHandler as _, OutErr};
use blaze_exec::actions::cache::action_cache::ActionCache;
use blaze_exec::actions::cache::compact_persistent_action_cache::CompactPersistentActionCache;
use blaze_optdefs::command::dump_command::{DumpCommandDumpOptions, SkyframeDumpOption};
use blaze_optdefs::common::regex_filter::RegexFilter;
use blaze_options::{HelpVerbosity, OptionsClass};
use blaze_packages::native_rules::RuleClassProvider;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_skyframe::Evaluator;

use crate::blaze_command_utils::{BlazeCommandUtils, command_resource_package};
use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

/// Implementation of the dump command.
// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:99 DumpCommand
pub struct DumpCommand;

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:324 DumpCommand.WARNING_MESSAGE
pub const WARNING_MESSAGE: &str = "This information is intended for consumption by developers \
                                   only, and may change at any time. Script against it at your \
                                   own risk!";

/// The server's Skyframe graph at the start of a command that does not analyze.
///
/// PORT: Java reads `env.getSkyframeExecutor().getEvaluator()`, the server's evaluator, which
/// holds what earlier commands computed. Blaze discards the graph after each command (decision
/// D2, see the module comment), so it is an empty evaluator.
pub(crate) fn server_evaluator() -> Evaluator {
    blaze_skyframe::EvaluatorBuilder::new().build()
}

impl BlazeCommand for DumpCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::DUMP_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:329 DumpCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let dump_options: DumpCommandDumpOptions = options
            .get_options::<DumpCommandDumpOptions>()
            .expect("NullPointerException: DumpOptions");

        let any_output = dump_options.dump_packages
            || dump_options.dump_action_cache
            || dump_options.dump_rule_classes
            || dump_options.dump_rules
            || dump_options.starlark_memory.is_some()
            || dump_options.dump_skyframe != SkyframeDumpOption::OFF
            || dump_options.memory.is_some();
        if !any_output {
            let annotation = self.annotation();
            env.get_reporter()
                .get_out_err()
                .print_err_ln(&BlazeCommandUtils::expand_help_topic(
                    annotation.name(),
                    annotation.help,
                    command_resource_package(annotation.name()),
                    &[DumpCommandDumpOptions::class()],
                    HelpVerbosity::LONG,
                    env.get_runtime().get_product_name(),
                ));
            return create_failure_result(
                "no output specified",
                failure_details::dump_command::Code::NoOutputSpecified,
            );
        }
        // new PrintStream(new BufferedOutputStream(outErr.getOutputStream(), 1024 * 1024)), with
        // the default charset (ISO-8859-1); flushed in the finally block.
        // PORT: the buffer is flushed once, at the end (Java also flushes whenever 1 MiB is
        // buffered; the bytes and their order on stdout are the same).
        let out_err = env.get_reporter().get_out_err().clone();
        let mut out = PrintStream::default();
        let result = self.dump(env, &dump_options, &mut out);
        out.flush_to(&out_err);
        result
    }
}

impl DumpCommand {
    /// The `try` block of `exec`.
    // Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:360 DumpCommand.exec
    fn dump(
        &self,
        env: &mut CommandEnvironment,
        dump_options: &DumpCommandDumpOptions,
        out: &mut PrintStream,
    ) -> BlazeCommandResult {
        env.get_reporter().handle(&Event::warn(WARNING_MESSAGE));
        let mut failure: Option<BlazeCommandResult> = None;
        let evaluator = server_evaluator();

        if dump_options.dump_packages {
            // env.getPackageManager().dump(out): SkyframePackageManager.dump ->
            // SequencedSkyframeExecutor.dumpPackages.
            dump_packages(&evaluator, out);
            out.println("");
        }

        if dump_options.dump_action_cache {
            if !dump_action_cache(env, out) {
                failure = Some(create_failure_result(
                    "action cache dump failed",
                    failure_details::dump_command::Code::ActionCacheDumpFailed,
                ));
            }
            out.println("");
        }

        if dump_options.dump_rule_classes {
            dump_rule_classes(out);
            out.println("");
        }

        if dump_options.dump_rules {
            dump_rule_stats(&evaluator, out);
            out.println("");
        }

        if dump_options.starlark_memory.is_some() {
            // The InstrumentationOutput is only created here; dumpStarlarkHeap opens its stream
            // only when there is an allocation tracker.
            // PORT: Blaze has no AllocationTracker (Java installs one only when the server runs
            // with the allocation instrumenter's Java agent), so `workspace.getAllocationTracker()`
            // is null.
            dump_starlark_heap(out);
        }

        if dump_options.memory.is_some() {
            // SkyframeMemoryDumper measures the JVM objects of Skyframe values (RamUsageEstimator,
            // object graph walking): no Rust counterpart.
            blaze_util::unported!("DumpCommand.dumpSkyframeMemory");
        }

        match dump_options.dump_skyframe {
            SkyframeDumpOption::SUMMARY => dump_summary(&evaluator, out),
            SkyframeDumpOption::COUNT => dump_count(&evaluator, out),
            SkyframeDumpOption::VALUE => dump_values(&evaluator, out, &dump_options.sky_key_filter),
            SkyframeDumpOption::DEPS => dump_deps(&evaluator, out, &dump_options.sky_key_filter),
            SkyframeDumpOption::RDEPS => dump_rdeps(&evaluator, out, &dump_options.sky_key_filter),
            SkyframeDumpOption::FUNCTION_GRAPH => {
                dump_function_graph(&evaluator, out, &dump_options.sky_key_filter)
            }
            SkyframeDumpOption::ACTIVE_DIRECTORIES => {
                blaze_util::unported!("SkyfocusState.dumpActiveDirectories")
            }
            SkyframeDumpOption::ACTIVE_DIRECTORIES_FRONTIER_DEPS => {
                blaze_util::unported!("SkyfocusState.dumpFrontierSet")
            }
            SkyframeDumpOption::OFF => {}
        }
        // PORT: the `catch (InterruptedException e)` of exec is not reachable: none of the dumps
        // above evaluates or polls for interrupts on an empty graph.
        failure.unwrap_or_else(BlazeCommandResult::success)
    }
}

/// `java.io.PrintStream` over a buffer: `print` encodes with the default charset (ISO-8859-1).
#[derive(Default)]
pub(crate) struct PrintStream {
    bytes: Vec<u8>,
}

impl PrintStream {
    pub(crate) fn print(&mut self, s: &str) {
        self.bytes
            .extend_from_slice(&blaze_util::string_encoding::latin1_encode_lossy(s));
    }

    pub(crate) fn println(&mut self, s: &str) {
        self.print(s);
        self.bytes.push(b'\n');
    }

    /// `out.flush()` onto the command's stdout.
    pub(crate) fn flush_to(&mut self, out_err: &OutErr) {
        let _ = out_err.get_output_stream().write(&self.bytes);
        let _ = out_err.get_output_stream().flush();
        self.bytes.clear();
    }
}

impl std::io::Write for PrintStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Fails loudly at the first node of a graph dump: see the module comment.
fn unported_graph_entries(method: &'static str, node_count: usize) -> ! {
    blaze_util::unported!(
        method,
        "a non-empty Skyframe graph ({} nodes): Java's ConcurrentHashMap order, the node's edges \
         and SkyValue.debugPrint",
        node_count
    )
}

// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:788 SequencedSkyframeExecutor.dumpPackages
fn dump_packages(evaluator: &Evaluator, out: &mut PrintStream) {
    let package_sky_keys: Vec<_> = evaluator
        .get_values()
        .into_keys()
        .filter(|k| k.function_name().0 == "PACKAGE")
        .collect();
    out.println(&format!("{} packages", package_sky_keys.len()));
    if !package_sky_keys.is_empty() {
        // Iterables.filter over the values map (Java's ConcurrentHashMap order), then
        // Package.dump for each.
        unported_graph_entries(
            "SequencedSkyframeExecutor.dumpPackages",
            package_sky_keys.len(),
        );
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:443 DumpCommand.dumpActionCache
fn dump_action_cache(env: &CommandEnvironment, out: &mut PrintStream) -> bool {
    let reporter = env.get_reporter();
    // env.getBlazeWorkspace().getOrLoadPersistentActionCache(reporter).
    // PORT: no action cache stays loaded between commands; it is loaded from the output base.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:386 BlazeWorkspace.getOrLoadPersistentActionCache
    let output_base = env.get_output_base();
    match CompactPersistentActionCache::create(
        &output_base.get_child("action_cache"),
        &output_base.get_child("action_cache.bad"),
        &output_base.get_child("action_cache.tmp"),
        Arc::new(blaze_events::JavaClock),
        reporter.as_ref() as &dyn blaze_events::EventHandler,
    ) {
        Ok(cache) => {
            crate::idle_tasks::note_persistent_action_cache_loaded();
            cache.dump(out);
            true
        }
        Err(e) => {
            reporter.handle(&Event::error(format!(
                "Cannot dump action cache: {}",
                java_io_message(&e)
            )));
            false
        }
    }
}

/// Java `IOException.getMessage()` for an [`std::io::Error`] (`EOFException` has a null
/// message).
fn java_io_message(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::UnexpectedEof && e.get_ref().is_none() {
        "null".to_string()
    } else {
        e.to_string()
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:454 DumpCommand.dumpRuleClasses
fn dump_rule_classes(out: &mut PrintStream) {
    // runtime.getRuleClassProvider().getRuleClassMap()
    let provider = RuleClassProvider::create_bazel();
    let rule_class_map = provider.get_rule_class_map();
    let mut rule_class_names: Vec<&String> = rule_class_map.keys().collect();
    // Collections.sort: String.compareTo (UTF-16 code units; the names are ASCII).
    rule_class_names.sort();
    for name in rule_class_names {
        if name.starts_with('$') {
            continue;
        }
        let rule_class = &rule_class_map[name];
        out.print(&format!("{}(", rule_class));
        let mut first = true;
        for attribute in rule_class.get_attribute_provider().get_attributes() {
            if attribute.is_implicit() {
                continue;
            }
            if first {
                first = false;
            } else {
                out.print(", ");
            }
            out.print(attribute.get_name());
        }
        out.println(")");
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:480 DumpCommand.dumpRuleStats
fn dump_rule_stats(evaluator: &Evaluator, out: &mut PrintStream) {
    // executor.getSkyframeStats(eventHandler): SequencedSkyframeExecutor.getSkyframeStats walks
    // the done values for rule configured targets and aspects.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:522 SequencedSkyframeExecutor.getSkyframeStats
    let done_values = evaluator.get_done_values();
    if !done_values.is_empty() {
        unported_graph_entries(
            "SequencedSkyframeExecutor.getSkyframeStats",
            done_values.len(),
        );
    }
    // skyframeStats.ruleStats().isEmpty()
    out.print("No rules in Bazel server, please run a build command first.");
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:691 DumpCommand.dumpStarlarkHeap
fn dump_starlark_heap(out: &mut PrintStream) {
    // allocationTracker == null (see the caller).
    out.println(
        "Cannot dump Starlark heap without running in memory tracking mode. \
         Please refer to the user manual for the dump commnd \
         for information how to turn on memory tracking.",
    );
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:293 AbstractInMemoryMemoizingEvaluator.dumpSummary
fn dump_summary(evaluator: &Evaluator, out: &mut PrintStream) {
    let nodes = evaluator.get_all_keys_for_testing().len();
    if nodes != 0 {
        // The edge count reads each done entry's direct deps.
        unported_graph_entries("AbstractInMemoryMemoizingEvaluator.dumpSummary", nodes);
    }
    let edges: i64 = 0;
    out.println(&format!("Node count: {}", nodes));
    out.println(&format!("Edge count: {}", edges));
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:307 AbstractInMemoryMemoizingEvaluator.dumpCount
fn dump_count(evaluator: &Evaluator, _out: &mut PrintStream) {
    let nodes = evaluator.get_all_keys_for_testing().len();
    if nodes != 0 {
        // Multisets.copyHighestCountFirst of a HashMultiset: ties in HashMap order.
        unported_graph_entries("AbstractInMemoryMemoizingEvaluator.dumpCount", nodes);
    }
}

/// `processGraphForDumpCommand`: every done entry whose canonical name passes `filter`.
// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:317 AbstractInMemoryMemoizingEvaluator.processGraphForDumpCommand
fn process_graph_for_dump_command(
    evaluator: &Evaluator,
    _filter: &RegexFilter,
    method: &'static str,
) {
    let nodes = evaluator.get_all_keys_for_testing().len();
    if nodes != 0 {
        unported_graph_entries(method, nodes);
    }
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:336 AbstractInMemoryMemoizingEvaluator.dumpValues
fn dump_values(evaluator: &Evaluator, _out: &mut PrintStream, filter: &RegexFilter) {
    process_graph_for_dump_command(
        evaluator,
        filter,
        "AbstractInMemoryMemoizingEvaluator.dumpValues",
    );
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:349 AbstractInMemoryMemoizingEvaluator.dumpDeps
fn dump_deps(evaluator: &Evaluator, _out: &mut PrintStream, filter: &RegexFilter) {
    process_graph_for_dump_command(
        evaluator,
        filter,
        "AbstractInMemoryMemoizingEvaluator.dumpDeps",
    );
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:375 AbstractInMemoryMemoizingEvaluator.dumpFunctionGraph
fn dump_function_graph(evaluator: &Evaluator, out: &mut PrintStream, filter: &RegexFilter) {
    out.println("digraph {");
    process_graph_for_dump_command(
        evaluator,
        filter,
        "AbstractInMemoryMemoizingEvaluator.dumpFunctionGraph",
    );
    out.println("}");
}

// Java: com/google/devtools/build/skyframe/AbstractInMemoryMemoizingEvaluator.java:398 AbstractInMemoryMemoizingEvaluator.dumpRdeps
fn dump_rdeps(evaluator: &Evaluator, _out: &mut PrintStream, filter: &RegexFilter) {
    process_graph_for_dump_command(
        evaluator,
        filter,
        "AbstractInMemoryMemoizingEvaluator.dumpRdeps",
    );
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:712 DumpCommand.createFailureResult
fn create_failure_result(
    message: &str,
    detailed_code: failure_details::dump_command::Code,
) -> BlazeCommandResult {
    BlazeCommandResult::failure_detail(FailureDetail {
        message: message.to_string(),
        category: Some(Category::DumpCommand(failure_details::DumpCommand {
            code: detailed_code as i32,
        })),
    })
}

#[cfg(test)]
mod tests {
    //! `DumpCommandTest` (src/test/java/com/google/devtools/build/lib/runtime/commands).

    /// `dump --skyframe count`: the warning goes to stderr, not stdout.
    /// Checked end to end against the oracle instead (`dump --skyframe=count`, identical stdout
    /// and stderr; docs/reviews/2026-11-25-commands-response.md).
    #[test]
    #[ignore = "needs a dispatcher test fixture with a recording OutErr (dispatcher_tests.rs keeps its fixture private)"]
    fn does_not_contain_warning_in_stdout() {}

    /// Builds `//foo:foo` with `--nobuild`, then dumps rule classes, rules and the Skyframe
    /// summary from the same server.
    #[test]
    #[ignore = "needs the Skyframe graph kept between commands (Blaze discards it, decision D2)"]
    fn multi_option_smoke() {}
}
