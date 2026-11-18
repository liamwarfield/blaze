//! Port of `query2/aquery/ActionGraphTextOutputFormatterCallback.java` (`--output=text`, the
//! default of aquery, and `--output=commands`).
//!
//! PORT: `processOutput` and `writeAction` are ported: the walk over the actions of the result,
//! the contents of parameter files and the action filters. The text of one action (`writeText`,
//! `writeCommand`) is not: the first action that passes the filters fails loudly there. A result
//! without actions (source files, filegroups, incompatible targets) prints nothing, as in Java.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_actions::skyframe::actiongraph::v2::action_graph_dump::{
    ActionGraphDumpError, matches_aquery_filters,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, InterruptedException, OutputFormatterCallback, QueryError,
    ThreadSafeOutputFormatterCallback,
};

use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, KeyedConfiguredTargetValue,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:62 ActionGraphTextOutputFormatterCallback.OutputType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputType {
    TEXT,
    COMMANDS,
}

impl OutputType {
    /// `formatName`.
    pub fn format_name(self) -> &'static str {
        match self {
            OutputType::TEXT => "text",
            OutputType::COMMANDS => "commands",
        }
    }
}

/// Output callback for aquery, prints human readable output.
// Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:61 ActionGraphTextOutputFormatterCallback
pub struct ActionGraphTextOutputFormatterCallback<'a> {
    // AqueryThreadsafeCallback
    // Java: com/google/devtools/build/lib/query2/aquery/AqueryThreadsafeCallback.java:24 AqueryThreadsafeCallback
    #[allow(dead_code)]
    event_handler: Arc<dyn ExtendedEventHandler>,
    options: &'a AqueryOptions,
    /// `printStream`: read by `writeAction` once `writeText` and `writeCommand` are ported.
    #[allow(dead_code)]
    print_stream: OutputStream,
    accessor: &'a ActionGraphQueryEnvironment,
    // ActionGraphTextOutputFormatterCallback
    output_type: OutputType,
    action_filters: &'a AqueryActionFilter,
    /// `paramFileNameToContentMap` (lazily created in Java).
    param_file_name_to_content_map: RefCell<HashMap<String, String>>,
    io_exception: IoExceptionSlot,
}

impl<'a> ActionGraphTextOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:79 ActionGraphTextOutputFormatterCallback.<init>
    pub fn new(
        event_handler: Arc<dyn ExtendedEventHandler>,
        options: &'a AqueryOptions,
        out: OutputStream,
        accessor: &'a ActionGraphQueryEnvironment,
        output_type: OutputType,
        action_filters: &'a AqueryActionFilter,
    ) -> Self {
        // PORT: the LabelPrinter argument is read only by `writeText`.
        ActionGraphTextOutputFormatterCallback {
            event_handler,
            options,
            print_stream: out,
            accessor,
            output_type,
            action_filters,
            param_file_name_to_content_map: RefCell::new(HashMap::new()),
            io_exception: IoExceptionSlot::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:129 ActionGraphTextOutputFormatterCallback.writeAction
    fn write_action(&self, action: &dyn Action) -> Result<(), ActionGraphDumpError> {
        if self.options.include_param_files {
            // `action instanceof ParameterFileWriteAction parameterFileWriteAction`
            if let Some(arguments) = action.aquery_parameter_file_arguments() {
                let file_content = arguments?.join(" \\\n    ");
                let param_file_name = action
                    .get_primary_output()
                    .get_exec_path_string()
                    .to_string();

                self.param_file_name_to_content_map
                    .borrow_mut()
                    .insert(param_file_name, file_content);
            }
        }

        if !matches_aquery_filters(
            action,
            self.action_filters,
            self.options.include_pruned_inputs,
        ) {
            return Ok(());
        }

        match self.output_type {
            OutputType::TEXT => blaze_util::unported!(
                "ActionGraphTextOutputFormatterCallback.writeText",
                "aquery --output=text of a {} action (use --output=jsonproto)",
                action.get_mnemonic()
            ),
            OutputType::COMMANDS => blaze_util::unported!(
                "ActionGraphTextOutputFormatterCallback.writeCommand",
                "aquery --output=commands of a {} action (use --output=jsonproto)",
                action.get_mnemonic()
            ),
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:93 ActionGraphTextOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        self.output_type.format_name().to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
}

impl Callback<KeyedConfiguredTargetValue> for ActionGraphTextOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[KeyedConfiguredTargetValue]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<KeyedConfiguredTargetValue>
    for ActionGraphTextOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:98 ActionGraphTextOutputFormatterCallback.processOutput
    fn process_output(
        &self,
        partial_result: &[KeyedConfiguredTargetValue],
    ) -> Result<(), OutputFormatterError> {
        // Enabling includeParamFiles should enable includeCommandline by default.
        // PORT: `options.includeCommandline |= options.includeParamFiles` is read only by
        // `writeText`, which is not ported.

        for configured_target_value in partial_result {
            if !matches!(
                configured_target_value.get_configured_target(),
                ConfiguredTarget::Rule(_)
            ) {
                // We have to include non-rule values in the graph to visit their dependencies,
                // but they don't have any actions to print out.
                continue;
            }
            let actions: Vec<Arc<dyn Action>> = configured_target_value.get_actions().to_vec();
            for action in &actions {
                match self.write_action(&**action) {
                    Ok(()) => {}
                    Err(ActionGraphDumpError::Interrupted) => {
                        return Err(OutputFormatterError::Interrupted(
                            InterruptedException::new(),
                        ));
                    }
                    Err(ActionGraphDumpError::Io(e)) => return Err(e.into()),
                    // catch (CommandLineExpansionException | EvalException e) {
                    //   throw new IOException(e.getMessage()); }
                    Err(e) => return Err(std::io::Error::other(e.to_string()).into()),
                }
            }
            if self.options.use_aspects {
                // Java: writeAction(action, printStream) for the actions of each aspect value.
                #[allow(clippy::never_loop)]
                // the aspect value type is uninhabited until aspects are ported
                for aspect_value in self.accessor.get_aspect_values(configured_target_value) {
                    match aspect_value {}
                }
            }
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The format names of the Java enum (`TEXT("text")`, `COMMANDS("commands")`), which
    /// `getName` returns and `--output` selects.
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphTextOutputFormatterCallback.java:62 ActionGraphTextOutputFormatterCallback.OutputType
    #[test]
    fn output_type_format_names() {
        assert_eq!(OutputType::TEXT.format_name(), "text");
        assert_eq!(OutputType::COMMANDS.format_name(), "commands");
    }
}
