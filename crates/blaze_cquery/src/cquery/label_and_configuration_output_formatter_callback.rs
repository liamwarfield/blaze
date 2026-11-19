//! Port of `query2/cquery/LabelAndConfigurationOutputFormatterCallback.java`.

use blaze_optdefs::fragment::core_options::CoreOptionsIncludeConfigFragmentsEnum;
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::cquery::cquery_threadsafe_callback::{CqueryThreadsafeCallback, short_id};
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// Default Output callback for cquery. Prints a label and configuration pair per result.
// Java: com/google/devtools/build/lib/query2/cquery/LabelAndConfigurationOutputFormatterCallback.java:31 LabelAndConfigurationOutputFormatterCallback
pub struct LabelAndConfigurationOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    show_kind: bool,
    label_printer: LabelPrinter,
}

impl<'a> LabelAndConfigurationOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/LabelAndConfigurationOutputFormatterCallback.java:35 LabelAndConfigurationOutputFormatterCallback.<init>
    pub fn new(
        base: CqueryThreadsafeCallback<'a>,
        show_kind: bool,
        label_printer: LabelPrinter,
    ) -> Self {
        LabelAndConfigurationOutputFormatterCallback {
            base,
            show_kind,
            label_printer,
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode>
    for LabelAndConfigurationOutputFormatterCallback<'_>
{
    // Java: com/google/devtools/build/lib/query2/cquery/LabelAndConfigurationOutputFormatterCallback.java:49 LabelAndConfigurationOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        if self.show_kind {
            "label_kind"
        } else {
            "label"
        }
        .to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode>
    for LabelAndConfigurationOutputFormatterCallback<'_>
{
}

impl Callback<CqueryNode> for LabelAndConfigurationOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

/// The required configuration fragments of a node, as `ImmutableSortedSet.toString()`.
// Java: com/google/devtools/build/lib/query2/cquery/LabelAndConfigurationOutputFormatterCallback.java:77 LabelAndConfigurationOutputFormatterCallback.requiredFragmentStrings
fn required_fragment_strings(keyed_configured_target: &CqueryNode) -> String {
    if keyed_configured_target.as_configured_target().is_none() {
        // ImmutableSortedSet.of()
        return "[]".to_owned();
    }
    blaze_util::unported!(
        "LabelAndConfigurationOutputFormatterCallback.requiredFragmentStrings",
        "RequiredConfigFragmentsProvider"
    )
}

impl OutputFormatterCallback<CqueryNode> for LabelAndConfigurationOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/LabelAndConfigurationOutputFormatterCallback.java:54 LabelAndConfigurationOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        for keyed_configured_target in partial_result {
            let mut output = String::new();
            if self.show_kind {
                let actual_target = self
                    .base
                    .accessor
                    .get_accessor_target(keyed_configured_target);
                output.push_str(&actual_target.get_target_kind());
                output.push(' ');
            }
            output.push_str(&keyed_configured_target.get_description(&self.label_printer));
            output.push_str(" (");
            output.push_str(&short_id(
                self.base
                    .get_configuration(keyed_configured_target.get_configuration_key().as_ref())
                    .as_ref(),
            ));
            output.push(')');

            if self.base.options.show_required_config_fragments
                != CoreOptionsIncludeConfigFragmentsEnum::OFF
            {
                output.push(' ');
                output.push_str(&required_fragment_strings(keyed_configured_target));
            }

            self.base.add_result(output);
        }
        Ok(())
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}
