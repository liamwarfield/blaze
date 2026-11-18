//! Port of `query2/query/output/LocationOutputFormatter.java`.

use std::cell::Cell;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::QueryOptions;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::query;
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, ContainsFunctionQueryExpressionVisitor, OutputFormatterCallback, QueryError,
    QueryException, QueryExpression, SynchronizedDelegatingOutputFormatterCallback,
    ThreadSafeOutputFormatterCallback,
};
use blaze_vfs::DigestHashFunction;

use crate::common::abstract_blaze_query_environment::AbstractBlazeQueryEnvironment;
use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::output::format_utils;
use crate::query::output::output_formatter::{
    OutputFormatter, OutputStream, StreamedFormatter, TextWriter,
};
use crate::query::output::query_options::get_line_terminator;

/// An output formatter that prints the labels of the targets, preceded by their locations and
/// kinds, in topological order. For output files, the location of the generating rule is given;
/// for input files, the location of line 1 is given.
// Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:41 LocationOutputFormatter
#[derive(Default)]
pub struct LocationOutputFormatter {
    relative_locations: Cell<bool>,
}

impl OutputFormatter for LocationOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:46 LocationOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "location"
    }

    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:58 LocationOutputFormatter.verifyCompatible
    fn verify_compatible(
        &self,
        _env: &dyn AbstractBlazeQueryEnvironment,
        expr: &QueryExpression,
    ) -> Result<(), QueryException> {
        // env instanceof AbstractBlazeQueryEnvironment: always true here.
        let mut note_build_files_and_load_liles_visitor =
            ContainsFunctionQueryExpressionVisitor::new([
                "loadfiles".to_owned(),
                "buildfiles".to_owned(),
            ]);
        if expr.accept(&mut note_build_files_and_load_liles_visitor, &()) {
            return Err(QueryException::new(
                None,
                "Query expressions involving 'buildfiles' or 'loadfiles' cannot be used with \
                 --output=location",
                query::Code::BuildfilesAndLoadfilesCannotUseOutputLocationError,
            ));
        }
        Ok(())
    }

    fn as_streamed_formatter(&self) -> Option<&dyn StreamedFormatter> {
        Some(self)
    }
}

impl StreamedFormatter for LocationOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:51 LocationOutputFormatter.setOptions
    fn set_options(
        &self,
        options: &QueryOptions,
        _aspect_resolver: Arc<dyn AspectResolver>,
        _hash_function: DigestHashFunction,
    ) {
        self.relative_locations.set(options.relative_locations);
    }

    fn set_event_handler(&self, _event_handler: Option<Arc<dyn EventHandler>>) {}

    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:97 LocationOutputFormatter.createStreamCallback
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

    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:76 LocationOutputFormatter.createPostFactoStreamCallback
    fn create_post_facto_stream_callback(
        &self,
        out: OutputStream,
        options: &QueryOptions,
        label_printer: &LabelPrinter,
    ) -> Box<dyn OutputFormatterCallback<Target>> {
        Box::new(LocationCallback {
            writer: TextWriter::new(out),
            relative_locations: self.relative_locations.get(),
            line_term: get_line_terminator(options),
            label_printer: label_printer.clone(),
            io_exception: IoExceptionSlot::new(),
        })
    }
}

struct LocationCallback {
    writer: TextWriter,
    relative_locations: bool,
    line_term: &'static str,
    label_printer: LabelPrinter,
    io_exception: IoExceptionSlot,
}

impl Callback<Target> for LocationCallback {
    fn process(&self, partial_result: &[Target]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<Target> for LocationCallback {
    // Java: com/google/devtools/build/lib/query2/query/output/LocationOutputFormatter.java:81 LocationOutputFormatter.processOutput
    fn process_output(&self, partial_result: &[Target]) -> Result<(), OutputFormatterError> {
        for target in partial_result {
            self.writer
                .append(&format_utils::get_location(target, self.relative_locations))
                .append(": ")
                .append(&target.get_target_kind())
                .append(" ")
                .append(&self.label_printer.to_string(target.get_label()))
                .append(self.line_term);
        }
        Ok(())
    }

    fn close(&self, _fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.writer.flush()?;
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}
