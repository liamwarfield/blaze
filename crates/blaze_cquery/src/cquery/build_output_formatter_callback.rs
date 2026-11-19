//! Port of `query2/cquery/BuildOutputFormatterCallback.java`.

use blaze_packages::attribute::{Attribute, AttributeMap};
use blaze_packages::target::{LabelPrinter, Rule};
use blaze_packages::types::AttrValue;
use blaze_query::query::output::build_output_formatter::TargetOutputter;
use blaze_query::query::output::output_formatter::TextWriter;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};

use blaze_analysis::config::configured_attribute_mapper::ConfiguredAttributeMapper;
use blaze_analysis::configured_target::ConfiguredTarget;

use crate::cquery::configured_target_query_environment::{
    CqueryNode, configured_attribute_mapper, get_config_conditions,
};
use crate::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// Cquery implementation of BUILD-style output.
// Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:40 BuildOutputFormatterCallback
pub struct BuildOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    label_printer: LabelPrinter,
    /// Java builds a `TargetOutputter` per `processOutput` call over `printStream`.
    writer_out: blaze_query::query::output::output_formatter::OutputStream,
    pending: std::cell::RefCell<Vec<TextWriter>>,
}

impl<'a> BuildOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:44 BuildOutputFormatterCallback.<init>
    pub fn new(base: CqueryThreadsafeCallback<'a>, label_printer: LabelPrinter) -> Self {
        let writer_out = base.out.clone().expect("cquery output stream");
        BuildOutputFormatterCallback {
            base,
            label_printer,
            writer_out,
            pending: std::cell::RefCell::new(Vec::new()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:77 BuildOutputFormatterCallback.getAttributeMap
    fn get_attribute_map(&self, kct: &CqueryNode) -> Option<ConfiguredAttributeMapper> {
        let target = self.base.accessor.get_accessor_target(kct);
        let associated_rule = target.get_associated_rule()?;
        let checksum = kct
            .get_configuration_key()
            .map(|k| k.get_options().checksum().to_owned());
        if let Some(ConfiguredTarget::OutputFile(output_file)) = kct.as_configured_target() {
            // accessor.getGeneratingConfiguredTarget(kct).getConfigConditions()
            Some(configured_attribute_mapper(
                associated_rule,
                output_file
                    .get_generating_rule()
                    .get_config_conditions()
                    .clone(),
                checksum,
                /* always_succeed= */ false,
            ))
        } else {
            Some(configured_attribute_mapper(
                associated_rule,
                get_config_conditions(kct),
                checksum,
                /* always_succeed= */ false,
            ))
        }
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for BuildOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:55 BuildOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "build".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for BuildOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for BuildOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for BuildOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:96 BuildOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        let outputter = TargetOutputter::new(
            TextWriter::new(self.writer_out.clone()),
            // This tells TargetOutputter which attributes to print as selects without resolving
            // those selects. For cquery we never have to do this since we can always resolve
            // selects. Going forward we could expand this to show both the complete select
            // and which path is chosen, which people may find even more informative.
            Box::new(|_rule: &Rule, _attr: &Attribute| false),
            "\n",
            self.label_printer.clone(),
        );
        for configured_target in partial_result {
            let target = self.base.accessor.get_accessor_target(configured_target);
            let attribute_map = self.get_attribute_map(configured_target);
            // Java: com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallback.java:66 CqueryAttributeReader.getPossibleValues
            let reader = |_rule: &Rule, attr: &Attribute| -> Vec<Option<AttrValue>> {
                // Cquery knows which select path is taken so it knows the exact value the
                // attribute takes. Note that null values are also possible - these are
                // represented as an empty value list.
                let attribute_map = attribute_map
                    .as_ref()
                    .expect("NullPointerException: attributeMap");
                match attribute_map.get(attr.get_name(), attr.get_type()) {
                    None => Vec::new(),
                    Some(actual_value) => vec![Some(actual_value)],
                }
            };
            outputter.output(&target, &reader);
        }
        // The outputter writes to `printStream`, which `close` flushes.
        self.pending.borrow_mut().push(outputter.writer);
        Ok(())
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        for writer in self.pending.borrow_mut().drain(..) {
            writer.flush()?;
        }
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}
