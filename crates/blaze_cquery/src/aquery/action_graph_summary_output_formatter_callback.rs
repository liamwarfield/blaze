//! Port of `query2/aquery/ActionGraphSummaryOutputFormatterCallback.java` (`--output=summary`).

use std::cell::RefCell;
use std::io::Write;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::matches_aquery_filters;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::java_concurrent_hash_map::java_hash_map_order;
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, KeyedConfiguredTargetValue,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// A `java.util.HashMap<String, Integer>` filled with `merge(key, 1, Integer::sum)`: the keys in
/// the order they were first put, with their counts.
#[derive(Default)]
struct CountMap {
    entries: Vec<(String, i32)>,
}

impl CountMap {
    /// `map.merge(key, 1, Integer::sum)`.
    fn merge(&mut self, key: String) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, count)) => *count += 1,
            None => self.entries.push((key, 1)),
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `map.entrySet()` in the iteration order of Java's `HashMap`.
    ///
    /// PORT: java-hash-order. A key's place in the table depends only on the keys (Java
    /// `String.hashCode()` over the internal form) and the order they were first put; `merge` of
    /// a present key changes the value in place.
    fn entry_set(&self) -> Vec<(String, i32)> {
        java_hash_map_order(self.entries.clone())
    }
}

/// Output callback for aquery, prints a human readable summary.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:35 ActionGraphSummaryOutputFormatterCallback
pub struct ActionGraphSummaryOutputFormatterCallback<'a> {
    // AqueryThreadsafeCallback
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryThreadsafeCallback.java:24 AqueryThreadsafeCallback
    #[allow(dead_code)]
    event_handler: Arc<dyn ExtendedEventHandler>,
    options: &'a AqueryOptions,
    print_stream: Option<OutputStream>,
    accessor: &'a ActionGraphQueryEnvironment,
    // ActionGraphSummaryOutputFormatterCallback
    action_filters: &'a AqueryActionFilter,
    mnemonic_to_count: RefCell<CountMap>,
    configuration_to_count: RefCell<CountMap>,
    exec_platform_to_count: RefCell<CountMap>,
    aspect_to_count: RefCell<CountMap>,
    io_exception: IoExceptionSlot,
}

impl<'a> ActionGraphSummaryOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:43 ActionGraphSummaryOutputFormatterCallback.<init>
    pub fn new(
        event_handler: Arc<dyn ExtendedEventHandler>,
        options: &'a AqueryOptions,
        out: Option<OutputStream>,
        accessor: &'a ActionGraphQueryEnvironment,
        action_filters: &'a AqueryActionFilter,
    ) -> Self {
        ActionGraphSummaryOutputFormatterCallback {
            event_handler,
            options,
            print_stream: out,
            accessor,
            action_filters,
            mnemonic_to_count: RefCell::new(CountMap::default()),
            configuration_to_count: RefCell::new(CountMap::default()),
            exec_platform_to_count: RefCell::new(CountMap::default()),
            aspect_to_count: RefCell::new(CountMap::default()),
            io_exception: IoExceptionSlot::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:83 ActionGraphSummaryOutputFormatterCallback.processAction
    fn process_action(&self, action: &dyn Action) {
        if !matches_aquery_filters(
            action,
            self.action_filters,
            self.options.include_pruned_inputs,
        ) {
            return;
        }

        self.mnemonic_to_count
            .borrow_mut()
            .merge(action.get_mnemonic().to_string());
        // PORT: Rust actions always have an owner (Java: `if (actionOwner != null)`).
        let action_owner = action.get_owner();
        // BuildEvent configuration = actionOwner.getBuildConfigurationEvent();
        // configuration.asStreamProto(null).getConfiguration().getMnemonic()
        let configuration = action_owner
            .get_build_configuration_event()
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: no build configuration event for the owner of {}",
                    action.get_mnemonic()
                )
            });
        self.configuration_to_count
            .borrow_mut()
            .merge(configuration.configuration.mnemonic.clone());

        if let Some(execution_platform) = action_owner.get_execution_platform() {
            self.exec_platform_to_count
                .borrow_mut()
                .merge(execution_platform.label().to_string());
        }

        // In the case of aspect-on-aspect, AspectDescriptors are listed in
        // topological order of the dependency graph.
        // e.g. [A -> B] would imply that aspect A is applied on top of aspect B.
        for aspect_descriptor in action_owner.get_aspect_descriptors().iter().rev() {
            self.aspect_to_count
                .borrow_mut()
                .merge(aspect_descriptor.aspect_class_name.clone());
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:134 ActionGraphSummaryOutputFormatterCallback.printSummary
    fn print_summary(
        &self,
        print_stream: &mut dyn Write,
        actions_count: &CountMap,
        s: &str,
    ) -> std::io::Result<()> {
        if !actions_count.is_empty() {
            writeln!(print_stream)?;
            writeln!(print_stream, "{s}")?;
            // actionsCount.entrySet().stream().sorted(Comparator.comparingInt(Entry::getValue)):
            // a stable sort of the HashMap's iteration order.
            let mut entries = actions_count.entry_set();
            entries.sort_by_key(|(_, count)| *count);
            for (key, count) in entries {
                writeln!(print_stream, "  {key}: {count}")?;
            }
        }
        Ok(())
    }
}

impl NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphSummaryOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:53 ActionGraphSummaryOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "summary".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphSummaryOutputFormatterCallback<'_>
{
}

impl Callback<KeyedConfiguredTargetValue> for ActionGraphSummaryOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[KeyedConfiguredTargetValue]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphSummaryOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:58 ActionGraphSummaryOutputFormatterCallback.processOutput
    fn process_output(
        &self,
        partial_result: &[KeyedConfiguredTargetValue],
    ) -> Result<(), OutputFormatterError> {
        // Enabling includeParamFiles should enable includeCommandline by default.
        // PORT: `options.includeCommandline |= options.includeParamFiles`: this callback does
        // not read includeCommandline.

        for configured_target_value in partial_result {
            // PORT: `configuredTargetValue instanceof RuleConfiguredTargetValue`, tested on the
            // configured target as the text and proto callbacks do.
            if !matches!(
                configured_target_value.get_configured_target(),
                ConfiguredTarget::Rule(_)
            ) {
                // We have to include non-rule values in the graph to visit their dependencies,
                // but they don't have any actions to print out.
                continue;
            }
            for action in configured_target_value.get_actions() {
                self.process_action(&**action);
            }
            if self.options.use_aspects {
                for aspect_value in self.accessor.get_aspect_values(configured_target_value) {
                    // aspectValue.getActions(): a null AspectValue throws.
                    let aspect_value = aspect_value.unwrap_or_else(|| {
                        panic!("NullPointerException: AspectValue missing from the graph")
                    });
                    for action in aspect_value.get_actions() {
                        self.process_action(&**action);
                    }
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphSummaryOutputFormatterCallback.java:114 ActionGraphSummaryOutputFormatterCallback.close
    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        if fail_fast {
            return Ok(());
        }
        let Some(print_stream) = &self.print_stream else {
            return Ok(());
        };
        let mut print_stream = print_stream.borrow_mut();
        let print_stream: &mut dyn Write = &mut *print_stream;

        let total_actions: i32 = self
            .mnemonic_to_count
            .borrow()
            .entries
            .iter()
            .map(|(_, v)| *v)
            .sum();
        if total_actions == 0 {
            writeln!(print_stream, "No actions matched.")?;
        } else {
            writeln!(
                print_stream,
                "{} total action{}.",
                total_actions,
                if total_actions == 1 { "" } else { "s" }
            )?;
        }

        self.print_summary(print_stream, &self.mnemonic_to_count.borrow(), "Mnemonics:")?;
        self.print_summary(
            print_stream,
            &self.configuration_to_count.borrow(),
            "Configurations:",
        )?;
        self.print_summary(
            print_stream,
            &self.exec_platform_to_count.borrow(),
            "Execution Platforms:",
        )?;
        self.print_summary(print_stream, &self.aspect_to_count.borrow(), "Aspects:")?;
        print_stream.flush()?;
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HashMap<String, Integer>` iteration then a stable sort by count: for equal counts, the
    /// order of the strings' Java hash codes in a 16-slot table.
    #[test]
    fn print_summary_order_is_java_hash_map_order_then_count() {
        // The hash code of a one-character key is the character: "b" = 98 -> bin 98 & 15 = 2,
        // "q" = 113 -> bin 1, "a" = 97 -> bin 1 (put after "q", so after it in the bin).
        // Iteration: q, a, b. Counts: b = 1, q = 2, a = 1; the stable sort by count keeps a
        // before b.
        let mut map = CountMap::default();
        for key in ["b", "q", "a", "q"] {
            map.merge(key.to_owned());
        }
        assert_eq!(
            map.entry_set(),
            vec![
                ("q".to_owned(), 2),
                ("a".to_owned(), 1),
                ("b".to_owned(), 1)
            ]
        );
        let mut out: Vec<u8> = Vec::new();
        let mut entries = map.entry_set();
        entries.sort_by_key(|(_, c)| *c);
        for (k, c) in entries {
            writeln!(out, "  {k}: {c}").unwrap();
        }
        assert_eq!(String::from_utf8(out).unwrap(), "  a: 1\n  b: 1\n  q: 2\n");
    }
}
