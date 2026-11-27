//! `blaze dump`: dumps the internal state of the server process.
//!
//! The state `dump` reads is the server's: the Skyframe graph left by earlier commands
//! ([`server_evaluator`]: the graph the server keeps between commands; an empty one when it
//! keeps none, decision D2), the loaded action cache, the rule class provider. Where Java lists
//! graph entries in its `ConcurrentHashMap` order (hash order), the dumps here sort them
//! (`--packages` by package, `--skyframe` by key; see `blaze_skyframe::dump`). `--skyframe=value`
//! needs each value's Java `toString` and fails loudly on a non-empty selection; `--memory`
//! (`SkyframeMemoryDumper`) measures JVM objects and is not ported.

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
/// Java reads `env.getSkyframeExecutor().getEvaluator()`, the server's evaluator, which holds
/// what earlier commands computed: here the graph the server keeps between commands. PORT: when
/// none is kept (a fresh server, `--batch`, or a server that does not keep graphs, decision D2)
/// it is an empty evaluator, as Java's after `resetEvaluator`.
pub(crate) fn server_evaluator(env: &CommandEnvironment) -> Arc<Evaluator> {
    env.get_blaze_workspace()
        .get_incremental_state()
        .kept_evaluator()
        .unwrap_or_else(|| Arc::new(blaze_skyframe::EvaluatorBuilder::new().build()))
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
    // Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:329 DumpCommand.exec
    fn dump(
        &self,
        env: &mut CommandEnvironment,
        dump_options: &DumpCommandDumpOptions,
        out: &mut PrintStream,
    ) -> BlazeCommandResult {
        env.get_reporter().handle(&Event::warn(WARNING_MESSAGE));
        let mut failure: Option<BlazeCommandResult> = None;
        let evaluator = server_evaluator(env);

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
            // PORT: skipped, memory instrumentation (dump --memory; out of scope by contract): DumpCommand.getConfigurationKey, DumpCommand.getMemoryDumpSkyKey, DumpCommand.dumpSkyframeMemory
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

/// `dumpPackages`: the count of PACKAGE nodes with a value, then `Package.dump` of each.
///
/// PORT: Java lists the packages in the order of the graph's `ConcurrentHashMap` (hash order of
/// the package keys); here they are sorted by package identifier (canonical form).
// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:788 SequencedSkyframeExecutor.dumpPackages
fn dump_packages(evaluator: &Evaluator, out: &mut PrintStream) {
    use blaze_sky_loading::package_function::PackageValue;
    let mut packages: Vec<Arc<blaze_packages::package::package::Package>> = Vec::new();
    let mut count = 0usize;
    for (key, value) in evaluator.get_values() {
        if key.function_name().0 != "PACKAGE" {
            continue;
        }
        count += 1;
        if let Some(package_value) = value.downcast_ref::<PackageValue>() {
            packages.push(package_value.get_package().clone());
        }
        // Java: a null PackageValue (`pkgVal == null`) prints nothing for that key.
    }
    out.println(&format!("{count} packages"));
    packages.sort_by_key(|p| p.get_package_identifier().get_canonical_form());
    for package in packages {
        dump_package(&package, out);
    }
}

/// `Package.dump(out)`: the rules with every attribute's possible values, then the files.
// Java: com/google/devtools/build/lib/packages/Package.java:479 Package.dump
fn dump_package(package: &blaze_packages::package::package::Package, out: &mut PrintStream) {
    use blaze_packages::attribute::AggregatingAttributeMapper;
    use blaze_packages::target::target::Target;
    out.println(&format!(
        "  Package {} ({})",
        package.get_name(),
        package
            .get_metadata()
            .build_filename
            .as_path()
            .get_path_string()
    ));

    // Rules:
    out.println("    Rules");
    for target in package.get_targets().values() {
        let Target::Rule(rule) = target else {
            continue;
        };
        out.println(&format!(
            "      {} {}",
            target.get_target_kind(),
            rule.get_label()
        ));
        let mapper = AggregatingAttributeMapper::of(&**rule);
        for attr in rule
            .get_rule_class_object()
            .get_attribute_provider()
            .get_attributes()
        {
            for possible_value in mapper.visit_attribute(attr.get_name(), attr.get_type()) {
                out.println(&format!(
                    "        {} = {}",
                    attr.get_name(),
                    java_object_to_string(possible_value.as_ref())
                ));
            }
        }
    }

    // Files:
    out.println("    Files");
    for target in package.get_targets().values() {
        match target {
            Target::InputFile(_) => {
                out.println(&format!(
                    "      {} {}",
                    target.get_target_kind(),
                    target.get_label()
                ));
            }
            Target::OutputFile(file) => {
                out.println(&format!(
                    "      {} {} (generated by {})",
                    target.get_target_kind(),
                    target.get_label(),
                    file.get_generating_rule().get_label()
                ));
            }
            _ => {}
        }
    }
}

/// `String.valueOf(possibleValue)`: Java's `toString` of an attribute value (`null`, `String` as
/// is, `List` as `[a, b]`, `Map` as `{k=v}`, `Label.toString`, `TriState` and `License` names).
fn java_object_to_string(value: Option<&blaze_packages::types::AttrValue>) -> String {
    use blaze_packages::types::AttrValue;
    fn list<T: std::fmt::Display>(items: impl Iterator<Item = T>) -> String {
        let items: Vec<String> = items.map(|i| i.to_string()).collect();
        format!("[{}]", items.join(", "))
    }
    fn dict<K: std::fmt::Display, V>(
        items: impl Iterator<Item = (K, V)>,
        value: impl Fn(V) -> String,
    ) -> String {
        let items: Vec<String> = items.map(|(k, v)| format!("{}={}", k, value(v))).collect();
        format!("{{{}}}", items.join(", "))
    }
    let Some(value) = value else {
        return "null".to_owned();
    };
    match value {
        AttrValue::Int(i) => i.to_string(),
        AttrValue::String(s) => s.clone(),
        AttrValue::Bool(b) => b.to_string(),
        AttrValue::Label(l) => l.to_string(),
        AttrValue::StringList(l) => list(l.iter()),
        AttrValue::StringSet(l) => list(l.iter()),
        AttrValue::IntList(l) => list(l.iter()),
        AttrValue::LabelList(l) => list(l.iter()),
        AttrValue::StringDict(d) => dict(d.iter(), |v| v.clone()),
        AttrValue::StringListDict(d) => dict(d.iter(), |v| list(v.iter())),
        AttrValue::LabelDictUnary(d) => dict(d.iter(), |v| v.to_string()),
        AttrValue::LabelKeyedStringDict(d) => dict(d.iter(), |v| v.clone()),
        AttrValue::LabelListDict(d) => dict(d.iter(), |v| list(v.iter())),
        AttrValue::License(l) => l.to_string(),
        // TriState.toString(): the enum constant's name.
        AttrValue::TriState(t) => match t {
            blaze_packages::types::TriState::Yes => "YES",
            blaze_packages::types::TriState::No => "NO",
            blaze_packages::types::TriState::Auto => "AUTO",
        }
        .to_owned(),
        other => blaze_util::unported!(
            "Package.dump",
            "the Java toString of the attribute value {other:?}"
        ),
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:443 DumpCommand.dumpActionCache
fn dump_action_cache(env: &CommandEnvironment, out: &mut PrintStream) -> bool {
    let reporter = env.get_reporter();
    // env.getBlazeWorkspace().getOrLoadPersistentActionCache(reporter).
    // PORT: no action cache stays loaded between commands; it is loaded from the output base.
    // Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:387 BlazeWorkspace.getOrLoadPersistentActionCache
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

/// `SkyKeyStats`: a rule class's (or aspect class's) count and action count.
// Java: com/google/devtools/build/lib/skyframe/SkyKeyStats.java:19 SkyKeyStats
// PORT: skipped, built with a struct literal (`count` and `action_count` start at 0): SkyKeyStats.<init>
// PORT: skipped, the fields are read directly in this file: SkyKeyStats.getKey, SkyKeyStats.getName, SkyKeyStats.getCount, SkyKeyStats.getActionCount
// PORT: skipped, no caller at the pin (getSkyframeStats counts with countWithActions): SkyKeyStats.count
struct SkyKeyStats {
    key: String,
    name: String,
    count: i64,
    action_count: i64,
}

impl SkyKeyStats {
    // Java: com/google/devtools/build/lib/skyframe/SkyKeyStats.java:34 SkyKeyStats.countWithActions
    fn count_with_actions(&mut self, action_count: i64) {
        self.count += 1;
        self.action_count += action_count;
    }
}

/// `ImmutableList.sortedCopyOf(SkyKeyStats.BY_COUNT_DESC, stats.values())` of a
/// `HashMap<String, SkyKeyStats>` filled by `computeIfAbsent` in the order of the graph's done
/// values: a stable sort by descending count of the map's iteration order.
///
/// PORT: the map's order is `java.util.HashMap`'s for the keys' `String.hashCode` (reproduced:
/// a table of the size the entries grew it to, bins in index order); within one bin it is the
/// insertion order, which follows the graph's `ConcurrentHashMap` (hash order of the SkyKeys,
/// not reproduced): entries of one bin are sorted by key. That order shows only when two entries
/// of one bin have the same count.
fn sorted_by_count_desc(stats: Vec<SkyKeyStats>) -> Vec<SkyKeyStats> {
    use blaze_util::profiler::java_hash_order::string_hash_code;
    let mut capacity = 16usize;
    while stats.len() > capacity * 3 / 4 {
        capacity *= 2;
    }
    // HashMap.hash: h ^ (h >>> 16), then the table index.
    let bin = |k: &str| {
        let h = string_hash_code(k);
        ((h ^ ((h as u32) >> 16) as i32) as u32 as usize) & (capacity - 1)
    };
    let mut ordered = stats;
    ordered.sort_by(|a, b| (bin(&a.key), &a.key).cmp(&(bin(&b.key), &b.key)));
    // Comparator.comparing(SkyKeyStats::getCount).reversed(): a stable sort.
    ordered.sort_by(|a, b| b.count.cmp(&a.count));
    ordered
}

/// `getSkyframeStats(eventHandler)`: the rule and aspect statistics of the graph's done values.
// Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:522 SequencedSkyframeExecutor.getSkyframeStats
fn get_skyframe_stats(evaluator: &Evaluator) -> (Vec<SkyKeyStats>, Vec<SkyKeyStats>) {
    use blaze_analysis::configured_target::ConfiguredTarget;
    use blaze_analysis::configured_target_value::ConfiguredTargetValue;
    let mut rule_stats: indexmap::IndexMap<String, SkyKeyStats> = indexmap::IndexMap::new();
    let mut aspect_stats: indexmap::IndexMap<String, SkyKeyStats> = indexmap::IndexMap::new();
    for (key, value) in evaluator.get_done_values() {
        if let Some(ct_value) = value.downcast_ref::<ConfiguredTargetValue>() {
            // value instanceof RuleConfiguredTargetValue: configuredTarget instanceof
            // RuleConfiguredTarget.
            if let ConfiguredTarget::Rule(_) = ct_value.get_configured_target() {
                let rule_class_id = match ct_value.get_target() {
                    Some(blaze_packages::target::Target::Rule(rule)) => {
                        rule.get_rule_class_object().get_rule_class_id().clone()
                    }
                    _ => blaze_util::unported!(
                        "RuleConfiguredTarget.getRuleClassId",
                        "a rule configured target value without its rule ({:?})",
                        key
                    ),
                };
                rule_stats
                    .entry(rule_class_id.key.clone())
                    .or_insert_with(|| SkyKeyStats {
                        key: rule_class_id.key.clone(),
                        name: rule_class_id.name.clone(),
                        count: 0,
                        action_count: 0,
                    })
                    .count_with_actions(ct_value.get_actions().len() as i64);
            }
        } else if key.function_name() == blaze_analysis::aspect_key_creator::ASPECT {
            let aspect_value = value
                .downcast_ref::<blaze_analysis::aspect_value::AspectValue>()
                .expect("ClassCastException: AspectValue");
            // PORT: Blaze's AspectValue is never cleared (no --discard_analysis_cache
            // clearing of aspects), so `isCleared()` is false.
            let aspect_class = aspect_value.get_aspect().get_aspect_class();
            let aspect_key = aspect_class.get_key();
            aspect_stats
                .entry(aspect_key.clone())
                .or_insert_with(|| SkyKeyStats {
                    key: aspect_key.clone(),
                    name: aspect_class.get_name().to_owned(),
                    count: 0,
                    action_count: 0,
                })
                .count_with_actions(aspect_value.get_actions().len() as i64);
        }
    }
    (
        sorted_by_count_desc(rule_stats.into_values().collect()),
        sorted_by_count_desc(aspect_stats.into_values().collect()),
    )
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:480 DumpCommand.dumpRuleStats
fn dump_rule_stats(evaluator: &Evaluator, out: &mut PrintStream) {
    let (rules, aspects) = get_skyframe_stats(evaluator);
    if rules.is_empty() {
        out.print("No rules in Bazel server, please run a build command first.");
        return;
    }
    // allocationTracker == null: no BYTES/EACH columns.
    print_rule_stats_of_type(&rules, "RULE", out, false);
    print_rule_stats_of_type(&aspects, "ASPECT", out, true);
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:503 DumpCommand.printRuleStatsOfType
fn print_rule_stats_of_type(
    rule_stats: &[SkyKeyStats],
    type_: &str,
    out: &mut PrintStream,
    trim_key: bool,
) {
    if rule_stats.is_empty() {
        return;
    }
    // ruleStats are already sorted. Lengths are String.length() (UTF-16 units).
    let utf16_len = |s: &str| s.encode_utf16().count();
    let longest_name = rule_stats
        .iter()
        .map(|r| utf16_len(&r.name))
        .max()
        .expect("non-empty");
    let max_name_width = 30;
    let name_column_width = longest_name.min(max_name_width);
    let number_column_width = 10;
    print_with_padding(out, type_, name_column_width);
    print_with_padding_before(out, "COUNT", number_column_width);
    print_with_padding_before(out, "ACTIONS", number_column_width);
    out.println("");
    for rule_stat in rule_stats {
        print_with_padding(
            out,
            &truncate_name(&rule_stat.name, trim_key, max_name_width),
            name_column_width,
        );
        print_with_padding_before(out, &format_long(rule_stat.count), number_column_width);
        print_with_padding_before(
            out,
            &format_long(rule_stat.action_count),
            number_column_width,
        );
        out.println("");
    }
    out.println("");
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:545 DumpCommand.truncateName
fn truncate_name(name: &str, trim_key: bool, max_name_width: usize) -> String {
    let mut name: Vec<u16> = name.encode_utf16().collect();
    // If this is an aspect, we'll chop off everything except the aspect name
    if trim_key && let Some(divider_index) = name.iter().rposition(|&c| c == u16::from(b'%')) {
        name = name[divider_index + 1..].to_vec();
    }
    if name.len() <= max_name_width {
        return String::from_utf16_lossy(&name);
    }
    let starti = name.len() - max_name_width + "...".len();
    format!("...{}", String::from_utf16_lossy(&name[starti..]))
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:560 DumpCommand.printWithPadding
fn print_with_padding(out: &mut PrintStream, str: &str, column_width: usize) {
    out.print(str);
    pad(out, column_width + 2, str.encode_utf16().count());
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:565 DumpCommand.printWithPaddingBefore
fn print_with_padding_before(out: &mut PrintStream, str: &str, column_width: usize) {
    pad(out, column_width, str.encode_utf16().count());
    out.print(str);
    pad(out, 2, 0);
}

// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:571 DumpCommand.pad
fn pad(out: &mut PrintStream, column_width: usize, consumed: usize) {
    for _ in consumed..column_width {
        out.print(" ");
    }
}

/// `String.format("%,d", number)` in the server's default locale.
///
/// The grouping separator `,` (root and English locales), as the other ports of `%,d`
/// (`blaze_repo_fetch::downloader::download_progress_event::format_grouped`).
// Java: com/google/devtools/build/lib/runtime/commands/DumpCommand.java:577 DumpCommand.formatLong
fn format_long(number: i64) -> String {
    let digits = number.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if number < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
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

// The graph dumps are blaze_skyframe::dump's (Java: AbstractInMemoryMemoizingEvaluator's), which
// also documents the order they list nodes in.

fn dump_summary(evaluator: &Evaluator, out: &mut PrintStream) {
    out.print(&blaze_skyframe::dump::dump_summary(evaluator));
}

fn dump_count(evaluator: &Evaluator, out: &mut PrintStream) {
    out.print(&blaze_skyframe::dump::dump_count(evaluator));
}

fn dump_values(evaluator: &Evaluator, out: &mut PrintStream, filter: &RegexFilter) {
    out.print(&blaze_skyframe::dump::dump_values(evaluator, &|n| {
        filter.test(n)
    }));
}

fn dump_deps(evaluator: &Evaluator, out: &mut PrintStream, filter: &RegexFilter) {
    out.print(&blaze_skyframe::dump::dump_deps(evaluator, &|n| {
        filter.test(n)
    }));
}

fn dump_function_graph(evaluator: &Evaluator, out: &mut PrintStream, filter: &RegexFilter) {
    out.print(&blaze_skyframe::dump::dump_function_graph(
        evaluator,
        &|n| filter.test(n),
    ));
}

fn dump_rdeps(evaluator: &Evaluator, out: &mut PrintStream, filter: &RegexFilter) {
    out.print(&blaze_skyframe::dump::dump_rdeps(evaluator, &|n| {
        filter.test(n)
    }));
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

// The tests (DumpCommandTest) are in tests/dump_command_test.rs: they run several commands on one
// runtime (blaze_buildtool_testing::BuildIntegrationTestCase).
