//! Port of `query2/query/output/LabelOutputFormatter.java`.

use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, SynchronizedDelegatingOutputFormatterCallback,
    ThreadSafeOutputFormatterCallback,
};
use blaze_vfs::DigestHashFunction;

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::output::output_formatter::{
    OutputFormatter, OutputStream, StreamedFormatter, TextWriter,
};
use crate::query::output::query_options::get_line_terminator;

/// An output formatter that prints the labels of the resulting target set in topological order,
/// optionally with the target's kind.
// Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:32 LabelOutputFormatter
pub struct LabelOutputFormatter {
    show_kind: bool,
}

impl LabelOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:36 LabelOutputFormatter.<init>
    pub fn new(show_kind: bool) -> Self {
        LabelOutputFormatter { show_kind }
    }
}

impl OutputFormatter for LabelOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:40 LabelOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        if self.show_kind {
            "label_kind"
        } else {
            "label"
        }
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for LabelOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/AbstractUnorderedFormatter.java:35 AbstractUnorderedFormatter.setOptions
    fn set_options(
        &self,
        _options: &QueryOptions,
        _aspect_resolver: Arc<dyn AspectResolver>,
        _hash_function: DigestHashFunction,
    ) {
    }

    // Java: com/google/devtools/build/lib/query2/query/output/AbstractUnorderedFormatter.java:39 AbstractUnorderedFormatter.setEventHandler
    fn set_event_handler(&self, _event_handler: Option<Arc<dyn EventHandler>>) {}

    // Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:64 LabelOutputFormatter.createStreamCallback
    fn create_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        env: &dyn AbstractBlazeQueryEnvironment,
    ) -> Box<dyn ThreadSafeOutputFormatterCallback<Target>> {
        Box::new(SynchronizedDelegatingOutputFormatterCallback::new(
            self.create_post_facto_stream_callback(out, options, env.get_label_printer()),
        ))
    }

    // Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:45 LabelOutputFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        Box::new(LabelCallback {
            writer: TextWriter::new(out),
            show_kind: self.show_kind,
            line_term: get_line_terminator(options),
            label_printer: label_printer.clone(),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

struct LabelCallback {
    writer: TextWriter,
    show_kind: bool,
    line_term: &'static str,
    label_printer: LabelPrinter,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for LabelCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for LabelCallback {
    // Java: com/google/devtools/build/lib/query2/query/output/LabelOutputFormatter.java:49 LabelOutputFormatter.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            if self.show_kind {
                self.writer.append(&target.get_target_kind());
                self.writer.append(" ");
            }
            let label = target.get_label();
            self.writer
                .append(&self.label_printer.to_string(label))
                .append(self.line_term);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/TextOutputFormatterCallback.java:36 TextOutputFormatterCallback.close
    fn close(&self, _fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.writer.flush()?;
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
